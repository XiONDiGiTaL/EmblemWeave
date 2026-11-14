// FLAGS:
// EXPECT-INSN: \bbr\s+x\d+
#include "t.h"

/* Switch and dispatch test. Every case of the dense switches calls its own NOINLINE helper (h0..h79), so no case can
 * be turned into a lookup table of constants; clang must emit real jump tables (br xN). The inputs come from a volatile
 * table, so nothing is constant-folded. Results are folded into a rolling hash over results[]. */

#define NV 40
static volatile u64 V[NV];
static u32 K;

static NOINLINE void put(u64 x) {
    results[K] = results[K] * 0x100000001b3ull + x;
    K = (K + 1) & (NRES - 1);
}
#define R(x) put((u64)(x))

#define H(n) static NOINLINE u64 h##n(u64 x) { \
    return (x * (0x9E3779B97F4A7C15ull ^ ((u64)(n) << 9))) ^ (x >> ((n) % 13 + 1)) ^ ((u64)(n) * 1000003ull + (x & 0xff)); }

H(0) H(1) H(2) H(3) H(4) H(5) H(6) H(7) H(8) H(9) H(10) H(11) H(12) H(13) H(14) H(15)
H(16) H(17) H(18) H(19) H(20) H(21) H(22) H(23) H(24) H(25) H(26) H(27) H(28) H(29) H(30) H(31)
H(32) H(33) H(34) H(35) H(36) H(37) H(38) H(39) H(40) H(41) H(42) H(43) H(44) H(45) H(46) H(47)
H(48) H(49) H(50) H(51) H(52) H(53) H(54) H(55) H(56) H(57) H(58) H(59) H(60) H(61) H(62) H(63)
H(64) H(65) H(66) H(67) H(68) H(69) H(70) H(71) H(72) H(73) H(74) H(75) H(76) H(77) H(78) H(79)

#define CASEX(lbl, id) case lbl: r = h##id(x) + (u64)(lbl); break;
#define CASE(n) CASEX(n, n)

/* 1. dense, 20 cases, default reachable */
static NOINLINE u64 sw_dense(u32 sel, u64 x) {
    u64 r = 0;
    switch (sel) {
        CASE(0) CASE(1) CASE(2) CASE(3) CASE(4) CASE(5) CASE(6) CASE(7) CASE(8) CASE(9)
        CASE(10) CASE(11) CASE(12) CASE(13) CASE(14) CASE(15) CASE(16) CASE(17) CASE(18) CASE(19)
        default: r = x ^ 0xdeadull; break;
    }
    return r;
}

/* 2. dense, 12 cases, default is unreachable: no bounds check should be emitted */
static NOINLINE u64 sw_unreach(u32 sel, u64 x) {
    u64 r = 0;
    switch (sel) {
        CASE(20) CASE(21) CASE(22) CASE(23) CASE(24) CASE(25)
        CASE(26) CASE(27) CASE(28) CASE(29) CASE(30) CASE(31)
        default: __builtin_unreachable();
    }
    return r;
}

/* 3. switch over a u8, 40 dense cases */
static NOINLINE u64 sw_u8(u8 c, u64 x) {
    u64 r = 0;
    switch (c) {
        CASE(32) CASE(33) CASE(34) CASE(35) CASE(36) CASE(37) CASE(38) CASE(39) CASE(40) CASE(41)
        CASE(42) CASE(43) CASE(44) CASE(45) CASE(46) CASE(47) CASE(48) CASE(49) CASE(50) CASE(51)
        CASE(52) CASE(53) CASE(54) CASE(55) CASE(56) CASE(57) CASE(58) CASE(59) CASE(60) CASE(61)
        CASE(62) CASE(63) CASE(64) CASE(65) CASE(66) CASE(67) CASE(68) CASE(69) CASE(70) CASE(71)
        default: r = c * 3u; break;
    }
    return r;
}

/* 4. switch over a negative-valued s32 range -20..19 (40 cases), default for the rest */
static NOINLINE u64 sw_s32(s32 v, u64 x) {
    u64 r = 0;
    switch (v) {
        CASEX(-20, 40) CASEX(-19, 41) CASEX(-18, 42) CASEX(-17, 43) CASEX(-16, 44) CASEX(-15, 45) CASEX(-14, 46)
        CASEX(-13, 47) CASEX(-12, 48) CASEX(-11, 49) CASEX(-10, 50) CASEX(-9, 51) CASEX(-8, 52) CASEX(-7, 53)
        CASEX(-6, 54) CASEX(-5, 55) CASEX(-4, 56) CASEX(-3, 57) CASEX(-2, 58) CASEX(-1, 59) CASEX(0, 60)
        CASEX(1, 61) CASEX(2, 62) CASEX(3, 63) CASEX(4, 64) CASEX(5, 65) CASEX(6, 66) CASEX(7, 67)
        CASEX(8, 68) CASEX(9, 69) CASEX(10, 70) CASEX(11, 71) CASEX(12, 72) CASEX(13, 73) CASEX(14, 74)
        CASEX(15, 75) CASEX(16, 76) CASEX(17, 77) CASEX(18, 78) CASEX(19, 79)
        default: r = (u64)(s64)v * 7; break;
    }
    return r;
}

/* 5. sparse switch: keys spread over 32 bits, no table possible (binary search tree) */
static NOINLINE u64 sw_sparse(u32 key, u64 x) {
    u64 r = 0;
    switch (key) {
        CASEX(0x10u, 0) CASEX(0x203u, 1) CASEX(0x4001u, 2) CASEX(0x80000u, 3) CASEX(0x12345678u, 4)
        CASEX(0xdeadbeefu, 5) CASEX(0x7fffffffu, 6) CASEX(0x100u, 7) CASEX(0x3cu, 8) CASEX(0x9999u, 9)
        default: r = key ^ x; break;
    }
    return r;
}

/* 6. fallthrough, continue, break and an inner loop, all inside a loop */
static NOINLINE u64 sw_fall(u64 x, u32 n) {
    u64 acc = 1;
    for (u32 i = 0; i < n; i++) {
        switch ((x >> (i & 15)) & 15) {
            case 0: acc += 1;                       /* falls through */
            case 1: acc ^= h1(i);                   /* falls through */
            case 2: acc = acc * 5; break;
            case 3: continue;                       /* skips the tail below */
            case 4: if (acc & 1) break;
                    acc += 9;                       /* falls through */
            case 5: acc += h5(acc); break;
            case 6: acc ^= h6(acc); break;
            case 7: acc = h7(acc);                  /* falls through */
            case 8: acc -= 3; break;
            case 9: { u32 k = (u32)acc & 3; if (k == 0) break; acc = h9(acc) + k; } break;
            case 10: for (u32 j = 0; j < 3; j++) { if (j == 1) continue; acc += h10(j); } break;
            case 11: acc = h11(acc) >> 3; break;
            case 12: acc = h12(acc ^ 0x5a5a); break;
            case 13: acc = ~acc; break;
            case 14: acc ^= (u64)i << 9; break;
            case 15: acc = h15(acc ^ i) | 1; break;
            default: acc ^= 0x55; break;
        }
        acc += i;
    }
    return acc;
}

/* 7. nested switches, 8 x 8 */
static NOINLINE u64 sw_nested(u32 a, u32 b, u64 x) {
    u64 r = 0;
    switch (a) {
        case 0: switch (b) { CASEX(0, 0) CASEX(1, 1) CASEX(2, 2) CASEX(3, 3) CASEX(4, 4) CASEX(5, 5) CASEX(6, 6) CASEX(7, 7) default: r = 1; break; } break;
        case 1: switch (b) { CASEX(0, 8) CASEX(1, 9) CASEX(2, 10) CASEX(3, 11) CASEX(4, 12) CASEX(5, 13) CASEX(6, 14) CASEX(7, 15) default: r = 2; break; } break;
        case 2: switch (b) { CASEX(0, 16) CASEX(1, 17) CASEX(2, 18) CASEX(3, 19) CASEX(4, 20) CASEX(5, 21) CASEX(6, 22) CASEX(7, 23) default: r = 3; break; } break;
        case 3: switch (b) { CASEX(0, 24) CASEX(1, 25) CASEX(2, 26) CASEX(3, 27) CASEX(4, 28) CASEX(5, 29) CASEX(6, 30) CASEX(7, 31) default: r = 4; break; } break;
        case 4: switch (b) { CASEX(0, 32) CASEX(1, 33) CASEX(2, 34) CASEX(3, 35) CASEX(4, 36) CASEX(5, 37) CASEX(6, 38) CASEX(7, 39) default: r = 5; break; } break;
        case 5: switch (b) { CASEX(0, 40) CASEX(1, 41) CASEX(2, 42) CASEX(3, 43) CASEX(4, 44) CASEX(5, 45) CASEX(6, 46) CASEX(7, 47) default: r = 6; break; } break;
        case 6: switch (b) { CASEX(0, 48) CASEX(1, 49) CASEX(2, 50) CASEX(3, 51) CASEX(4, 52) CASEX(5, 53) CASEX(6, 54) CASEX(7, 55) default: r = 7; break; } break;
        case 7: switch (b) { CASEX(0, 56) CASEX(1, 57) CASEX(2, 58) CASEX(3, 59) CASEX(4, 60) CASEX(5, 61) CASEX(6, 62) CASEX(7, 63) default: r = 8; break; } break;
        default: r = x; break;
    }
    return r;
}

/* 8. a switch with 128 cases in one function; each body is a different inline computation */
#define C(n) case n: x = (x ^ (x >> ((n) & 15))) * (0x100000001b3ull * ((n) + 3)) + (u64)(n) * 0x9e37ull; break;
static NOINLINE u64 sw_big(u32 sel, u64 x) {
    switch (sel & 127) {
        C(0) C(1) C(2) C(3) C(4) C(5) C(6) C(7) C(8) C(9) C(10) C(11) C(12) C(13) C(14) C(15)
        C(16) C(17) C(18) C(19) C(20) C(21) C(22) C(23) C(24) C(25) C(26) C(27) C(28) C(29) C(30) C(31)
        C(32) C(33) C(34) C(35) C(36) C(37) C(38) C(39) C(40) C(41) C(42) C(43) C(44) C(45) C(46) C(47)
        C(48) C(49) C(50) C(51) C(52) C(53) C(54) C(55) C(56) C(57) C(58) C(59) C(60) C(61) C(62) C(63)
        C(64) C(65) C(66) C(67) C(68) C(69) C(70) C(71) C(72) C(73) C(74) C(75) C(76) C(77) C(78) C(79)
        C(80) C(81) C(82) C(83) C(84) C(85) C(86) C(87) C(88) C(89) C(90) C(91) C(92) C(93) C(94) C(95)
        C(96) C(97) C(98) C(99) C(100) C(101) C(102) C(103) C(104) C(105) C(106) C(107) C(108) C(109) C(110) C(111)
        C(112) C(113) C(114) C(115) C(116) C(117) C(118) C(119) C(120) C(121) C(122) C(123) C(124) C(125) C(126) C(127)
    }
    return x;
}
#undef C

/* 9. computed-goto interpreter (GNU &&label and goto *ptr) running a fixed bytecode program.
 * Opcodes: 0 PUSH imm8, 1 ADD, 2 MUL, 3 XOR, 4 SHL, 5 DUP, 6 SWAP, 7 JNZ target8, 8 DEC, 9 OUT, 10 HALT.
 * The program is a 200-iteration loop: x = (x * 13 + 7) ^ ((x * 13 + 7) << 5), OUT accumulates every value. */
static const u8 PROG[] = {
    0, 200,            /* 0  PUSH 200          stack: seed, cnt       */
    6,                 /* 2  SWAP              stack: cnt, s          */
    0, 13, 2,          /* 3  PUSH 13, MUL      stack: cnt, s*13       */
    0, 7, 1,           /* 6  PUSH 7, ADD       stack: cnt, s'         */
    5, 0, 5, 4, 3,     /* 9  DUP, PUSH 5, SHL, XOR                    */
    5, 9,              /* 14 DUP, OUT                                 */
    6, 8,              /* 16 SWAP, DEC         stack: s, cnt-1        */
    5, 7, 2,           /* 18 DUP, JNZ 2                               */
    10                 /* 21 HALT                                     */
};

static NOINLINE u64 interp(u64 seed) {
    static void *const tbl[] = { &&L_PUSH, &&L_ADD, &&L_MUL, &&L_XOR, &&L_SHL, &&L_DUP, &&L_SWAP, &&L_JNZ,
                                 &&L_DEC, &&L_OUT, &&L_HALT };
    const u8 *code = PROG;
    u64 st[16];
    u32 sp = 0, pc = 0;
    u64 out = 0;
    st[sp++] = seed;
#define NEXT() goto *tbl[code[pc++]]
    NEXT();
L_PUSH: st[sp++] = code[pc++]; NEXT();
L_ADD:  { u64 b = st[--sp]; st[sp - 1] += b; NEXT(); }
L_MUL:  { u64 b = st[--sp]; st[sp - 1] *= b; NEXT(); }
L_XOR:  { u64 b = st[--sp]; st[sp - 1] ^= b; NEXT(); }
L_SHL:  { u64 b = st[--sp]; st[sp - 1] <<= (b & 63); NEXT(); }
L_DUP:  st[sp] = st[sp - 1]; sp++; NEXT();
L_SWAP: { u64 t = st[sp - 1]; st[sp - 1] = st[sp - 2]; st[sp - 2] = t; NEXT(); }
L_JNZ:  { u32 t = code[pc++]; u64 v = st[--sp]; if (v) pc = t; NEXT(); }
L_DEC:  st[sp - 1] -= 1; NEXT();
L_OUT:  { u64 v = st[--sp]; out = out * 31 + v; NEXT(); }
L_HALT: return out ^ st[sp - 1];
#undef NEXT
}

static const u32 SPARSE_KEYS[10] = { 0x10u, 0x203u, 0x4001u, 0x80000u, 0x12345678u, 0xdeadbeefu, 0x7fffffffu, 0x100u, 0x3cu, 0x9999u };

void test_main(void) {
    u64 seed = 0x2468ace013579bdfull;
    for (u32 i = 0; i < NV; i++) V[i] = lcg(&seed);
    for (u32 i = 0; i < NV; i++) {
        u64 x = V[i];
        R(sw_dense((u32)(x % 26), x));
        R(sw_unreach((u32)((x >> 7) % 12) + 20, x));
        R(sw_u8((u8)(x >> 16), x));
        R(sw_s32((s32)(x % 61) - 30, x));
        R(sw_sparse(SPARSE_KEYS[(x >> 40) % 10], x));
        R(sw_fall(x, 24 + (i & 7)));
        R(sw_nested((u32)((x >> 3) % 8), (u32)((x >> 11) % 8), x));
        R(sw_big((u32)(x >> 20), x));
        R(interp(x));
    }
    R(0x5ca1ab1eull);
}
