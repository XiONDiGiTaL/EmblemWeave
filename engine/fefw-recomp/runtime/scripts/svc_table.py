#!/usr/bin/env python3
"""Command table for the service library (src/services/*.cpp).

Finds the IPC command registrations (add_cmd(id, "Name", handler), the cmd(svc, "service", id, "Name", handler)
helper, and the literal lists fed to rows(...) and to the *CmdNames* tables), the interface each belongs to
(innermost make_iface("...") call, else the last make_shared<ipc::Service>("...") in the function, else the
enclosing function name) and the names registered with add("name", ...).

Regular expressions over a copy of each file in which comments and literal contents are blanked, plus a
bracket matcher. Anything that does not parse is listed under "unparsed" with file:line.

Usage (standard library only):
    python -I scripts/svc_table.py                      print Markdown to stdout
    python -I scripts/svc_table.py --out docs/S3c_commands.md   write the file
"""
import argparse
import bisect
import glob
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "src", "services")

KEYWORDS = {"namespace", "struct", "class", "using", "template", "typedef", "enum", "return", "if", "for", "while",
            "switch", "static_assert", "union", "extern", "else", "do", "case", "throw", "try", "catch", "delete", "new"}
ZERO_LITERAL = re.compile(r"0[uU]?|0[xX]0+[uU]?")
NUMBER = re.compile(r"\d+[uU]?|0[xX][0-9A-Fa-f]+[uU]?")


def mask(text):
    """Same length as text. Comments and the inside of string/char literals become spaces; quotes and newlines stay."""
    out = list(text)
    i, n = 0, len(text)
    while i < n:
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            for k in range(i, j):
                out[k] = " "
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
        elif text[i] in "\"'":
            quote = text[i]
            j = i + 1
            while j < n and text[j] != quote and text[j] != "\n":
                j += 2 if text[j] == "\\" else 1
            for k in range(i + 1, min(j, n)):
                if out[k] != "\n":
                    out[k] = " "
            i = j + 1
        else:
            i += 1
    return "".join(out)


def match_close(text, i):
    """Index of the bracket that closes the one at text[i] (len(text) when it never closes)."""
    depth = 0
    for k in range(i, len(text)):
        c = text[k]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
            if depth == 0:
                return k
    return len(text)


def split_args(m, open_idx):
    """Top-level comma-separated argument spans (trimmed, on the masked text) of the call opened at open_idx."""
    close = match_close(m, open_idx)
    spans, depth, start = [], 0, open_idx + 1
    for k in range(open_idx + 1, close):
        c = m[k]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        elif c == "," and depth == 0:
            spans.append((start, k))
            start = k + 1
    spans.append((start, close))
    trimmed = []
    for a, b in spans:
        seg = m[a:b]
        trimmed.append((a + len(seg) - len(seg.lstrip()), b - (len(seg) - len(seg.rstrip()))))
    return trimmed, close


def classify(handler):
    """Status of a handler expression (masked text)."""
    h = handler.strip()
    head = re.match(r"(\w+)\s*\(", h)
    if head:
        name = head.group(1)
        if name in ("ignored", "accept"):
            return "ignored"
        if name in ("refuse", "nfc_no_device", "nfp_no_device"):
            return "error"
        return "implemented"
    if h.startswith("["):
        p = h.find("{")
        if p < 0:
            return "implemented"
        body = h[p + 1:match_close(h, p)]
        stmts = [s.strip() for s in body.split(";") if s.strip()]
        if not stmts:
            return "ignored"
        if len(stmts) == 1:
            r = re.fullmatch(r"return\s+(.+)", stmts[0], re.S)
            if r:
                expr = r.group(1).strip()
                if ZERO_LITERAL.fullmatch(expr):
                    return "ignored"
                if re.fullmatch(r"res\(.*\)|k[A-Z]\w*", expr, re.S):
                    return "error"
                if NUMBER.fullmatch(expr):
                    try:
                        if int(expr.rstrip("uU"), 0) != 0:
                            return "error"
                    except ValueError:
                        pass
    return "implemented"


def scan_file(path):
    rel = os.path.relpath(path, ROOT).replace(os.sep, "/")
    raw = open(path, encoding="utf-8", errors="replace").read()
    m = mask(raw)
    unparsed, cmds, names = [], [], []

    def line_of(pos):
        return m.count("\n", 0, pos) + 1

    def lit(q):
        """Literal text of the string whose opening quote is at index q."""
        e = m.find('"', q + 1)
        return raw[q + 1:e] if e > q else None

    def arg_lit(span):
        a, b = span
        if b - a >= 2 and m[a] == '"' and m[b - 1] == '"':
            return raw[a + 1:b - 1]
        return None

    # Top-level function definitions (column 0, depth 0 or inside one namespace).
    fns = []
    for mt in re.finditer(r"^[A-Za-z_][^\n]*", m, re.M):
        text = mt.group(0)
        if "(" not in text or text.rstrip().endswith(";"):
            continue
        first = re.match(r"\w+", text).group(0)
        if first in KEYWORDS:
            continue
        fm = re.search(r"(\w+)\s*\(", text)
        if fm:
            fns.append((mt.start(), fm.group(1)))
    fn_pos = [p for p, _ in fns]

    # Interface scopes: a make_iface(...) call spans its whole argument list, lambda included.
    scopes = []
    for mt in re.finditer(r"\bmake_iface\(\s*(\"|\w+\s*,)", m):
        name = lit(mt.end() - 1) if mt.group(1) == '"' else None
        _, close = split_args(m, m.index("(", mt.start()))
        scopes.append((mt.start(), close, name))
    shared = []  # make_shared<ipc::Service>("name"): applies until the next one or the function's end
    for mt in re.finditer(r"make_shared\s*<\s*(?:ipc::)?Service\s*>\(\s*\"", m):
        shared.append((mt.start(), lit(mt.end() - 1)))
    # make_iface("name", setup_fn): the setup function's commands belong to that name.
    setup_names = {}
    for mt in re.finditer(r"make_iface\(\s*\"([^\"]*)\"\s*,\s*(\w+)\s*\)", raw):
        setup_names[mt.group(2)] = mt.group(1)

    def resolve(pos):
        i = bisect.bisect_right(fn_pos, pos) - 1
        fn, fstart = (fns[i][1], fns[i][0]) if i >= 0 else ("(top)", 0)
        inner = [s for s in scopes if s[0] < pos < s[1]]
        if inner:
            s = max(inner, key=lambda x: x[0])
            return s[2] or fn
        prev = [s for s in shared if fstart <= s[0] < pos]
        if prev:
            return max(prev)[1] or fn
        return setup_names.get(fn, fn)

    # add_cmd(id, "Name", handler) and cmd(svc, "service", id, "Name", handler).
    for mt in re.finditer(r"\badd_cmd\(|\bcmd\(", m):
        start = mt.start()
        before = m[m.rfind("\n", 0, start) + 1:start]
        if mt.group(0) == "cmd(" and re.search(r"\w\s+$", before):
            continue  # a definition such as "void cmd("
        spans, _ = split_args(m, m.index("(", start))
        line = line_of(start)
        is_helper = mt.group(0) == "cmd("
        need = 5 if is_helper else 3
        if len(spans) < need:
            unparsed.append((rel, line, "call with too few arguments"))
            continue
        id_span, name_span = (spans[2], spans[3]) if is_helper else (spans[0], spans[1])
        id_text = raw[id_span[0]:id_span[1]]
        name = arg_lit(name_span)
        if not NUMBER.fullmatch(id_text.strip()) or name is None:
            unparsed.append((rel, line, "non-literal id or name (loop or helper; see rows and list entries)"))
            continue
        handler = m[spans[4][0]:spans[4][1]] if is_helper else m[spans[2][0]:spans[2][1]]
        cmds.append({"iface": resolve(start), "id": id_text.strip(), "name": name,
                     "status": classify(handler), "line": line})

    # rows(s, "service", { {id, "Name", ...}, ... }): table-driven commands, not resolved per row.
    for mt in re.finditer(r"\brows\(\s*\w+\s*,\s*\"", m):
        open_idx = m.index("(", mt.start())
        _, close = split_args(m, open_idx)
        for pm in re.finditer(r"\{\s*(\d+|0[xX][0-9A-Fa-f]+)\s*,\s*\"", m[open_idx:close]):
            q = open_idx + pm.end() - 1
            cmds.append({"iface": resolve(q), "id": pm.group(1), "name": lit(q), "status": "(loop)",
                         "line": line_of(q)})

    # CmdNames tables (vector of {id, "Name"}) fed to make_tma(...): registered by a loop.
    for mt in re.finditer(r"\bCmdNames\s+(\w+)\s*\(\)\s*\{", m):
        fname = mt.group(1)
        close = match_close(m, mt.end() - 1)
        ref = re.search(r"make_tma\(\s*\"([^\"]*)\"\s*,\s*" + fname + r"\(\)", raw)
        iface = ref.group(1) if ref else fname
        for pm in re.finditer(r"\{\s*(\d+|0[xX][0-9A-Fa-f]+)\s*,\s*\"", m[mt.end():close]):
            q = mt.end() + pm.end() - 1
            cmds.append({"iface": iface, "id": pm.group(1), "name": lit(q), "status": "(loop)", "line": line_of(q)})

    # add("service", ...): service names registered with the service manager.
    for mt in re.finditer(r"\badd\(\s*\"", m):
        names.append(lit(mt.end() - 1))

    cmds.sort(key=lambda c: c["line"])
    return {"file": rel, "cmds": cmds, "names": names, "unparsed": unparsed}


def cell(text):
    return str(text).replace("|", "\\|")


def render(results):
    out = []
    w = out.append
    total = {"implemented": 0, "ignored": 0, "error": 0, "(loop)": 0}
    for r in results:
        for c in r["cmds"]:
            total[c["status"]] = total.get(c["status"], 0) + 1
    all_unparsed = sum(len(r["unparsed"]) for r in results)
    w("# Service command table (S3c)")
    w("")
    w("Generated by `scripts/svc_table.py` from `src/services/*.cpp`. Do not edit by hand: change the source or the "
      "script and regenerate with `python -I scripts\\svc_table.py --out docs\\S3c_commands.md`.")
    w("")
    w("Status: **implemented** = real handler; **ignored** = answers success and does nothing (`ignored()`, `accept()`, "
      "an empty lambda or one that only returns 0); **error** = answers a non-zero result (`refuse()`, "
      "`nfc_no_device()`, `nfp_no_device()`, a lambda that only returns a non-zero constant or `res(...)`); "
      "**(loop)** = registered from a literal table (`rows(...)` or a `*CmdNames` list), handler not resolved per row.")
    w("")
    w(f"Totals: {sum(total.values())} registrations listed: {total['implemented']} implemented, "
      f"{total['ignored']} ignored, {total['error']} error, {total['(loop)']} (loop); "
      f"{all_unparsed} unparsed (see the end).")
    w("")
    w("## Service names registered with add(\"name\", ...)")
    w("")
    w("| service | file |")
    w("|---|---|")
    for r in results:
        for n in r["names"]:
            w(f"| `{cell(n)}` | `{r['file']}` |")
    w("")
    empty = []
    for r in results:
        if not r["cmds"] and not r["names"]:
            empty.append(r["file"])
            continue
        counts = {"implemented": 0, "ignored": 0, "error": 0, "(loop)": 0}
        for c in r["cmds"]:
            counts[c["status"]] = counts.get(c["status"], 0) + 1
        w(f"## `{r['file']}`")
        w("")
        w(f"{len(r['cmds'])} registrations: {counts['implemented']} implemented, {counts['ignored']} ignored, "
          f"{counts['error']} error, {counts['(loop)']} (loop), {len(r['unparsed'])} unparsed.")
        if r["names"]:
            w("")
            w("Service names: " + ", ".join(f"`{cell(n)}`" for n in r["names"]) + ".")
        w("")
        if r["cmds"]:
            w("| interface | id | name | status |")
            w("|---|---|---|---|")
            for c in r["cmds"]:
                w(f"| {cell(c['iface'])} | {cell(c['id'])} | {cell(c['name'])} | {c['status']} |")
        else:
            w("No command registrations in this file.")
        w("")
    if empty:
        w("Files with no registrations: " + ", ".join(f"`{f}`" for f in empty) + ".")
        w("")
    w("## unparsed")
    w("")
    if all_unparsed == 0:
        w("None.")
    for r in results:
        for file, line, why in r["unparsed"]:
            w(f"- `{file}:{line}`: {why}")
    w("")
    return "\n".join(out)


def main():
    ap = argparse.ArgumentParser(description="Command table for src/services (Markdown).")
    ap.add_argument("--out", help="write the Markdown to this file instead of stdout")
    args = ap.parse_args()
    results = []
    for path in sorted(glob.glob(os.path.join(SRC, "*.cpp"))):
        try:
            results.append(scan_file(path))
        except Exception as e:  # keep going: report the file instead of stopping
            rel = os.path.relpath(path, ROOT).replace(os.sep, "/")
            results.append({"file": rel, "cmds": [], "names": [], "unparsed": [(rel, 0, f"scan failed: {e}")]})
    text = render(results)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as f:
            f.write(text)
        print(f"wrote {args.out}: {sum(len(r['cmds']) for r in results)} registrations", file=sys.stderr)
    else:
        sys.stdout.reconfigure(encoding="utf-8")
        sys.stdout.write(text)


if __name__ == "__main__":
    main()
