# SPDX-License-Identifier: GPL-3.0-or-later
#
# Fire Emblem: Fortune's Weave: lift the NSOs of the user's dump and build one static library per module.
#
#   scripts\build_game.cmd fefw                 (configure + build into work\build\fefw; NX_ROOT is the repository root)
#
# Inputs (read-only): <root>\game\dump\exefs\{main,sdk,subsdk0,subsdk1,subsdk2,rtld}; main must be the uncompressed NSO (the console's
# is ZBIC-compressed, which nxrecomp/nso.py does not read). Generated C++ goes to <root>\work\lift\fefw\<module>, build output to
# work\build\fefw: neither is ever committed. Targets: fefw_main, fefw_sdk, fefw_subsdk0, fefw_subsdk1, fefw_subsdk2, fefw_rtld
# (static libraries, each provides `nxmod_<name>`: ctx.h NxModule), `fefw_modules` builds all six, `fefw_link_check` links them.
cmake_minimum_required(VERSION 3.25)
project(fefw_lift C CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
include("${CMAKE_CURRENT_LIST_DIR}/../../cmake/NxRecomp.cmake")

get_filename_component(FEFW_ROOT_DEFAULT "${CMAKE_CURRENT_LIST_DIR}/../../.." ABSOLUTE)
set(FEFW_ROOT "${FEFW_ROOT_DEFAULT}" CACHE PATH "repository root (game/, work/)")
if(DEFINED NX_ROOT)
    get_filename_component(FEFW_ROOT "${NX_ROOT}" ABSOLUTE)
    set(FEFW_ROOT "${FEFW_ROOT}" CACHE PATH "repository root (game/, work/)" FORCE)
endif()
set(FEFW_EXEFS "${FEFW_ROOT}/game/dump/exefs" CACHE PATH "folder with the NSOs main (uncompressed), sdk, subsdk0-2, rtld")
set(FEFW_GEN "${FEFW_ROOT}/work/lift/fefw" CACHE PATH "where the generated sources go")
# translation units per module: about 45k instruction words each (docs/S1b.md)
set(FEFW_SHARDS_MAIN 153 CACHE STRING "translation units of main (6.89 M instruction words)")
set(FEFW_SHARDS_SDK 50 CACHE STRING "translation units of sdk (2.23 M)")
set(FEFW_SHARDS_SUBSDK0 5 CACHE STRING "translation units of subsdk0, dlss (233 k)")
set(FEFW_SHARDS_SUBSDK1 14 CACHE STRING "translation units of subsdk1, cuda (620 k)")
set(FEFW_SHARDS_SUBSDK2 3 CACHE STRING "translation units of subsdk2, movie (136 k)")
set(FEFW_SITES "${CMAKE_CURRENT_LIST_DIR}/main.sites" CACHE FILEPATH "hook sites of main for this build")
# the lifts run one at a time, each with FEFW_LIFT_JOBS processes, and every compile waits for all of them (target fefw_lift)
set(FEFW_LIFT_JOBS 10 CACHE STRING "lifter processes")
set_property(GLOBAL APPEND PROPERTY JOB_POOLS fefw_lift=1)

nxrecomp_add_module(fefw_main INPUT "${FEFW_EXEFS}/main" NAME main INDEX 0 BASE 0x7100000000 GAME fefw SHARDS ${FEFW_SHARDS_MAIN}
                    SITES "${FEFW_SITES}" JOBS ${FEFW_LIFT_JOBS} JOB_POOL fefw_lift OUTDIR "${FEFW_GEN}/main")
nxrecomp_add_module(fefw_sdk INPUT "${FEFW_EXEFS}/sdk" NAME sdk INDEX 1 BASE 0x7500000000 GAME fefw SHARDS ${FEFW_SHARDS_SDK}
                    JOBS ${FEFW_LIFT_JOBS} JOB_POOL fefw_lift OUTDIR "${FEFW_GEN}/sdk")
nxrecomp_add_module(fefw_subsdk0 INPUT "${FEFW_EXEFS}/subsdk0" NAME subsdk0 INDEX 2 BASE 0x7600000000 GAME fefw SHARDS ${FEFW_SHARDS_SUBSDK0}
                    JOBS ${FEFW_LIFT_JOBS} JOB_POOL fefw_lift OUTDIR "${FEFW_GEN}/subsdk0")
nxrecomp_add_module(fefw_subsdk1 INPUT "${FEFW_EXEFS}/subsdk1" NAME subsdk1 INDEX 3 BASE 0x7640000000 GAME fefw SHARDS ${FEFW_SHARDS_SUBSDK1}
                    JOBS ${FEFW_LIFT_JOBS} JOB_POOL fefw_lift OUTDIR "${FEFW_GEN}/subsdk1")
nxrecomp_add_module(fefw_subsdk2 INPUT "${FEFW_EXEFS}/subsdk2" NAME subsdk2 INDEX 4 BASE 0x7680000000 GAME fefw SHARDS ${FEFW_SHARDS_SUBSDK2}
                    JOBS ${FEFW_LIFT_JOBS} JOB_POOL fefw_lift OUTDIR "${FEFW_GEN}/subsdk2")
nxrecomp_add_module(fefw_rtld INPUT "${FEFW_EXEFS}/rtld" NAME rtld INDEX 5 BASE 0x7700000000 GAME fefw SHARDS 1
                    JOBS ${FEFW_LIFT_JOBS} JOB_POOL fefw_lift OUTDIR "${FEFW_GEN}/rtld")
set(FEFW_MODULES fefw_main fefw_sdk fefw_subsdk0 fefw_subsdk1 fefw_subsdk2 fefw_rtld)
set(lifted "")
foreach(m main sdk subsdk0 subsdk1 subsdk2 rtld)
    list(APPEND lifted "${FEFW_GEN}/${m}/funcs.h")
endforeach()
add_custom_target(fefw_lift DEPENDS ${lifted})
foreach(t ${FEFW_MODULES})
    add_dependencies(${t} fefw_lift)
endforeach()
add_custom_target(fefw_modules DEPENDS ${FEFW_MODULES})
# FMA3 code generation: the lifted FMADD / FMLA family is std::fma, which without it is a call into the CRT's fmaf (16 % of the game's
# job workers in Dagsion, notes/pacing.md). A fused result is exactly rounded either way; -ffp-contract=off still keeps separate
# multiplies and adds apart. Needs an x64 CPU with FMA3 (2013 on).
foreach(t ${FEFW_MODULES})
    target_compile_options(${t} PRIVATE -mfma)
endforeach()
# Indirect calls look the target up at the call site in the runtime's flat table (nxrecomp/cxx.h, runtime core/dispatch.cpp).
foreach(t ${FEFW_MODULES})
    target_compile_definitions(${t} PRIVATE NX_INLINE_INDIRECT)
endforeach()

# Link check (target fefw_link_check): the six libraries plus stubs for what the runtime provides. Proves every symbol resolves.
add_executable(fefw_link_check EXCLUDE_FROM_ALL link_check.cpp)
target_link_libraries(fefw_link_check PRIVATE ${FEFW_MODULES})
target_compile_options(fefw_link_check PRIVATE /w /EHs-c- /GR-)
