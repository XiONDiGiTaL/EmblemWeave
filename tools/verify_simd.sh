#!/usr/bin/env bash
# Differential verification of the Advanced SIMD unit.
#
# Every distinct SIMD encoding in the module is executed once per seeded
# register file (NaNs, infinities, signed zeros, subnormals, integer extremes)
# by the reference interpreter and by the emitted C, and every register of
# every run is compared.
#
# Usage: tools/verify_simd.sh [text.bin] [count]
set -euo pipefail

TEXT="${1:-work/decoded/main/text.bin}"
COUNT="${2:-100000}"
OUT="build/simd"
PY="${PY:-./.venv/Scripts/python.exe}"

if [ ! -f "$TEXT" ]; then
    echo "missing $TEXT (run tools/zbic_extract.py first)" >&2
    exit 1
fi
mkdir -p "$OUT"

echo "== record reference states =="
cargo run -q --release -p ew-lift --bin ew-lift -- \
    --text "$TEXT" --module app --out "$OUT" --limit 1 --simd-campaign "$COUNT" | grep "simd campaign"

echo "== compile =="
$PY -m ziglang cc -std=c99 -O1 -Wall -Wextra -Wno-unused-parameter -Wno-unused-function \
    -I "$OUT" "$OUT/simd_campaign.c" -o "$OUT/simd_campaign.exe" -lm

echo "== run lifted C =="
"$OUT/simd_campaign.exe" | tr -d '\r' > "$OUT/native.txt"

echo "== compare =="
if diff -u "$OUT/simd_expected.txt" "$OUT/native.txt" > "$OUT/diff.txt"; then
    echo "OK: $(wc -l < "$OUT/simd_expected.txt") SIMD runs, every compared register identical"
else
    echo "MISMATCH: $(grep -c '^-fn' "$OUT/diff.txt") of $(wc -l < "$OUT/simd_expected.txt") runs differ"
    head -12 "$OUT/diff.txt"
    exit 1
fi
