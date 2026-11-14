// FLAGS:
// EXPECT-INSN: \bb\s+0x[0-9a-f]+ <
// EXPECT-INSN: \bbr\s+x\d+
// EXPECT-INSN: \bbl\s+0x
/* Tail calls: musttail mutual recursion (100000 deep without stack growth), self tail recursion, tail calls through
 * function pointers, an lcg-driven 8-state machine that tail-calls between states, and plain calls that clang turns
 * into tail calls at -O2. */
#include "t.h"

typedef u64 (*tfn)(u64, u64, u64);   /* 3-argument targets, so musttail signatures match */
typedef u64 (*sfn)(u64, u64);

static volatile u64 g_seed = 0x7777aaaa5555ccccull;
static u64 ri, hsh;
static inline void put(u64 v) {
    hsh = (hsh ^ v) * 0x100000001b3ull + 0x9e37ull;
    if (ri < 400) results[ri++] = v;
}

/* 1. even/odd mutual recursion with musttail */
NOINLINE static u64 ev2(u64 n, u64 acc);
NOINLINE static u64 od2(u64 n, u64 acc);
NOINLINE static u64 ev2(u64 n, u64 acc) {
    if (n == 0) return acc;
    __attribute__((musttail)) return od2(n - 1, acc * 31 + n);
}
NOINLINE static u64 od2(u64 n, u64 acc) {
    if (n == 0) return acc ^ 0xffull;
    __attribute__((musttail)) return ev2(n - 1, acc + n * 7);
}

/* 2. self tail recursion */
NOINLINE static u64 selfrec(u64 n, u64 a) {
    if (n == 0) return a;
    __attribute__((musttail)) return selfrec(n - 1, a * 5 + n);
}

/* 3. eight operations, reached through tail calls via pointer */
#define OPF(K) NOINLINE static u64 op##K(u64 a, u64 b, u64 c) { return (a ^ ((u64)(K) << 56)) * 0x100000001b3ull + b + c + (K); }
OPF(0) OPF(1) OPF(2) OPF(3) OPF(4) OPF(5) OPF(6) OPF(7)
static const tfn ops8[8] = { op0, op1, op2, op3, op4, op5, op6, op7 };

NOINLINE static u64 mt_ptr(u64 idx, u64 b, u64 c) {
    tfn f = ops8[idx & 7];
    __attribute__((musttail)) return f(idx, b, c);
}
NOINLINE static u64 tail_ptr(tfn f, u64 a, u64 b, u64 c) { return f(a, b, c); }

/* 4. eight-state machine: each state tail-calls the next state chosen from the data */
NOINLINE static u64 st0(u64 s, u64 c);
NOINLINE static u64 st1(u64 s, u64 c);
NOINLINE static u64 st2(u64 s, u64 c);
NOINLINE static u64 st3(u64 s, u64 c);
NOINLINE static u64 st4(u64 s, u64 c);
NOINLINE static u64 st5(u64 s, u64 c);
NOINLINE static u64 st6(u64 s, u64 c);
NOINLINE static u64 st7(u64 s, u64 c);
static const sfn sm[8] = { st0, st1, st2, st3, st4, st5, st6, st7 };

#define ST(K) \
NOINLINE static u64 st##K(u64 s, u64 c) { \
    if (c == 0) return s; \
    s = (s * 0x9e3779b97f4a7c15ull) ^ ((u64)((K) + 1) << 40) ^ (s >> 29); \
    __attribute__((musttail)) return sm[(s >> 17) & 7](s, c - 1); }
ST(0) ST(1) ST(2) ST(3) ST(4) ST(5) ST(6) ST(7)

/* 5. plain code that clang turns into tail calls at -O2 */
NOINLINE static u64 helper_a(u64 x) { return x * 3 + 1; }
NOINLINE static u64 helper_b(u64 x) { return (x << 2) ^ 0x1234ull; }
NOINLINE static u64 plain1(u64 x) { return helper_a(x + 9); }
NOINLINE static u64 plain2(u64 x, u64 y) { return helper_b(x ^ y); }

void test_main(void) {
    u64 s = g_seed;
    put(ev2(100000, 1));                     /* 100000 tail-called steps */
    put(selfrec(30000, 3));
    for (int r = 0; r < 64; r++) {
        u64 k = lcg(&s);
        u64 a = lcg(&s);
        u64 b = lcg(&s);
        put(mt_ptr(k, a, b));
        put(tail_ptr(ops8[(k >> 3) & 7], a, b, k >> 3));
        put(plain1(a));
        put(plain2(a, b));
        put(sm[(k >> 6) & 7](a, 50 + (k & 63)));
        put(ev2(k & 1023, a));
        put(od2(k & 511, b));
    }
    results[ri++] = hsh;
}
