# SPDX-License-Identifier: GPL-3.0-or-later
"""Name the fefw_rt.dll frames of a crash report, freeze report or log.

    python -I tools/symbolize.py fefw_crash.txt --symbols <folder>
    python -I tools/symbolize.py nxrt.log fefw_freeze.txt --symbols <folder> > named.txt

<folder> holds fefw_rt.dll and fefw_rt.pdb from fefw-recomp-<version>-win64-symbols.zip (a release asset) of the version the report
came from: a PDB of another build names the wrong functions. Every "fefw_rt.dll+0x..." in the input gets " = function file:line"
after it (inlined callers too, innermost first: "inner <- caller"); the rest is printed unchanged. Frames below "#0" are return
addresses and are looked up one byte back, at their call. Frames of fefw.exe (the game code, linked on the
player's PC), the driver and Windows are left as they are.

Needs llvm-symbolizer (LLVM 13 or newer: the release build's LLVM, or e.g. brew install llvm): found through --llvm-bin, NX_LLVM_BIN,
PATH, or Homebrew's folder.

    python -I tools/symbolize.py --self-test <folder>/fefw_rt.dll

checks that the PDB beside the DLL names the DLL's exports (the release workflow runs it before publishing the symbols).
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys

MODULE = "fefw_rt.dll"
FRAME = re.compile(r"\bfefw_rt\.dll\+0x([0-9a-fA-F]+)\b")


def find_symbolizer(llvm_bin):
    exe = "llvm-symbolizer.exe" if os.name == "nt" else "llvm-symbolizer"
    for d in (llvm_bin, os.environ.get("NX_LLVM_BIN")):
        if d and os.path.isfile(os.path.join(d, exe)):
            return os.path.join(d, exe)
    found = shutil.which("llvm-symbolizer")
    if found:
        return found
    for d in ("/opt/homebrew/opt/llvm/bin", "/usr/local/opt/llvm/bin"):
        if os.path.isfile(os.path.join(d, exe)):
            return os.path.join(d, exe)
    sys.exit("symbolize: llvm-symbolizer not found (--llvm-bin, NX_LLVM_BIN, PATH; on macOS: brew install llvm)")


def symbolize(symbolizer, dll, pdb, rvas):
    """{rva: [(function, file, line), ...]} for image-relative addresses, innermost frame first."""
    if not rvas:
        return {}
    cmd = [symbolizer, "--obj=" + dll, "--pdb=" + pdb, "--relative-address", "--inlines", "--demangle",
           "--output-style=JSON", "--relativenames"]
    text = "\n".join("0x%x" % r for r in rvas) + "\n"
    out = subprocess.run(cmd, input=text, capture_output=True, text=True, check=False)
    if out.returncode:
        sys.exit("symbolize: llvm-symbolizer failed: " + out.stderr.strip())
    names = {}
    for line in out.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        entry = json.loads(line)
        rva = int(entry["Address"], 16)
        frames = []
        for f in entry.get("Symbol", []):
            fn = f.get("FunctionName") or "??"
            frames.append((fn, f.get("FileName") or "", f.get("Line") or 0))
        names[rva] = frames
    return names


# the build machine's checkout folder is cut off: .../fefw-recomp/runtime/src/x.cpp -> runtime/src/x.cpp
TOP_DIRS = ("runtime/", "game-build/", "recomp/", "tools/", "launcher/")


def short_path(path):
    path = path.replace("\\", "/")
    for top in TOP_DIRS:
        i = path.find("/" + top)
        if i >= 0:
            return path[i + 1:]
        if path.startswith(top):
            return path
    return path


def lookup_rva(line, match):
    """A frame below the innermost (#0) is a return address: the call is the instruction before it."""
    rva = int(match.group(1), 16)
    return rva if re.match(r"\s*#0\s", line) else rva - 1


def describe(frames):
    parts = []
    for fn, file, line in frames:
        if fn == "??" and not file:
            continue
        where = "%s:%d" % (short_path(file), line) if file and line else short_path(file)
        parts.append(fn + (" " + where if where else ""))
    return " <- ".join(parts)


def symbol_files(folder):
    dll, pdb = os.path.join(folder, MODULE), os.path.join(folder, "fefw_rt.pdb")
    for f in (dll, pdb):
        if not os.path.isfile(f):
            sys.exit("symbolize: %s not found (unpack fefw-recomp-<version>-win64-symbols.zip there)" % f)
    return dll, pdb


def run(args):
    dll, pdb = symbol_files(args.symbols)
    symbolizer = find_symbolizer(args.llvm_bin)
    texts = []
    for path in args.reports or ["-"]:
        if path == "-":
            texts.append(sys.stdin.read())
        else:
            with open(path, encoding="utf-8", errors="replace") as f:
                texts.append(f.read())
    rvas = sorted({lookup_rva(line, m) for t in texts for line in t.splitlines() for m in FRAME.finditer(line)})
    names = symbolize(symbolizer, dll, pdb, rvas)
    unnamed = 0
    for path, text in zip(args.reports or ["-"], texts):
        if len(texts) > 1:
            print("==> %s <==" % path)
        for line in text.splitlines():
            found = [lookup_rva(line, m) for m in FRAME.finditer(line)]
            named = [describe(names.get(r, [])) for r in found]
            unnamed += sum(1 for n in named if not n)
            named = [n for n in named if n]
            print(line + ("  = " + " | ".join(named) if named else ""))
    if rvas:
        print("symbolize: %d distinct %s addresses, %d without a name" % (len(rvas), MODULE, unnamed), file=sys.stderr)
    return 0


def self_test(args):
    dll = args.self_test
    folder = os.path.dirname(os.path.abspath(dll))
    dll, pdb = symbol_files(folder)
    symbolizer = find_symbolizer(args.llvm_bin)
    readobj = os.path.join(os.path.dirname(symbolizer), "llvm-readobj" + (".exe" if os.name == "nt" else ""))
    if not os.path.isfile(readobj):
        sys.exit("symbolize: llvm-readobj not found beside " + symbolizer)
    out = subprocess.run([readobj, "--coff-exports", dll], capture_output=True, text=True, check=True).stdout
    # Export { Ordinal: 1  Name: fefw_rt_main  RVA: 0x1234 }; DATA exports (tables) have no function to name
    exports = [(m.group(1), int(m.group(2), 16)) for m in re.finditer(r"Name: (\S+)\s+RVA: (0x[0-9A-Fa-f]+)", out)]
    names = symbolize(symbolizer, dll, pdb, [rva for _, rva in exports])
    checked = bad = 0
    for name, rva in exports:
        fns = [fn for fn, _, _ in names.get(rva, [])]
        if not fns or fns == ["??"]:
            continue  # data, or folded into a function of another name
        checked += 1
        if not any(name in fn for fn in fns):
            bad += 1
            print("symbolize: export %s at 0x%x is named %s" % (name, rva, fns), file=sys.stderr)
    print("symbolize: %d exports, %d named after themselves, %d wrong" % (len(exports), checked - bad, bad))
    if not checked or bad:
        print("symbolize: the PDB does not match the DLL", file=sys.stderr)
        return 1
    return 0


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("reports", nargs="*", help="crash report, freeze report or log (default: stdin)")
    p.add_argument("--symbols", help="folder with fefw_rt.dll and fefw_rt.pdb of the report's version")
    p.add_argument("--llvm-bin", help="folder of llvm-symbolizer")
    p.add_argument("--self-test", metavar="DLL", help="check that the fefw_rt.pdb beside DLL names its exports")
    args = p.parse_args()
    if args.self_test:
        return self_test(args)
    if not args.symbols:
        p.error("--symbols is needed")
    return run(args)


if __name__ == "__main__":
    sys.exit(main())
