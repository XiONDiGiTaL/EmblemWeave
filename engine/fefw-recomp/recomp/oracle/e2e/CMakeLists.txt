# SPDX-License-Identifier: GPL-3.0-or-later
# The end-to-end oracle's build: every test ELF listed in ${E2E_CASES} (a cmake file written by run_e2e.py, setting
# E2E_CASE_NAMES and E2E_ELF_<case>) is lifted by nxrecomp and linked with the x64 runner into run_<case>.exe.
cmake_minimum_required(VERSION 3.25)
project(nx_e2e C CXX)
set(CMAKE_CXX_STANDARD 20)
include("${CMAKE_CURRENT_LIST_DIR}/../../cmake/NxRecomp.cmake")
include("${E2E_CASES}")
foreach(case ${E2E_CASE_NAMES})
  nxrecomp_add_module(lift_${case} INPUT "${E2E_ELF_${case}}" NAME main SHARDS 2 KIND static_elf JOBS 2 SITES "${E2E_SITES_${case}}")
  add_executable(run_${case} rt/x64_runner.cpp rt/eh_rt.cpp)
  target_link_libraries(run_${case} PRIVATE lift_${case})
  target_include_directories(run_${case} PRIVATE rt)
  target_compile_options(run_${case} PRIVATE /w /EHsc -mcx16)
endforeach()
