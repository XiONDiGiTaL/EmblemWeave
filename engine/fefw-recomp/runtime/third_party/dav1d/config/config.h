/*
 * dav1d 1.5.1 configuration for nxrt: Windows x64, clang-cl, 8-bit only. With NXRT_DAV1D_ASM=1 (CMake option NXRT_DAV1D_ASM, needs NASM)
 * the x86-64 assembly is built, as meson does by default; otherwise it is dav1d's generic C build (`-Denable_asm=false`).
 */

#pragma once

#ifndef NXRT_DAV1D_ASM
#define NXRT_DAV1D_ASM 0
#endif

#define ARCH_AARCH64 0

#define ARCH_ARM 0

#define ARCH_LOONGARCH 0

#define ARCH_LOONGARCH32 0

#define ARCH_LOONGARCH64 0

#define ARCH_PPC64LE 0

#define ARCH_RISCV 0

#define ARCH_RV32 0

#define ARCH_RV64 0

#define ARCH_X86 NXRT_DAV1D_ASM

#define ARCH_X86_32 0

#define ARCH_X86_64 NXRT_DAV1D_ASM

#define CONFIG_16BPC 0

#define CONFIG_8BPC 1

#define CONFIG_LOG 1

#define CONFIG_MACOS_KPERF 0

#define ENDIANNESS_BIG 0

#define HAVE_ALIGNED_ALLOC 0

#define HAVE_ASM NXRT_DAV1D_ASM

#define HAVE_AVX512ICL 0

#define HAVE_C11_GENERIC 1

#define HAVE_CLOCK_GETTIME 0

#define HAVE_DLSYM 0

#define HAVE_ELF_AUX_INFO 0

#define HAVE_GETAUXVAL 0

#define HAVE_IO_H 1

#define HAVE_MEMALIGN 0

#define HAVE_POSIX_MEMALIGN 0

#define HAVE_PTHREAD_GETAFFINITY_NP 0

#define HAVE_PTHREAD_NP_H 0

#define HAVE_PTHREAD_SETAFFINITY_NP 0

#define HAVE_PTHREAD_SETNAME_NP 0

#define HAVE_PTHREAD_SET_NAME_NP 0

#define HAVE_SYS_TYPES_H 1

#define HAVE_UNISTD_H 0

#define PIC 3

#define STACK_ALIGNMENT 16

#define TRIM_DSP_FUNCTIONS 0
