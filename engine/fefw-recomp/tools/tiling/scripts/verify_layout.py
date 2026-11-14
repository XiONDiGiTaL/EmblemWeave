#!/usr/bin/env python3
# MIT License.
# Independent cross-check of LAYOUT_NOTES.md against the Ryujinx output in tools/tiling/cases.
# It re-derives every offset from the written rules (no Ryujinx call), checks the cases.tsv
# offsets/sizes, and checks every block of every case: tiled bytes at the derived address must equal the
# linear bytes at the derived address. Bytes of the tiled buffer that no block maps to must be zero.
#
# Usage: python verify_layout.py <cases_dir>
import csv
import os
import sys

import numpy as np

GOB = 512


def ceil_div(a, b):
    return -(-a // b)


def lg2(v):
    return int(v).bit_length() - 1


def model(c):
    W, H, D = c["W"], c["H"], c["D"]
    levels, layers = c["levels"], c["layers"]
    bw, bh, bpb = c["bw"], c["bh"], c["bpb"]
    gy0, gz0 = c["gy"], c["gz"]
    is3d = D > 1 or gz0 > 1

    mgy, mgz = gy0, gz0
    layer_unaligned = 0  # Ryujinx layerSize (trimmed last level for 3D)
    layer_aligned = 0    # Ryujinx layerSizeAligned (untrimmed)
    mip_off, lv = [], []
    for L in range(levels):
        w = max(1, W >> L)
        h = max(1, H >> L)
        d = max(1, D >> L)
        wb = ceil_div(w, bw)
        hb = ceil_div(h, bh)
        while hb <= (mgy >> 1) * 8 and mgy != 1:
            mgy >>= 1
        if L > 0 and d <= (mgz >> 1) and mgz != 1:
            mgz >>= 1
        wig = ceil_div(wb * bpb, 64)
        rob = wig * mgy * mgz * GOB
        slice_ = ceil_div(ceil_div(hb, 8), mgy) * rob
        lvl_size = ceil_div(d, mgz) * slice_
        mip_off.append(layer_unaligned)
        layer_aligned += lvl_size
        if is3d and d % mgz != 0 and L == levels - 1:
            lvl_size -= mgy * GOB * (mgz - d % mgz)
        layer_unaligned += lvl_size
        lv.append(dict(mgy=mgy, mgz=mgz, rob=rob, slice=slice_, wb=wb, hb=hb, d=d))

    if layers > 1:
        g_y, g_z = gy0, gz0
        hh = ceil_div(H, bh)
        while hh <= (g_y >> 1) * 8 and g_y != 1:
            g_y >>= 1
        while D <= (g_z >> 1) and g_z != 1:
            g_z >>= 1
        bog = g_y * g_z * GOB
        layer_stride = ceil_div(layer_aligned, bog) * bog
        total = layer_stride * layers
    else:
        layer_stride = layer_aligned
        total = layer_unaligned

    lin_stride, lin_slice, lin_base = [], [], []
    acc = 0
    for L in range(levels):
        w = ceil_div(max(1, W >> L), bw)
        h = ceil_div(max(1, H >> L), bh)
        d = max(1, D >> L)
        stride = (w * bpb + 3) // 4 * 4
        sl = stride * h * d
        lin_stride.append(stride)
        lin_slice.append(sl)
        lin_base.append(acc)
        acc += sl * layers
    return dict(lv=lv, mip_off=mip_off, layer_stride=layer_stride, total=total,
                lin_stride=lin_stride, lin_slice=lin_slice, lin_base=lin_base, lin_total=acc)


def tiled_index(lv, bpb, X, Y, Z):
    xb = X * bpb
    yh = Y // 8
    gy, gz = lv["mgy"], lv["mgz"]
    off = (Z >> lg2(gz)) * lv["slice"] + (yh >> lg2(gy)) * lv["rob"]
    off = off + ((xb // 64) << lg2(GOB * gy * gz))
    off = off + ((Z & (gz - 1)) * gy + (yh & (gy - 1))) * GOB
    off = off + ((xb & 63) >> 5) * 256 + ((Y & 7) >> 1) * 64 + ((xb & 31) >> 4) * 32 + (Y & 1) * 16 + (xb & 15)
    return off


def read_case(cdir, row):
    c = dict(W=int(row["width"]), H=int(row["height"]), D=int(row["depth"]), levels=int(row["levels"]),
             layers=int(row["layers"]), bw=int(row["block_w"]), bh=int(row["block_h"]),
             bpb=int(row["bytes_per_block"]), gy=int(row["gob_blocks_y"]), gz=int(row["gob_blocks_z"]))
    name = row["id"]
    lin = np.fromfile(os.path.join(cdir, f"case_{name}.linear.bin"), dtype=np.uint8)
    til = np.fromfile(os.path.join(cdir, f"case_{name}.tiled.bin"), dtype=np.uint8)
    return name, c, lin, til


def check(cdir, row):
    name, c, lin, til = read_case(cdir, row)
    m = model(c)
    problems = []

    def want(col, val):
        if row[col] != val:
            problems.append(f"{col}: tsv={row[col]} model={val}")

    want("linear_size", str(m["lin_total"]))
    want("tiled_size", str(m["total"]))
    want("linear_level_offsets", ",".join(str(x) for x in m["lin_base"]))
    want("tiled_level_offsets", ",".join(str(x) for x in m["mip_off"]))
    want("tiled_layer_stride", str(m["layer_stride"]))
    if lin.size != m["lin_total"]:
        problems.append(f"linear file size {lin.size} != {m['lin_total']}")
    if til.size != m["total"]:
        problems.append(f"tiled file size {til.size} != {m['total']}")
    if problems:
        return name, problems, 0, 0

    bpb = c["bpb"]
    mark = np.zeros(m["total"], dtype=bool)
    blocks = 0
    bad = 0
    for layer in range(c["layers"]):
        for L in range(c["levels"]):
            lv = m["lv"][L]
            w, h, d = lv["wb"], lv["hb"], lv["d"]
            Z, Y, X = np.meshgrid(np.arange(d), np.arange(h), np.arange(w), indexing="ij")
            Z, Y, X = Z.ravel().astype(np.int64), Y.ravel().astype(np.int64), X.ravel().astype(np.int64)
            t_base = layer * m["layer_stride"] + m["mip_off"][L]
            t_idx = t_base + tiled_index(lv, bpb, X, Y, Z)
            l_idx = (m["lin_base"][L] + layer * m["lin_slice"][L]
                     + (Z * h + Y) * m["lin_stride"][L] + X * bpb)
            blocks += X.size
            for b in range(bpb):
                if t_idx.max() + b >= m["total"]:
                    problems.append("model address past TotalSize")
                    return name, problems, blocks, bad
                bad += int(np.count_nonzero(til[t_idx + b] != lin[l_idx + b]))
                mark[t_idx + b] = True
    unmarked = til[~mark]
    pad_nonzero = int(np.count_nonzero(unmarked))
    if bad:
        problems.append(f"{bad} byte mismatches")
    if pad_nonzero:
        problems.append(f"{pad_nonzero} nonzero bytes in unaddressed tiled padding")
    return name, problems, blocks, bad + pad_nonzero


def main():
    cdir = sys.argv[1]
    with open(os.path.join(cdir, "cases.tsv"), newline="") as f:
        rows = list(csv.DictReader(f, delimiter="\t"))
    fails = 0
    total_blocks = 0
    for row in rows:
        name, problems, blocks, _ = check(cdir, row)
        total_blocks += blocks
        if problems:
            fails += 1
            print(f"FAIL case {name}: " + "; ".join(problems))
    print(f"cases={len(rows)} failed={fails} blocks_checked={total_blocks}")
    return 1 if fails else 0


if __name__ == "__main__":
    sys.exit(main())
