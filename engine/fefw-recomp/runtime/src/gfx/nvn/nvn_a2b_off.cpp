// SPDX-License-Identifier: GPL-3.0-or-later
// The a2b API (tools/textures/include/astc2bc7.h) without the transcoder: built instead of astc2bc7 when NXRT_ASTC2BC7 is off
// (cmake/gfx.cmake). Without ISPC the transcoder is built from source (tools/textures/cmake/a2b_cpp.cmake), so this is only for a tree
// without tools/textures/third_party. Sizes and BCn pass-through answer as the library does; every ASTC decode or transcode fails with
// A2B_ERR_NO_ASTC_LIB, so ASTC textures are not prepared.
#include "astc2bc7.h"

#include <cstring>

namespace {
uint64_t div_up(uint64_t a, uint64_t b) { return (a + b - 1) / b; }
}  // namespace

extern "C" {

a2b_status a2b_init(const char*) { return A2B_ERR_NO_ASTC_LIB; }

a2b_encoder a2b_bc7_encoder(const char** name) {
    if (name) *name = "none";
    return A2B_ENCODER_NONE;
}

const char* a2b_status_string(a2b_status s) {
    switch (s) {
        case A2B_OK: return "ok";
        case A2B_ERR_ARGS: return "bad arguments";
        case A2B_ERR_SIZE: return "buffer size does not match dimensions";
        case A2B_ERR_UNSUPPORTED: return "unsupported format";
        case A2B_ERR_NO_ASTC_LIB: return "no ASTC transcoder in this build (NXRT_ASTC2BC7=OFF)";
        case A2B_ERR_DECODE: return "astcenc decode error";
    }
    return "?";
}

int a2b_is_astc(a2b_format f) { return f == A2B_ASTC_UNORM || f == A2B_ASTC_SRGB; }

int a2b_is_srgb(a2b_format f) {
    return f == A2B_ASTC_SRGB || f == A2B_BC1_SRGB || f == A2B_BC2_SRGB || f == A2B_BC3_SRGB || f == A2B_BC7_SRGB;
}

uint32_t a2b_bcn_block_bytes(a2b_format f) {
    switch (f) {
        case A2B_BC1_UNORM: case A2B_BC1_SRGB: case A2B_BC4_UNORM: case A2B_BC4_SNORM: return 8;
        case A2B_BC2_UNORM: case A2B_BC2_SRGB: case A2B_BC3_UNORM: case A2B_BC3_SRGB: case A2B_BC5_UNORM:
        case A2B_BC5_SNORM: case A2B_BC6H_UF16: case A2B_BC6H_SF16: case A2B_BC7_UNORM: case A2B_BC7_SRGB: return 16;
        default: return 0;
    }
}

uint64_t a2b_astc_size(uint32_t w, uint32_t h, uint32_t bw, uint32_t bh) {
    if (!w || !h || bw < 4 || bw > 12 || bh < 4 || bh > 12) return 0;
    return div_up(w, bw) * div_up(h, bh) * 16;
}

uint64_t a2b_output_size(const a2b_input* in) {
    if (!in || !in->width || !in->height) return 0;
    if (a2b_is_astc(in->format)) return div_up(in->width, 4) * div_up(in->height, 4) * 16;
    const uint32_t bb = a2b_bcn_block_bytes(in->format);
    return bb ? div_up(in->width, 4) * div_up(in->height, 4) * bb : 0;
}

a2b_status a2b_transcode(const a2b_input* in, const void* data, size_t data_size, const a2b_options*, a2b_format* out_format, void* out,
                         size_t out_capacity, a2b_stats* stats) {
    if (stats) std::memset(stats, 0, sizeof *stats);
    if (!in || !data || !out) return A2B_ERR_ARGS;
    if (a2b_is_astc(in->format)) return A2B_ERR_NO_ASTC_LIB;
    const uint64_t n = a2b_output_size(in);
    if (!n) return A2B_ERR_UNSUPPORTED;
    if (data_size != n || out_capacity < n) return A2B_ERR_SIZE;
    std::memcpy(out, data, n);
    if (out_format) *out_format = in->format;
    return A2B_OK;
}

a2b_status a2b_decode_astc_rgba8(const a2b_input* in, const void* data, size_t, int, uint8_t* rgba, a2b_stats* stats) {
    if (stats) std::memset(stats, 0, sizeof *stats);
    if (!in || !data || !rgba || !a2b_is_astc(in->format)) return A2B_ERR_ARGS;
    return A2B_ERR_NO_ASTC_LIB;
}

a2b_status a2b_encode_bc7(uint32_t, uint32_t, const uint8_t*, const a2b_options*, void*, size_t, a2b_stats* stats) {
    if (stats) std::memset(stats, 0, sizeof *stats);
    return A2B_ERR_UNSUPPORTED;
}

}  // extern "C"
