# SPDX-License-Identifier: GPL-3.0-or-later
# The known-answer tests' host program: ka_host.cpp + the generated cases (KA_GEN_DIR/ka_cases.cpp, written by run_ka.py).
cmake_minimum_required(VERSION 3.25)
project(nx_ka C CXX)
set(CMAKE_CXX_STANDARD 20)
add_executable(ka ka_host.cpp "${KA_GEN_DIR}/ka_cases.cpp")
target_include_directories(ka PRIVATE "${CMAKE_CURRENT_LIST_DIR}/../../include")
# as the difftest: -O0 (thousands of small statements, compile speed), -ffp-contract=off (no fused multiply-add),
# -fno-strict-aliasing and -mcx16 as in the real build
target_compile_options(ka PRIVATE /Od /w /EHsc -fno-strict-aliasing -ffp-contract=off -mcx16 -ferror-limit=0)
