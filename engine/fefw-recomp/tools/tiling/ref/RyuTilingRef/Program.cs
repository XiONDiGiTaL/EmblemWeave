// MIT License. This file is MIT-licensed.
// It calls Ryujinx.Graphics.Texture (Ryujinx, MIT License, Ryujinx Team; Ryubing fork, Codeberg archive a82350bb77),
// staged read-only from refs\ryubing into tools\tiling\third_party. The project setup pattern is from
// SuperMarioWonderRecomp (MIT), as used in tools\shaders\harness.
//
// RyuTilingRef gen --out <dir> [--matrix default] [--seed N]
//
// Writes, per test case N (4 digits):
//   case_NNNN.linear.bin   packed linear input (deterministic xorshift bytes, position dependent)
//   case_NNNN.tiled.bin    Ryujinx ConvertLinearToBlockLinear output, exactly SizeInfo.TotalSize bytes
// and cases.tsv (one row per case). Candidates that Ryujinx rejects go to rejected.tsv, candidates dropped by the
// output budget go to skipped.tsv. Every tiled output is also run back through ConvertBlockLinearToLinear and the
// valid bytes must match the input, otherwise the case is not written and counts as a round-trip failure.
//
// ===== LAYOUT RULES (as implemented by Ryujinx; the C++ port must reproduce these) =====
// Inputs: W,H,D texels; levels; layers; block BW x BH texels (BPB = bytes per block, passed as "bytesPerPixel");
//   gobBlocksInY (GY), gobBlocksInZ (GZ); gobBlocksInTileX fixed to 1 in this generator.
// Level L dimensions: w = max(1, W>>L), h = max(1, H>>L), d = max(1, D>>L); w,h are then divided (round up) by BW,BH.
//
// LINEAR buffer (input to ConvertLinearToBlockLinear, output of ConvertBlockLinearToLinear):
//   stride_L   = AlignUp(w_blocks * BPB, 4)                 (rows padded to 4 bytes)
//   slice_L    = stride_L * h_blocks * d                     (one layer of one level)
//   level_base = sum over earlier levels of (layers * slice)  (levels are back to back, no alignment)
//   layer k of level L starts at level_base_L + k * slice_L   (all layers of a level are contiguous)
//   inside a slice, rows are ordered z-major then y: offset(z,y) = (z * h_blocks + y) * stride_L
//   pad bytes at the end of a row (w_blocks*BPB .. stride_L) are never read by the linear->tiled direction.
//   linear_size = layers * sum over levels of slice_L (Ryujinx GetTextureSize).
//
// TILED buffer (SizeCalculator.GetBlockLinearTextureSize -> SizeInfo):
//   mipGobY starts at GY and is halved while h_blocks <= (mipGobY>>1)*8 and mipGobY != 1 (per level, cumulative).
//   mipGobZ starts at GZ; for level > 0 it is halved while d <= (mipGobZ>>1) and mipGobZ != 1 (per level, cumulative).
//   Level L in one layer: robSize = widthInGobs * mipGobY * mipGobZ * 512, widthInGobs = ceil(w_blocks*BPB / 64),
//     sliceSize = ceil(h_blocks / (8*mipGobY)) * robSize, levelSize = ceil(d / mipGobZ) * sliceSize.
//   mipOffset_L = sum of earlier levelSize (SizeInfo.GetMipOffset(L)), relative to the start of the layer.
//   2D (not is3D): layer stride (SizeInfo.LayerSize) = sum of level sizes, and when layers > 1 it is rounded up
//     to a multiple of (gy0 * gz0 * 512) where gy0/gz0 are the base-level GOB counts (AlignLayerSize, tileX=1).
//     Layer k starts at k * LayerSize.  TotalSize = LayerSize * layers.
//   3D (is3D = D>1 || GZ>1): layers is always 1 in this matrix. The last level's size is trimmed when
//     d % mipGobZ != 0 (the unused GOBs beyond the depth are dropped), so TotalSize can be below the untrimmed sum.
//   Texel (x,y,z), x in bytes (x = texel * BPB), offset inside the layer:
//     yh = y / 8
//     off = (z >> log2(mipGobZ)) * sliceSize + (yh >> log2(mipGobY)) * robSize
//         + (x / 64) << log2(512 * mipGobY * mipGobZ)
//         + ((z mod mipGobZ) * mipGobY + (yh mod mipGobY)) * 512
//         + ((x&63)>>5)*256 + ((y&7)>>1)*64 + ((x&31)>>4)*32 + (y&1)*16 + (x&15)
//   (GOB = 64 bytes x 8 rows = 512 bytes; GOBs in a block are stacked in y then z; blocks are side by side in x.)
//   Bytes of the tiled buffer that no texel maps to (GOB padding) are zero in the output.
//
// Ryujinx calls used (namespace Ryujinx.Graphics.Texture, public API):
//   SizeCalculator.GetBlockLinearTextureSize(width, height, depth, levels, layers, blockW, blockH, bpb, gobY, gobZ, tileX)
//   LayoutConverter.ConvertLinearToBlockLinear(output, width, height, depth, sliceDepth, levels, layers, blockW, blockH,
//                                              bpb, gobY, gobZ, tileX, sizeInfo, linear)     sliceDepth = depth
//   LayoutConverter.ConvertBlockLinearToLinear(width, height, depth, sliceDepth, levels, layers, blockW, blockH,
//                                              bpb, gobY, gobZ, tileX, sizeInfo, tiled)     returns MemoryOwner<byte>
// Ryujinx GPU (Texture.cs, around line 980) passes sliceDepth = 1 when its `single` flag is set, otherwise depth; this generator always passes depth.
// bpb values supported by the converter: 1, 2, 4, 8, 12, 16. This generator uses 1, 2, 4, 8, 16.
// ==========================================================================================

using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Numerics;
using System.Text;
using Ryujinx.Common.Memory;
using Ryujinx.Graphics.Texture;

namespace RyuTilingRef
{
    internal sealed class Case
    {
        public int W, H, D, Levels, Layers, BW, BH, Bpb, GY, GZ;
        public bool Is3D => D > 1 || GZ > 1;
    }

    internal static class Program
    {
        private const long BudgetBytes = 290L * 1024 * 1024; // linear + tiled bytes, kept under ~300 MB
        private const int Slack = 4096;                      // canary tail on tiled output, must stay zero
        private const int GobSize = 512;

        private static readonly int[] GobYs = { 1, 2, 4, 8, 16, 32 };
        private static readonly int[] Bpbs = { 1, 2, 4, 8, 16 };

        private static int Main(string[] args)
        {
            if (args.Length < 2 || args[0] != "gen")
            {
                Console.Error.WriteLine("usage: RyuTilingRef gen --out <dir> [--matrix default] [--seed N]");
                return 1;
            }

            string outDir = null;
            string matrix = "default";
            ulong seed = 1;
            for (int i = 1; i < args.Length; i++)
            {
                switch (args[i])
                {
                    case "--out": outDir = args[++i]; break;
                    case "--matrix": matrix = args[++i]; break;
                    case "--seed": seed = ulong.Parse(args[++i], CultureInfo.InvariantCulture); break;
                    default:
                        Console.Error.WriteLine("unknown argument: " + args[i]);
                        return 1;
                }
            }

            if (outDir == null)
            {
                Console.Error.WriteLine("--out is required");
                return 1;
            }

            if (matrix != "default")
            {
                Console.Error.WriteLine("only --matrix default exists");
                return 1;
            }

            return Run(outDir, seed);
        }

        private static int Run(string outDir, ulong seed)
        {
            Directory.CreateDirectory(outDir);
            foreach (string f in Directory.GetFiles(outDir))
            {
                string name = Path.GetFileName(f);
                if (name.StartsWith("case_", StringComparison.Ordinal) || name == "cases.tsv" || name == "rejected.tsv" || name == "skipped.tsv")
                {
                    File.Delete(f);
                }
            }

            const string header = "id\twidth\theight\tdepth\tlevels\tlayers\tblock_w\tblock_h\tbytes_per_block\tgob_blocks_y\tgob_blocks_z\tis_3d\tlinear_size\ttiled_size\tlinear_level_offsets\ttiled_level_offsets\ttiled_layer_stride";
            var cases = new StringBuilder(header + "\n");
            var rejected = new StringBuilder("cand\twidth\theight\tdepth\tlevels\tlayers\tblock_w\tblock_h\tbytes_per_block\tgob_blocks_y\tgob_blocks_z\treason\n");
            var skipped = new StringBuilder("cand\twidth\theight\tdepth\tlevels\tlayers\tblock_w\tblock_h\tbytes_per_block\tgob_blocks_y\tgob_blocks_z\treason\n");

            List<Case> matrix = BuildDefaultMatrix();
            long total = 0;
            int written = 0;
            int rejectedCount = 0;
            int skippedCount = 0;
            long mismatchTotal = 0;

            for (int cand = 0; cand < matrix.Count; cand++)
            {
                Case c = matrix[cand];
                string prefix = $"{cand}\t{c.W}\t{c.H}\t{c.D}\t{c.Levels}\t{c.Layers}\t{c.BW}\t{c.BH}\t{c.Bpb}\t{c.GY}\t{c.GZ}";

                try
                {
                    long linearSize = LinearSize(c);
                    SizeInfo si = SizeCalculator.GetBlockLinearTextureSize(c.W, c.H, c.D, c.Levels, c.Layers, c.BW, c.BH, c.Bpb, c.GY, c.GZ, 1);
                    long est = linearSize + si.TotalSize;

                    if (total + est > BudgetBytes)
                    {
                        skipped.Append(prefix).Append("\tbudget\n");
                        skippedCount++;
                        continue;
                    }

                    int id = written;
                    byte[] linear = new byte[linearSize];
                    FillDeterministic(linear, seed, id);

                    int tiledLen = si.TotalSize;
                    byte[] tiled = new byte[tiledLen + Slack];
                    LayoutConverter.ConvertLinearToBlockLinear(
                        tiled, c.W, c.H, c.D, c.D, c.Levels, c.Layers, c.BW, c.BH, c.Bpb, c.GY, c.GZ, 1, si, linear);

                    for (int i = tiledLen; i < tiled.Length; i++)
                    {
                        if (tiled[i] != 0)
                        {
                            throw new InvalidOperationException("converter wrote past SizeInfo.TotalSize");
                        }
                    }

                    long mismatches;
                    using (MemoryOwner<byte> back = LayoutConverter.ConvertBlockLinearToLinear(
                        c.W, c.H, c.D, c.D, c.Levels, c.Layers, c.BW, c.BH, c.Bpb, c.GY, c.GZ, 1, si,
                        new ReadOnlySpan<byte>(tiled, 0, tiledLen)))
                    {
                        mismatches = CompareValid(c, linear, back.Span);
                    }

                    if (mismatches != 0)
                    {
                        mismatchTotal += mismatches;
                        rejected.Append(prefix).Append("\troundtrip_mismatch_bytes=").Append(mismatches).Append('\n');
                        rejectedCount++;
                        continue;
                    }

                    string name = id.ToString("D4", CultureInfo.InvariantCulture);
                    File.WriteAllBytes(Path.Combine(outDir, $"case_{name}.linear.bin"), linear);
                    File.WriteAllBytes(Path.Combine(outDir, $"case_{name}.tiled.bin"), new ReadOnlySpan<byte>(tiled, 0, tiledLen).ToArray());

                    long[] linBase = LinearLevelBases(c);
                    cases.Append(name).Append('\t')
                        .Append(c.W).Append('\t').Append(c.H).Append('\t').Append(c.D).Append('\t')
                        .Append(c.Levels).Append('\t').Append(c.Layers).Append('\t')
                        .Append(c.BW).Append('\t').Append(c.BH).Append('\t').Append(c.Bpb).Append('\t')
                        .Append(c.GY).Append('\t').Append(c.GZ).Append('\t').Append(c.Is3D ? 1 : 0).Append('\t')
                        .Append(linearSize).Append('\t').Append(tiledLen).Append('\t')
                        .Append(string.Join(",", linBase)).Append('\t')
                        .Append(TiledLevelOffsets(si, c.Levels)).Append('\t')
                        .Append(si.LayerSize).Append('\n');

                    total += linearSize + tiledLen;
                    written++;
                }
                catch (Exception e)
                {
                    rejected.Append(prefix).Append('\t').Append(e.GetType().Name).Append(": ").Append(e.Message.Replace('\n', ' ')).Append('\n');
                    rejectedCount++;
                }
            }

            File.WriteAllText(Path.Combine(outDir, "cases.tsv"), cases.ToString());
            File.WriteAllText(Path.Combine(outDir, "rejected.tsv"), rejected.ToString());
            File.WriteAllText(Path.Combine(outDir, "skipped.tsv"), skipped.ToString());

            Console.WriteLine($"candidates={matrix.Count} written={written} bytes={total} rejected={rejectedCount} skipped_budget={skippedCount} roundtrip_mismatch_bytes={mismatchTotal}");
            Console.WriteLine($"out={outDir} seed={seed}");
            return mismatchTotal != 0 ? 2 : 0;
        }

        // ---- Matrix "default" ----
        private static List<Case> BuildDefaultMatrix()
        {
            var list = new List<Case>();

            // G1: plain 1x1 blocks, single level, 1 layer, every bytes_per_block x every gob_blocks_y.
            var small = new (int W, int H)[] { (1, 1), (7, 5), (33, 65), (100, 37), (256, 256) };
            foreach (int bpb in Bpbs)
                foreach (int gy in GobYs)
                    foreach (var s in small)
                        list.Add(Make(s.W, s.H, 1, false, 1, 1, 1, bpb, gy, 1));

            // G2: full mip chains, 1 layer, rotating gob_blocks_y.
            var mips = new (int W, int H)[] { (7, 5), (33, 65), (100, 37), (256, 256), (300, 300) };
            for (int bi = 0; bi < Bpbs.Length; bi++)
                for (int si = 0; si < mips.Length; si++)
                    for (int r = 0; r < 2; r++)
                        list.Add(Make(mips[si].W, mips[si].H, 1, true, 1, 1, 1, Bpbs[bi], GobYs[(bi * 2 + si + r * 3) % 6], 1));

            // G3: layers 2 and 6, single and full chains.
            var lay = new (int W, int H)[] { (33, 65), (100, 37), (256, 256) };
            int g3 = 0;
            foreach (var s in lay)
                foreach (bool full in new[] { false, true })
                    foreach (int layers in new[] { 2, 6 })
                        foreach (int bpb in new[] { 1, 4, 16 })
                            list.Add(Make(s.W, s.H, 1, full, layers, 1, 1, bpb, GobYs[g3++ % 6], 1));

            // G4: large and awkward sizes, sparse on purpose to stay under the budget.
            for (int bi = 0; bi < Bpbs.Length; bi++)
                list.Add(Make(300, 300, 1, false, 1, 1, 1, Bpbs[bi], GobYs[(bi + 2) % 6], 1));
            foreach (int bpb in new[] { 1, 2, 4, 8 })
                list.Add(Make(1024, 512, 1, false, 1, 1, 1, bpb, GobYs[(bpb + 1) % 6], 1));
            list.Add(Make(1024, 512, 1, true, 1, 1, 1, 4, 16, 1));
            list.Add(Make(1024, 512, 1, true, 2, 1, 1, 4, 8, 1));
            list.Add(Make(1920, 1080, 1, false, 1, 1, 1, 1, 8, 1));
            list.Add(Make(1920, 1080, 1, false, 1, 1, 1, 1, 16, 1));
            list.Add(Make(1920, 1080, 1, false, 1, 1, 1, 4, 16, 1));

            // G5: block-compressed. 4x4 BCn at bpb 8 and 16, ASTC 5x5..12x12 at bpb 16.
            var blocks = new (int BW, int BH, int Bpb)[]
            {
                (4, 4, 8), (4, 4, 16), (5, 5, 16), (6, 6, 16), (8, 8, 16), (10, 10, 16), (12, 12, 16),
            };
            var bsizes = new (int W, int H)[] { (7, 5), (100, 37), (256, 256), (1024, 512), (1920, 1080) };
            for (int bi = 0; bi < blocks.Length; bi++)
            {
                for (int si = 0; si < bsizes.Length; si++)
                    list.Add(Make(bsizes[si].W, bsizes[si].H, 1, false, 1, blocks[bi].BW, blocks[bi].BH, blocks[bi].Bpb, GobYs[(bi + si) % 6], 1));
                list.Add(Make(256, 256, 1, true, 1, blocks[bi].BW, blocks[bi].BH, blocks[bi].Bpb, GobYs[(bi * 2) % 6], 1));
            }

            foreach (int layers in new[] { 2, 6 })
            {
                list.Add(Make(256, 256, 1, false, layers, 4, 4, 16, 8, 1));
                list.Add(Make(256, 256, 1, false, layers, 6, 6, 16, 16, 1));
            }

            // G6: 3D, depth {4,9,16} x gob_blocks_z {1,2,4,8,16}, single and full chains, layers 1.
            var s3 = new (int W, int H)[] { (7, 5), (33, 65), (100, 37) };
            int[] depths = { 4, 9, 16 };
            int[] gzs = { 1, 2, 4, 8, 16 };
            foreach (bool full in new[] { false, true })
                for (int di = 0; di < depths.Length; di++)
                    for (int gi = 0; gi < gzs.Length; gi++)
                    {
                        var s = s3[(di + gi) % 3];
                        list.Add(Make(s.W, s.H, depths[di], full, 1, 1, 1, Bpbs[(di * 5 + gi) % 5], GobYs[(di + gi) % 6], gzs[gi]));
                    }

            // 3D with depth 1 but gob_blocks_z > 1 (is_3d is 1 because GZ > 1).
            for (int gi = 1; gi < gzs.Length; gi++)
            {
                var s = s3[gi % 3];
                list.Add(Make(s.W, s.H, 1, false, 1, 1, 1, 4, GobYs[gi], gzs[gi]));
            }

            // 3D block-compressed.
            list.Add(Make(100, 37, 4, false, 1, 4, 4, 8, 2, 2));
            list.Add(Make(100, 37, 9, true, 1, 6, 6, 16, 4, 4));

            return list;
        }

        private static Case Make(int w, int h, int d, bool full, int layers, int bw, int bh, int bpb, int gy, int gz)
        {
            return new Case
            {
                W = w, H = h, D = d,
                Levels = full ? FullLevels(w, h, d) : 1,
                Layers = layers,
                BW = bw, BH = bh, Bpb = bpb,
                GY = gy, GZ = gz,
            };
        }

        private static int FullLevels(int w, int h, int d) => 1 + BitOperations.Log2((uint)Math.Max(w, Math.Max(h, d)));

        // ---- Linear layout helpers (mirror Ryujinx's own loops) ----
        private static int DivUp(int v, int a) => (v + a - 1) / a;
        private static int AlignUp4(int v) => (v + 3) & ~3;

        private static int LevelW(Case c, int l) => DivUp(Math.Max(1, c.W >> l), c.BW);
        private static int LevelH(Case c, int l) => DivUp(Math.Max(1, c.H >> l), c.BH);
        private static int LevelD(Case c, int l) => Math.Max(1, c.D >> l);
        private static int LinStride(Case c, int l) => AlignUp4(LevelW(c, l) * c.Bpb);
        private static long LinSlice(Case c, int l) => (long)LinStride(c, l) * LevelH(c, l) * LevelD(c, l);

        private static long LinearSize(Case c)
        {
            long sum = 0;
            for (int l = 0; l < c.Levels; l++)
            {
                sum += LinSlice(c, l);
            }

            return sum * c.Layers;
        }

        private static long[] LinearLevelBases(Case c)
        {
            var r = new long[c.Levels];
            long acc = 0;
            for (int l = 0; l < c.Levels; l++)
            {
                r[l] = acc;
                acc += LinSlice(c, l) * c.Layers;
            }

            return r;
        }

        private static string TiledLevelOffsets(SizeInfo si, int levels)
        {
            var parts = new string[levels];
            for (int l = 0; l < levels; l++)
            {
                parts[l] = si.GetMipOffset(l).ToString(CultureInfo.InvariantCulture);
            }

            return string.Join(",", parts);
        }

        // Compares valid (non-padding) bytes of the reverse conversion against the linear input.
        private static long CompareValid(Case c, byte[] linear, ReadOnlySpan<byte> back)
        {
            long[] bases = LinearLevelBases(c);
            long bad = 0;
            for (int l = 0; l < c.Levels; l++)
            {
                int w = LevelW(c, l);
                int h = LevelH(c, l);
                int d = LevelD(c, l);
                int stride = LinStride(c, l);
                long slice = LinSlice(c, l);
                int n = w * c.Bpb;
                for (int layer = 0; layer < c.Layers; layer++)
                {
                    for (int z = 0; z < d; z++)
                    {
                        for (int y = 0; y < h; y++)
                        {
                            long off = bases[l] + layer * slice + (long)(z * h + y) * stride;
                            for (int i = 0; i < n; i++)
                            {
                                if (linear[off + i] != back[(int)off + i])
                                {
                                    bad++;
                                }
                            }
                        }
                    }
                }
            }

            return bad;
        }

        // Deterministic, position-dependent fill: xorshift64 stream, seeded per case through SplitMix64.
        private static void FillDeterministic(byte[] buf, ulong seed, int caseId)
        {
            ulong s = SplitMix64(seed ^ ((ulong)caseId * 0x9E3779B97F4A7C15UL));
            for (int i = 0; i < buf.Length; i += 8)
            {
                s ^= s << 13;
                s ^= s >> 7;
                s ^= s << 17;
                ulong v = s;
                for (int k = 0; k < 8 && i + k < buf.Length; k++)
                {
                    buf[i + k] = (byte)(v >> (8 * k));
                }
            }
        }

        private static ulong SplitMix64(ulong x)
        {
            x += 0x9E3779B97F4A7C15UL;
            x = (x ^ (x >> 30)) * 0xBF58476D1CE4E5B9UL;
            x = (x ^ (x >> 27)) * 0x94D049BB133111EBUL;
            x ^= x >> 31;
            return x == 0 ? 1UL : x;
        }
    }
}
