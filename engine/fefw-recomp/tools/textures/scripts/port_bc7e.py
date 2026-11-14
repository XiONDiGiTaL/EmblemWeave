#!/usr/bin/env python3
"""Translates bc7e.ispc (bc7enc_rdo, Binomial LLC, Apache-2.0) to scalar C++: the same BC7 encoder without the ISPC compiler.

    python -I tools/textures/scripts/port_bc7e.py [third_party/bc7enc_rdo/bc7e.ispc] [out_dir (default tools/textures/src)]

Writes bc7e_cpp.h and bc7e_cpp.cpp (namespace bc7e_cpp, the API of the header ISPC generates). One program instance of the
ISPC code encodes one block, so the scalar code is the ISPC code run for one block at a time:
  * the uniform / varying qualifiers and `export` are dropped; foreach becomes a for loop over the blocks, cif / cfor plain if / for;
  * ISPC's `uint16_t` is a 64-bit type in bc7e.ispc (its typedef); it stays 64-bit (bc7e_u16);
  * unsuffixed floating constants are float in ISPC: they get an f suffix (C++ would compute in double);
  * the uniform / varying overload pairs become identical C++ functions: the later copy of each is dropped (the bodies are
    checked to be the same);
  * min, max, clamp, select, floor, sqrt, abs and all get scalar definitions.
The output does not depend on the ISPC compiler in any way. Rerun the script when bc7e.ispc changes; do not edit its output.
"""
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
SRC = Path(sys.argv[1]) if len(sys.argv) > 1 else HERE.parent / "third_party" / "bc7enc_rdo" / "bc7e.ispc"
OUT = Path(sys.argv[2]) if len(sys.argv) > 2 else HERE.parent / "src"

PRELUDE = """\
typedef uint64_t bc7e_u16;   // bc7e.ispc: typedef unsigned int64 uint16_t

static inline int32_t min(int32_t a, int32_t b) { return a < b ? a : b; }
static inline uint32_t min(uint32_t a, uint32_t b) { return a < b ? a : b; }
static inline int64_t min(int64_t a, int64_t b) { return a < b ? a : b; }
static inline uint64_t min(uint64_t a, uint64_t b) { return a < b ? a : b; }
static inline uint8_t min(uint8_t a, uint8_t b) { return a < b ? a : b; }
static inline float min(float a, float b) { return a < b ? a : b; }
static inline int32_t max(int32_t a, int32_t b) { return a > b ? a : b; }
static inline uint32_t max(uint32_t a, uint32_t b) { return a > b ? a : b; }
static inline int64_t max(int64_t a, int64_t b) { return a > b ? a : b; }
static inline uint64_t max(uint64_t a, uint64_t b) { return a > b ? a : b; }
static inline uint8_t max(uint8_t a, uint8_t b) { return a > b ? a : b; }
static inline float max(float a, float b) { return a > b ? a : b; }
static inline int32_t clamp(int32_t v, int32_t lo, int32_t hi) { return min(max(v, lo), hi); }
static inline uint32_t clamp(uint32_t v, uint32_t lo, uint32_t hi) { return min(max(v, lo), hi); }
static inline int64_t clamp(int64_t v, int64_t lo, int64_t hi) { return min(max(v, lo), hi); }
static inline uint64_t clamp(uint64_t v, uint64_t lo, uint64_t hi) { return min(max(v, lo), hi); }
static inline float clamp(float v, float lo, float hi) { return min(max(v, lo), hi); }
template <typename T> static inline T select(bool c, T a, T b) { return c ? a : b; }
static inline float floor(float x) { return floorf(x); }
static inline float sqrt(float x) { return sqrtf(x); }
static inline int32_t abs(int32_t x) { return x < 0 ? -x : x; }
static inline float abs(float x) { return fabsf(x); }
static inline bool all(bool b) { return b; }
"""


def strip_comments_and_space(s):
    s = re.sub(r"//[^\n]*", "", s)
    s = re.sub(r"/\*.*?\*/", "", s, flags=re.S)
    s = re.sub(r"\s+", " ", s).strip()
    return re.sub(r"\s*([*&(),\[\]{};=<>+\-/])\s*", r"\1", s)


def top_level_functions(text):
    """(decl_start, body_end, signature, body) of every function definition at file scope."""
    out = []
    i, n, depth = 0, len(text), 0
    decl_start = 0
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        if c in "\"'":
            j = i + 1
            while j < n and text[j] != c:
                j += 2 if text[j] == "\\" else 1
            i = j + 1
            continue
        if depth == 0 and c == "#" and (i == 0 or text[i - 1] == "\n"):
            j = text.find("\n", i)
            i = n if j < 0 else j + 1
            decl_start = i
            continue
        if c == "{":
            if depth == 0:
                brace = i
                decl = text[decl_start:brace]
                code = strip_comments_and_space(decl)
                # a function definition: a parameter list, no initializer (tables are `= {`), not a struct
                is_func = code.endswith(")") and "=" not in code.split("(")[0] and not code.startswith("struct")
                k, d = i, 0
                while k < n:
                    if text[k] == "{":
                        d += 1
                    elif text[k] == "}":
                        d -= 1
                        if d == 0:
                            break
                    k += 1
                if is_func:
                    out.append((decl_start, k + 1, code, strip_comments_and_space(text[brace:k + 1])))
                i = k + 1
                decl_start = i
                continue
            depth += 1
        elif c == "}":
            depth -= 1
        elif c == ";" and depth == 0:
            decl_start = i + 1
        i += 1
    return out


def main():
    text = SRC.read_text(encoding="utf-8")

    # the ISPC integer typedefs (block at the top) and ISPC-only pragmas
    text, n = re.subn(r"^typedef unsigned int8 uint8_t;\n(?:typedef [^\n]*\n)*", "", text, flags=re.M)
    assert n == 1, "typedef block not found"
    text = re.sub(r"^[ \t]*#pragma ignore warning\([^)]*\)[ \t]*\n", "", text, flags=re.M)
    text = re.sub(r"\buint16_t\b", "bc7e_u16", text)
    text = re.sub(r"\bint64\b", "int64_t", text)
    text = re.sub(r"\b(?:uniform|varying)\b[ \t]*", "", text)
    text = re.sub(r"\bcif\b", "if", text)
    text = re.sub(r"\bcfor\b", "for", text)
    text, n = re.subn(r"\bforeach\s*\(\s*(\w+)\s*=\s*0\s*\.\.\.\s*(\w+)\s*\)", r"for (uint32_t \1 = 0; \1 < \2; \1++)", text)
    assert n == 1, "foreach not found"
    text = re.sub(r"^export\s+", "", text, flags=re.M)
    # unsuffixed floating constants are float in ISPC
    text = re.sub(r"(?<![\w.])((?:\d+\.\d*|\.\d+)(?:[eE][+-]?\d+)?|\d+[eE][+-]?\d+)(?![\w.])", r"\1f", text)

    # the parameter struct goes to the header
    m = re.search(r"^struct bc7e_compress_block_params\n\{.*?^\};\n", text, flags=re.M | re.S)
    assert m, "bc7e_compress_block_params not found"
    params_struct = m.group(0)
    text = text[:m.start()] + text[m.end():]

    # uniform / varying overload pairs are identical now: keep the first of each signature
    funcs = top_level_functions(text)
    seen = {}
    drop = []
    for start, end, sig, body in funcs:
        if sig in seen:
            if seen[sig] != body:
                sys.exit(f"overloads of {sig} differ in their bodies: translate by hand")
            drop.append((start, end))
        else:
            seen[sig] = body
    for start, end in reversed(drop):
        seg = text[start:end]
        start += len(seg) - len(seg.lstrip())   # keep the line breaks before the dropped copy
        if text.startswith("\n", end):
            end += 1
        text = text[:start] + text[end:]

    exported = [sig for _, _, sig, _ in funcs if re.match(r"void bc7e_compress_block", sig)]
    exported = list(dict.fromkeys(exported))
    assert any(s.startswith("void bc7e_compress_blocks(") for s in exported), "bc7e_compress_blocks not found"

    origin = ("bc7e.ispc of bc7enc_rdo (github.com/richgel999/bc7enc_rdo), Copyright (C) 2018-2020 Binomial LLC, Apache 2.0 license\n"
              "// (tools/textures/licenses/LICENSE.bc7enc_rdo), translated to scalar C++ by tools/textures/scripts/port_bc7e.py. Generated: do not edit.")
    header = ["// bc7e_cpp.h: the API of " + origin,
              "// bc7e_cpp.cpp is compiled once per instruction set into bc7e_cpp::<BC7E_CPP_VARIANT> (scalar: baseline x86-64, avx2: AVX2 + FMA);",
              "// the functions directly in bc7e_cpp pick one of them by CPUID (bc7e_cpp_dispatch.cpp).",
              "#pragma once", "#include <cstdint>", "", "#define BC7E_CPP_DECLARE_API \\"]
    for sig in exported:
        header.append("    " + sig.replace(",", ", ").replace("*", "* ") + "; \\")
    header += ["", "namespace bc7e_cpp {", "",
               "// in a namespace of its own: argument-dependent lookup inside the builds then finds no function of the dispatching API",
               "namespace param_types {", params_struct.rstrip(), "}  // namespace param_types",
               "using param_types::bc7e_compress_block_params;", "", "BC7E_CPP_DECLARE_API",
               "const char* bc7e_cpp_variant();   // \"avx2\" or \"scalar\": the build the functions above use", "",
               "namespace scalar { BC7E_CPP_DECLARE_API }", "namespace avx2 { BC7E_CPP_DECLARE_API }", "", "}  // namespace bc7e_cpp", ""]
    (OUT / "bc7e_cpp.h").write_text("\n".join(header), encoding="utf-8")

    cpp = ["// bc7e_cpp.cpp: " + origin,
           "#include \"bc7e_cpp.h\"", "",
           "#ifndef NDEBUG", "#define NDEBUG   // as the ISPC build (--opt=disable-assertions)", "#endif",
           "#include <assert.h>", "#include <math.h>", "#include <string.h>", "",
           "#ifndef BC7E_CPP_VARIANT", "#define BC7E_CPP_VARIANT scalar", "#endif", "",
           "namespace bc7e_cpp {", "namespace BC7E_CPP_VARIANT {", "", PRELUDE, text.strip(), "",
           "}  // namespace BC7E_CPP_VARIANT", "}  // namespace bc7e_cpp", ""]
    (OUT / "bc7e_cpp.cpp").write_text("\n".join(cpp), encoding="utf-8")
    print(f"{len(funcs)} functions, {len(drop)} duplicate overloads dropped, {len(exported)} exported -> {OUT}")


if __name__ == "__main__":
    main()
