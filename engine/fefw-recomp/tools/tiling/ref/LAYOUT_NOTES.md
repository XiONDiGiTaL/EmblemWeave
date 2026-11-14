# Tegra block-linear layout, as implemented by Ryujinx

Source (read-only): `refs\ryubing\src\Ryujinx.Graphics.Texture` (MIT). Files read:
`SizeCalculator.cs`, `SizeInfo.cs`, `LayoutConverter.cs`, `BlockLinearLayout.cs`, `BlockLinearConstants.cs`.
Ryujinx's `Ryujinx.Common/Utilities/BitUtils.cs` supplies AlignUp, AlignDown and DivRoundUp; they are used with power-of-two alignments only.

Verified by: `tools\tiling\scripts\verify_layout.py`. It re-derives every address from the rules below, without calling Ryujinx,
and checks all 332 cases in `tools\tiling\cases`: 20,455,125 blocks compared, 0 mismatches, sizes and offsets in `cases.tsv` equal, and every tiled byte that no block maps to is zero.
Negative control: flipping one tiled byte makes the verifier fail, so it is not vacuous.

Notation: `W,H,D` texels. `BW,BH` block size in texels. `BPB` bytes per block. `GY,GZ` gob_blocks_y and gob_blocks_z. Tile-X is fixed to 1.
For level `L`: `w = max(1, W>>L)`, `h = max(1, H>>L)`, `d = max(1, D>>L)`, and `wb = ceil(w/BW)`, `hb = ceil(h/BH)` (block counts). `d` is never divided by block size.

## 1. Linear buffer

Used as the input to `ConvertLinearToBlockLinear` and the output of `ConvertBlockLinearToLinear`.

- Row pitch per level: `stride_L = align4(wb * BPB)`. Rows are padded to a 4-byte multiple.
- One slice (one layer of one level): `slice_L = stride_L * hb * d`.
- Level order: levels are stored back to back, level 0 first, with no alignment between them. `levelBase_0 = 0`, `levelBase_(L+1) = levelBase_L + layers * slice_L`.
- Layer order: inside a level, all layers are contiguous. Layer `k` starts at `levelBase_L + k * slice_L`.
- Row order inside a slice: z-major, then y. The byte offset of block `(x, y, z)` is `levelBase_L + k*slice_L + (z*hb + y)*stride_L + x*BPB`.
- Row padding (`wb*BPB` up to `stride_L`) is never read on input. On the reverse output it is not written, so it holds garbage; the round-trip check compares only valid bytes.
- Total: `linear_size = layers * sum_L(slice_L)` (Ryujinx `GetTextureSize`).

## 2. Tiled buffer (block-linear)

Produced by `SizeCalculator.GetBlockLinearTextureSize`, which returns a `SizeInfo`. Its `GetMipOffset(L)` and `LayerSize` and `TotalSize` are the values in `cases.tsv`.

Per-level GOB counts (these are cumulative: once a value shrinks it stays shrunk for higher levels):
- `mgy` starts at GY. For each level, while `hb <= (mgy>>1)*8` and `mgy != 1`, halve `mgy`.
- `mgz` starts at GZ. For level > 0 only, `if d <= (mgz>>1)` and `mgz != 1`, halve `mgz` ONCE (an `if`, not a loop, unlike `mgy`; the first version of this note said "while" and was wrong, see tools/docs/debug/003.md). Level 0 never shrinks `mgz`.

Per level, inside one layer:
- `widthInGobs = ceil(wb*BPB / 64)`. Tile-X = 1, so no width alignment is applied.
- `robSize = widthInGobs * mgy * mgz * 512` (one row of GOB blocks).
- `sliceSize = ceil(hb / (8*mgy)) * robSize`.
- `levelSize = ceil(d / mgz) * sliceSize`.
- `mipOffset_L = sum of earlier levelSize`, measured from the start of the layer. No alignment between levels.

Layers:
- If layers = 1: `LayerSize = sum of levelSize` and `TotalSize = sum of levelSize`.
- If layers > 1 (2D only in this matrix): `LayerSize = roundUp(sum of levelSize, g_y*g_z*512)`, where `g_y` and `g_z` are GY and GZ after the base-level reduction (`hb <= (g_y>>1)*8` and `D <= (g_z>>1)`, both only while not 1). `TotalSize = LayerSize * layers`.
- Layer `k` starts at `k * LayerSize`.

3D (`is3D = D > 1 || GZ > 1`):
- Layers are 1 in this matrix. For 3D with layers > 1 nothing is tested.
- The last level is trimmed when `d % mgz != 0`: `levelSize -= mgy*512*(mgz - d % mgz)`. `TotalSize` uses the trimmed value, so it can be smaller than the untrimmed sum. `LayerSize` (`tiled_layer_stride` in cases.tsv) is still the untrimmed sum. Both are recorded in cases.tsv.
- Depth slices are grouped by `mgz` in z, then stacked as GOBs inside each z group (see the formula below).

## 3. Address of a block inside a level (tiled)

Let `x` be the byte x of the block (`x = blockX * BPB`), `y` the block row, `z` the slice. `yh = y / 8`, `zm = z mod mgz`, `ym = yh mod mgy`.

```
off = (z >> log2(mgz)) * sliceSize
    + (yh >> log2(mgy)) * robSize
    + (x / 64) << log2(512 * mgy * mgz)
    + (zm * mgy + ym) * 512
    + ((x & 63) >> 5) * 256 + ((y & 7) >> 1) * 64 + ((x & 31) >> 4) * 32 + (y & 1) * 16 + (x & 15)
```

Address of the block in the file: `layer * LayerSize + mipOffset_L + off`.

In words: a GOB is 64 bytes by 8 rows (512 bytes). Inside it, rows are paired (`y>>1` in 64-byte steps), each 32-byte half is interleaved, and each 16-byte piece is contiguous. GOBs are stacked by `mgy` in y, then `mgz` in z, and blocks of GOBs are placed side by side in x.

The address is derived from `BlockLinearLayout.GetOffsetWithLineOffset` (converter), which matches this formula.

## 4. Ryujinx functions used (namespace `Ryujinx.Graphics.Texture`)

- `SizeCalculator.GetBlockLinearTextureSize(width, height, depth, levels, layers, blockW, blockH, bpb, gobY, gobZ, tileX)` (line 24). Returns `SizeInfo`.
- `LayoutConverter.ConvertLinearToBlockLinear(output, width, height, depth, sliceDepth, levels, layers, blockW, blockH, bpb, gobY, gobZ, tileX, sizeInfo, linear)` (line 373). `sliceDepth = depth`.
- `LayoutConverter.ConvertBlockLinearToLinear(width, height, depth, sliceDepth, levels, layers, blockW, blockH, bpb, gobY, gobZ, tileX, sizeInfo, tiled)` (line 98). Returns `MemoryOwner<byte>`, which is a pooled array and not zeroed, so the padding bytes are garbage.
- Ryujinx's own layout loops (`LayoutConverter.cs` lines 188 and 456): `inBaseOffset`/`outBaseOffset = base + layer * sizeInfo.LayerSize + sizeInfo.GetMipOffset(level)`. This is the layer-plus-level addressing used above.

The converter only accepts `bpb` in {1, 2, 4, 8, 12, 16} (`NotSupportedException` otherwise). This matrix uses {1, 2, 4, 8, 16}. Ryujinx rejected 0 of 332 candidates.

## 5. Matrix covered (default)

- BPB 1, 2, 4, 8, 16 with 1x1 blocks, gob_blocks_y in {1, 2, 4, 8, 16, 32}.
- Block sizes: 4x4 at BPB 8 and 16, ASTC 5x5, 6x6, 8x8, 10x10, 12x12 at BPB 16.
- Sizes: 1x1, 7x5, 33x65, 100x37, 256x256, 300x300, 1024x512, 1920x1080.
- Single level and full mip chains. Layers 1, 2, 6 (2D only).
- 3D: depth 4, 9, 16 with gob_blocks_z in {1, 2, 4, 8, 16}. Plus depth 1 with gob_blocks_z > 1 (is_3d = 1). Plus two 3D block-compressed cases.

## 6. Not covered

- tileX > 1 (`gobBlocksInTileX`), which changes width alignment and layer rounding. Always 1 here.
- 3D with layers > 1.
- BPB 12 (`Bpp12Pixel`).
- Linear pitch textures (`ConvertLinearStridedToLinear`) and the `sliceDepth != depth` path.
- Pixel formats themselves: only bytes per block and block dimensions are passed.
