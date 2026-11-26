# SPDX-License-Identifier: GPL-3.0-or-later
# fefw.exe: the runtime (../../runtime) + the six lifted game libraries (built by ../../recomp/scripts/build_game.cmd fefw into ../../work/build/fefw).
#   pwsh game-build\scripts\build_fefw.ps1          configure + build into game-build\fefw\build\release
# Nothing derived from the game lives here: the libraries are imported from the work folder; the NSOs and the RomFS are read at run time
# where the dump is (the defaults below are baked into fefw_paths.h; [fs] exefs / main / romfs in fefw.ini override them).
cmake_minimum_required(VERSION 3.25)
# Static MSVC runtime (/MT), as the runtime and the lifted libraries are built.
if(NOT DEFINED CMAKE_MSVC_RUNTIME_LIBRARY)
    set(CMAKE_MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()
# The resource compiler: llvm-rc, next to the clang-cl this build already requires. The Windows SDK's rc.exe under a newer SDK
# folder has been seen to hang on the preprocessed .rc this project feeds it (it wrote its temp files and never returned), and
# llvm-rc compiles the same file at once. Set before project(), because the resource language is not enabled yet there: setting
# the cache afterwards makes CMake announce "you have changed variables that require your cache to be deleted", wipe the cache
# and configure again with its defaults, which is how a Release clang-cl build turned into a Debug clang++ one.
if(NOT CMAKE_RC_COMPILER OR CMAKE_RC_COMPILER MATCHES "[Rr][Cc]\\.exe$")
    get_filename_component(_llvm_bin "${CMAKE_CXX_COMPILER}" DIRECTORY)
    foreach(_llvm_rc "${_llvm_bin}/llvm-rc.exe" "${_llvm_bin}/llvm-rc")
        if(EXISTS "${_llvm_rc}")
            set(CMAKE_RC_COMPILER "${_llvm_rc}" CACHE FILEPATH "resource compiler" FORCE)
            break()
        endif()
    endforeach()
    # The compiler may be named without its folder; then take the first llvm-rc on PATH.
    if(NOT CMAKE_RC_COMPILER OR CMAKE_RC_COMPILER MATCHES "[Rr][Cc]\\.exe$")
        find_program(_llvm_rc_prog NAMES llvm-rc llvm-rc.exe)
        if(_llvm_rc_prog)
            set(CMAKE_RC_COMPILER "${_llvm_rc_prog}" CACHE FILEPATH "resource compiler" FORCE)
        endif()
    endif()
endif()
project(fefw_game C CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

get_filename_component(FEFW_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../.." ABSOLUTE)
set(FEFW_LIB_DIR "${FEFW_ROOT}/work/build/fefw" CACHE PATH "folder with fefw_{main,sdk,subsdk0,subsdk1,subsdk2,rtld}.lib (recomp scripts build_game.cmd fefw)")
# Defaults only: fefw.ini ([fs] exefs / main / romfs, written by EmblemWeave) says where the dump is. game/dump/exefs is the copy
# tools/nso/prepare_exefs.py makes from the dump (main uncompressed), so [fs] main can stay empty.
set(FEFW_EXEFS_DEFAULT "game/dump/exefs" CACHE PATH "default [fs] exefs")
set(FEFW_ROMFS_DEFAULT "game/dump/romfs" CACHE PATH "default [fs] romfs")
set(FEFW_MAIN_DEFAULT "" CACHE FILEPATH "default [fs] main: main as an uncompressed NSO (empty: <exefs>/main)")
configure_file(src/fefw_paths.h.in "${CMAKE_CURRENT_BINARY_DIR}/fefw_paths.h" @ONLY)

# clang-cl takes a clang argument through -clang:, the other drivers take it as it is. What tells the two apart is the frontend variant: on a
# Windows host the clang++ driver also reports a simulated MSVC ABI (CMAKE_CXX_SIMULATE_ID is "MSVC" for it), so asking that handed
# "-clang:" to clang++ and the build stopped on "unknown argument: '-clang:-fmacro-prefix-map=...'" (issues #22 and #25).
if(CMAKE_CXX_COMPILER_FRONTEND_VARIANT STREQUAL "MSVC")
    add_compile_options("-clang:-fmacro-prefix-map=${FEFW_ROOT}=.")
else()
    add_compile_options("-fmacro-prefix-map=${FEFW_ROOT}=.")
endif()
set(FEFW_RUNTIME_DIR "${FEFW_ROOT}/runtime" CACHE PATH "runtime source tree compiled into fefw.exe")
set(NXRT_BUILD_TESTS OFF CACHE BOOL "" FORCE)
set(NXRT_GAMES "fefw" CACHE STRING "" FORCE)            # runtime/games/fefw only
set(NXRT_RECOMP_INCLUDE "${FEFW_ROOT}/recomp/include" CACHE PATH "")
# The ASTC transcoder: the ISPC build of tools/textures when it exists, else built from its sources (cmake/gfx.cmake). Only without
# either is the runtime built with its stand-in. An OFF that an earlier configure cached for want of the ISPC build is dropped once.
if(DEFINED CACHE{NXRT_ASTC2BC7})
    get_property(_fefw_a2b_doc CACHE NXRT_ASTC2BC7 PROPERTY HELPSTRING)
    if(_fefw_a2b_doc STREQUAL "Link the ASTC -> BC7 transcoder of tools/textures")
        unset(NXRT_ASTC2BC7 CACHE)
    endif()
endif()
if(NOT DEFINED NXRT_ASTC2BC7 AND NOT EXISTS "${FEFW_ROOT}/tools/textures/build/release/astc2bc7.lib" AND
   NOT EXISTS "${FEFW_ROOT}/tools/textures/third_party/astc-encoder/Source/astcenc.h")
    set(NXRT_ASTC2BC7 OFF CACHE BOOL "Link the ASTC -> BC7 transcoder of tools/textures (OFF: no ISPC build and no sources)")
endif()
# Release package (FEFW_RT_DLL=ON): no game code in anything built here. The runtime and the boot code become fefw_rt.dll (+ its import
# library fefw_rt.lib); fefw.exe is linked on the player's PC from the libraries lifted out of their own dump and src/fefw_stub.cpp
# (tools/build/fefw_build.py in the package, with the bundled Zig). Nothing lifted is imported here.
option(FEFW_RT_DLL "build the runtime as fefw_rt.dll for the release package" OFF)
if(FEFW_RT_DLL)
    # Debug info in every object (/Z7: in the .obj, which sccache can cache; /Zi's shared .pdb it cannot), so the link below can write
    # fefw_rt.pdb. The release publishes it beside the package (not in it) to turn crash reports' fefw_rt.dll+0x... into functions and lines
    # (tools/symbolize.py). Set before the runtime's targets exist: CMP0141 (3.25) reads it when a target is made. Codegen is unchanged.
    set(CMAKE_MSVC_DEBUG_INFORMATION_FORMAT Embedded)
endif()
add_subdirectory("${FEFW_RUNTIME_DIR}" runtime)

if(FEFW_RT_DLL AND NOT NXRT_ASTC2BC7)
    message(FATAL_ERROR "FEFW_RT_DLL=ON needs the ASTC transcoder (NXRT_ASTC2BC7=ON): without it desktop GPUs render most textures black")
endif()
if(FEFW_RT_DLL)
    add_library(fefw_rt SHARED src/fefw_main.cpp src/fefw_rt.def)
    target_include_directories(fefw_rt PRIVATE "${CMAKE_CURRENT_BINARY_DIR}" src)
    target_compile_definitions(fefw_rt PRIVATE FEFW_RT_DLL=1)
    target_link_libraries(fefw_rt PRIVATE nxrt_game_fefw nxrt_gfx nxrt user32)
    # fefw_rt.pdb next to the DLL, with only its bare name recorded in the DLL (/PDBALTPATH: no build machine path, as before);
    # /DEBUG turns /OPT:REF and /OPT:ICF off unless they are named, so they are: unused code stays dropped and identical functions folded.
    # Low host allocations as for fefw.exe (the guest ranges at 0x71_0000_0000 stay free).
    target_link_options(fefw_rt PRIVATE /HIGHENTROPYVA:NO /DEBUG /PDBALTPATH:fefw_rt.pdb /OPT:REF /OPT:ICF /INCREMENTAL:NO /Brepro)
    target_compile_options(fefw_rt PRIVATE "-clang:-ffile-prefix-map=${FEFW_ROOT}=.")
    add_custom_command(TARGET fefw_rt POST_BUILD COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${FEFW_RUNTIME_DIR}/assets/fonts/DejaVuSans.ttf" "$<TARGET_FILE_DIR:fefw_rt>/fonts/DejaVuSans.ttf"
        COMMAND ${CMAKE_COMMAND} -E copy_if_different
        "${FEFW_RUNTIME_DIR}/assets/fonts/Sora.ttf" "$<TARGET_FILE_DIR:fefw_rt>/fonts/Sora.ttf")
    foreach(dll ${NXRT_A2B_DLLS})
        add_custom_command(TARGET fefw_rt POST_BUILD COMMAND ${CMAKE_COMMAND} -E copy_if_different "${dll}" "$<TARGET_FILE_DIR:fefw_rt>")
    endforeach()
    return()
endif()

set(FEFW_LIBS "")
foreach(m main sdk subsdk0 subsdk1 subsdk2 rtld)
    add_library(fefw_${m} STATIC IMPORTED GLOBAL)
    set_target_properties(fefw_${m} PROPERTIES IMPORTED_LOCATION "${FEFW_LIB_DIR}/fefw_${m}.lib")
    list(APPEND FEFW_LIBS fefw_${m})
endforeach()

# The native routines the lifted code calls (GameConfig.inline_replace in recomp/games/fefw/config.py, such as
# nx_hle_zstd_multiframe) live in runtime/games/fefw/fefw_hle.cpp: nxrt_game_fefw below carries them into this exe, and
# fefw_rt.dll carries and exports them for the package (game-build/release/fefw_build.py links only fefw_stub.cpp and fefw_rt.lib).
add_executable(fefw src/fefw_main.cpp res/fefw.rc)
set_source_files_properties(res/fefw.rc PROPERTIES OBJECT_DEPENDS "${CMAKE_CURRENT_SOURCE_DIR}/res/fefw.manifest;${CMAKE_CURRENT_SOURCE_DIR}/res/fefw.ico")
target_include_directories(fefw PRIVATE "${CMAKE_CURRENT_BINARY_DIR}")
target_link_libraries(fefw PRIVATE ${FEFW_LIBS} nxrt_game_fefw nxrt_gfx nxrt nxrt_zstd)
# As pkp.exe: a linker map next to the exe (host frames get names), a big main stack (lifted code nests deeply), low host allocations so
# the guest ranges at 0x71_0000_0000 and up stay free. res/fefw.rc carries the manifest (UTF-8 code page).
# /OPT:REF discards code nothing reaches and /OPT:ICF folds identical functions and
# constants; /DEBUG leaves both off by default, so say them. The recompiled code repeats
# the same small helpers in every translation unit, and folding them makes the executable
# smaller and its instruction cache footprint lower.
target_link_options(fefw PRIVATE /MAP /STACK:134217728 /HIGHENTROPYVA:NO /DEBUG /MANIFEST:NO /OPT:REF /OPT:ICF)

# The shared-font fallback (runtime src/services/pl.cpp looks in fonts\ next to the exe) and the ASTC decoder DLLs when the transcoder is in.
# These two files are copied when the build is configured, not in a POST_BUILD step: CMake wraps that step in cmd.exe's
# `cd /D "<build dir>"`, and an `&` in the path splits the command (issue #38).
file(MAKE_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/fonts")
configure_file("${FEFW_RUNTIME_DIR}/assets/fonts/DejaVuSans.ttf" "${CMAKE_CURRENT_BINARY_DIR}/fonts/DejaVuSans.ttf" COPYONLY)
configure_file("${FEFW_RUNTIME_DIR}/assets/fonts/Sora.ttf" "${CMAKE_CURRENT_BINARY_DIR}/fonts/Sora.ttf" COPYONLY)
# The F1 menu's translation tables ([gfx] ui_language, runtime src/gfx/menu_i18n.cpp reads locales\<name>.ini next to the exe). Copied at
# configure like the fonts, so every build carries the current ones; build_fefw.ps1 configures before each build.
file(GLOB FEFW_MENU_LOCALES CONFIGURE_DEPENDS "${FEFW_ROOT}/locales/*.ini")
foreach(_loc ${FEFW_MENU_LOCALES})
    get_filename_component(_loc_name "${_loc}" NAME)
    configure_file("${_loc}" "${CMAKE_CURRENT_BINARY_DIR}/locales/${_loc_name}" COPYONLY)
endforeach()
# The processor features this build's lifted libraries need (recomp/games/fefw/CMakeLists.txt FEFW_CPU_VARIANT): the stub passes the
# them to the runtime, which checks the processor before the first game instruction (fefw_cpu.h). EmblemWeave names the variant in
# FEFW_CPU_VARIANT (apps/emblemweave/src/cpu.rs): fma (FMA3 + AVX, the default) or compat (SSE4.2 + POPCNT).
set(FEFW_CPU_VARIANT "fma" CACHE STRING "processor target of the lifted code: fma (FMA3 + AVX) or compat (SSE4.2 + POPCNT)")
if(DEFINED ENV{FEFW_CPU_VARIANT} AND NOT "$ENV{FEFW_CPU_VARIANT}" STREQUAL "")
    set(FEFW_CPU_VARIANT "$ENV{FEFW_CPU_VARIANT}" CACHE STRING "processor target of the lifted code" FORCE)
endif()
if(FEFW_CPU_VARIANT STREQUAL "compat")
    target_compile_definitions(fefw PRIVATE FEFW_CPU_NEEDS=FEFW_CPU_NEEDS_COMPAT_BUILD)
else()
    target_compile_definitions(fefw PRIVATE FEFW_CPU_NEEDS=FEFW_CPU_NEEDS_FMA_BUILD)
endif()
# The site list the libraries were lifted with (recomp/games/fefw/CMakeLists.txt), as fefw.sites for the runtime's checks of code mods.
if(EXISTS "${FEFW_LIB_DIR}/main.sites")
    configure_file("${FEFW_LIB_DIR}/main.sites" "${CMAKE_CURRENT_BINARY_DIR}/fefw.sites" COPYONLY)
endif()
foreach(dll ${NXRT_A2B_DLLS})
    add_custom_command(TARGET fefw POST_BUILD COMMAND ${CMAKE_COMMAND} -E copy_if_different "${dll}" "$<TARGET_FILE_DIR:fefw>")
endforeach()
