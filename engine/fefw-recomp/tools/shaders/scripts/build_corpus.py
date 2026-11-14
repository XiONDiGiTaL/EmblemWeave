#!/usr/bin/env python3
"""Adapt Vulkan-style GLSL shaders to the uam (deko3d Maxwell SM 5.3) dialect and compile them to .dksh.

Usage:
  build_corpus.py --src <glsl root> --out <dir> [--uam <wsl path>] [--jobs 16]

Outputs:
  <out_adapted>/        adapted GLSL copies (sibling folder: <out>_adapted), same relative layout as --src
  <out>/<rel>.dksh      compiled shaders, <rel> with '/' replaced by '__' (e.g. base__uioverlay.vert.dksh)
  <out>/logs/           per-shader uam logs
  <out>/report.tsv      source, stage, status ok|fail, dksh bytes, first error line

Only .vert/.frag/.comp/.geom are compiled. Source files are never modified.
"""
import argparse
import collections
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

STAGES = {".vert": "vert", ".frag": "frag", ".comp": "comp", ".geom": "geom"}
UAM_DEFAULT = "/mnt/d/AI/Active/Metroid-Prime/work/corpus/uam/bin/uam"
WSL_DISTRO = "Ubuntu-22.04"

# Placeholder written for push_constant blocks; resolved to binding 15 in renumber().
PUSH_TOKEN = "PUSHCONST_MARK"
PUSH_BINDING = 15

# Layout qualifiers that uam does not know and that have no meaning once the Vulkan
# descriptor model is gone. 'set' is dropped, 'constant_id' turns the const into a plain const,
# 'input_attachment_index' is consumed by the subpassInput rewrite.
DROP_QUALS = {"set", "constant_id", "input_attachment_index"}

# #extension lines removed. Extensions uam rejects, plus include_directive (includes are inlined).
DENY_EXT = {
    "GL_EXT_nonuniform_qualifier",
    "GL_EXT_scalar_block_layout",
    "GL_EXT_fragment_shading_rate",
    "GL_EXT_fragment_shader_barycentric",
    "GL_EXT_descriptor_heap",
    "GL_GOOGLE_include_directive",
    "GL_EXT_debug_printf",
}

# Per-kind binding limits given by uam: UBOs 0..15, SSBOs 0..15, samplers 0..31, images 0..7.
MAX_BINDING = {"ubo": 15, "ssbo": 15, "sampler": 31, "image": 7}
MAX_COUNT_PER_KIND = {"ubo": 16, "ssbo": 16, "sampler": 32, "image": 8}

INCLUDE_RE = re.compile(r'^[ \t]*#[ \t]*include[ \t]+"([^"]+)"[ \t]*$', re.M)
VERSION_RE = re.compile(r"^[ \t]*#[ \t]*version.*$", re.M)
EXT_RE = re.compile(r"^[ \t]*#[ \t]*extension[ \t]+(\w+)[ \t]*:.*\n?", re.M)
LAYOUT_RE = re.compile(r"layout\s*\(([^()]*)\)")
OFFSET_BLOCK_RE = re.compile(r"layout\s*\(([^()]*)\)(\s*uniform\s+\w+\s*\{[^}]*?\boffset\s*=)")
SUBPASS_DECL_RE = re.compile(r"layout\s*\(([^()]*)\)\s*uniform\s+subpassInput(MS)?\b")
SUBPASS_LOAD2_RE = re.compile(r"subpassLoad\s*\(\s*(\w+)\s*,\s*([^()]+?)\s*\)")
SUBPASS_LOAD1_RE = re.compile(r"subpassLoad\s*\(\s*(\w+)\s*\)")
BINDING_TOKEN_RE = re.compile(r"binding\s*=\s*(\w+)")
DECL_RE = re.compile(
    r"\s*(?:(?:coherent|volatile|restrict|readonly|writeonly|const)\s+)*(uniform|buffer)\s+"
    r"(?:(?:coherent|volatile|restrict|readonly|writeonly)\s+)*([A-Za-z_]\w*)"
)


class AdaptError(Exception):
    pass


def to_wsl(p):
    p = os.path.abspath(str(p)).replace("\\", "/")
    if len(p) > 1 and p[1] == ":":
        return "/mnt/" + p[0].lower() + p[2:]
    return p


def inline_includes(text, here, root, depth=0):
    if depth > 6:
        raise AdaptError("include depth exceeded")

    def repl(m):
        name = m.group(1)
        for base in (here, root):
            cand = os.path.join(base, name)
            if os.path.isfile(cand):
                with open(cand, encoding="utf-8", errors="replace") as f:
                    body = VERSION_RE.sub("", f.read())
                return inline_includes(body, os.path.dirname(cand), root, depth + 1)
        raise AdaptError(f"missing include '{name}'")

    return INCLUDE_RE.sub(repl, text)


def drop_extensions(text):
    def repl(m):
        return "" if m.group(1) in DENY_EXT else m.group(0)

    return EXT_RE.sub(repl, text)


def rewrite_subpass_inputs(text):
    def decl(m):
        quals, ms = m.group(1), m.group(2) or ""
        b = re.search(r"binding\s*=\s*(\d+)", quals)
        prefix = f"layout (binding = {b.group(1)}) " if b else ""
        return f"{prefix}uniform sampler2D{ms}"

    text = SUBPASS_DECL_RE.sub(decl, text)
    text = SUBPASS_LOAD2_RE.sub(r"texelFetch(\1, ivec2(gl_FragCoord.xy), \2)", text)
    text = SUBPASS_LOAD1_RE.sub(r"texelFetch(\1, ivec2(gl_FragCoord.xy), 0)", text)
    return text


def fix_qualifiers(m):
    out = []
    push = False
    for q in m.group(1).split(","):
        q = q.strip()
        if not q:
            continue
        name = re.match(r"\w+", q).group(0)
        if name in DROP_QUALS:
            continue
        if name == "push_constant":
            push = True
            out.append(f"binding = {PUSH_TOKEN}")
            continue
        out.append(q)
    if push and not any(x in ("std140", "std430", "packed", "shared") for x in out):
        out.insert(0, "std140")
    return f"layout ({', '.join(out)})" if out else ""


def add_std140(m):
    # Blocks with explicit member offsets need an std layout; uam rejects them on the default layout.
    quals = m.group(1)
    if re.search(r"\b(std140|std430|packed|shared)\b", quals):
        return m.group(0)
    return f"layout ({quals.strip()}, std140){m.group(2)}"


def kind_of(keyword, typ):
    if keyword == "buffer":
        return "ssbo"
    if re.match(r"^[iu]?(sampler|texture)", typ):
        return "sampler"
    if re.match(r"^[iu]?image", typ):
        return "image"
    if typ.startswith("accelerationStructure"):
        return "as"
    return "ubo"


def renumber_bindings(text):
    """Keep original binding numbers when legal; otherwise renumber that kind 0..n-1 in declaration order."""
    entries = []
    for m in LAYOUT_RE.finditer(text):
        bm = BINDING_TOKEN_RE.search(m.group(1))
        if not bm:
            continue
        d = DECL_RE.match(text, m.end())
        if not d:
            continue
        value = bm.group(1)
        start = m.start(1) + bm.start(1)
        end = m.start(1) + bm.end(1)
        if value == PUSH_TOKEN:
            entries.append({"start": start, "end": end, "kind": "push", "value": None, "new": str(PUSH_BINDING)})
            continue
        kind = kind_of(d.group(1), d.group(2))
        if kind == "as":
            continue
        if not value.isdigit():
            raise AdaptError(f"non-literal binding '{value}'")
        entries.append({"start": start, "end": end, "kind": kind, "value": int(value), "new": None})

    has_push = any(e["kind"] == "push" for e in entries)
    for kind in ("ubo", "ssbo", "sampler", "image"):
        lst = [e for e in entries if e["kind"] == kind]
        if not lst:
            continue
        limit = MAX_BINDING[kind]
        if kind == "ubo" and has_push:
            limit -= 1  # binding 15 is the push-constant block
        vals = [e["value"] for e in lst]
        if len(set(vals)) == len(vals) and all(v <= limit for v in vals):
            for e in lst:
                e["new"] = str(e["value"])
        else:
            if len(lst) > limit + 1:
                raise AdaptError(f"too many {kind} bindings ({len(lst)})")
            for i, e in enumerate(lst):
                e["new"] = str(i)

    pieces, last = [], 0
    for e in sorted(entries, key=lambda x: x["start"]):
        pieces.append(text[last:e["start"]])
        pieces.append(e["new"])
        last = e["end"]
    pieces.append(text[last:])
    return "".join(pieces)


def adapt(src_text, path, root):
    text = inline_includes(src_text, os.path.dirname(path), root)
    text = drop_extensions(text)
    text = re.sub(r"\bgl_VertexIndex\b", "gl_VertexID", text)
    text = re.sub(r"\bgl_InstanceIndex\b", "gl_InstanceID", text)
    text = re.sub(r"\bgl_ViewIndex\b", "0", text)
    text = re.sub(r"\bnonuniformEXT\s*\(([^()]*)\)", r"(\1)", text)
    text = rewrite_subpass_inputs(text)
    text = OFFSET_BLOCK_RE.sub(add_std140, text)
    text = LAYOUT_RE.sub(fix_qualifiers, text)
    text = renumber_bindings(text)
    return text


def first_error(log_path):
    try:
        with open(log_path, encoding="utf-8", errors="replace") as f:
            lines = f.read().splitlines()
    except OSError:
        return "no log"
    for line in lines:
        if "error" in line.lower():
            return line.strip().replace("\t", " ")
    nonempty = [l for l in lines if l.strip()]
    return nonempty[-1].strip() if nonempty else "no output from uam"


def reason_key(err):
    if "error:" in err:
        err = err.split("error:", 1)[1]
    err = re.sub(r"\d+", "N", err.strip())
    return err[:110]


RUN_SH = r"""#!/bin/bash
# Written by build_corpus.py. Usage: _run_uam.sh <jobs> <uam> <jobfile>
JOBS="$1"; export UAM="$2"; JOBFILE="$3"
run_one() {
  local IFS='|'
  read -r rel stage src out log <<< "$1"
  "$UAM" -s "$stage" -o "$out" "$src" > "$log" 2>&1
  local rc=$?
  if [ $rc -ne 0 ]; then rm -f "$out"; fi
  echo "$rel|$rc"
}
export -f run_one
xargs -a "$JOBFILE" -d '\n' -P "$JOBS" -I{} bash -c 'run_one "$1"' _ {}
"""


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--src", required=True, help="GLSL root (walked recursively)")
    ap.add_argument("--out", required=True, help="output folder for .dksh files (created fresh)")
    ap.add_argument("--uam", default=UAM_DEFAULT, help="uam binary as a WSL path")
    ap.add_argument("--jobs", type=int, default=16, help="parallel uam processes")
    args = ap.parse_args()

    src = Path(args.src).resolve()
    out = Path(args.out).resolve()
    adapted = out.parent / (out.name + "_adapted")
    logs = out / "logs"

    for d in (out, adapted):
        if d.exists():
            shutil.rmtree(d)
    adapted.mkdir(parents=True)
    logs.mkdir(parents=True)

    rows = {}  # rel -> dict(stage, status, bytes, err)
    jobs_lines = []
    for dirpath, dirnames, filenames in os.walk(src):
        dirnames.sort()
        for fn in sorted(filenames):
            ext = os.path.splitext(fn)[1]
            if ext not in STAGES:
                continue
            full = os.path.join(dirpath, fn)
            rel = os.path.relpath(full, src).replace(os.sep, "/")
            stage = STAGES[ext]
            flat = rel.replace("/", "__")
            try:
                with open(full, encoding="utf-8", errors="replace") as f:
                    text = f.read()
                new_text = adapt(text, full, str(src))
            except AdaptError as e:
                rows[rel] = {"stage": stage, "status": "fail", "bytes": 0, "err": f"adapter: {e}"}
                continue
            adapted_path = adapted / rel
            adapted_path.parent.mkdir(parents=True, exist_ok=True)
            with open(adapted_path, "w", encoding="utf-8", newline="") as f:
                f.write(new_text)
            jobs_lines.append(
                "|".join([rel, stage, to_wsl(adapted_path), to_wsl(out / (flat + ".dksh")),
                          to_wsl(logs / (flat + ".log"))])
            )

    run_sh = adapted / "_run_uam.sh"
    jobfile = adapted / "_jobs.txt"
    with open(run_sh, "w", encoding="utf-8", newline="\n") as f:
        f.write(RUN_SH)
    with open(jobfile, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(jobs_lines) + "\n")

    cmd = ["wsl", "-d", WSL_DISTRO, "-e", "bash", to_wsl(run_sh), str(args.jobs), args.uam, to_wsl(jobfile)]
    proc = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", errors="replace")
    rcs = {}
    for line in proc.stdout.splitlines():
        if "|" in line:
            rel, rc = line.rsplit("|", 1)
            if rc.strip().lstrip("-").isdigit():
                rcs[rel] = int(rc)
    if proc.returncode != 0 and not rcs:
        sys.exit(f"wsl run failed: {proc.stderr.strip()}")

    for line in jobs_lines:
        rel = line.split("|", 1)[0]
        flat = rel.replace("/", "__")
        stage = STAGES[os.path.splitext(rel)[1]]
        dksh = out / (flat + ".dksh")
        rc = rcs.get(rel)
        if rc == 0 and dksh.exists():
            rows[rel] = {"stage": stage, "status": "ok", "bytes": dksh.stat().st_size, "err": ""}
        else:
            err = first_error(logs / (flat + ".log")) if rc is not None else "no result from uam run"
            rows[rel] = {"stage": stage, "status": "fail", "bytes": 0, "err": err}

    with open(out / "report.tsv", "w", encoding="utf-8", newline="\n") as f:
        f.write("source\tstage\tstatus\tdksh_bytes\tfirst_error\n")
        for rel in sorted(rows):
            r = rows[rel]
            f.write(f"{rel}\t{r['stage']}\t{r['status']}\t{r['bytes']}\t{r['err']}\n")

    total = len(rows)
    ok = [r for r in rows.values() if r["status"] == "ok"]
    fails = {k: v for k, v in rows.items() if v["status"] == "fail"}
    reasons = collections.Counter(reason_key(v["err"]) for v in fails.values())
    examples = {}
    for k, v in sorted(fails.items()):
        examples.setdefault(reason_key(v["err"]), k)

    print(f"total {total}  ok {len(ok)}  fail {len(fails)}")
    print("failure reasons (count, normalized first error, example):")
    for reason, n in reasons.most_common():
        print(f"  {n:4d}  {reason}  [{examples[reason]}]")
    stage_counts = collections.Counter(r["stage"] for r in ok)
    print("ok by stage: " + ", ".join(f"{s} {stage_counts.get(s, 0)}" for s in ("vert", "frag", "comp", "geom")))
    print(f"total .dksh bytes: {sum(r['bytes'] for r in ok)}")
    print(f"report: {out / 'report.tsv'}")


if __name__ == "__main__":
    main()
