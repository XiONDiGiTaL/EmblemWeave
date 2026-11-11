#!/usr/bin/env bash
# Differential verification of recovered control flow.
#
# 1. recover functions from the real decompressed text (with rodata mapped, so
#    jump tables can be read)
# 2. have the reference interpreter run the N largest functions that terminate,
#    recording the exact register state each must produce
# 3. emit one C program that runs the same functions, compile it, run it
# 4. compare every recorded value
#
# Usage: tools/verify_cfg.sh [text.bin] [rodata.bin] [rodata_addr] [count]
set -euo pipefail

TEXT="${1:-work/decoded/main/text.bin}"
RODATA="${2:-work/decoded/main/rodata.bin}"
RODATA_ADDR="${3:-0x1a46000}"
COUNT="${4:-400}"
MODULE="app"
OUT="build/cfg"
PY="${PY:-./.venv/Scripts/python.exe}"

if [ ! -f "$TEXT" ]; then
    echo "missing $TEXT (run tools/zbic_extract.py first)" >&2
    exit 1
fi

echo "== recover control flow and record reference states =="
cargo run -q -p ew-lift --bin ew-lift -- \
    --text "$TEXT" --segment "${RODATA_ADDR}:${RODATA}" \
    --module "$MODULE" --out "$OUT" \
    --limit 4000 --max-insns 4096 --campaign "$COUNT" | tee "$OUT/lift.txt"

echo "== compile =="
$PY -m ziglang cc -std=c99 -Wall -Wextra -Wno-unused-parameter -I "$OUT" \
    "$OUT/campaign.c" -o "$OUT/campaign.exe"

echo "== run lifted C =="
"$OUT/campaign.exe" | tr -d '\r' > "$OUT/native.txt"

echo "== compare =="
if diff -u "$OUT/expected.txt" "$OUT/native.txt" > "$OUT/diff.txt"; then
    echo "OK: $(wc -l < "$OUT/expected.txt") functions, every compared register identical"
else
    echo "MISMATCH:"
    head -40 "$OUT/diff.txt"
    exit 1
fi
