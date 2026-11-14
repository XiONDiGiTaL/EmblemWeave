// tegra_tile: untile / tile a block-linear texture file. GPL-3.0-or-later.
//   tegra_tile untile|tile <in> <out> --width W --height H [--depth D] [--levels N] [--layers L]
//              [--bpb B] [--block WxH] --gob-y G [--gob-z G] [--tight]
// untile: <in> is tiled data (a BNTX/NVN image), <out> is the linear buffer (levels back to back, then layers, rows
// padded to 4 bytes unless --tight). tile: the reverse. The layout is printed to stdout.
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include "tegra_tiling.h"

int main(int argc, char** argv) {
    if (argc < 4 || (strcmp(argv[1], "untile") && strcmp(argv[1], "tile"))) {
        fprintf(stderr, "usage: tegra_tile untile|tile <in> <out> --width W --height H [--depth D] [--levels N] [--layers L] [--bpb B] [--block WxH] --gob-y G [--gob-z G] [--tight]\n");
        return 2;
    }
    const bool do_untile = !strcmp(argv[1], "untile");
    tegra::TextureDesc d;
    d.gob_blocks_y = 0;
    for (int i = 4; i < argc; i++) {
        std::string a = argv[i];
        auto v = [&]() -> const char* { return i + 1 < argc ? argv[++i] : (fprintf(stderr, "missing value for %s\n", a.c_str()), exit(2), nullptr); };
        if (a == "--width") d.width = atoi(v());
        else if (a == "--height") d.height = atoi(v());
        else if (a == "--depth") d.depth = atoi(v());
        else if (a == "--levels") d.levels = atoi(v());
        else if (a == "--layers") d.layers = atoi(v());
        else if (a == "--bpb") d.bytes_per_block = atoi(v());
        else if (a == "--gob-y") d.gob_blocks_y = atoi(v());
        else if (a == "--gob-z") d.gob_blocks_z = atoi(v());
        else if (a == "--tight") d.linear_stride_align = 1;
        else if (a == "--block") { if (sscanf(v(), "%ux%u", &d.block_w, &d.block_h) != 2) return 2; }
        else { fprintf(stderr, "unknown option %s\n", a.c_str()); return 2; }
    }
    if (!d.gob_blocks_y) d.gob_blocks_y = tegra::default_gob_blocks_y((d.height + d.block_h - 1) / d.block_h);
    tegra::Layout l = tegra::compute_layout(d);
    if (!l.valid) { fprintf(stderr, "invalid texture parameters\n"); return 1; }

    FILE* f = fopen(argv[2], "rb");
    if (!f) { fprintf(stderr, "cannot read %s\n", argv[2]); return 1; }
    fseek(f, 0, SEEK_END);
    size_t n = (size_t)ftell(f);
    fseek(f, 0, SEEK_SET);
    std::vector<uint8_t> in(n);
    if (fread(in.data(), 1, n, f) != n) { fclose(f); return 1; }
    fclose(f);
    const size_t need_in = do_untile ? l.tiled_size : l.linear_size;
    if (n < need_in) { fprintf(stderr, "input has %zu bytes, layout needs %zu\n", n, need_in); return 1; }

    std::vector<uint8_t> out(do_untile ? l.linear_size : l.tiled_size, 0);
    if (do_untile) tegra::untile(d, l, in.data(), out.data());
    else tegra::tile(d, l, in.data(), out.data());
    f = fopen(argv[3], "wb");
    if (!f || fwrite(out.data(), 1, out.size(), f) != out.size()) { fprintf(stderr, "cannot write %s\n", argv[3]); return 1; }
    fclose(f);
    printf("%s: linear %llu bytes, tiled %llu bytes, layer stride %llu\n", argv[1], (unsigned long long)l.linear_size,
           (unsigned long long)l.tiled_size, (unsigned long long)l.tiled_layer_stride);
    for (size_t i = 0; i < l.levels.size(); i++)
        printf("  level %zu: %ux%ux%u blocks %ux%u gob_y %u gob_z %u linear@%llu tiled@%llu (%llu)\n", i, l.levels[i].width, l.levels[i].height,
               l.levels[i].depth, l.levels[i].blocks_x, l.levels[i].blocks_y, l.levels[i].gob_blocks_y, l.levels[i].gob_blocks_z,
               (unsigned long long)l.levels[i].linear_offset, (unsigned long long)l.levels[i].tiled_offset, (unsigned long long)l.levels[i].tiled_size);
    return 0;
}
