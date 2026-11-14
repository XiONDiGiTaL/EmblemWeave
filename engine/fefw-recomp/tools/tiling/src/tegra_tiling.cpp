// tegra_tiling implementation. GPL-3.0-or-later. See include/tegra_tiling.h.
#include "tegra_tiling.h"

#include <algorithm>
#include <cstring>

namespace tegra {
namespace {

constexpr uint32_t kGobBytes = 512;
constexpr uint32_t kGobWidth = 64;
constexpr uint32_t kGobRows = 8;

uint64_t div_up(uint64_t a, uint64_t b) { return (a + b - 1) / b; }
uint64_t align_up(uint64_t a, uint64_t b) { return div_up(a, b) * b; }
bool pow2(uint32_t v) { return v && !(v & (v - 1)); }
uint32_t log2u(uint32_t v) { uint32_t n = 0; while (v > 1) { v >>= 1; n++; } return n; }

// Byte offset of the 16-byte piece `piece` (0..3, x / 16 inside the 64-byte row) of row `y` (0..7) inside a GOB.
inline uint32_t gob_piece_offset(uint32_t piece, uint32_t y, bool switch2) {
    if (switch2) return (piece >> 1) * 256 + (y >> 2) * 128 + (piece & 1) * 64 + ((y >> 1) & 1) * 32 + (y & 1) * 16;
    return (piece >> 1) * 256 + (piece & 1) * 32 + (y >> 1) * 64 + (y & 1) * 16;
}

template <bool Untile>
void convert_level(const TextureDesc& d, const LevelLayout& lv, uint8_t* tiled_rw, const uint8_t* tiled_ro,
                   uint8_t* linear_rw, const uint8_t* linear_ro) {
    const uint32_t bpb = d.bytes_per_block;
    const uint64_t row_bytes = (uint64_t)lv.blocks_x * bpb;
    const uint32_t gobs_x = (uint32_t)div_up(row_bytes, kGobWidth);
    const uint32_t mgy = lv.gob_blocks_y, mgz = lv.gob_blocks_z;
    const uint64_t block_bytes = (uint64_t)kGobBytes * mgy * mgz;           // one block of GOBs
    const uint64_t rob_size = gobs_x * block_bytes;                         // one row of blocks
    const uint64_t slice_size = div_up(lv.blocks_y, kGobRows * mgy) * rob_size;
    const uint32_t zs = log2u(mgz), ys = log2u(mgy);

    for (uint32_t z = 0; z < lv.depth; z++) {
        const uint64_t z_base = (uint64_t)(z >> zs) * slice_size;
        const uint32_t zm = z & (mgz - 1);
        for (uint32_t gy = 0; gy < div_up(lv.blocks_y, kGobRows); gy++) {   // GOB rows
            const uint64_t rob_base = z_base + (uint64_t)(gy >> ys) * rob_size;
            const uint32_t gob_in_block = zm * mgy + (gy & (mgy - 1));
            const uint32_t rows = std::min<uint32_t>(kGobRows, lv.blocks_y - gy * kGobRows);
            for (uint32_t gx = 0; gx < gobs_x; gx++) {
                const uint64_t gob = rob_base + (uint64_t)gx * block_bytes + (uint64_t)gob_in_block * kGobBytes;
                const uint64_t x0 = (uint64_t)gx * kGobWidth;
                const uint64_t valid = std::min<uint64_t>(kGobWidth, row_bytes - x0);  // bytes of this GOB row in the image
                for (uint32_t y = 0; y < rows; y++) {
                    const uint64_t lin = lv.linear_slice_size / lv.depth * z + (uint64_t)(gy * kGobRows + y) * lv.linear_row_stride + x0;
                    if (valid == kGobWidth) {
                        for (uint32_t p = 0; p < 4; p++) {
                            const uint64_t t = gob + gob_piece_offset(p, y, d.gob_switch2);
                            if (Untile) std::memcpy(linear_rw + lin + p * 16, tiled_ro + t, 16);
                            else std::memcpy(tiled_rw + t, linear_ro + lin + p * 16, 16);
                        }
                    } else {
                        for (uint32_t p = 0; p * 16 < valid; p++) {
                            const uint64_t t = gob + gob_piece_offset(p, y, d.gob_switch2);
                            const size_t n = (size_t)std::min<uint64_t>(16, valid - p * 16);
                            if (Untile) std::memcpy(linear_rw + lin + p * 16, tiled_ro + t, n);
                            else std::memcpy(tiled_rw + t, linear_ro + lin + p * 16, n);
                        }
                    }
                }
            }
        }
    }
}

}  // namespace

uint32_t default_gob_blocks_y(uint32_t block_rows) {
    uint32_t g = 1;
    while (g < 16 && g * 8 < block_rows) g <<= 1;
    return g;
}

Layout compute_layout(const TextureDesc& d) {
    Layout out;
    const bool bpb_ok = d.bytes_per_block == 1 || d.bytes_per_block == 2 || d.bytes_per_block == 4 ||
                        d.bytes_per_block == 8 || d.bytes_per_block == 12 || d.bytes_per_block == 16;
    const bool is3d = d.depth > 1 || d.gob_blocks_z > 1;
    if (!d.width || !d.height || !d.depth || !d.levels || !d.layers || !d.block_w || !d.block_h || !bpb_ok ||
        !pow2(d.gob_blocks_y) || !pow2(d.gob_blocks_z) || d.gob_blocks_y > 32 || d.gob_blocks_z > 32 ||
        !d.linear_stride_align || (is3d && d.layers != 1))
        return out;

    uint32_t mgy = d.gob_blocks_y, mgz = d.gob_blocks_z;
    uint64_t linear = 0, tiled = 0, tiled_untrimmed = 0;
    uint32_t base_gy = d.gob_blocks_y, base_gz = d.gob_blocks_z;  // after the level-0 reduction (layer alignment)
    for (uint32_t l = 0; l < d.levels; l++) {
        LevelLayout lv{};
        lv.width = std::max(1u, d.width >> l);
        lv.height = std::max(1u, d.height >> l);
        lv.depth = std::max(1u, d.depth >> l);
        lv.blocks_x = (uint32_t)div_up(lv.width, d.block_w);
        lv.blocks_y = (uint32_t)div_up(lv.height, d.block_h);
        // the block height halves while the level fits in half a block (cumulative); depth likewise, from level 1 on
        while (lv.blocks_y <= (mgy >> 1) * kGobRows && mgy != 1) mgy >>= 1;
        if (l > 0)
            if (lv.depth <= (mgz >> 1) && mgz != 1) mgz >>= 1;  // Ryujinx halves the depth block once per level, not in a loop
        lv.gob_blocks_y = mgy;
        lv.gob_blocks_z = mgz;
        if (l == 0) {
            base_gy = mgy;
            base_gz = mgz;
            // Ryujinx's layer alignment also applies the depth reduction at level 0
            while (d.depth <= (base_gz >> 1) && base_gz != 1) base_gz >>= 1;
        }

        const uint64_t row_bytes = (uint64_t)lv.blocks_x * d.bytes_per_block;
        lv.linear_row_stride = align_up(row_bytes, d.linear_stride_align);
        lv.linear_slice_size = lv.linear_row_stride * lv.blocks_y * lv.depth;
        lv.linear_offset = linear;
        linear += lv.linear_slice_size * d.layers;

        const uint64_t gobs_x = div_up(row_bytes, kGobWidth);
        const uint64_t rob = gobs_x * mgy * mgz * kGobBytes;
        const uint64_t slice = div_up(lv.blocks_y, kGobRows * mgy) * rob;
        const uint64_t full = div_up(lv.depth, mgz) * slice;
        uint64_t size = full;
        // a 3D texture's last level is cut short when its depth does not fill the last GOB block
        if (is3d && l == d.levels - 1 && lv.depth % mgz) size -= (uint64_t)mgy * kGobBytes * (mgz - lv.depth % mgz);
        lv.tiled_offset = tiled;
        lv.tiled_size = size;
        tiled += size;
        tiled_untrimmed += full;
        out.levels.push_back(lv);
    }
    out.linear_size = linear;
    if (d.layers > 1) {
        out.tiled_layer_stride = align_up(tiled, (uint64_t)base_gy * base_gz * kGobBytes);
        out.tiled_size = out.tiled_layer_stride * d.layers;
    } else {
        out.tiled_layer_stride = tiled_untrimmed;  // Ryujinx's LayerSize is the untrimmed sum, TotalSize the trimmed one
        out.tiled_size = tiled;
    }
    out.valid = true;
    return out;
}

void untile_level(const TextureDesc& d, const LevelLayout& lv, const uint8_t* tiled, uint8_t* linear) {
    convert_level<true>(d, lv, nullptr, tiled, linear, nullptr);
}

void tile_level(const TextureDesc& d, const LevelLayout& lv, const uint8_t* linear, uint8_t* tiled) {
    convert_level<false>(d, lv, tiled, nullptr, nullptr, linear);
}

bool untile(const TextureDesc& d, const Layout& l, const uint8_t* tiled, uint8_t* linear) {
    if (!l.valid || !tiled || !linear) return false;
    for (const LevelLayout& lv : l.levels)
        for (uint32_t layer = 0; layer < d.layers; layer++)
            untile_level(d, lv, tiled + layer * l.tiled_layer_stride + lv.tiled_offset,
                         linear + lv.linear_offset + layer * lv.linear_slice_size);
    return true;
}

bool tile(const TextureDesc& d, const Layout& l, const uint8_t* linear, uint8_t* tiled) {
    if (!l.valid || !tiled || !linear) return false;
    std::memset(tiled, 0, (size_t)l.tiled_size);
    for (const LevelLayout& lv : l.levels)
        for (uint32_t layer = 0; layer < d.layers; layer++)
            tile_level(d, lv, linear + lv.linear_offset + layer * lv.linear_slice_size,
                       tiled + layer * l.tiled_layer_stride + lv.tiled_offset);
    return true;
}

uint64_t tiled_address(const LevelLayout& lv, uint32_t bpb, uint32_t bx, uint32_t by, uint32_t z, uint32_t byte) {
    const uint64_t x = (uint64_t)bx * bpb + byte;  // byte x
    const uint32_t mgy = lv.gob_blocks_y, mgz = lv.gob_blocks_z;
    const uint64_t gobs_x = div_up((uint64_t)lv.blocks_x * bpb, kGobWidth);
    const uint64_t rob = gobs_x * mgy * mgz * kGobBytes;
    const uint64_t slice = div_up(lv.blocks_y, kGobRows * mgy) * rob;
    const uint32_t yh = by / kGobRows;
    return (uint64_t)(z / mgz) * slice + (uint64_t)(yh / mgy) * rob + (x / 64) * ((uint64_t)kGobBytes * mgy * mgz) +
           (uint64_t)((z % mgz) * mgy + yh % mgy) * kGobBytes + ((x & 63) >> 5) * 256 + ((by & 7) >> 1) * 64 +
           ((x & 31) >> 4) * 32 + (by & 1) * 16 + (x & 15);
}

}  // namespace tegra
