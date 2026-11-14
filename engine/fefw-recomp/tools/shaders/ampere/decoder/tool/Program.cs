// SPDX-License-Identifier: MIT
// sassdecode: decode Ampere SM86 code blocks with SassDecoder.
//   --verify  <oracle dir> <corpus dir> [--every N] [--show OPC] [--examples K] [--jobs 4]
//             decodes every instruction of every block listed in <oracle>/index.tsv from the corpus code and compares
//             ToSassText() with the nvdisasm oracle line (whitespace normalised only). Prints match %, weighted by occurrence,
//             per-mnemonic gaps and example mismatches.
//   --corpus  <corpus dir> [--jobs 4]   decode the whole corpus (every unique block's program range), counts only.
//   --dump    <code.bin> <ctrl.bin>      print one block.
using System;
using System.Collections.Concurrent;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;
using System.Threading.Tasks;
using Nx.Shaders.Ampere;

static class Program
{
    static readonly Regex Ws = new(@"\s+", RegexOptions.Compiled);
    static readonly Regex Line = new(@"^/\*([0-9a-f]+)\*/ (.*) ; (0x[0-9a-f]{16}) (0x[0-9a-f]{16})$", RegexOptions.Compiled);

    static int Main(string[] args)
    {
        try { Process.GetCurrentProcess().PriorityClass = ProcessPriorityClass.BelowNormal; } catch { }
        if (args.Length >= 3 && args[0] == "--verify") return Verify(args[1], args[2], args.Skip(3).ToArray());
        if (args.Length >= 2 && args[0] == "--corpus") return WholeCorpus(args[1], args.Skip(2).ToArray());
        if (args.Length >= 3 && args[0] == "--dump") return Dump(args[1], args[2]);
        Console.Error.WriteLine("usage: sassdecode --verify <oracle dir> <corpus dir> [--every N] [--show OPC] [--examples K] [--jobs N]\n" +
                                "       sassdecode --corpus <corpus dir> [--jobs N]\n       sassdecode --dump <code.bin> <ctrl.bin>");
        return 2;
    }

    static string Opt(string[] a, string name, string def)
    {
        int i = Array.IndexOf(a, name);
        return i >= 0 && i + 1 < a.Length ? a[i + 1] : def;
    }

    /// program range per docs/AMPERE.md: stage u16 ctrl+0x56 (5 = compute, no SPH), end u32 ctrl+0x38, trailing zero words dropped.
    static (int start, int end) Program_(byte[] code, byte[] ctrl)
    {
        int stage = BitConverter.ToUInt16(ctrl, 0x56);
        int end = Math.Min((int)BitConverter.ToUInt32(ctrl, 0x38), code.Length);
        int start = stage == 5 ? 0 : 0x80;
        while (end - 16 >= start && code.AsSpan(end - 16, 16).IndexOfAnyExcept((byte)0) < 0) end -= 16;
        return (start, end);
    }

    static string Norm(string s) => Ws.Replace(s, " ").Trim();

    static string BaseOp(string text)
    {
        string t = text;
        if (t.StartsWith("@")) { int sp = t.IndexOf(' '); t = sp >= 0 ? t[(sp + 1)..] : t; }
        int e = t.IndexOfAny(new[] { ' ', '.', ';' });
        return e < 0 ? t : t[..e];
    }

    static int Verify(string oracle, string corpus, string[] a)
    {
        int every = int.Parse(Opt(a, "--every", "1"));
        string show = Opt(a, "--show", "");
        int examples = int.Parse(Opt(a, "--examples", "3"));
        int jobs = int.Parse(Opt(a, "--jobs", "4"));
        var firstCtrl = new Dictionary<string, string>();
        foreach (var ln in File.ReadLines(Path.Combine(corpus, "unique.tsv")).Skip(1))
        {
            var f = ln.Split('\t');
            firstCtrl[f[0]] = f[9];
        }
        var hashes = File.ReadLines(Path.Combine(oracle, "index.tsv")).Skip(1).Select(l => l.Split('\t')[0]).Where((h, i) => i % every == 0).ToList();
        var total = new ConcurrentDictionary<string, long>();
        var bad = new ConcurrentDictionary<string, long>();
        var ex = new ConcurrentDictionary<string, ConcurrentQueue<string>>();
        long nInst = 0, nOk = 0, nUnknown = 0;
        var sw = Stopwatch.StartNew();
        Parallel.ForEach(hashes, new ParallelOptions { MaxDegreeOfParallelism = jobs }, h =>
        {
            var code = File.ReadAllBytes(Path.Combine(corpus, "code", h + ".bin"));
            var ctrl = File.ReadAllBytes(Path.Combine(corpus, "ctrl", firstCtrl[h] + ".bin"));
            var (start, end) = Program_(code, ctrl);
            var insts = SassDecoder.DecodeBlock(code, start, end);
            var lines = File.ReadAllLines(Path.Combine(oracle, h + ".sass"));
            long ok = 0;
            var tl = new Dictionary<string, long>(); var bl = new Dictionary<string, long>();
            for (int k = 0; k < lines.Length; k++)
            {
                var m = Line.Match(lines[k]);
                if (!m.Success) continue;
                string want = Norm(m.Groups[2].Value + " ;");
                string op = BaseOp(want);
                tl[op] = tl.GetValueOrDefault(op) + 1;
                string got = k < insts.Count ? Norm(insts[k].ToSassText()) : "<missing>";
                bool addrOk = k < insts.Count && insts[k].Address == Convert.ToInt32(m.Groups[1].Value, 16);
                if (got == want && addrOk) { ok++; continue; }
                bl[op] = bl.GetValueOrDefault(op) + 1;
                if (k < insts.Count && insts[k].Opcode.StartsWith(".UNKNOWN")) Interlocked.Increment(ref nUnknown);
                if (show.Length == 0 || show == op)
                {
                    var q = ex.GetOrAdd(op, _ => new ConcurrentQueue<string>());
                    if (q.Count < examples) q.Enqueue($"  want {want}\n  got  {got}\n       {m.Groups[3].Value} {m.Groups[4].Value}  {h}@{m.Groups[1].Value}");
                }
            }
            Interlocked.Add(ref nInst, lines.Length);
            Interlocked.Add(ref nOk, ok);
            foreach (var kv in tl) total.AddOrUpdate(kv.Key, kv.Value, (_, v) => v + kv.Value);
            foreach (var kv in bl) bad.AddOrUpdate(kv.Key, kv.Value, (_, v) => v + kv.Value);
        });
        Console.WriteLine($"blocks {hashes.Count}  instructions {nInst}  exact {nOk}  ({100.0 * nOk / Math.Max(1, nInst):F4}%)  unknown-opcode {nUnknown}  [{sw.Elapsed.TotalSeconds:F1}s]");
        Console.WriteLine($"mnemonics {total.Count}; with >=100 occurrences: {total.Count(kv => kv.Value >= 100)}, of those exact: {total.Count(kv => kv.Value >= 100 && !bad.ContainsKey(kv.Key))}");
        Console.WriteLine("unmatched by mnemonic (count / occurrences):");
        foreach (var kv in bad.OrderByDescending(kv => kv.Value).Take(40))
            Console.WriteLine($"  {kv.Key,-10} {kv.Value,9} / {total[kv.Key],9}");
        foreach (var kv in ex.OrderByDescending(kv => bad.GetValueOrDefault(kv.Key)).Take(show.Length > 0 ? 1 : 25))
        {
            Console.WriteLine($"-- {kv.Key}");
            foreach (var s in kv.Value) Console.WriteLine(s);
        }
        return nOk == nInst ? 0 : 1;
    }

    static int WholeCorpus(string corpus, string[] a)
    {
        int jobs = int.Parse(Opt(a, "--jobs", "4"));
        var rows = File.ReadLines(Path.Combine(corpus, "unique.tsv")).Skip(1).Select(l => l.Split('\t')).ToList();
        long nInst = 0, nUnknown = 0, bytes = 0;
        var sw = Stopwatch.StartNew();
        Parallel.ForEach(rows, new ParallelOptions { MaxDegreeOfParallelism = jobs }, f =>
        {
            var code = File.ReadAllBytes(Path.Combine(corpus, "code", f[0] + ".bin"));
            var ctrl = File.ReadAllBytes(Path.Combine(corpus, "ctrl", f[9] + ".bin"));
            var (start, end) = Program_(code, ctrl);
            var insts = SassDecoder.DecodeBlock(code, start, end);
            long u = insts.Count(i => i.Opcode.StartsWith(".UNKNOWN"));
            Interlocked.Add(ref nInst, insts.Count);
            Interlocked.Add(ref nUnknown, u);
            Interlocked.Add(ref bytes, end - start);
        });
        Console.WriteLine($"blocks {rows.Count}  program bytes {bytes}  instructions {nInst}  unknown-opcode {nUnknown}  [{sw.Elapsed.TotalSeconds:F1}s]");
        return 0;
    }

    static int Dump(string codePath, string ctrlPath)
    {
        var code = File.ReadAllBytes(codePath);
        var (start, end) = Program_(code, File.ReadAllBytes(ctrlPath));
        foreach (var i in SassDecoder.DecodeBlock(code, start, end))
            Console.WriteLine($"/*{i.Address:x5}*/ {i.ToSassText()} ; 0x{i.Lo:x16} 0x{i.Hi:x16}  dst={i.DstCount} sched=0x{i.Sched:x6}");
        return 0;
    }
}
