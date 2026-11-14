// The bc7e_cpp API (bc7e_cpp.h) over the two builds of bc7e_cpp.cpp: AVX2 + FMA when the CPU and the OS have it, else baseline x86-64.
// The AVX2 build is about 1.35 times as fast (the compiler vectorizes the per-texel loops); both give valid BC7 of the same quality, not
// always the same bits. The environment variable A2B_BC7E_SCALAR=1 forces the baseline build (to check it on an AVX2 machine).
// GPL-3.0-or-later.
#ifndef _CRT_SECURE_NO_WARNINGS
#define _CRT_SECURE_NO_WARNINGS
#endif
#include "bc7e_cpp.h"

#include <intrin.h>
#include <stdlib.h>

namespace bc7e_cpp {
namespace {

bool cpu_has_avx2_fma() {
    int r[4];
    __cpuid(r, 0);
    if (r[0] < 7) return false;
    __cpuid(r, 1);
    const bool fma = (r[2] >> 12) & 1, osxsave = (r[2] >> 27) & 1, avx = (r[2] >> 28) & 1;
    if (!(fma && osxsave && avx) || (_xgetbv(0) & 6) != 6) return false;   // the OS saves the YMM state
    __cpuidex(r, 7, 0);
    return (r[1] >> 5) & 1;
}

bool use_avx2() {
#ifdef BC7E_CPP_HAVE_AVX2
    static const bool yes = [] {
        const char* e = getenv("A2B_BC7E_SCALAR");
        return !(e && *e && *e != '0') && cpu_has_avx2_fma();
    }();
    return yes;
#else
    return false;
#endif
}

}  // namespace

const char* bc7e_cpp_variant() { return use_avx2() ? "avx2" : "scalar"; }

void bc7e_compress_block_init() {
#ifdef BC7E_CPP_HAVE_AVX2
    if (use_avx2()) return avx2::bc7e_compress_block_init();
#endif
    scalar::bc7e_compress_block_init();
}

void bc7e_compress_blocks(uint32_t num_blocks, uint64_t* pBlocks, const uint32_t* pPixelsRGBA, const bc7e_compress_block_params* pComp_params) {
#ifdef BC7E_CPP_HAVE_AVX2
    if (use_avx2()) return avx2::bc7e_compress_blocks(num_blocks, pBlocks, pPixelsRGBA, pComp_params);
#endif
    scalar::bc7e_compress_blocks(num_blocks, pBlocks, pPixelsRGBA, pComp_params);
}

// The presets only fill in the parameter block: the same in both builds.
void bc7e_compress_block_params_init(bc7e_compress_block_params* p, bool perceptual) { scalar::bc7e_compress_block_params_init(p, perceptual); }
void bc7e_compress_block_params_init_slowest(bc7e_compress_block_params* p, bool perceptual) { scalar::bc7e_compress_block_params_init_slowest(p, perceptual); }
void bc7e_compress_block_params_init_veryslow(bc7e_compress_block_params* p, bool perceptual) { scalar::bc7e_compress_block_params_init_veryslow(p, perceptual); }
void bc7e_compress_block_params_init_slow(bc7e_compress_block_params* p, bool perceptual) { scalar::bc7e_compress_block_params_init_slow(p, perceptual); }
void bc7e_compress_block_params_init_basic(bc7e_compress_block_params* p, bool perceptual) { scalar::bc7e_compress_block_params_init_basic(p, perceptual); }
void bc7e_compress_block_params_init_fast(bc7e_compress_block_params* p, bool perceptual) { scalar::bc7e_compress_block_params_init_fast(p, perceptual); }
void bc7e_compress_block_params_init_veryfast(bc7e_compress_block_params* p, bool perceptual) { scalar::bc7e_compress_block_params_init_veryfast(p, perceptual); }
void bc7e_compress_block_params_init_ultrafast(bc7e_compress_block_params* p, bool perceptual) { scalar::bc7e_compress_block_params_init_ultrafast(p, perceptual); }

}  // namespace bc7e_cpp
