# SPDX-License-Identifier: GPL-3.0-or-later
# zstd (BSD-3-Clause) 1.5.7, the library sources only (no CLI, no x86-64 assembly). Two parts of the runtime decompress with it: the
# shader preload pack (src/gfx/nvn/nvn_shaders.cpp) and the .nx RomFS reader (src/loader/nx.cpp), which is why this is fetched for every
# build rather than next to the graphics shell.
include(FetchContent)
if(NOT DEFINED NXRT_DEPS_DIR)
    set(NXRT_DEPS_DIR "$ENV{LOCALAPPDATA}/nxrt/deps" CACHE PATH "Where the pinned third-party sources are downloaded to")
endif()
if(NOT COMMAND FetchContent_Declare)
    include(FetchContent)
endif()
if(NOT DEFINED CMAKE_POLICY_DEFAULT_CMP0135)
    set(CMAKE_POLICY_DEFAULT_CMP0135 NEW)   # extracted files take the time of extraction
endif()
FetchContent_Declare(zstd
    URL https://github.com/facebook/zstd/releases/download/v1.5.7/zstd-1.5.7.tar.gz
    DOWNLOAD_DIR "${NXRT_DEPS_DIR}"
    URL_HASH SHA256=eb33e51f49a15e023950cd7825ca74a4a2b43db8354825ac24fc1b7ee09e6fa3
    SOURCE_SUBDIR none)
FetchContent_MakeAvailable(zstd)

file(GLOB NXRT_ZSTD_SOURCES "${zstd_SOURCE_DIR}/lib/common/*.c" "${zstd_SOURCE_DIR}/lib/compress/*.c" "${zstd_SOURCE_DIR}/lib/decompress/*.c")
add_library(nxrt_zstd STATIC ${NXRT_ZSTD_SOURCES})
target_include_directories(nxrt_zstd SYSTEM PUBLIC "${zstd_SOURCE_DIR}/lib")
target_compile_definitions(nxrt_zstd PRIVATE ZSTD_DISABLE_ASM)
set_target_properties(nxrt_zstd PROPERTIES C_STANDARD 11)
