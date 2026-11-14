# SPDX-License-Identifier: GPL-3.0-or-later
# The differential test's host program: host.cpp + every generated test shard (DT_GEN_DIR, written by difftest.py gen).
cmake_minimum_required(VERSION 3.25)
project(nx_difftest C CXX)
set(CMAKE_CXX_STANDARD 20)
file(GLOB DT_SOURCES CONFIGURE_DEPENDS "${DT_GEN_DIR}/*.cpp")
add_executable(difftest host.cpp ${DT_SOURCES})
target_include_directories(difftest PRIVATE "${CMAKE_CURRENT_LIST_DIR}/../../include" "${DT_GEN_DIR}")
# -O0: thousands of tiny functions, compile speed matters; -ffp-contract=off and -fno-strict-aliasing as in the real build
target_compile_options(difftest PRIVATE /Od /w /EHsc -fno-strict-aliasing -ffp-contract=off -mcx16 -ferror-limit=0)
