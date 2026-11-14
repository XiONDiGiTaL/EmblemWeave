// FLAGS:
// EXPECT-INSN: \bmadd\b|\bmul\b
// EXPECT-INSN: \bsmulh\b
// EXPECT-INSN: \bumulh\b
// EXPECT-INSN: \bsdiv\b
// EXPECT-INSN: \budiv\b
// EXPECT-INSN: \bclz\b
// EXPECT-INSN: \brbit\b
// EXPECT-INSN: \bcsel\b
// EXPECT-INSN: \bcsinc\b|\bcset\b
// EXPECT-INSN: \bccmp\b
// EXPECT-INSN: \bldp\b
// EXPECT-INSN: \bstp\b
// EXPECT-INSN: \bldrsw\b
// EXPECT-INSN: \bldrsb\b
// EXPECT-INSN: \bubfx\b|\bubfiz\b|\bubfm\b|\blsr\b
// EXPECT-INSN: \bcbz\b|\bcbnz\b
// EXPECT-INSN: \btbz\b|\btbnz\b
// EXPECT-INSN: \bb\.(eq|ne|lt|ge|hi|ls)\b
// EXPECT-INSN: \badcs?\b|\bsbcs?\b
#include "t.h"

/* Integer test. Every value that feeds an operation is read from the volatile table V[] (edge values plus lcg() output),
 * and every operation lives in a NOINLINE function, so clang cannot fold the work away. Each result is folded into a
 * rolling hash over results[] (512 slots), so one wrong value changes some slot. Divisions that are undefined in C
 * (zero divisor, INT_MIN / -1) take their divisor from a volatile variable the compiler cannot see through, so the
 * compiler emits the plain divide instruction and the architectural result (x/0 = 0, INT_MIN/-1 = INT_MIN) is what is
 * tested. */

#define NV 48
#define NEDGE 26

static volatile u64 V[NV];
static u64 dat[512] __attribute__((aligned(16)));   /* 4 KiB data area */
static u32 K;

typedef u16 __attribute__((may_alias)) u16m;
typedef u32 __attribute__((may_alias)) u32m;
typedef s32 __attribute__((may_alias)) s32m;
typedef u8 __attribute__((may_alias)) u8m;
typedef u64 __attribute__((aligned(1), may_alias)) u64u;
typedef struct { u64 x, y; } P2;
typedef struct { s32 x, y; } P32;

static const u64 EDGE[NEDGE] = {
    0, 1, 2, 3, 7, 0xff, 0x7f, 0x80, 0xffff, 0x8000, 0x7fffffffull, 0x80000000ull, 0xffffffffull, 0x100000000ull,
    0x7fffffffffffffffull, 0x8000000000000000ull, 0xffffffffffffffffull, 0xfffffffffffffffeull,
    0x0123456789abcdefull, 0xfedcba9876543210ull, 0xdeadbeefull, 0xffffffff80000001ull, 0x80000001ull,
    0x40, 0x1000, 0xfffffffffffff000ull
};

static NOINLINE void put(u64 x) {
    results[K] = results[K] * 0x100000001b3ull + x;
    K = (K + 1) & (NRES - 1);
}
#define R(x) put((u64)(x))

/* Divisor sources the compiler cannot fold: a zero, and a minus one in each width. */
static volatile u64 VZERO = 0;
static volatile s64 VNEG1 = -1;
static volatile s32 VNEG1W = -1;

/* divides, operands opaque to the compiler (the divisor is always a volatile load at the call site) */
static NOINLINE u64 udiv64(u64 a, u64 b) { return a / b; }
static NOINLINE u32 udiv32(u32 a, u32 b) { return a / b; }
static NOINLINE s64 sdiv64(s64 a, s64 b) { return a / b; }
static NOINLINE s32 sdiv32(s32 a, s32 b) { return a / b; }

/* a carry chain of two comparisons joined by && and ||, with the operands arriving through volatile reads; clang at -O2
 * turns each into cmp + ccmp (or ccmn) */
static NOINLINE u32 chain_and(u64 a, u64 b, u64 c) {
    u32 r;
    if (a == b && c < b) r = 1; else r = 2;
    return r;
}
static NOINLINE u32 chain_or(u64 a, u64 b, u64 c) {
    u32 r;
    if (a < b || c > b) r = 3; else r = 4;
    return r;
}

static NOINLINE void sec_arith(u64 a, u64 b, u64 c) {
    u32 a32 = (u32)a, b32 = (u32)b, c32 = (u32)c;
    u64 r; int ov; s32 sr;
    R(a + b); R(a - b); R(a32 + b32); R(a32 - b32);
    R(a + (b << 4)); R(c - (a >> 3)); R(c32 + (a32 << 2));
    R(a + (u64)b32); R(a - (u64)(s64)(s32)b32); R(a + (u64)(u16)b); R(a + (u64)(s64)(s8)b);
    R(a + 0x123); R(a - 0x7ff); R(a32 + 0x1000); R(a32 - 0xabc000u);
    ov = __builtin_add_overflow(a, b, &r); R(r); R(ov);          /* adds + cset */
    ov = __builtin_sub_overflow(a, b, &r); R(r); R(ov);          /* subs + cset */
    ov = __builtin_add_overflow((s32)a32, (s32)b32, &sr); R(sr); R(ov);  /* adds w */
    R(a == b); R(a < b); R(a > c); R(a32 <= c32); R((s64)a < (s64)b); R((s32)a32 >= (s32)b32);
    R(a == (0 - b));                                    /* cmn */
    R((s64)a < (s64)(0 - b));                           /* cmn signed */
    R((s32)a32 < (s32)(0u - b32));
    R(0 - a); R(0 - a32);                               /* neg */
    R(a + b + (c & 1));
}

static NOINLINE void sec_mul(u64 a, u64 b, u64 c) {
    u32 a32 = (u32)a, b32 = (u32)b, c32 = (u32)c;
    R(a * b); R(c + a * b); R(c - a * b); R(0 - a * b);      /* mul, madd, msub, mneg */
    R(a32 * b32); R(c32 + a32 * b32); R(c32 - a32 * b32);     /* 32-bit forms */
    R((s64)(s32)a * (s64)(s32)b);                             /* smull */
    R((u64)a32 * b32);                                        /* umull */
    R(c + (s64)(s32)a * (s64)(s32)b);                         /* smaddl */
    R(c + (u64)a32 * (u64)b32);                               /* umaddl */
    R(c - (s64)(s32)a * (s64)(s32)b);                         /* smsubl */
    R((u64)(0 - ((s64)(s32)a * (s64)(s32)b)));                /* smnegl */
    R((u64)(((__int128)(s64)a * (s64)b) >> 64));              /* smulh */
    R((u64)(((unsigned __int128)a * b) >> 64));               /* umulh */
    /* carry chains: each addc/subc consumes the carry the previous one produced (adc / adcs / sbc / sbcs) */
    unsigned long long co = 0, bo = 0, co2 = 0, bo2 = 0;
    u64 s1 = __builtin_addcll(a, b, c & 1, &co);
    R(s1); R(co);
    R(__builtin_addcll(s1, c, co, &co2)); R(co2);
    u64 d1 = __builtin_subcll(a, b, c & 1, &bo);
    R(d1); R(bo);
    R(__builtin_subcll(d1, c, bo, &bo2)); R(bo2);
    unsigned int co32 = 0, bo32 = 0;
    R(__builtin_addc(a32, b32, c32 & 1, &co32)); R(co32);     /* 32-bit adc */
    R(__builtin_subc(a32, b32, c32 & 1, &bo32)); R(bo32);     /* 32-bit sbc */
}

static NOINLINE void sec_div(u64 a, u64 b) {
    u32 a32 = (u32)a, b32 = (u32)b;
    /* well-defined C divisions: divisor non-zero, no INT_MIN / -1 */
    if (b) R(a / b);
    if (b32) R(a32 / b32);
    if (b && (s64)b != -1) R((s64)a / (s64)b);
    if (b32 && (s32)b32 != -1) R((s32)a32 / (s32)b32);
    if (b && (s64)b != -1) R((s64)a % (s64)b);
    /* architectural edge cases through the divide instruction: zero divisor, INT_MIN / -1 (32 and 64 bit) */
    R(udiv64(a, b)); R(udiv32(a32, b32));
    R(sdiv64((s64)a, (s64)b)); R(sdiv32((s32)a32, (s32)b32));
    R(udiv64(a, VZERO)); R(udiv32(a32, (u32)VZERO));
    R(sdiv64((s64)a, (s64)VZERO)); R(sdiv32((s32)a32, (s32)VZERO));
    R(sdiv64((s64)0x8000000000000000ull, VNEG1));
    R(sdiv32((s32)0x80000000u, VNEG1W));
    R(sdiv64((s64)(a ^ 0x8000000000000000ull), VNEG1));
    R(sdiv32((s32)(a32 ^ 0x80000000u), VNEG1W));
}

static NOINLINE u64 ror64v(u64 x, u32 s) { return (x >> s) | (x << ((64 - s) & 63)); }
static NOINLINE u32 ror32v(u32 x, u32 s) { return (x >> s) | (x << ((32 - s) & 31)); }

static NOINLINE void sec_shift(u64 a, u64 b) {
    u32 a32 = (u32)a, s = (u32)b;
    u64 sx = b & 63; u32 sw = s & 31;
    R(a << 5); R(a >> 13); R((s64)a >> 7); R((a >> 11) | (a << 53));          /* lsl lsr asr ror imm */
    R(a32 << 7); R(a32 >> 9); R((s32)a32 >> 11); R((a32 >> 3) | (a32 << 29)); /* w forms */
    R(a << sx); R(a >> sx); R((s64)a >> sx); R(ror64v(a, (u32)sx));          /* reg forms */
    R(a32 << sw); R(a32 >> sw); R((s32)a32 >> sw); R(ror32v(a32, sw));
    R(a >> 63); R((s64)a >> 63); R(a32 >> 31);
    R(a << (b & 127 ? 9 : 0));
}

static NOINLINE void sec_bits(u64 a, u64 b, u64 c) {
    u32 a32 = (u32)a;
    R((a >> 8) & 0xfff);                              /* ubfx x */
    R((a32 >> 5) & 0x7f);                             /* ubfx w */
    R((u64)((s64)(a << 20) >> 45));                   /* sbfx x */
    R((u64)((s32)(a32 << 9) >> 21));                  /* sbfx w */
    R((c & ~0x0ff0ull) | ((a & 0x3f) << 4));         /* bfi x */
    R((c & ~0xffull) | ((a >> 4) & 0xff));           /* bfxil x */
    R((a32 & ~(0xfu << 16)) | ((b & 0x7) << 16));    /* bfi w */
    R((a & 0xffff) << 9);                             /* ubfiz x */
    R(((u64)(s16)a) << 5);                            /* sbfiz x */
    R((u64)(s64)(s8)a); R((u64)(s64)(s16)a); R((u64)(s64)(s32)a);   /* sxtb sxth sxtw */
    R((u8)a); R((u16)a); R((u32)a);                                   /* uxtb uxth mov w */
    R(a ? __builtin_clzll(a) : 64);                   /* clz x */
    R(a32 ? __builtin_clz(a32) : 32);                 /* clz w */
    R(__builtin_clrsbll((s64)a));                     /* cls x */
    R(__builtin_clrsb((s32)a32));                     /* cls w */
    R(__builtin_bitreverse64(a));                     /* rbit x */
    R(__builtin_bitreverse32(a32));                   /* rbit w */
    R(__builtin_bswap64(a)); R(__builtin_bswap32(a32)); R(__builtin_bswap16((u16)a));   /* rev */
    R(((a & 0x00ff00ff00ff00ffull) << 8) | ((a >> 8) & 0x00ff00ff00ff00ffull));          /* rev16 x */
    R((u64)__builtin_bswap32((u32)a) | ((u64)__builtin_bswap32((u32)(a >> 32)) << 32)); /* rev32 */
    R(((a32 & 0x00ff00ffu) << 8) | ((a32 >> 8) & 0x00ff00ffu));                          /* rev16 w */
}

static NOINLINE void sec_logic(u64 a, u64 b, u64 c) {
    u32 a32 = (u32)a, b32 = (u32)b;
    R(a & b); R(a | b); R(a ^ b); R(a & ~b); R(a | ~b); R(a ^ ~b); R(~a);   /* and orr eor bic orn eon mvn */
    R(a & 0x5555555555555555ull); R(a ^ 0xffffff00ull);
    R(a32 & ~b32); R(a32 | ~b32); R(~a32); R(a32 ^ ~b32);
    R(a & (b << 3)); R(a | (b >> 7)); R(a ^ ~(b >> 9)); R(a & ~(b << 2));
    R(0 - a); R(0 - a32);                                                  /* neg */
    if (a & b) put(0x11); else put(0x12);                                  /* tst */
    if (a32 & 0x80000000u) put(0x13); else put(0x14);                      /* tst w */
    s64 rs; int ov = __builtin_sub_overflow((s64)0, (s64)a, &rs); R(rs); R(ov);   /* negs */
    u64 rr; R(__builtin_sub_overflow(a, c, &rr)); R(rr);
    R(0x123456789abcdef0ull ^ a);                                          /* movz + movk */
    R(a ^ 0xffffffffdeadbeefull);                                          /* movn-style */
    R(a + 0x0000ffff0000ffffull);
    R(a & 0xfffffffffffe0000ull);
}

static NOINLINE void sec_csel(u64 a, u64 b, u64 c, u64 d) {
    s64 sa = (s64)a, sb = (s64)b;
    u32 a32 = (u32)a, b32 = (u32)b;
    R(a < b ? c : d);                    /* csel */
    R(sa < sb ? c : d + 1);             /* csinc */
    R(sa >= sb ? c : ~d);               /* csinv */
    R(sa <= sb ? c : 0 - d);            /* csneg */
    R(a == b ? 1 : 0);                  /* cset */
    R(a != c ? 0 : ~0ull);              /* csetm */
    R(a > b ? c + 1 : c);               /* cinc */
    R(a > c ? ~d : d);                  /* cinv */
    R(sa > sb ? c : 0 - d);
    R(a32 < b32 ? c : d);
    R(a32 < b32 ? (u32)c : (u32)d + 1);
    R((s32)a32 >= (s32)b32 ? (u32)c : ~(u32)d);
    R(a == 0 ? 5 : 9);
    R(sa < 0 ? (s64)c : (s64)d);
}

static NOINLINE void sec_ccmp(u64 a, u64 b, u64 c, u64 d) {
    s64 sa = (s64)a, sb = (s64)b, sc = (s64)c, sd = (s64)d;
    if (a == b && c < d) put(1); else put(2);                    /* ccmp */
    if (sa < sb && sc > sd) put(3); else put(4);
    if ((a == 0 || b == 7) && sc <= sd) put(5); else put(6);
    if (((a & 1) != 0) && c != d) put(7); else put(8);
    if ((a == b) || (sc < sd)) put(9); else put(10);
    R(((a == b) & (c > d)) ? 5 : 6);
    if (((u32)a < (u32)b) && ((u32)c != 0)) put(11); else put(12);
    if (a + b == 0 && c < d) put(13); else put(14);                /* ccmn */
    R(chain_and(a, b, c));                                          /* ccmp in a called chain */
    R(chain_or(a, b, c));                                           /* ccmp / ccmn in a called chain */
}

static NOINLINE void hit(u32 id) { put(id); }

static NOINLINE void sec_branch(s64 a, s64 b, u64 c) {
    /* b.cond on all condition codes, signed and unsigned */
    if (a == b) hit(1); else hit(2);                        /* eq ne */
    if (a < b) hit(3); else hit(4);                         /* lt ge */
    if (a > b) hit(5); else hit(6);                         /* gt le */
    if ((u64)a < (u64)b) hit(7); else hit(8);               /* lo hs */
    if ((u64)a > (u64)b) hit(9); else hit(10);              /* hi ls */
    if (a <= b) hit(11); else hit(12);
    if (a >= b) hit(13); else hit(14);
    if (__builtin_sub_overflow(a, b, &a)) hit(15); else hit(16);   /* vs vc */
    if ((s64)((u64)a - (u64)b) < 0) hit(17); else hit(18);          /* mi pl */
    if (a == 0) hit(19);                                    /* cbz */
    if (c != 0) hit(20);                                    /* cbnz */
    if (c & (1ull << 37)) hit(21);                          /* tbnz */
    if (!(c & (1ull << 3))) hit(22);                        /* tbz */
    if (b != 0 && (u32)b != 0) hit(23);
}

static NOINLINE void sec_mem(u64 a, u64 b, u32 i) {
    u32 ui = i & 255;
    s64 si = (s64)(s32)(i & 127) - 64;                      /* sxtw index, negative offsets */
    u8m *B = (u8m *)dat; u16m *H = (u16m *)dat; u32m *W = (u32m *)dat; s32m *SW = (s32m *)dat;
    u64 *Q = dat;
    Q[ui] = a;                                              /* str x, lsl #3 with uxtw index */
    W[ui] = (u32)b;                                         /* str w */
    H[ui] = (u16)(a >> 3);                                  /* strh */
    B[ui] = (u8)(b >> 5);                                   /* strb */
    Q[256 + si] = a ^ b;                                    /* sxtw index store */
    R(Q[ui]); R(W[ui]); R(H[ui]); R(B[ui]);                /* ldr ldr ldrh ldrb */
    R(Q[256 + si]);
    R((s64)(s8)B[ui + 1]);                                  /* ldrsb */
    R((s64)(s16)H[ui + 2]);                                 /* ldrsh */
    R((s64)SW[ui + 3]);                                     /* ldrsw */
    R((s64)SW[(ui & 127) ^ 1]);
    R((u64)W[ui] * 3 + Q[(u32)(i & 63)]);                   /* uxtw index load */
}

static NOINLINE void pair_copy(P2 *d, const P2 *s) { *d = *s; }          /* ldp / stp */
static NOINLINE u64 pair_sum(const P2 *s) { return s->x * 3 + s->y; }
static NOINLINE void pair_set(P2 *d, u64 x, u64 y) { d->x = x; d->y = y; }
static NOINLINE s64 pair32(const P32 *s) { return (s64)s->x + (s64)s->y; } /* ldpsw */

static NOINLINE u64 walk_post(u64 *p, u32 n) {                           /* ldr post-index */
    u64 acc = 0;
    for (u32 i = 0; i < n; i++) acc = (acc ^ *p++) * 3;
    return acc;
}
static NOINLINE u64 walk_pre(u64 *p, u32 n) {                            /* ldr pre-index writeback */
    u64 acc = 0;
    for (u32 i = 0; i < n; i++) acc += *++p;
    return acc;
}
static NOINLINE void store_pre(u64 *p, u32 n, u64 v) {                    /* str pre-index */
    for (u32 i = 0; i < n; i++) *++p = v + i;
}
static NOINLINE u64 store_post(u64 *p, u32 n, u64 v) {                    /* str post-index */
    u64 acc = 0;
    for (u32 i = 0; i < n; i++) { *p++ = v ^ (u64)i; acc += v; }
    return acc;
}
static NOINLINE u64 walk_bytes(u8 *p, u32 n) {                           /* ldrb post-index */
    u64 acc = 0;
    while (n--) acc = (acc << 1) ^ *p++;
    return acc;
}
static NOINLINE u64 unal_ld(u8 *p) { return *(u64u *)(p - 3); }         /* ldur */
static NOINLINE void unal_st(u8 *p, u64 v) { *(u64u *)(p - 5) = v; }   /* stur */

void test_main(void) {
    u64 seed = 0x5eed1234abcdull;
    for (u32 i = 0; i < NV; i++) V[i] = i < NEDGE ? EDGE[i] : lcg(&seed);
    for (u32 i = 0; i < 512; i++) dat[i] = lcg(&seed);
    for (u32 i = 0; i < NV; i++) {
        u64 a = V[i], b = V[(i * 7 + 5) % NV], c = V[(i * 11 + 3) % NV], d = V[(i * 13 + 1) % NV];
        sec_arith(a, b, c);
        sec_mul(a, b, c);
        sec_div(a, b);
        sec_shift(a, b);
        sec_bits(a, b, c);
        sec_logic(a, b, c);
        sec_csel(a, b, c, d);
        sec_ccmp(a, b, c, d);
        sec_branch((s64)a, (s64)b, c);
        sec_mem(a, b, i);
    }
    /* pointer walking, writeback forms, pairs, unaligned */
    R(walk_post(dat, 64));
    R(walk_pre(dat, 64));
    store_pre(dat + 100, 32, V[3]);
    R(store_post(dat + 200, 32, V[5]));
    R(dat[100]); R(dat[131]); R(dat[200]); R(dat[231]);
    P2 p2a, p2b;
    pair_set(&p2a, V[9], V[10]);
    pair_copy(&p2b, &p2a);
    R(p2b.x); R(p2b.y); R(pair_sum(&p2b));
    P32 p32; p32.x = (s32)V[11]; p32.y = (s32)V[12];
    R(pair32(&p32));
    R(unal_ld((u8 *)dat + 40));
    unal_st((u8 *)dat + 60, V[7]);
    R(dat[7]); R(dat[8]);
    R(walk_bytes((u8 *)dat, 37));
    R(0xc0ffeeull);
}
