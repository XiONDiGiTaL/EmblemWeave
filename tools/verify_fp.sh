#!/usr/bin/env bash
# Differential verification of the scalar floating point families.
#
# The retail code is full of floating point, so the emitter has to be checked
# against real compiler output rather than against the decoder's own idea of an
# encoding:
#
# 1. compile tools/fp_probe.c for AArch64 with clang
# 2. take the .text section as a flat guest image
# 3. lift it and run the reference interpreter over every function that
#    completes, recording the exact register state each must produce
# 4. compile the emitted C, run it, compare every register bit for bit
#
# Usage: tools/verify_fp.sh [count]
set -euo pipefail

COUNT="${1:-4}"
OUT="build/fp"
PY="${PY:-./.venv/Scripts/python.exe}"
CC="${CC:-$PY -m ziglang cc}"

mkdir -p "$OUT"

echo "== compile the probe for AArch64 =="
$CC -target aarch64-freestanding -O2 -ffreestanding -fno-math-errno -fno-stack-protector \
    -c tools/fp_probe.c -o "$OUT/fp_probe.o"
$PY tools/elf_text.py "$OUT/fp_probe.o" "$OUT/fp.bin"

echo "== lift and record reference states =="
cargo run -q --release -p ew-lift --bin ew-lift -- \
    --text "$OUT/fp.bin" --module fp --out "$OUT" \
    --limit 8 --max-insns 4096 --campaign "$COUNT" | tee "$OUT/lift.txt"

echo "== compile the emitted C =="
$CC -std=c99 -Wall -Wextra -Wno-unused-parameter -I "$OUT" \
    "$OUT/campaign.c" -o "$OUT/campaign.exe"

echo "== run lifted C =="
"$OUT/campaign.exe" | tr -d '\r' > "$OUT/native.txt"

echo "== compare every register =="
if diff -u "$OUT/expected.txt" "$OUT/native.txt" > "$OUT/diff.txt"; then
    echo "OK: $(wc -l < "$OUT/expected.txt") functions, every register identical"
else
    echo "MISMATCH:"
    head -20 "$OUT/diff.txt"
    exit 1
fi

echo "== confirm the probe actually exercised floating point =="
python_count=$(grep -c "ew_[sd]get\|ew_fp_" "$OUT/campaign.c" || true)
if [ "$python_count" -lt 10 ]; then
    echo "only $python_count floating point operations reached the emitted C" >&2
    exit 1
fi
echo "OK: $python_count floating point operations in the emitted C"
