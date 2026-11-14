// tegra_tiling tests. GPL-3.0-or-later.
//   tiling_test crosscheck <cases dir>   compare against Ryujinx's output (cases made by ref/RyuTilingRef)
//   tiling_test roundtrip [--seed N]     tile -> untile identity + independent address formula over a large matrix
//   tiling_test bench                    throughput of tile / untile
// Exit status 0 = all passed.
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

#include "tegra_tiling.h"

using namespace tegra;

static uint64_t g_rng = 88172645463325252ull;
static uint64_t xorshift() {
    g_rng ^= g_rng << 13;
    g_rng ^= g_rng >> 7;
    g_rng ^= g_rng << 17;
    return g_rng;
}

static std::vector<uint8_t> read_file(const std::string& p) {
    std::ifstream f(p, std::ios::binary);
    return std::vector<uint8_t>((std::istreambuf_iterator<char>(f)), std::istreambuf_iterator<char>());
}

static std::vector<std::string> split(const std::string& s, char c) {
    std::vector<std::string> out;
    std::stringstream ss(s);
    std::string item;
    while (std::getline(ss, item, c)) out.push_back(item);
    if (!s.empty() && s.back() == c) out.push_back("");
    return out;
}

// Compare only the bytes of the linear buffer that carry blocks (rows are padded).
static bool linear_valid_equal(const TextureDesc& d, const Layout& l, const uint8_t* a, const uint8_t* b) {
    for (const LevelLayout& lv : l.levels) {
        const uint64_t row = (uint64_t)lv.blocks_x * d.bytes_per_block;
        for (uint32_t layer = 0; layer < d.layers; layer++)
            for (uint32_t z = 0; z < lv.depth; z++)
                for (uint32_t y = 0; y < lv.blocks_y; y++) {
                    const uint64_t o = lv.linear_offset + layer * lv.linear_slice_size +
                                       ((uint64_t)z * lv.blocks_y + y) * lv.linear_row_stride;
                    if (std::memcmp(a + o, b + o, (size_t)row)) return false;
                }
    }
    return true;
}

static int crosscheck(const std::string& dir) {
    std::ifstream f(dir + "/cases.tsv");
    if (!f) { fprintf(stderr, "cannot open %s/cases.tsv\n", dir.c_str()); return 2; }
    std::string line;
    std::getline(f, line);
    int total = 0, fail = 0;
    uint64_t blocks = 0;
    while (std::getline(f, line)) {
        if (line.empty()) continue;
        auto c = split(line, '\t');
        if (c.size() < 17) { fprintf(stderr, "bad row: %s\n", line.c_str()); return 2; }
        TextureDesc d;
        d.width = atoi(c[1].c_str()); d.height = atoi(c[2].c_str()); d.depth = atoi(c[3].c_str());
        d.levels = atoi(c[4].c_str()); d.layers = atoi(c[5].c_str());
        d.block_w = atoi(c[6].c_str()); d.block_h = atoi(c[7].c_str()); d.bytes_per_block = atoi(c[8].c_str());
        d.gob_blocks_y = atoi(c[9].c_str()); d.gob_blocks_z = atoi(c[10].c_str());
        d.linear_stride_align = 4;
        Layout l = compute_layout(d);
        total++;
        std::string why;
        if (!l.valid) why = "layout invalid";
        else {
            if (l.linear_size != strtoull(c[12].c_str(), nullptr, 10)) why = "linear_size";
            else if (l.tiled_size != strtoull(c[13].c_str(), nullptr, 10)) why = "tiled_size";
            else if (l.tiled_layer_stride != strtoull(c[16].c_str(), nullptr, 10)) why = "tiled_layer_stride";
            else {
                auto lo = split(c[14], ','), to = split(c[15], ',');
                if (lo.size() != l.levels.size() || to.size() != l.levels.size()) why = "level count";
                for (size_t i = 0; why.empty() && i < l.levels.size(); i++) {
                    if (l.levels[i].linear_offset != strtoull(lo[i].c_str(), nullptr, 10)) why = "linear level offset";
                    else if (l.levels[i].tiled_offset != strtoull(to[i].c_str(), nullptr, 10)) why = "tiled level offset";
                }
            }
        }
        if (why.empty()) {
            auto lin = read_file(dir + "/case_" + c[0] + ".linear.bin");
            auto til = read_file(dir + "/case_" + c[0] + ".tiled.bin");
            if (lin.size() != l.linear_size || til.size() != l.tiled_size) why = "file sizes";
            else {
                std::vector<uint8_t> t(l.tiled_size, 0xAA), u(l.linear_size, 0x55);
                tile(d, l, lin.data(), t.data());
                if (t != til) why = "tile() differs from Ryujinx";
                else {
                    untile(d, l, til.data(), u.data());
                    if (!linear_valid_equal(d, l, u.data(), lin.data())) why = "untile() differs from Ryujinx input";
                }
                for (auto& lv : l.levels) blocks += (uint64_t)lv.blocks_x * lv.blocks_y * lv.depth * d.layers;
            }
        }
        if (!why.empty()) { fail++; printf("FAIL case %s: %s\n", c[0].c_str(), why.c_str()); }
    }
    printf("crosscheck: %d cases, %d failed, %llu blocks compared\n", total, fail, (unsigned long long)blocks);
    return fail ? 1 : 0;
}

static int roundtrip() {
    const uint32_t bpbs[] = {1, 2, 4, 8, 12, 16};
    struct Blk { uint32_t w, h; };
    const Blk blks[] = {{1, 1}, {4, 4}, {5, 5}, {6, 5}, {8, 6}, {10, 8}, {12, 12}};
    const uint32_t sizes[][2] = {{1, 1}, {3, 5}, {7, 5}, {16, 16}, {33, 65}, {64, 8}, {100, 37}, {255, 129}, {300, 300}, {1024, 70}, {1920, 1080}};
    int total = 0, fail = 0;
    uint64_t checked_blocks = 0;
    for (uint32_t bpb : bpbs)
        for (const Blk& bk : blks) {
            if (bk.w * bk.h > 1 && bpb != 8 && bpb != 16) continue;  // compressed blocks are 8 or 16 bytes
            for (auto& sz : sizes)
                for (uint32_t gy = 1; gy <= 32; gy <<= 1)
                    for (uint32_t layers : {1u, 3u})
                        for (uint32_t levels : {1u, 99u}) {
                            if (sz[0] * sz[1] > 600000 && (levels > 1 || layers > 1 || gy > 4) && (xorshift() & 7)) continue;  // thin the huge ones
                            TextureDesc d;
                            d.width = sz[0]; d.height = sz[1]; d.layers = layers;
                            d.block_w = bk.w; d.block_h = bk.h; d.bytes_per_block = bpb; d.gob_blocks_y = gy;
                            d.linear_stride_align = (xorshift() & 1) ? 1 : 4;
                            uint32_t maxl = 1;
                            while ((std::max(d.width, d.height) >> maxl) && maxl < 14) maxl++;
                            d.levels = std::min(levels, maxl);
                            Layout l = compute_layout(d);
                            total++;
                            std::string why;
                            if (!l.valid) why = "invalid layout";
                            else {
                                std::vector<uint8_t> lin(l.linear_size), back(l.linear_size, 0), t(l.tiled_size, 0x77);
                                for (auto& b : lin) b = (uint8_t)xorshift();
                                tile(d, l, lin.data(), t.data());
                                untile(d, l, t.data(), back.data());
                                if (!linear_valid_equal(d, l, lin.data(), back.data())) why = "tile->untile mismatch";
                                // independent formula: every block of every level/layer at its computed address
                                for (size_t li = 0; why.empty() && li < l.levels.size(); li++) {
                                    const LevelLayout& lv = l.levels[li];
                                    for (uint32_t layer = 0; why.empty() && layer < d.layers; layer++)
                                        for (uint32_t y = 0; why.empty() && y < lv.blocks_y; y++)
                                            for (uint32_t x = 0; x < lv.blocks_x; x += 1 + (lv.blocks_x > 64 ? (xorshift() % 7) : 0)) {
                                                uint64_t o = lv.linear_offset + layer * lv.linear_slice_size + (uint64_t)y * lv.linear_row_stride + (uint64_t)x * bpb;
                                                for (uint32_t j = 0; j < bpb; j++) {
                                                    uint64_t a = layer * l.tiled_layer_stride + lv.tiled_offset + tiled_address(lv, bpb, x, y, 0, j);
                                                    if (a >= l.tiled_size || t[a] != lin[o + j]) { why = "address formula mismatch"; break; }
                                                }
                                                checked_blocks++;
                                            }
                                }
                            }
                            if (!why.empty()) {
                                fail++;
                                if (fail <= 20) printf("FAIL %ux%u bpb %u block %ux%u gy %u layers %u levels %u align %u: %s\n", d.width, d.height, bpb, bk.w, bk.h, gy, layers, d.levels, d.linear_stride_align, why.c_str());
                            }
                        }
        }
    // 3D textures
    for (uint32_t bpb : bpbs)
        for (uint32_t depth : {1u, 2u, 4u, 5u, 9u, 16u, 33u})
            for (uint32_t gz : {1u, 2u, 4u, 8u, 16u, 32u})
                for (uint32_t gy : {1u, 4u, 16u})
                    for (uint32_t levels : {1u, 4u}) {
                        TextureDesc d;
                        d.width = 37; d.height = 70; d.depth = depth; d.levels = levels; d.bytes_per_block = bpb;
                        d.gob_blocks_y = gy; d.gob_blocks_z = gz; d.linear_stride_align = 1;
                        Layout l = compute_layout(d);
                        total++;
                        std::string why;
                        if (!l.valid) why = "invalid layout";
                        else {
                            std::vector<uint8_t> lin(l.linear_size), back(l.linear_size, 0), t(l.tiled_size, 0x77);
                            for (auto& b : lin) b = (uint8_t)xorshift();
                            tile(d, l, lin.data(), t.data());
                            untile(d, l, t.data(), back.data());
                            if (!linear_valid_equal(d, l, lin.data(), back.data())) why = "3D tile->untile mismatch";
                            for (size_t li = 0; why.empty() && li < l.levels.size(); li++) {
                                const LevelLayout& lv = l.levels[li];
                                for (uint32_t z = 0; why.empty() && z < lv.depth; z++)
                                    for (uint32_t y = 0; why.empty() && y < lv.blocks_y; y++)
                                        for (uint32_t x = 0; x < lv.blocks_x; x++) {
                                            uint64_t o = lv.linear_offset + ((uint64_t)z * lv.blocks_y + y) * lv.linear_row_stride + (uint64_t)x * bpb;
                                            for (uint32_t j = 0; j < bpb; j++) {
                                                uint64_t a = lv.tiled_offset + tiled_address(lv, bpb, x, y, z, j);
                                                if (a >= l.tiled_size || t[a] != lin[o + j]) { why = "3D address formula mismatch"; break; }
                                            }
                                            checked_blocks++;
                                        }
                            }
                        }
                        if (!why.empty()) {
                            fail++;
                            if (fail <= 20) printf("FAIL 3D depth %u bpb %u gy %u gz %u levels %u: %s\n", depth, bpb, gy, gz, levels, why.c_str());
                        }
                    }
    printf("roundtrip: %d textures, %d failed, %llu blocks checked against the address formula\n", total, fail, (unsigned long long)checked_blocks);
    return fail ? 1 : 0;
}

static int bench() {
    TextureDesc d;
    d.width = 4096; d.height = 4096; d.levels = 1; d.bytes_per_block = 4; d.gob_blocks_y = 16; d.linear_stride_align = 4;
    Layout l = compute_layout(d);
    std::vector<uint8_t> lin(l.linear_size, 1), t(l.tiled_size, 2);
    for (const char* name : {"tile", "untile"}) {
        const int reps = 10;
        auto t0 = std::chrono::steady_clock::now();
        for (int i = 0; i < reps; i++) {
            if (name[0] == 't') tile(d, l, lin.data(), t.data());
            else untile(d, l, t.data(), lin.data());
        }
        double s = std::chrono::duration<double>(std::chrono::steady_clock::now() - t0).count();
        printf("%-7s 4096x4096 RGBA8 gob_y 16: %.2f GB/s\n", name, (double)l.linear_size * reps / s / 1e9);
    }
    return 0;
}

int main(int argc, char** argv) {
    if (argc >= 2 && !strcmp(argv[1], "crosscheck") && argc >= 3) return crosscheck(argv[2]);
    if (argc >= 2 && !strcmp(argv[1], "roundtrip")) {
        if (argc >= 4 && !strcmp(argv[2], "--seed")) g_rng ^= strtoull(argv[3], nullptr, 10) * 0x9E3779B97F4A7C15ull;
        return roundtrip();
    }
    if (argc >= 2 && !strcmp(argv[1], "bench")) return bench();
    fprintf(stderr, "usage: tiling_test crosscheck <cases dir> | roundtrip [--seed N] | bench\n");
    return 2;
}
