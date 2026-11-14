/* SPDX-License-Identifier: GPL-3.0-or-later
 * Common header of the end-to-end test programs (oracle/e2e/progs/*.c, *.cpp).
 *
 * Each program is compiled by clang to a static AArch64 ELF and run twice: once by Unicorn (the reference) and once as
 * lifted code on x64. The harness then compares x0 at return and every writable byte (data, bss, stack).
 *
 * Contract of a program:
 *   - freestanding: no libc, no libm, no constructors, no host calls (except through rt.h, EH tests only);
 *   - defines `void test_main(void)` (extern "C" in C++), which the harness calls with a valid stack;
 *   - writes everything it wants checked into results[] (and may return a value in x0 by `return`ing it from test_main
 *     declared as u64);
 *   - is deterministic: no timers, no uninitialised reads, inputs from lcg();
 *   - first lines of the file: `// FLAGS: <extra clang flags>` (optional, e.g. -march=armv8.1-a) and any number of
 *     `// EXPECT-INSN: <regex>` lines: every regex must match at least one line of `llvm-objdump -d` of the final ELF,
 *     which proves that the program really exercises that instruction class (checked by oracle/e2e/run_e2e.py).
 */
#ifndef NX_T_H
#define NX_T_H
#include <stdint.h>
#include <stddef.h>

typedef uint8_t u8;   typedef int8_t s8;
typedef uint16_t u16; typedef int16_t s16;
typedef uint32_t u32; typedef int32_t s32;
typedef uint64_t u64; typedef int64_t s64;

#define NRES 512
u64 results[NRES];

#define NOINLINE __attribute__((noinline))
#define ALWAYS_INLINE __attribute__((always_inline)) inline

static inline u64 lcg(u64 *s) {            /* PCG-ish 64-bit step */
    *s = *s * 6364136223846793005ull + 1442695040888963407ull;
    u64 z = *s;
    z ^= z >> 33; z *= 0xff51afd7ed558ccdull; z ^= z >> 33;
    return z;
}
static inline u32 f2b(float f) { u32 b; __builtin_memcpy(&b, &f, 4); return b; }
static inline u64 d2b(double d) { u64 b; __builtin_memcpy(&b, &d, 8); return b; }
static inline float b2f(u32 b) { float f; __builtin_memcpy(&f, &b, 4); return f; }
static inline double b2d(u64 b) { double d; __builtin_memcpy(&d, &b, 8); return d; }
#endif
