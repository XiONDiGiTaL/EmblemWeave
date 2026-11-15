# SPDX-License-Identifier: GPL-3.0-or-later
# libvpx (BSD-3-Clause; x86inc macros ISC) for the host nn::codec VP9/VP8 decoders (src/media/video_codec.cpp).
# libvpx builds with its own configure script, not CMake, so it is taken prebuilt from vcpkg, static with the static MSVC runtime (/MT)
# like everything else here:
#     vcpkg install libvpx:x64-windows-static        (vcpkg port libvpx 1.16.0#3)
# NXRT_LIBVPX_ROOT points at that install (default: C:/vcpkg, then $VCPKG_ROOT) (the folder with include/vpx and lib/vpx.lib). Without it the runtime builds without the
# decoders (NXRT_HAVE_LIBVPX=0) and Switch 2 movies are left out.
set(NXRT_LIBVPX_ROOT "" CACHE PATH "vcpkg x64-windows-static install with libvpx (include/vpx, lib/vpx.lib); empty: look in the usual places")
set(_nxrt_vpx_roots "${NXRT_LIBVPX_ROOT}" "C:/vcpkg/installed/x64-windows-static")
if(DEFINED ENV{VCPKG_ROOT})
    list(APPEND _nxrt_vpx_roots "$ENV{VCPKG_ROOT}/installed/x64-windows-static")
endif()
list(TRANSFORM _nxrt_vpx_roots APPEND "/include" OUTPUT_VARIABLE _nxrt_vpx_inc)
list(TRANSFORM _nxrt_vpx_roots APPEND "/lib" OUTPUT_VARIABLE _nxrt_vpx_lib)
find_path(NXRT_LIBVPX_INCLUDE vpx/vpx_decoder.h PATHS ${_nxrt_vpx_inc} NO_DEFAULT_PATH)
find_library(NXRT_LIBVPX_LIB NAMES vpx vpxmt PATHS ${_nxrt_vpx_lib} NO_DEFAULT_PATH)
if(NXRT_LIBVPX_INCLUDE AND NXRT_LIBVPX_LIB)
    message(STATUS "nxrt: libvpx ${NXRT_LIBVPX_LIB} (host VP9/VP8 decoders)")
    target_include_directories(nxrt SYSTEM PRIVATE "${NXRT_LIBVPX_INCLUDE}")
    target_link_libraries(nxrt PUBLIC "${NXRT_LIBVPX_LIB}")
    target_compile_definitions(nxrt PRIVATE NXRT_HAVE_LIBVPX=1)
else()
    message(STATUS "nxrt: libvpx not found (looked in: ${_nxrt_vpx_roots}): no host video decoders")
    target_compile_definitions(nxrt PRIVATE NXRT_HAVE_LIBVPX=0)
endif()

# dav1d (BSD-2-Clause, third_party/dav1d) for the Switch 2 movie player's AV1 decoder (src/media/movie_vdec.cpp): built from source, C only.
option(NXRT_DAV1D "Build dav1d for AV1 movies (src/media/movie_vdec.cpp)" ON)
# third_party/ is not in the release zip (it is fetched, not shipped), so a build from a download has no dav1d. That
# used to be a configure error - "add_subdirectory given source .../third_party/dav1d which is not an existing
# directory" - at every build from a zip, whatever the player did. The AV1 movie decoder is the only thing lost:
set(_nxrt_dav1d "${CMAKE_CURRENT_LIST_DIR}/../third_party/dav1d")
if(NXRT_DAV1D AND EXISTS "${_nxrt_dav1d}/CMakeLists.txt")
    add_subdirectory("${_nxrt_dav1d}" "${CMAKE_BINARY_DIR}/third_party/dav1d")
    target_link_libraries(nxrt PUBLIC nxrt_dav1d)
    target_compile_definitions(nxrt PRIVATE NXRT_HAVE_DAV1D=1)
else()
    if(NXRT_DAV1D)
        message(STATUS "nxrt: third_party/dav1d is not here: building without the AV1 movie decoder "
                "(clone https://code.videolan.org/videolan/dav1d into third_party/dav1d, or pass -DNXRT_DAV1D=OFF, to be rid of this line)")
    endif()
    target_compile_definitions(nxrt PRIVATE NXRT_HAVE_DAV1D=0)
endif()
