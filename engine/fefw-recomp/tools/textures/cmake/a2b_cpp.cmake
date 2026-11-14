# SPDX-License-Identifier: GPL-3.0-or-later
# astc2bc7 built from source without the ISPC compiler: src/a2b.cpp with bc7e translated to C++ (src/bc7e_cpp.cpp, from
# scripts/port_bc7e.py; compiled for baseline x86-64 and for AVX2 + FMA, picked by CPUID in src/bc7e_cpp_dispatch.cpp) and ARM astcenc's
# decoder compiled in (baseline SSE2, no DLLs). Needs third_party/astc-encoder (scripts/fetch_third_party.ps1).
#   include(<tools/textures>/cmake/a2b_cpp.cmake)
#   a2b_add_cpp_library(<target>)     static library; links like astc2bc7.lib (include/astc2bc7.h)
set(A2B_TEXTURES_DIR "${CMAKE_CURRENT_LIST_DIR}/.." CACHE INTERNAL "")

function(a2b_cpp_sources_present out)
    if(EXISTS "${A2B_TEXTURES_DIR}/third_party/astc-encoder/Source/astcenc.h" AND EXISTS "${A2B_TEXTURES_DIR}/src/bc7e_cpp.cpp")
        set(${out} TRUE PARENT_SCOPE)
    else()
        set(${out} FALSE PARENT_SCOPE)
    endif()
endfunction()

function(a2b_add_cpp_library name)
    set(tex "${A2B_TEXTURES_DIR}")
    set(astc "${tex}/third_party/astc-encoder/Source")
    set(astc_sources "")
    foreach(f averages_and_directions block_sizes color_quantize color_unquantize compress_symbolic compute_variance decompress_symbolic
              diagnostic_trace entry find_best_partitioning ideal_endpoints_and_weights image integer_sequence mathlib mathlib_softfloat
              partition_tables percentile_tables pick_best_endpoint_format quantization symbolic_physical weight_align weight_quant_xfer_tables)
        list(APPEND astc_sources "${astc}/astcenc_${f}.cpp")
    endforeach()
    # the AVX2 + FMA build of the encoder (bc7e_cpp::avx2); the baseline one (bc7e_cpp::scalar) is in the library itself
    add_library(${name}_bc7e_avx2 OBJECT "${tex}/src/bc7e_cpp.cpp")
    target_compile_definitions(${name}_bc7e_avx2 PRIVATE BC7E_CPP_VARIANT=avx2)
    target_compile_options(${name}_bc7e_avx2 PRIVATE -mavx2 -mfma)
    add_library(${name} STATIC "${tex}/src/a2b.cpp" "${tex}/src/bc7e_cpp.cpp" "${tex}/src/bc7e_cpp_dispatch.cpp" ${astc_sources}
                $<TARGET_OBJECTS:${name}_bc7e_avx2>)
    target_include_directories(${name} PUBLIC "${tex}/include" PRIVATE "${tex}/src" "${astc}")
    target_compile_definitions(${name} PRIVATE A2B_BC7E_CPP A2B_ASTC_STATIC A2B_ASTC_STATIC_ISA="sse2" BC7E_CPP_HAVE_AVX2 _CRT_SECURE_NO_WARNINGS
                               ASTCENC_DECOMPRESS_ONLY ASTCENC_NEON=0 ASTCENC_SVE=0 ASTCENC_SSE=20 ASTCENC_AVX=0 ASTCENC_POPCNT=0 ASTCENC_F16C=0
                               ASTCENC_X86_GATHERS=0)
    # third-party and generated code: built as it is, without the warnings of the code around it
    set_source_files_properties(${astc_sources} "${tex}/src/bc7e_cpp.cpp" PROPERTIES COMPILE_OPTIONS "-w")
endfunction()
