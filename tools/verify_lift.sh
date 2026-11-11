#!/usr/bin/env bash
# End-to-end verification of the lift pipeline against a real dump.
#
# 1. lift blocks from the decompressed game text
# 2. run the reference interpreter on the largest register-only block
# 3. compile the emitted C and execute the same block natively
# 4. compare the two register dumps
#
# Usage: tools/verify_lift.sh [text.bin] [module]
set -euo pipefail

TEXT="${1:-work/decoded/main/text.bin}"
MODULE="${2:-app}"
OUT="build/lift"
CC="${CC:-zig cc}"

if [ ! -f "$TEXT" ]; then
    echo "missing $TEXT — run tools/zbic_extract.py first" >&2
    exit 1
fi

echo "== lift =="
cargo run -q -p ew-lift --bin ew-lift -- \
    --text "$TEXT" --module "$MODULE" --out "$OUT" \
    --limit 4000 --pure-leaf --harness --interpret | tee "$OUT/interpret.txt"

echo "== compile =="
$CC -std=c99 -Wall -Wextra -Wno-unused-parameter -I "$OUT" \
    "$OUT/${MODULE}_lift.c" "$OUT/harness.c" -o "$OUT/harness.exe"

echo "== run lifted C =="
"$OUT/harness.exe" | tee "$OUT/native.txt"

echo "== compare =="
# Interpreter lines look like "  x0=0x1000"; native lines like "x0=1000".
python - "$OUT/interpret.txt" "$OUT/native.txt" <<'PY'
import sys

import re

# The interpreter prints 0x-prefixed values; the C harness prints bare hex for
# registers. Both are read as hexadecimal regardless of the prefix.
# `true|false` must be tried before bare hex, or "false" parses as 0xfa.
TOKEN = re.compile(r"\b(x\d{1,2}|pc|nzcv|halted)=(0x[0-9a-fA-F]+|true|false|[0-9a-fA-F]+)")

def parse(path):
    out = {}
    for line in open(path, encoding="utf-8", errors="replace"):
        for key, val in TOKEN.findall(line):
            if val == "true":
                out[key] = 1
            elif val == "false":
                out[key] = 0
            else:
                out[key] = int(val[2:] if val.startswith("0x") else val, 16)
    return out

interp = parse(sys.argv[1])
native = parse(sys.argv[2])
checked = 0
mismatch = 0
for key in ("halted", "pc", "nzcv") + tuple(f"x{i}" for i in range(8)):
    if key in interp and key in native:
        checked += 1
        if interp[key] != native[key]:
            mismatch += 1
            print(f"MISMATCH {key}: interpreter=0x{interp[key]:x} native=0x{native[key]:x}")
print(f"{checked} values compared, {mismatch} mismatch(es)")
sys.exit(1 if mismatch or checked == 0 else 0)
PY

echo "OK: lifted C matches the reference interpreter"
