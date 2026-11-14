// bc7e_cpp.h: the API of bc7e.ispc of bc7enc_rdo (github.com/richgel999/bc7enc_rdo), Copyright (C) 2018-2020 Binomial LLC, Apache 2.0 license
// (tools/textures/licenses/LICENSE.bc7enc_rdo), translated to scalar C++ by tools/textures/scripts/port_bc7e.py. Generated: do not edit.
// bc7e_cpp.cpp is compiled once per instruction set into bc7e_cpp::<BC7E_CPP_VARIANT> (scalar: baseline x86-64, avx2: AVX2 + FMA);
// the functions directly in bc7e_cpp pick one of them by CPUID (bc7e_cpp_dispatch.cpp).
#pragma once
#include <cstdint>

#define BC7E_CPP_DECLARE_API \
    void bc7e_compress_block_init(); \
    void bc7e_compress_blocks(uint32_t num_blocks, uint64_t* pBlocks, const uint32_t* pPixelsRGBA, const bc7e_compress_block_params* pComp_params); \
    void bc7e_compress_block_params_init(bc7e_compress_block_params* p, bool perceptual); \
    void bc7e_compress_block_params_init_slowest(bc7e_compress_block_params* p, bool perceptual); \
    void bc7e_compress_block_params_init_veryslow(bc7e_compress_block_params* p, bool perceptual); \
    void bc7e_compress_block_params_init_slow(bc7e_compress_block_params* p, bool perceptual); \
    void bc7e_compress_block_params_init_basic(bc7e_compress_block_params* p, bool perceptual); \
    void bc7e_compress_block_params_init_fast(bc7e_compress_block_params* p, bool perceptual); \
    void bc7e_compress_block_params_init_veryfast(bc7e_compress_block_params* p, bool perceptual); \
    void bc7e_compress_block_params_init_ultrafast(bc7e_compress_block_params* p, bool perceptual); \

namespace bc7e_cpp {

// in a namespace of its own: argument-dependent lookup inside the builds then finds no function of the dispatching API
namespace param_types {
struct bc7e_compress_block_params
{
	uint32_t m_max_partitions_mode[8];

	uint32_t m_weights[4];

	uint32_t m_uber_level;
	uint32_t m_refinement_passes;
	
	uint32_t m_mode4_rotation_mask;
	uint32_t m_mode4_index_mask;
	uint32_t m_mode5_rotation_mask;
	uint32_t m_uber1_mask;
	
	bool m_perceptual;
	bool m_pbit_search;
	bool m_mode6_only;
	bool m_unused0;
	
	struct
	{
		uint32_t m_max_mode13_partitions_to_try;
		uint32_t m_max_mode0_partitions_to_try;
		uint32_t m_max_mode2_partitions_to_try;
		bool m_use_mode[7];
		bool m_unused1;
	} m_opaque_settings;

	struct
	{
		uint32_t m_max_mode7_partitions_to_try;
		uint32_t m_mode67_error_weight_mul[4];
				
		bool m_use_mode4;
		bool m_use_mode5;
		bool m_use_mode6;
		bool m_use_mode7;

		bool m_use_mode4_rotation;
		bool m_use_mode5_rotation;
		bool m_unused2;
		bool m_unused3;
	} m_alpha_settings;

};
}  // namespace param_types
using param_types::bc7e_compress_block_params;

BC7E_CPP_DECLARE_API
const char* bc7e_cpp_variant();   // "avx2" or "scalar": the build the functions above use

namespace scalar { BC7E_CPP_DECLARE_API }
namespace avx2 { BC7E_CPP_DECLARE_API }

}  // namespace bc7e_cpp
