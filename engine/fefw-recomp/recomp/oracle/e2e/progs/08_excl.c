// FLAGS:
// EXPECT-INSN: \bldaxr\b|\bldxr\b
// EXPECT-INSN: \bstlxr\b|\bstxr\b
// EXPECT-INSN: \bldaxp\b|\bldxp\b
// EXPECT-INSN: \bstlxp\b|\bstxp\b
// EXPECT-INSN: \bldar\b
// EXPECT-INSN: \bstlr\b
// EXPECT-INSN: \bldaxrb\b|\bldxrb\b
// EXPECT-INSN: \bldarh\b|\bldaxrh\b|\bldxrh\b
/* Plain ARMv8.0 (no LSE): every __atomic RMW/cas becomes an LDXR/STXR (LDXP/STXP for 128-bit) loop,
 * acquire loads and release stores become LDAR/STLR. Memory orders are switched at run time inside NOINLINE helpers. */
#include "t.h"

typedef unsigned __int128 u128;

#define NA 8
static u8   A8[NA];
static u16  A16[NA];
static u32  A32[NA];
static u64  A64[NA];
static u128 A128[NA];

static volatile u64 g_seed = 0x0badc0de13572468ull;
static u64 ri, hsh;

static inline void put(u64 v) {
    hsh = (hsh ^ v) * 0x100000001b3ull + 0x9e37ull;
    if (ri < 400) results[ri++] = v;
}
static inline void put128(u128 v) { put((u64)v); put((u64)(v >> 64)); }

#define ORD5(OP, ...) \
    switch (o) { \
    case 0:  return OP(__VA_ARGS__, __ATOMIC_RELAXED); \
    case 1:  return OP(__VA_ARGS__, __ATOMIC_ACQUIRE); \
    case 2:  return OP(__VA_ARGS__, __ATOMIC_RELEASE); \
    case 3:  return OP(__VA_ARGS__, __ATOMIC_ACQ_REL); \
    default: return OP(__VA_ARGS__, __ATOMIC_SEQ_CST); }

#define DEF_OPS(T, S) \
NOINLINE static T S##_add(T *p, T v, int o) { ORD5(__atomic_fetch_add, p, v) } \
NOINLINE static T S##_sub(T *p, T v, int o) { ORD5(__atomic_fetch_sub, p, v) } \
NOINLINE static T S##_and(T *p, T v, int o) { ORD5(__atomic_fetch_and, p, v) } \
NOINLINE static T S##_or(T *p, T v, int o)  { ORD5(__atomic_fetch_or, p, v) } \
NOINLINE static T S##_xor(T *p, T v, int o) { ORD5(__atomic_fetch_xor, p, v) } \
NOINLINE static T S##_nand(T *p, T v, int o) { ORD5(__atomic_fetch_nand, p, v) } \
NOINLINE static T S##_xchg(T *p, T v, int o) { ORD5(__atomic_exchange_n, p, v) } \
NOINLINE static T S##_load(T *p, int o) { \
    switch (o) { \
    case 0: return __atomic_load_n(p, __ATOMIC_RELAXED); \
    case 1: return __atomic_load_n(p, __ATOMIC_ACQUIRE); \
    default: return __atomic_load_n(p, __ATOMIC_SEQ_CST); } } \
NOINLINE static void S##_store(T *p, T v, int o) { \
    switch (o) { \
    case 0: __atomic_store_n(p, v, __ATOMIC_RELAXED); return; \
    case 1: __atomic_store_n(p, v, __ATOMIC_RELEASE); return; \
    default: __atomic_store_n(p, v, __ATOMIC_SEQ_CST); return; } } \
NOINLINE static int S##_cas(T *p, T *e, T d, int o) { \
    switch (o) { \
    case 0: return __atomic_compare_exchange_n(p, e, d, 0, __ATOMIC_RELAXED, __ATOMIC_RELAXED); \
    case 1: return __atomic_compare_exchange_n(p, e, d, 0, __ATOMIC_ACQUIRE, __ATOMIC_ACQUIRE); \
    case 2: return __atomic_compare_exchange_n(p, e, d, 0, __ATOMIC_RELEASE, __ATOMIC_RELAXED); \
    case 3: return __atomic_compare_exchange_n(p, e, d, 1, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE); \
    default: return __atomic_compare_exchange_n(p, e, d, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST); } }

#define DEF_MM(T, ST, S) \
NOINLINE static T S##_umax(T *p, T v) { \
    T old = __atomic_load_n(p, __ATOMIC_RELAXED); \
    for (;;) { if (!(old < v)) return old; \
        if (__atomic_compare_exchange_n(p, &old, v, 0, __ATOMIC_SEQ_CST, __ATOMIC_RELAXED)) return old; } } \
NOINLINE static T S##_umin(T *p, T v) { \
    T old = __atomic_load_n(p, __ATOMIC_RELAXED); \
    for (;;) { if (!(old > v)) return old; \
        if (__atomic_compare_exchange_n(p, &old, v, 0, __ATOMIC_SEQ_CST, __ATOMIC_RELAXED)) return old; } } \
NOINLINE static T S##_smax(T *p, T v) { \
    ST *q = (ST *)p; ST sv = (ST)v; \
    ST old = __atomic_load_n(q, __ATOMIC_RELAXED); \
    for (;;) { if (!(old < sv)) return (T)old; \
        if (__atomic_compare_exchange_n(q, &old, sv, 0, __ATOMIC_SEQ_CST, __ATOMIC_RELAXED)) return (T)old; } } \
NOINLINE static T S##_smin(T *p, T v) { \
    ST *q = (ST *)p; ST sv = (ST)v; \
    ST old = __atomic_load_n(q, __ATOMIC_RELAXED); \
    for (;;) { if (!(old > sv)) return (T)old; \
        if (__atomic_compare_exchange_n(q, &old, sv, 0, __ATOMIC_SEQ_CST, __ATOMIC_RELAXED)) return (T)old; } }

#define DEF_NR(T, S) \
NOINLINE static void S##_nadd_r(T *p, T v) { __atomic_fetch_add(p, v, __ATOMIC_RELAXED); } \
NOINLINE static void S##_nadd_l(T *p, T v) { __atomic_fetch_add(p, v, __ATOMIC_RELEASE); } \
NOINLINE static void S##_nclr_r(T *p, T v) { __atomic_fetch_and(p, (T)~v, __ATOMIC_RELAXED); } \
NOINLINE static void S##_nset_r(T *p, T v) { __atomic_fetch_or(p, v, __ATOMIC_RELAXED); } \
NOINLINE static void S##_neor_l(T *p, T v) { __atomic_fetch_xor(p, v, __ATOMIC_RELEASE); }

#define DEF_RUN(T, S, ARR) \
NOINLINE static void run_##S(u64 *s, int rounds) { \
    for (int r = 0; r < rounds; r++) { \
        int i = (int)(lcg(s) % NA); \
        int o = r % 5; \
        T v = (T)lcg(s); \
        T old; \
        put((u64)S##_add(&ARR[i], v, o)); put((u64)ARR[i]); \
        put((u64)S##_sub(&ARR[i], (T)(v * 3u + 1u), (o + 1) % 5)); put((u64)ARR[i]); \
        put((u64)S##_and(&ARR[i], (T)(~v | 0x0fu), (o + 2) % 5)); \
        put((u64)S##_or(&ARR[i], (T)(v >> 3), (o + 3) % 5)); \
        put((u64)S##_xor(&ARR[i], (T)(v ^ 0xa5u), (o + 4) % 5)); \
        put((u64)S##_nand(&ARR[i], (T)(v | 0x11u), (o + 1) % 5)); \
        put((u64)S##_xchg(&ARR[i], (T)(v + 7u), o)); \
        old = ARR[i]; \
        put((u64)S##_cas(&ARR[i], &old, (T)(v * 5u), o)); put((u64)old); put((u64)ARR[i]); \
        old = (T)(v + 1u); \
        put((u64)S##_cas(&ARR[i], &old, (T)(v - 9u), (o + 2) % 5)); put((u64)old); \
        put((u64)S##_load(&ARR[i], o % 3)); \
        S##_store(&ARR[i], (T)(v ^ 0x33u), o % 3); \
        S##_nadd_r(&ARR[i], v); S##_nadd_l(&ARR[i], (T)(v >> 1)); \
        S##_nclr_r(&ARR[i], v); S##_nset_r(&ARR[i], (T)(v >> 5)); S##_neor_l(&ARR[i], (T)(v << 1)); \
        put((u64)ARR[i]); \
        put((u64)S##_umax(&ARR[i], v)); put((u64)S##_umin(&ARR[i], (T)(v ^ 0x7fu))); \
        put((u64)S##_smax(&ARR[i], v)); put((u64)S##_smin(&ARR[i], (T)(v ^ 0x80u))); \
    } }

DEF_OPS(u8, f8)
DEF_OPS(u16, f16)
DEF_OPS(u32, f32)
DEF_OPS(u64, f64)
DEF_MM(u8, s8, f8)
DEF_MM(u16, s16, f16)
DEF_MM(u32, s32, f32)
DEF_MM(u64, s64, f64)
DEF_NR(u8, f8)
DEF_NR(u16, f16)
DEF_NR(u32, f32)
DEF_NR(u64, f64)
DEF_RUN(u8, f8, A8)
DEF_RUN(u16, f16, A16)
DEF_RUN(u32, f32, A32)
DEF_RUN(u64, f64, A64)

/* 128-bit: LDXP/STXP loops (LDAXP/STLXP for acquire/release orders) */
NOINLINE static u128 q_add(u128 *p, u128 v, int o) { ORD5(__atomic_fetch_add, p, v) }
NOINLINE static u128 q_and(u128 *p, u128 v, int o) { ORD5(__atomic_fetch_and, p, v) }
NOINLINE static u128 q_or(u128 *p, u128 v, int o)  { ORD5(__atomic_fetch_or, p, v) }
NOINLINE static u128 q_xor(u128 *p, u128 v, int o) { ORD5(__atomic_fetch_xor, p, v) }
NOINLINE static u128 q_xchg(u128 *p, u128 v, int o) { ORD5(__atomic_exchange_n, p, v) }
NOINLINE static int q_cas(u128 *p, u128 *e, u128 d, int o) {
    switch (o) {
    case 0: return __atomic_compare_exchange_n(p, e, d, 0, __ATOMIC_RELAXED, __ATOMIC_RELAXED);
    case 1: return __atomic_compare_exchange_n(p, e, d, 0, __ATOMIC_ACQUIRE, __ATOMIC_ACQUIRE);
    case 2: return __atomic_compare_exchange_n(p, e, d, 0, __ATOMIC_RELEASE, __ATOMIC_RELAXED);
    case 3: return __atomic_compare_exchange_n(p, e, d, 1, __ATOMIC_ACQ_REL, __ATOMIC_ACQUIRE);
    default: return __atomic_compare_exchange_n(p, e, d, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST); }
}
NOINLINE static u128 q_load(u128 *p, int o) {
    switch (o) {
    case 0: return __atomic_load_n(p, __ATOMIC_RELAXED);
    case 1: return __atomic_load_n(p, __ATOMIC_ACQUIRE);
    default: return __atomic_load_n(p, __ATOMIC_SEQ_CST); }
}

NOINLINE static void run_q(u64 *s, int rounds) {
    for (int r = 0; r < rounds; r++) {
        int i = (int)(lcg(s) % NA);
        int o = r % 5;
        u64 hi = lcg(s), lo = lcg(s);
        u128 v = ((u128)hi << 64) | lo;
        u128 old;
        put128(q_xchg(&A128[i], v, o));
        put128(q_add(&A128[i], v * 3 + 1, (o + 1) % 5));
        put128(q_and(&A128[i], ~v | ((u128)0xffffull << 100), (o + 2) % 5));
        put128(q_or(&A128[i], v >> 7, (o + 3) % 5));
        put128(q_xor(&A128[i], v ^ ((u128)1 << 127), (o + 4) % 5));
        old = A128[i];
        put((u64)q_cas(&A128[i], &old, v * 5, o)); put128(old); put128(A128[i]);
        old = v + 1;
        put((u64)q_cas(&A128[i], &old, ~v, (o + 2) % 5)); put128(old);
        put128(q_load(&A128[i], o % 3));
    }
}

static void init_all(u64 *s) {
    for (int i = 0; i < NA; i++) {
        A8[i]  = (u8)(lcg(s) | 0x80u);
        A16[i] = (u16)(lcg(s) | 0x8000u);
        A32[i] = (u32)(lcg(s) | 0x80000000u);
        A64[i] = lcg(s) | 0x8000000000000000ull;
        u64 hi = lcg(s) | 0x8000000000000000ull, lo = lcg(s);
        A128[i] = ((u128)hi << 64) | lo;
    }
}

void test_main(void) {
    u64 s = g_seed;
    init_all(&s);
    run_f8(&s, 40);
    run_f16(&s, 40);
    run_f32(&s, 40);
    run_f64(&s, 40);
    run_q(&s, 40);
    for (int i = 0; i < NA; i++) {
        put(A8[i]); put(A16[i]); put(A32[i]); put(A64[i]); put128(A128[i]);
    }
    results[ri++] = hsh;
}
