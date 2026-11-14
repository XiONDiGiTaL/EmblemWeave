# SPDX-License-Identifier: GPL-3.0-or-later
# Integration tests: test programs are built as AArch64 ELFs (clang), lifted by the recompiler (nxrecomp, S1) and linked against the
# REAL runtime (nxrt) instead of the oracle's stand-in runtime; the driver (driver.cpp) maps the ELF with the runtime's loader, registers
# the lifted module, runs test_main on a runtime guest thread and compares results[] with the expected output:
#   EXPECT unicorn  Unicorn executing the original AArch64 code (S1's reference, recomp/oracle/e2e/unicorn_side.py)
#   EXPECT native   the same program text compiled for the host (tests/lift/native/rt2.h) and run with the host's C++ exceptions
# Needs ../recomp (lifter, ABI header, oracle helpers), ../tools/.venv (Python with unicorn) and LLVM clang for the AArch64 target.
set(NXRT_RECOMP_ROOT "${CMAKE_SOURCE_DIR}/../recomp")
include("${NXRT_RECOMP_ROOT}/cmake/NxRecomp.cmake")
set(NXRT_LIFT_TOOLS "${CMAKE_SOURCE_DIR}/tests/tools")
set(NXRT_LIFT_GUEST "${CMAKE_CURRENT_SOURCE_DIR}/guest")
file(GLOB NXRT_LIFT_HEADERS CONFIGURE_DEPENDS "${NXRT_LIFT_GUEST}/*.h" "${NXRT_RECOMP_ROOT}/oracle/e2e/progs/*.h")

# nxrt_lift_case(<name> SRC <file> EXPECT unicorn|native [SITES] [IGNORE_LAST] [MASK i,j] [TERMINATE FRAMES a,b] [SPAWN_LOG a,b])
function(nxrt_lift_case name)
  cmake_parse_arguments(L "SITES;IGNORE_LAST;TERMINATE;ABI_TEST" "SRC;EXPECT;MASK;FRAMES;SPAWN_LOG;THREADS" "" ${ARGN})
  set(out "${CMAKE_CURRENT_BINARY_DIR}/${name}")
  set(elf "${out}.elf")
  set(expected "${out}.expected")
  add_custom_command(OUTPUT "${elf}"
    COMMAND "${NX_PYTHON}" "${NXRT_LIFT_TOOLS}/guest_build.py" "${L_SRC}" "${elf}" -O2
    DEPENDS "${L_SRC}" "${NXRT_LIFT_TOOLS}/guest_build.py" ${NXRT_LIFT_HEADERS}
    COMMENT "guest ELF ${name}" VERBATIM)
  if(L_EXPECT STREQUAL "unicorn")
    add_custom_command(OUTPUT "${expected}"
      COMMAND "${NX_PYTHON}" "${NXRT_LIFT_TOOLS}/unicorn_expect.py" "${elf}" "${expected}"
      DEPENDS "${elf}" "${NXRT_LIFT_TOOLS}/unicorn_expect.py"
      COMMENT "expected results ${name} (Unicorn)" VERBATIM)
  else()
    add_executable(native_${name} "${L_SRC}" "${CMAKE_CURRENT_SOURCE_DIR}/native/native_main.cpp")
    target_include_directories(native_${name} PRIVATE "${CMAKE_CURRENT_SOURCE_DIR}/native" "${NXRT_RECOMP_ROOT}/oracle/e2e/progs")
    target_compile_options(native_${name} PRIVATE -Wno-everything)
    set_target_properties(native_${name} PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/native")
    add_custom_command(OUTPUT "${expected}"
      COMMAND native_${name} "${expected}"
      DEPENDS native_${name}
      COMMENT "expected results ${name} (host C++ exceptions)" VERBATIM)
  endif()
  set(lift_extra "")
  if(L_SITES)
    add_custom_command(OUTPUT "${out}.sites"
      COMMAND "${NX_PYTHON}" "${NXRT_LIFT_TOOLS}/make_sites.py" "${elf}" "${out}.sites"
      DEPENDS "${elf}" "${NXRT_LIFT_TOOLS}/make_sites.py" VERBATIM)
    set(lift_extra SITES "${out}.sites")
  endif()
  nxrecomp_add_module(lift_${name} INPUT "${elf}" NAME main SHARDS 2 JOBS 2 KIND static_elf ${lift_extra})
  add_executable(liftrun_${name} "${CMAKE_CURRENT_SOURCE_DIR}/driver.cpp")
  target_link_libraries(liftrun_${name} PRIVATE lift_${name} nxrt $<TARGET_NAME_IF_EXISTS:nxrt_gfx>)   # nxrt calls into the graphics shell
  target_link_options(liftrun_${name} PRIVATE /MAP /STACK:67108864 /HIGHENTROPYVA:NO /DEBUG)
  set_target_properties(liftrun_${name} PROPERTIES RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/bin")
  add_custom_target(lift_data_${name} DEPENDS "${elf}" "${expected}")
  add_dependencies(liftrun_${name} lift_data_${name})
  set(args --elf "${elf}" --expected "${expected}" --log "${out}.log")
  if(L_MASK)
    list(APPEND args --mask "${L_MASK}")
  endif()
  if(L_IGNORE_LAST)
    list(APPEND args --ignore-last)
  endif()
  if(L_TERMINATE)
    list(APPEND args --expect-terminate --frames "${L_FRAMES}")
  endif()
  if(L_THREADS)
    list(APPEND args --threads ${L_THREADS})
  endif()
  add_test(NAME lift_${name} COMMAND liftrun_${name} ${args} WORKING_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}")
  set_tests_properties(lift_${name} PROPERTIES TIMEOUT 300)
  if(L_ABI_TEST)
    add_test(NAME lift_${name}_abi_mismatch COMMAND liftrun_${name} ${args} --abi-mismatch WORKING_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}")
    set_tests_properties(lift_${name}_abi_mismatch PROPERTIES TIMEOUT 300 PASS_REGULAR_EXPRESSION "ABI mismatch refused")
  endif()
  if(L_SPAWN_LOG)
    add_test(NAME lift_${name}_exit COMMAND liftrun_${name} --elf "${elf}" --expected "${expected}" --log "${out}_exit.log"
             --spawn-expect-exit 134 --log-contains "${L_SPAWN_LOG}" WORKING_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}")
    set_tests_properties(lift_${name}_exit PROPERTIES TIMEOUT 300)
  endif()
endfunction()

set(S1 "${NXRT_RECOMP_ROOT}/oracle/e2e/progs")
set(PR "${CMAKE_CURRENT_SOURCE_DIR}/progs")

# S1's end-to-end programs (recomp/oracle/e2e/progs): every one of them, lifted and run on the real runtime. The expected results are what Unicorn
# computes from the original AArch64 code. The C++ exception programs 12-14 reach the unwinder through host services at fixed addresses
# (progs/rt.h); the rest need no exception support.
foreach(p 00_smoke.c 01_int.c 02_switch.c 03_fp.c 04_neon_int.c 05_neon_fp_mem.c 06_crypto_crc.c 07_lse.c 08_excl.c 10_virtual.cpp 11_tailcall.c
          12_eh_basic.cpp 13_eh_classes.cpp 14_eh_calls.cpp)
  string(REGEX REPLACE "[.](c|cpp)$" "" pname "${p}")
  set(abi_arg "")
  if(pname STREQUAL "00_smoke")
    set(abi_arg ABI_TEST)     # also: the module registered with a wrong ABI version must be refused
  endif()
  nxrt_lift_case(${pname} SRC "${S1}/${p}" EXPECT unicorn ${abi_arg})
endforeach()
# 09_tls reads TPIDRRO_EL0 and words behind it (results[100..110]), the distance of the thread pointer from a static buffer ([113]) and ends
# in a hash of everything it recorded ([285]): those depend on where the harness put the TLS regions (Unicorn: fixed addresses and a
# pattern; the runtime: its kernel-region TLS blocks), so they are masked. The 500 other words (TLS variable offsets and contents,
# local-exec addressing, TPIDR_EL0 writes) must match.
nxrt_lift_case(09_tls SRC "${S1}/09_tls.c" EXPECT unicorn MASK "100,101,102,103,104,105,106,107,108,109,110,113,285")

# This work's exception programs (libc++abi-shaped guest runtime, tests/lift/guest/rt2.h; expected from the host's exceptions)
nxrt_lift_case(19_eh_probe SRC "${PR}/19_eh_probe.cpp" EXPECT native SITES)
nxrt_lift_case(20_eh_hier SRC "${PR}/20_eh_hier.cpp" EXPECT native SITES)
nxrt_lift_case(21_eh_ptr SRC "${PR}/21_eh_ptr.cpp" EXPECT native SITES)
nxrt_lift_case(22_eh_rethrow_nested SRC "${PR}/22_eh_rethrow_nested.cpp" EXPECT native SITES)
nxrt_lift_case(23_eh_catchall_misc SRC "${PR}/23_eh_catchall_misc.cpp" EXPECT native SITES)
nxrt_lift_case(26_eh_threads SRC "${PR}/26_eh_threads.cpp" EXPECT native SITES THREADS 8)
nxrt_lift_case(24_eh_term_uncaught SRC "${PR}/24_eh_term_uncaught.cpp" EXPECT native SITES TERMINATE
               FRAMES "terminate_thrower,terminate_depth3,terminate_depth2,terminate_depth1,test_main"
               SPAWN_LOG "uncaught C++ exception,guest backtrace,terminate_thrower")
nxrt_lift_case(25_eh_term_noexcept SRC "${PR}/25_eh_term_noexcept.cpp" EXPECT native SITES TERMINATE
               FRAMES "noexcept_barrier,noexcept_depth2,noexcept_depth1,test_main")
