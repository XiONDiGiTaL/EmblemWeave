# SPDX-License-Identifier: GPL-3.0-or-later
# nxrt: game-independent runtime skeleton (OS/HLE layer) for the Metroid Prime Remastered static recompilation.
# Build: scripts\build.ps1   (clang-cl + Ninja, Release)   Test: ctest --test-dir build\release
cmake_minimum_required(VERSION 3.25)
# Static MSVC runtime (/MT): players need no Visual C++ Redistributable. Set before project() so every target below, third-party code
# included, uses it; the libraries this one links must agree (a mismatch is a link error).
if(NOT DEFINED CMAKE_MSVC_RUNTIME_LIBRARY)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()
project(nxrt C CXX)

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)
set(CMAKE_C_STANDARD 11)

# The host services behind nxrt/platform.h (memory, threads, sync, time, files). Windows and POSIX (Linux, SteamOS) each
# have a backend; only the host's is compiled. Both build the whole runtime and are expected to keep doing so (docs/LINUX.md).
# NXRT_PLATFORM_ONLY builds just the layer and its self-test, which is how a new backend is brought up without the rest.
set(NXRT_PLATFORM_DIR "${CMAKE_CURRENT_SOURCE_DIR}/src/platform/win32")
if(NOT WIN32)
    set(NXRT_PLATFORM_DIR "${CMAKE_CURRENT_SOURCE_DIR}/src/platform/posix")
endif()
# Default: Windows builds the whole runtime; Linux and SteamOS build the platform layer and its self-test, the part of the
# runtime that has a POSIX backend and a passing test today. -DNXRT_PLATFORM_ONLY=OFF builds the rest of the runtime on
# Linux, which is the port in progress (docs/LINUX.md has the exact state per subsystem).
if(NOT WIN32)
    set(_nxrt_platform_only_default ON)
else()
    set(_nxrt_platform_only_default OFF)
endif()
option(NXRT_PLATFORM_ONLY "Build only nxrt's platform layer and its self-test" ${_nxrt_platform_only_default})
if(NXRT_PLATFORM_ONLY)
    file(GLOB NXRT_PLATFORM_SOURCES CONFIGURE_DEPENDS "${NXRT_PLATFORM_DIR}/*.cpp")
    add_library(nxrt_platform STATIC ${NXRT_PLATFORM_SOURCES})
    target_include_directories(nxrt_platform PUBLIC include)
    # The POSIX backend is pthreads; the Windows build gets threading through the CRT, and FindThreads reports no
    # Threads::Threads target there.
    if(NOT WIN32)
        find_package(Threads REQUIRED)
        target_link_libraries(nxrt_platform PUBLIC Threads::Threads)
    endif()
    # A diagnostic binary, not a test: it reports the loader and the devices a Linux bug report has to name.
    option(NXRT_BUILD_LINUX_PROBE "Build nxrt_linux_probe (Vulkan loader and device report)" ON)
    if(NXRT_BUILD_LINUX_PROBE AND NOT WIN32)
        add_executable(nxrt_linux_probe tools/linux_probe.cpp)
        target_link_libraries(nxrt_linux_probe PRIVATE nxrt_platform ${CMAKE_DL_LIBS})
    endif()
    enable_testing()
    add_executable(nxrt_platform_tests tests/platform_selftest.cpp)
    target_link_libraries(nxrt_platform_tests PRIVATE nxrt_platform)
    add_test(NAME platform COMMAND nxrt_platform_tests)
    message(STATUS "nxrt: platform layer only (${NXRT_PLATFORM_DIR})")
    return()
endif()

option(NXRT_BUILD_TESTS "Build the nxrt_tests executable and register its CTest entries" ON)

# Optional upscaler backends (header-only SDKs, runtime DLLs loaded from upscalers/ folders).
# Disabling removes the corresponding headers from the build and the mode from the UI.
option(NXRT_UPSCALER_FSR "AMD FidelityFX FSR 1/3/4 via FFX API (temporal_ffx.cpp)" ON)
option(NXRT_UPSCALER_DLSS "NVIDIA DLSS/DLAA via Streamline SDK (temporal_dlss.cpp)" ON)
option(NXRT_UPSCALER_XESS "Intel XeSS via libxess.dll (temporal_xess.cpp)" ON)
option(NXRT_UPSCALER_NIS "NVIDIA Image Scaling v1.0.3 spatial (nis_spatial.cpp)" ON)
option(NXRT_UPSCALER_MAKO "MAKO spatial scaler (mako_spatial.cpp)" ON)

# Optional video decoders for Switch 2 movies.
option(NXRT_DAV1D "Build dav1d for AV1 movies (src/media/movie_vdec.cpp)" ON)
option(NXRT_LIBVPX "Enable libvpx VP9/VP8 decoders via vcpkg (src/media/video_codec.cpp)" ON)

# The generated-code ABI header (Ctx, call_indirect, ...) lives in the recompiler (recomp/include/nxrecomp/).
# Found next to the checkout (../recomp) or, for a git worktree under runtime-wt/<name>, two levels up (../../recomp).
set(NXRT_RECOMP_INCLUDE "${CMAKE_CURRENT_SOURCE_DIR}/../recomp/include" CACHE PATH "Directory that holds nxrecomp/")
if(NOT EXISTS "${NXRT_RECOMP_INCLUDE}/nxrecomp/ctx.h" AND EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/../../recomp/include/nxrecomp/ctx.h")
    get_filename_component(_nxrt_ri "${CMAKE_CURRENT_SOURCE_DIR}/../../recomp/include" ABSOLUTE)
    set(NXRT_RECOMP_INCLUDE "${_nxrt_ri}" CACHE PATH "Directory that holds nxrecomp/" FORCE)
endif()

if(CMAKE_CXX_COMPILER_FRONTEND_VARIANT STREQUAL "MSVC")
    # cl.exe and clang-cl share the MSVC frontend variant but not the warning switches: cl.exe rejects every -Wno-*
    # option with D8021 (issue #115). MSVC spells the one suppression this tree needs as /wd4100 (unused parameter);
    # the other options here silence clang diagnostics that MSVC does not emit at /W3, so it needs no equivalent and
    # they must not be passed to it. /EHsc: host C++ (ipc.cpp's service call, crash.cpp's terminate path) uses try/throw;
    # guest exceptions have their own layer in src/eh.
    if(CMAKE_CXX_COMPILER_ID STREQUAL "MSVC")
        add_compile_options(/EHsc /W3 /wd4100)
    else()
        add_compile_options(/EHsc /W3 -Wno-unused-parameter -Wno-missing-field-initializers -Wno-cast-function-type-mismatch
                            -Wno-cast-function-type-strict -Wno-language-extension-token)
    endif()
    # CMPXCHG16B. MSVC's <atomic> gives all its functions target("cx16") under clang on x64: without the flag clang may not inline
    # them into code that lacks it, and every std::atomic load, store or add becomes a call ("conflicting target features"). Both
    # builds of the game code need the instruction anyway.
    if(CMAKE_CXX_COMPILER_ID MATCHES "Clang" AND CMAKE_SYSTEM_PROCESSOR MATCHES "AMD64|x86_64")   # clang-cl only: MSVC has no -mcx16
        add_compile_options($<$<COMPILE_LANGUAGE:C,CXX>:-mcx16>)
    endif()
elseif(CMAKE_CXX_COMPILER_ID MATCHES "GNU|Clang")
    # GCC and Clang on Linux. -Wno-* for the same reasons the clang-cl line has them: generated and third-party code.
    add_compile_options(-Wall -Wextra -Wno-unused-parameter -Wno-missing-field-initializers -Wno-cast-function-type)
    add_compile_definitions(_GNU_SOURCE)
endif()
if(WIN32)
    add_compile_definitions(NOMINMAX WIN32_LEAN_AND_MEAN _CRT_SECURE_NO_WARNINGS _WIN32_WINNT=0x0A00)
endif()
# ThinLTO: the C and C++ objects are LLVM bitcode, and lld-link optimises across source files at the link. On by default for Release
# builds with clang and lld-link; the configure says which it is.
set(_nxrt_lto_default OFF)
if(CMAKE_BUILD_TYPE STREQUAL "Release" AND CMAKE_CXX_COMPILER_ID MATCHES "Clang" AND CMAKE_LINKER MATCHES "lld-link")
    set(_nxrt_lto_default ON)
endif()
option(NXRT_LTO "Build the runtime with ThinLTO (needs clang-cl and lld-link)" ${_nxrt_lto_default})
if(NXRT_LTO)
    if(NOT CMAKE_CXX_COMPILER_ID MATCHES "Clang" OR NOT CMAKE_LINKER MATCHES "lld-link")
        message(FATAL_ERROR "NXRT_LTO needs clang-cl and lld-link (compiler ${CMAKE_CXX_COMPILER_ID}, linker ${CMAKE_LINKER})")
    endif()
    add_compile_options($<$<COMPILE_LANGUAGE:C,CXX>:-flto=thin>)
endif()
message(STATUS "nxrt: ThinLTO ${NXRT_LTO} (linker ${CMAKE_LINKER})")
# Profile-guided builds of the runtime (docs/PGO.md). Release builds with clang 23 or newer use the committed profile,
# pgo/fefw_rt.profdata; a function whose source changed since it was recorded is built without it. NXRT_PGO_PROFILE names
# another profile, or "off" none. NXRT_PGO_GENERATE builds the instrumented runtime that records a new one (a scripted test run
# writes the counters when it ends: games/fefw/fefw_play.cpp).
option(NXRT_PGO_GENERATE "Instrumented runtime that records an LLVM profile (never for a release)" OFF)
set(NXRT_PGO_PROFILE "auto" CACHE STRING "LLVM profile of the runtime: auto (pgo/fefw_rt.profdata), off, or a .profdata file")
set(NXRT_PGO_RUNTIME_DIR "" CACHE PATH "Folder of clang_rt.profile.lib for NXRT_PGO_GENERATE, where the linker does not find it itself")
set(_nxrt_pgo "${NXRT_PGO_PROFILE}")
if(_nxrt_pgo STREQUAL "auto")
    set(_nxrt_pgo "")
    if(NOT NXRT_PGO_GENERATE AND CMAKE_BUILD_TYPE STREQUAL "Release" AND CMAKE_CXX_COMPILER_ID MATCHES "Clang"
       AND NOT CMAKE_CXX_COMPILER_VERSION VERSION_LESS 23 AND EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/pgo/fefw_rt.profdata")
        set(_nxrt_pgo "${CMAKE_CURRENT_SOURCE_DIR}/pgo/fefw_rt.profdata")
    endif()
elseif(_nxrt_pgo STREQUAL "off")
    set(_nxrt_pgo "")
endif()
if((NXRT_PGO_GENERATE OR _nxrt_pgo) AND NOT CMAKE_CXX_COMPILER_ID MATCHES "Clang")
    message(FATAL_ERROR "NXRT_PGO_GENERATE and NXRT_PGO_PROFILE need clang-cl")
endif()
if(NXRT_PGO_GENERATE AND _nxrt_pgo)
    message(FATAL_ERROR "NXRT_PGO_GENERATE records a profile and NXRT_PGO_PROFILE uses one: set only one of them")
endif()
if(NXRT_PGO_GENERATE)
    add_compile_options($<$<COMPILE_LANGUAGE:C,CXX>:-fprofile-instr-generate>)
    message(WARNING "nxrt: instrumented runtime (NXRT_PGO_GENERATE): for recording a profile only, not for a release")
elseif(_nxrt_pgo)
    if(NOT EXISTS "${_nxrt_pgo}")
        message(FATAL_ERROR "NXRT_PGO_PROFILE: no such file: ${_nxrt_pgo}")
    endif()
    # the file's hash in the command line: a new profile under the same name recompiles everything (stale objects would not link
    # together under ThinLTO), and a change of the file configures again
    file(MD5 "${_nxrt_pgo}" _nxrt_pgo_id)
    set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS "${_nxrt_pgo}")
    add_compile_options("$<$<COMPILE_LANGUAGE:C,CXX>:-fprofile-instr-use=${_nxrt_pgo}>"
                        $<$<COMPILE_LANGUAGE:C,CXX>:-Wno-profile-instr-out-of-date>
                        $<$<COMPILE_LANGUAGE:C,CXX>:-Wno-profile-instr-unprofiled>)
    add_compile_definitions($<$<COMPILE_LANGUAGE:C,CXX>:NXRT_PGO_PROFILE_ID_${_nxrt_pgo_id}>)
    message(STATUS "nxrt: profile ${_nxrt_pgo} (id ${_nxrt_pgo_id})")
else()
    message(STATUS "nxrt: no profile")
endif()
add_compile_definitions(NOMINMAX WIN32_LEAN_AND_MEAN _CRT_SECURE_NO_WARNINGS _WIN32_WINNT=0x0A00)

# Build performance options
option(NXRT_UNITY_BUILD "Enable unity (jumbo) builds for faster compilation" OFF)
option(NXRT_PCH "Use precompiled headers for common system headers (SDL3, Vulkan, ImGui, VMA)" ON)
if(NXRT_PCH AND NOT CMAKE_CXX_COMPILER_ID MATCHES "Clang")
    message(WARNING "Precompiled headers work best with clang-cl; disabling for ${CMAKE_CXX_COMPILER_ID}")
    set(NXRT_PCH OFF)
endif()

# The graphics shell is portable in its core (SDL3 window/input, Vulkan, ImGui) but still carries Windows-only pieces
# (DLSS/FFX/XeSS backends, the Win32 display-mode helper, the crash shortcut), so a Linux configure turns it off until those
# are ported. Windows keeps it on, which is the regression configuration.
if(WIN32)
    set(_nxrt_gfx_default ON)
else()
    set(_nxrt_gfx_default OFF)
endif()
option(NXRT_BUILD_GFX "Build the graphics shell (SDL3 + Vulkan + NVN front end, src/gfx, cmake/gfx.cmake)" ${_nxrt_gfx_default})
# Game modules (games/<id>/, docs/GAME_BOUNDARY.md): the title-specific code a game exe links. The test executables register one of them, so
# the tests of the base run against that game's tables (title facts, sites, probes, hooks) as the game exe does.
option(NXRT_BUILD_GAMES "Build the game modules in games/" ON)
set(NXRT_GAMES "fefw" CACHE STRING "ids of the game modules to build (games/<id>/), a list")
set(NXRT_TEST_GAME "fefw" CACHE STRING "id of the game module the test executables register (empty: none)")
if(NOT NXRT_BUILD_GAMES OR NOT NXRT_TEST_GAME IN_LIST NXRT_GAMES)
    set(NXRT_TEST_GAME "")      # the test game must be one of the modules built
endif()

file(GLOB_RECURSE NXRT_SOURCES CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/src/*.cpp")
list(FILTER NXRT_SOURCES EXCLUDE REGEX "/src/gfx/")   # the graphics shell is its own library (cmake/gfx.cmake)
list(FILTER NXRT_SOURCES EXCLUDE REGEX "/src/platform/")   # both backends live there; only this host's is compiled

# Subsystems that are Windows-only today: they call Win32 APIs that have no POSIX backend yet, not portable features
# that happen to include windows.h. A Linux build leaves them out, and docs/LINUX.md lists them with the replacement each
# one needs. Anything not on this list is expected to compile and run on both platforms, and a Windows-only call added to
# one of those files is a portability bug, not a configuration choice.
if(NOT WIN32)
    set(_nxrt_windows_only_sources
        src/diag/crash.cpp src/diag/ftrace.cpp src/diag/memguard.cpp src/diag/registry.cpp src/diag/sampler.cpp
        src/diag/symbols.cpp src/diag/watchdog.cpp src/diag/cpu_info.cpp src/diag/gauge.cpp src/diag/stackwalk.cpp
        src/media/movie_mf.cpp src/media/movie_ffmpeg.cpp
        src/services/album.cpp src/services/audio_devices.cpp src/services/audio_xaudio2.cpp
        src/services/logfault.cpp src/services/mods_dll.cpp
        src/eh/unwind.cpp
        src/testtools/tt_core.cpp src/testtools/tt_game.cpp src/testtools/tt_runner.cpp src/testtools/tt_script.cpp
    )
    foreach(_s ${_nxrt_windows_only_sources})
        list(FILTER NXRT_SOURCES EXCLUDE REGEX "/${_s}$")
    endforeach()
    list(LENGTH _nxrt_windows_only_sources _nxrt_excluded)
    message(STATUS "nxrt: ${_nxrt_excluded} Windows-only source files are not part of this build (docs/LINUX.md)")
endif()
file(GLOB NXRT_PLATFORM_SOURCES CONFIGURE_DEPENDS "${NXRT_PLATFORM_DIR}/*.cpp")
list(APPEND NXRT_SOURCES ${NXRT_PLATFORM_SOURCES})
include(cmake/zstd_deps.cmake)   # zstd for the shader pack and the .nx RomFS reader

# Precompiled header for nxrt core
if(NXRT_PCH)
    set(NXRT_PCH_HEADER "${CMAKE_CURRENT_SOURCE_DIR}/src/nxrt_pch.h")
    file(WRITE "${NXRT_PCH_HEADER}"
        "// nxrt precompiled header\n"
        "#include <nxrt/platform.h>\n"
        "#include <nxrt/memory.h>\n"
        "#include <nxrt/log.h>\n"
        "#include <nxrt/config.h>\n"
        "#include <nxrt/diag.h>\n"
        "#include <nxrt/gauge.h>\n"
        "#include <algorithm>\n"
        "#include <atomic>\n"
        "#include <chrono>\n"
        "#include <cstdint>\n"
        "#include <cstring>\n"
        "#include <memory>\n"
        "#include <mutex>\n"
        "#include <string>\n"
        "#include <string_view>\n"
        "#include <vector>\n"
    )
endif()

add_library(nxrt STATIC ${NXRT_SOURCES})
if(NXRT_PCH)
    target_precompile_headers(nxrt PRIVATE "${NXRT_PCH_HEADER}")
endif()
if(NXRT_UNITY_BUILD)
    set_target_properties(nxrt PROPERTIES UNITY_BUILD ON UNITY_BUILD_BATCH_SIZE 8)
endif()
target_include_directories(nxrt PUBLIC include)
if(EXISTS "${NXRT_RECOMP_INCLUDE}")
    target_include_directories(nxrt PUBLIC "${NXRT_RECOMP_INCLUDE}")
endif()
target_link_libraries(nxrt PUBLIC nxrt_zstd)
if(WIN32)
    target_link_libraries(nxrt PUBLIC psapi winhttp)   # winhttp: the release check (src/diag/update_check.cpp)
else()
    find_package(Threads REQUIRED)
    target_link_libraries(nxrt PUBLIC Threads::Threads ${CMAKE_DL_LIBS})
endif()
# Convert ON/OFF option() values to 1/0 for C++ #if expressions
if(NXRT_UPSCALER_FSR)
    set(_nxrt_upscaler_fsr 1)
else()
    set(_nxrt_upscaler_fsr 0)
endif()
if(NXRT_UPSCALER_DLSS)
    set(_nxrt_upscaler_dlss 1)
else()
    set(_nxrt_upscaler_dlss 0)
endif()
if(NXRT_UPSCALER_XESS)
    set(_nxrt_upscaler_xess 1)
else()
    set(_nxrt_upscaler_xess 0)
endif()
if(NXRT_UPSCALER_NIS)
    set(_nxrt_upscaler_nis 1)
else()
    set(_nxrt_upscaler_nis 0)
endif()
if(NXRT_UPSCALER_MAKO)
    set(_nxrt_upscaler_mako 1)
else()
    set(_nxrt_upscaler_mako 0)
endif()

# Propagate feature gates as compile definitions for conditional compilation in headers
target_compile_definitions(nxrt PUBLIC
    NXRT_DAV1D=${NXRT_DAV1D}
    NXRT_LIBVPX=${NXRT_LIBVPX}
    NXRT_UPSCALER_FSR=${_nxrt_upscaler_fsr}
    NXRT_UPSCALER_DLSS=${_nxrt_upscaler_dlss}
    NXRT_UPSCALER_XESS=${_nxrt_upscaler_xess}
    NXRT_UPSCALER_NIS=${_nxrt_upscaler_nis}
    NXRT_UPSCALER_MAKO=${_nxrt_upscaler_mako}
)
# The release check (include/nxrt/update_check.h). EmblemWeave builds the runtime on the player's PC, so the launcher names its
# release in fefw.ini ([update] version, repo) at every Play; a build may also name one here (NXRT_RELEASE_VERSION / NXRT_UPDATE_REPO,
# fefw-recomp's release package), which the ini overrides.
if(NXRT_RELEASE_VERSION)
    set(_nxrt_update_defs "NXRT_RELEASE_VERSION=\"${NXRT_RELEASE_VERSION}\"" "NXRT_UPDATE_REPO=\"${NXRT_UPDATE_REPO}\"")
    if(NXRT_RELEASE_BY_TAG)
        list(APPEND _nxrt_update_defs NXRT_RELEASE_BY_TAG=1)
    endif()
    set_source_files_properties("${CMAKE_CURRENT_SOURCE_DIR}/src/diag/update_check.cpp" PROPERTIES COMPILE_DEFINITIONS "${_nxrt_update_defs}")
endif()
include(cmake/media_deps.cmake)   # libvpx for the Switch 2 video decoders (optional)

if(NXRT_BUILD_TESTS)
    enable_testing()
    file(GLOB NXRT_TEST_SOURCES CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/tests/*.cpp")
    list(FILTER NXRT_TEST_SOURCES EXCLUDE REGEX "platform_selftest\\.cpp$")   # its own executable, below
    # The tests of the test game's own runtime-side code (games/<id>/tests/core) run in the same executable.
    if(NXRT_TEST_GAME)
        file(GLOB NXRT_GAME_CORE_TEST_SOURCES CONFIGURE_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/games/${NXRT_TEST_GAME}/tests/core/*.cpp")
        list(APPEND NXRT_TEST_SOURCES ${NXRT_GAME_CORE_TEST_SOURCES})
    endif()
    # The platform layer answers the runtime directly, so its self-test is its own executable rather than a group of nxrt_tests.
    add_executable(nxrt_platform_selftest "${CMAKE_CURRENT_SOURCE_DIR}/tests/platform_selftest.cpp")
    target_link_libraries(nxrt_platform_selftest PRIVATE nxrt)
    add_test(NAME platform_selftest COMMAND nxrt_platform_selftest)
    add_executable(nxrt_tests ${NXRT_TEST_SOURCES})
    target_include_directories(nxrt_tests PRIVATE tests)
    # nxrt calls into the graphics shell (switch2.cpp: HDR metadata, tt_core.cpp: screenshot writes), so the tests link it when it is built
    target_link_libraries(nxrt_tests PRIVATE nxrt $<TARGET_NAME_IF_EXISTS:nxrt_gfx>)
    # A code mod built against the public header, loaded by tests/test_mod_api.cpp: the mod side of the ABI.
    if(WIN32)
        add_library(nxrt_test_mod SHARED tests/mods/test_mod.cpp)
        target_include_directories(nxrt_test_mod PRIVATE include)
        if(EXISTS "${NXRT_RECOMP_INCLUDE}")
            target_include_directories(nxrt_test_mod PRIVATE "${NXRT_RECOMP_INCLUDE}")
        endif()
        add_dependencies(nxrt_tests nxrt_test_mod)
        target_compile_definitions(nxrt_tests PRIVATE NXRT_TEST_MOD_PATH="$<TARGET_FILE:nxrt_test_mod>")
    endif()
    target_compile_definitions(nxrt_tests PRIVATE NXRT_TEST_DATA_DIR="${CMAKE_CURRENT_SOURCE_DIR}/tests/data")
    # The NXFormat conformance fixtures are shared with EmblemWeave's Rust reader; keep one copy, not two.
    set(NXRT_EW_NX_FIXTURES "${CMAKE_CURRENT_SOURCE_DIR}/../../../crates/ew-format/tests/nx-fixtures")
    if(EXISTS "${NXRT_EW_NX_FIXTURES}/minimal_valid.nx")
        target_compile_definitions(nxrt_tests PRIVATE EW_NX_FIXTURE_DIR="${NXRT_EW_NX_FIXTURES}")
    endif()
    # A linker map next to the exe: the crash handler and the watchdog name host code from it. Keep host allocations low
    # (HIGHENTROPYVA:NO) so the guest ranges at 0x71_0000_0000 and up are free; a 64 MiB main stack.
    if(WIN32)
        target_link_options(nxrt_tests PRIVATE /MAP /STACK:67108864 /HIGHENTROPYVA:NO /DEBUG)
    endif()

    # One CTest entry per test group; the group names are listed by `nxrt_tests --list`. The list is read at configure
    # time from the NXTEST_GROUP lines of the test sources so that CMake needs no edit when a group is added.
    set(NXRT_GROUPS "")
    foreach(src ${NXRT_TEST_SOURCES})
        file(STRINGS "${src}" lines REGEX "^NXTEST_GROUP\\(")
        foreach(l ${lines})
            string(REGEX REPLACE "^NXTEST_GROUP\\(([a-z0-9_]+)\\).*$" "\\1" g "${l}")
            list(APPEND NXRT_GROUPS ${g})
        endforeach()
    endforeach()
    list(REMOVE_DUPLICATES NXRT_GROUPS)
    foreach(g ${NXRT_GROUPS})
        add_test(NAME ${g} COMMAND nxrt_tests ${g} WORKING_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}")
        set_tests_properties(${g} PROPERTIES TIMEOUT 120)
    endforeach()
    if(TEST write_watch)
        set_tests_properties(write_watch PROPERTIES TIMEOUT 300)   # 3000 region rebuilds under four faulting writers: 90-110 s on 4 cores
    endif()
    message(STATUS "nxrt test groups: ${NXRT_GROUPS}")

    # Lifted-code integration tests (tests/lift): need the recompiler, its Python environment and the AArch64 clang.
    option(NXRT_BUILD_LIFT_TESTS "Lift test programs with nxrecomp and run them on the runtime (CTest entries lift_*)" ON)
    if(NXRT_BUILD_LIFT_TESTS AND EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/../recomp/cmake/NxRecomp.cmake" AND EXISTS "${CMAKE_CURRENT_SOURCE_DIR}/../tools/.venv/Scripts/python.exe")
        add_subdirectory(tests/lift)
    else()
        message(STATUS "nxrt: lifted-code integration tests (tests/lift) are off: ../recomp or ../tools/.venv not found")
    endif()
endif()

if(NXRT_BUILD_GFX)
    include(cmake/gfx.cmake)
endif()

if(NXRT_BUILD_GAMES)
    foreach(_game ${NXRT_GAMES})
        add_subdirectory(games/${_game})
    endforeach()
endif()
# The launcher's branding and factory settings: the data part of one game module and its launcher.rc (version info, manifest). Without that
# module the launcher builds unbranded, with no resources.
if(TARGET mpr_launcher AND TARGET nxrt_game_${NXRT_LAUNCHER_GAME}_data)
    target_sources(mpr_launcher PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/games/${NXRT_LAUNCHER_GAME}/launcher.rc")
    target_link_libraries(mpr_launcher PRIVATE nxrt_game_${NXRT_LAUNCHER_GAME}_data)
    target_compile_definitions(mpr_launcher PRIVATE NXRT_LAUNCHER_GAME_REGISTER=nxrt_register_game_${NXRT_LAUNCHER_GAME}_data)
    target_link_options(mpr_launcher PRIVATE /MANIFEST:NO)   # launcher.rc carries the manifest
endif()
# Each test executable registers the most of the test game it links: the data part (launcher tests), the core part (runtime tests), the whole
# module (graphics tests). tests/test_main.cpp calls NXRT_TEST_GAME_REGISTER before the first test.
if(NXRT_BUILD_TESTS AND NXRT_TEST_GAME)
    set(_g "${NXRT_TEST_GAME}")
    foreach(pair "nxrt_tests;nxrt_game_${_g}_core;nxrt_register_game_${_g}_core" "nxrt_gfx_tests;nxrt_game_${_g};nxrt_register_game_${_g}"
                 "mpr_launcher_tests;nxrt_game_${_g}_data;nxrt_register_game_${_g}_data")
        list(GET pair 0 exe)
        list(GET pair 1 lib)
        list(GET pair 2 fn)
        if(TARGET ${exe} AND TARGET ${lib})
            target_link_libraries(${exe} PRIVATE ${lib})
            target_compile_definitions(${exe} PRIVATE NXRT_TEST_GAME_REGISTER=${fn})
        endif()
    endforeach()
endif()
