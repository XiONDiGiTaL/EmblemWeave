// FLAGS: -fno-math-errno -ffp-contract=off
// EXPECT-INSN: \bfadd\b
// EXPECT-INSN: \bfmul\b
// EXPECT-INSN: \bfdiv\b
// EXPECT-INSN: \bfsqrt\b
// EXPECT-INSN: \bfmadd\b|\bfnmsub\b|\bfmsub\b
// EXPECT-INSN: \bfmaxnm\b|\bfmax\b
// EXPECT-INSN: \bfcmpe?\b
// EXPECT-INSN: \bfcsel\b
// EXPECT-INSN: \bfccmp\b
// EXPECT-INSN: \bscvtf\b
// EXPECT-INSN: \bfcvtzs\b
// EXPECT-INSN: \bfcvtzu\b
// EXPECT-INSN: \bfcvt\b
// EXPECT-INSN: \bfrint[a-z]\b
// EXPECT-INSN: \bfmov\b
#include "t.h"
#include <arm_neon.h>

/* Floating point test: scalar float and double over a table of special bit patterns (zeros, infinities, quiet and
 * signalling NaNs with payloads, denormals, boundary normals, and values around the int conversion limits). Every
 * result is stored as its raw bit pattern (f2b / d2b), so NaN payload and sign handling are compared exactly.
 * Compiled with -fno-math-errno (so sqrt and friends never call libm) and -ffp-contract=off (no implicit fusing).
 * Conversions whose C result is undefined (NaN, out of range) use the arm_neon.h scalar conversion intrinsics, which
 * are the hardware instructions themselves, so the architectural result (saturation, NaN -> 0) is what is tested. */

#define NS 25
static volatile u32 SF[NS] = {
    0x00000000u, 0x80000000u, 0x7f800000u, 0xff800000u, 0x7fc00000u, 0x7fc12345u, 0x7f800001u, 0xffc00000u,
    0x00000001u, 0x007fffffu, 0x00800000u,
    0x3f800000u, 0xbf800000u, 0x3f000000u, 0x3fc00000u, 0x40200000u, 0x40600000u, 0xc0200000u,
    0x4b000000u, 0x4effffffu, 0x4f000000u, 0x4f800000u, 0x5f01103du, 0x5f79ccd9u, 0xcf000001u
};
static volatile u64 SD[NS] = {
    0x0000000000000000ull, 0x8000000000000000ull, 0x7ff0000000000000ull, 0xfff0000000000000ull,
    0x7ff8000000000000ull, 0x7ff82468a0000000ull, 0x7ff0000020000000ull, 0xfff8000000000000ull,
    0x0000000000000001ull, 0x000fffffffffffffull, 0x0010000000000000ull,
    0x3ff0000000000000ull, 0xbff0000000000000ull, 0x3fe0000000000000ull, 0x3ff8000000000000ull,
    0x4004000000000000ull, 0x400c000000000000ull, 0xc004000000000000ull,
    0x4160000000000000ull, 0x41dfffffe0000000ull, 0x41e0000000000000ull, 0x41f0000000000000ull,
    0x43e02207973f6440ull, 0x43ef399b1438a100ull, 0xc1e0000020000000ull
};

static u32 K;
static NOINLINE void put(u64 x) {
    results[K] = results[K] * 0x100000001b3ull + x;
    K = (K + 1) & (NRES - 1);
}
#define R(x) put((u64)(x))

/* ---- conversions through the arm_neon.h scalar intrinsics (each one is a single fcvt* instruction) ---- */

#define CVTF(NAME, T, DT, INTR) \
    static NOINLINE DT NAME(T a) { return INTR(a); }

CVTF(f_fcvtzs_w, float, s32, vcvts_s32_f32)
CVTF(f_fcvtzs_x, float, s64, vcvts_s64_f32)
CVTF(f_fcvtzu_w, float, u32, vcvts_u32_f32)
CVTF(f_fcvtzu_x, float, u64, vcvts_u64_f32)
CVTF(f_fcvtas_w, float, s32, vcvtas_s32_f32)
CVTF(f_fcvtas_x, float, s64, vcvtas_s64_f32)
CVTF(f_fcvtau_w, float, u32, vcvtas_u32_f32)
CVTF(f_fcvtau_x, float, u64, vcvtas_u64_f32)
CVTF(f_fcvtms_w, float, s32, vcvtms_s32_f32)
CVTF(f_fcvtms_x, float, s64, vcvtms_s64_f32)
CVTF(f_fcvtmu_w, float, u32, vcvtms_u32_f32)
CVTF(f_fcvtmu_x, float, u64, vcvtms_u64_f32)
CVTF(f_fcvtps_w, float, s32, vcvtps_s32_f32)
CVTF(f_fcvtps_x, float, s64, vcvtps_s64_f32)
CVTF(f_fcvtpu_w, float, u32, vcvtps_u32_f32)
CVTF(f_fcvtpu_x, float, u64, vcvtps_u64_f32)
CVTF(f_fcvtns_w, float, s32, vcvtns_s32_f32)
CVTF(f_fcvtns_x, float, s64, vcvtns_s64_f32)
CVTF(f_fcvtnu_w, float, u32, vcvtns_u32_f32)
CVTF(f_fcvtnu_x, float, u64, vcvtns_u64_f32)

CVTF(d_fcvtzs_w, double, s32, vcvtd_s32_f64)
CVTF(d_fcvtzs_x, double, s64, vcvtd_s64_f64)
CVTF(d_fcvtzu_w, double, u32, vcvtd_u32_f64)
CVTF(d_fcvtzu_x, double, u64, vcvtd_u64_f64)
CVTF(d_fcvtas_w, double, s32, vcvtad_s32_f64)
CVTF(d_fcvtas_x, double, s64, vcvtad_s64_f64)
CVTF(d_fcvtau_w, double, u32, vcvtad_u32_f64)
CVTF(d_fcvtau_x, double, u64, vcvtad_u64_f64)
CVTF(d_fcvtms_w, double, s32, vcvtmd_s32_f64)
CVTF(d_fcvtms_x, double, s64, vcvtmd_s64_f64)
CVTF(d_fcvtmu_w, double, u32, vcvtmd_u32_f64)
CVTF(d_fcvtmu_x, double, u64, vcvtmd_u64_f64)
CVTF(d_fcvtps_w, double, s32, vcvtpd_s32_f64)
CVTF(d_fcvtps_x, double, s64, vcvtpd_s64_f64)
CVTF(d_fcvtpu_w, double, u32, vcvtpd_u32_f64)
CVTF(d_fcvtpu_x, double, u64, vcvtpd_u64_f64)
CVTF(d_fcvtns_w, double, s32, vcvtnd_s32_f64)
CVTF(d_fcvtns_x, double, s64, vcvtnd_s64_f64)
CVTF(d_fcvtnu_w, double, u32, vcvtnd_u32_f64)
CVTF(d_fcvtnu_x, double, u64, vcvtnd_u64_f64)

/* ---- comparisons: one NOINLINE function per ARM condition code, written as the C predicate that selects it ---- */

#define CMPF(P, T, NAME, EXPR) \
    static NOINLINE u32 P##_cc_##NAME(T a, T b) { return (EXPR); }

#define DEF_CMP(T, P) \
    CMPF(P, T, eq, a == b)                              \
    CMPF(P, T, ne, a != b)                              \
    CMPF(P, T, cs, !(a < b))                            \
    CMPF(P, T, cc, a < b)                               \
    CMPF(P, T, mi, __builtin_isless(a, b))              \
    CMPF(P, T, pl, !__builtin_isless(a, b))             \
    CMPF(P, T, vs, __builtin_isunordered(a, b))         \
    CMPF(P, T, vc, !__builtin_isunordered(a, b))        \
    CMPF(P, T, hi, !__builtin_islessequal(a, b))        \
    CMPF(P, T, ls, __builtin_islessequal(a, b))         \
    CMPF(P, T, ge, __builtin_isgreaterequal(a, b))      \
    CMPF(P, T, lt, !__builtin_isgreaterequal(a, b))     \
    CMPF(P, T, gt, __builtin_isgreater(a, b))           \
    CMPF(P, T, le, !__builtin_isgreater(a, b))          \
    /* a chain of two comparisons joined by && or ||: clang at -O2 lowers it to fcmp + fccmp */ \
    static NOINLINE u32 P##_ccmp(T a, T b, T c) {       \
        u32 r;                                          \
        if (a < b && c < a) r = 1; else r = 2;          \
        return r; }                                     \
    static NOINLINE u32 P##_ccmpor(T a, T b, T c) {     \
        u32 r;                                          \
        if (a > b || c == a) r = 3; else r = 4;         \
        return r; }

/* ---- the operations (one NOINLINE function each, so no two are fused by the optimiser) ---- */

#define DEF_FROMINT(T, P) \
    static NOINLINE T P##_scvtf_w(s32 v) { return (T)v; } \
    static NOINLINE T P##_scvtf_x(s64 v) { return (T)v; } \
    static NOINLINE T P##_ucvtf_w(u32 v) { return (T)v; } \
    static NOINLINE T P##_ucvtf_x(u64 v) { return (T)v; }

DEF_FROMINT(float, f)
DEF_FROMINT(double, d)
DEF_CMP(float, f)
DEF_CMP(double, d)

#define DEF_OPS(T, P, SUF, BITF) \
    static NOINLINE T P##_add(T a, T b) { return a + b; } \
    static NOINLINE T P##_sub(T a, T b) { return a - b; } \
    static NOINLINE T P##_mul(T a, T b) { return a * b; } \
    static NOINLINE T P##_div(T a, T b) { return a / b; } \
    static NOINLINE T P##_fmaxb(T a, T b) { return __builtin_fmax##SUF(a, b); } \
    static NOINLINE T P##_fminb(T a, T b) { return __builtin_fmin##SUF(a, b); } \
    static NOINLINE T P##_tmax(T a, T b) { return a > b ? a : b; } \
    static NOINLINE T P##_tmin(T a, T b) { return a < b ? a : b; } \
    static NOINLINE T P##_tmaxe(T a, T b) { return a >= b ? a : b; } \
    static NOINLINE T P##_tmine(T a, T b) { return a <= b ? a : b; } \
    static NOINLINE T P##_neg(T a) { return -a; } \
    static NOINLINE T P##_abs(T a) { return __builtin_fabs##SUF(a); } \
    static NOINLINE T P##_sqrt(T a) { return __builtin_sqrt##SUF(a); } \
    static NOINLINE T P##_fma(T a, T b, T c) { return __builtin_fma##SUF(a, b, c); } \
    static NOINLINE T P##_fmsub(T a, T b, T c) { return __builtin_fma##SUF(-a, b, c); } \
    static NOINLINE T P##_fnmadd(T a, T b, T c) { return __builtin_fma##SUF(-a, b, -c); } \
    static NOINLINE T P##_fnmsub(T a, T b, T c) { return __builtin_fma##SUF(a, b, -c); } \
    static NOINLINE T P##_rn(T a) { return __builtin_roundeven##SUF(a); } \
    static NOINLINE T P##_rm(T a) { return __builtin_floor##SUF(a); } \
    static NOINLINE T P##_rp(T a) { return __builtin_ceil##SUF(a); } \
    static NOINLINE T P##_rz(T a) { return __builtin_trunc##SUF(a); } \
    static NOINLINE T P##_ra(T a) { return __builtin_round##SUF(a); } \
    static NOINLINE T P##_ri(T a) { return __builtin_nearbyint##SUF(a); } \
    static NOINLINE T P##_rx(T a) { return __builtin_rint##SUF(a); } \
    static NOINLINE T P##_csel_lt(T a, T b, T c, T d) { return a < b ? c : d; } \
    static NOINLINE T P##_csel_ge(T a, T b, T c, T d) { return a >= b ? c : d; } \
    static NOINLINE T P##_csel_ne(T a, T b, T c, T d) { return a != b ? c : d; } \
    static NOINLINE T P##_csel_nu(T a, T b, T c, T d) { return !(a < b) ? c : d; } \
    static NOINLINE T P##_csel_ule(T a, T b, T c, T d) { return !(a > b) ? c : d; } \
    static NOINLINE T P##_fcsel(T a, T b, T c, T d) { return __builtin_isunordered(a, b) ? c : d; } \
    static NOINLINE u32 P##_cmp(T a, T b) { \
        u32 m = 0; \
        m |= (u32)(a == b) << 0;  m |= (u32)(a != b) << 1;  m |= (u32)(a < b) << 2;   m |= (u32)(a <= b) << 3; \
        m |= (u32)(a > b) << 4;   m |= (u32)(a >= b) << 5;  m |= (u32)!(a < b) << 6;  m |= (u32)!(a >= b) << 7; \
        m |= (u32)!(a > b) << 8;  m |= (u32)!(a <= b) << 9; m |= (u32)!(a == b) << 10; \
        return m; } \
    static NOINLINE void P##_unary(T a) { \
        put(P##_fcvtzs_w(a)); put(P##_fcvtzs_x(a)); put(P##_fcvtzu_w(a)); put(P##_fcvtzu_x(a)); \
        put(P##_fcvtas_w(a)); put(P##_fcvtas_x(a)); put(P##_fcvtau_w(a)); put(P##_fcvtau_x(a)); \
        put(P##_fcvtms_w(a)); put(P##_fcvtms_x(a)); put(P##_fcvtmu_w(a)); put(P##_fcvtmu_x(a)); \
        put(P##_fcvtps_w(a)); put(P##_fcvtps_x(a)); put(P##_fcvtpu_w(a)); put(P##_fcvtpu_x(a)); \
        put(P##_fcvtns_w(a)); put(P##_fcvtns_x(a)); put(P##_fcvtnu_w(a)); put(P##_fcvtnu_x(a)); \
        if (a > -2147483649.0 && a < 2147483648.0) put((u64)(s64)(s32)a); \
        if (a > -1.0 && a < 4294967296.0) put((u64)(u32)a); \
        if (a > -9.0e18 && a < 9.0e18) put((u64)(s64)a); \
        if (a > -1.0 && a < 1.8e19) put((u64)a); \
        if (a > -1.0e9 && a < 1.0e9) put((u64)__builtin_lround##SUF(a)); \
        if (a > -1.0e9 && a < 1.0e9) put((u64)__builtin_lrint##SUF(a)); \
        if (a > -1.0e9 && a < 1.0e9) put((u64)(s64)__builtin_floor##SUF(a)); \
        if (a > -1.0e9 && a < 1.0e9) put((u64)(s64)__builtin_ceil##SUF(a)); \
    } \
    static NOINLINE void P##_fromint(u32 w, u64 x) { \
        put(BITF(P##_scvtf_w((s32)w))); put(BITF(P##_scvtf_x((s64)x))); \
        put(BITF(P##_ucvtf_w(w))); put(BITF(P##_ucvtf_x(x))); } \
    static NOINLINE void P##_pair(T a, T b, T c) { \
        put(BITF(P##_add(a, b))); put(BITF(P##_sub(a, b))); put(BITF(P##_mul(a, b))); put(BITF(P##_div(a, b))); \
        put(BITF(P##_fmaxb(a, b))); put(BITF(P##_fminb(a, b))); put(BITF(P##_tmax(a, b))); put(BITF(P##_tmin(a, b))); \
        put(BITF(P##_tmaxe(a, b))); put(BITF(P##_tmine(a, b))); \
        put(BITF(P##_neg(a))); put(BITF(P##_abs(a))); put(BITF(P##_sqrt(a))); \
        put(BITF(P##_fma(a, b, c))); put(BITF(P##_fmsub(a, b, c))); \
        put(BITF(P##_fnmadd(a, b, c))); put(BITF(P##_fnmsub(a, b, c))); \
        put(BITF(P##_rn(a))); put(BITF(P##_rm(a))); put(BITF(P##_rp(a))); put(BITF(P##_rz(a))); \
        put(BITF(P##_ra(a))); put(BITF(P##_ri(a))); put(BITF(P##_rx(a))); \
        put(BITF(P##_csel_lt(a, b, c, a))); put(BITF(P##_csel_ge(a, b, b, c))); put(BITF(P##_csel_ne(a, b, c, b))); \
        put(BITF(P##_csel_nu(a, b, a, c))); put(BITF(P##_csel_ule(a, b, c, b))); \
        put(BITF(P##_fcsel(a, b, c, a))); \
        put(P##_cmp(a, b)); \
        put(P##_cc_eq(a, b)); put(P##_cc_ne(a, b)); put(P##_cc_cs(a, b)); put(P##_cc_cc(a, b)); \
        put(P##_cc_mi(a, b)); put(P##_cc_pl(a, b)); put(P##_cc_vs(a, b)); put(P##_cc_vc(a, b)); \
        put(P##_cc_hi(a, b)); put(P##_cc_ls(a, b)); put(P##_cc_ge(a, b)); put(P##_cc_lt(a, b)); \
        put(P##_cc_gt(a, b)); put(P##_cc_le(a, b)); \
        put(P##_ccmp(a, b, c)); put(P##_ccmpor(a, b, c)); \
    }

DEF_OPS(float, f, f, f2b)
DEF_OPS(double, d, , d2b)

/* ---- half precision and float<->double ---- */

static NOINLINE u64 half_ops(float f, double d) {
    _Float16 h1 = (_Float16)f;          /* fcvt h, s */
    _Float16 h2 = (_Float16)d;          /* fcvt h, d */
    _Float16 hm = h1 * h2 + h1;         /* half arithmetic (promoted) */
    u16 hb; __builtin_memcpy(&hb, &hm, 2);
    put(f2b((float)h1)); put(d2b((double)h2));                 /* fcvt s, h and fcvt d, h */
    put(f2b((float)hm)); put(d2b((double)hm));
    put(hb);
    put((u64)((_Float16)(s32)f2b(f) == h1));
    return hb;
}
static NOINLINE void conv_misc(float f, double d) {
    put(f2b((float)d));          /* fcvt s, d */
    put(d2b((double)f));         /* fcvt d, s */
    put(d2b((double)(float)d));  /* fcvt s, d then fcvt d, s */
    put(f2b((float)(double)f));
}

/* ---- fmov immediates ---- */
static NOINLINE float imm_s0(void) { return 1.5f; }
static NOINLINE float imm_s1(void) { return 0.125f; }
static NOINLINE float imm_s2(void) { return -2.0f; }
static NOINLINE double imm_d0(void) { return 31.0; }
static NOINLINE double imm_d1(void) { return -0.5; }
static NOINLINE double imm_d2(void) { return 4096.0; }

void test_main(void) {
    for (u32 i = 0; i < NS; i++) {
        float fa = b2f(SF[i]);
        double da = b2d(SD[i]);
        f_unary(fa);
        d_unary(da);
        f_fromint(SF[i], SD[i]);
        d_fromint(SF[i], SD[i]);
        conv_misc(fa, da);
        R(half_ops(fa, da));
        put(f2b(imm_s0())); put(f2b(imm_s1())); put(f2b(imm_s2()));
        put(d2b(imm_d0())); put(d2b(imm_d1())); put(d2b(imm_d2()));
    }
    for (u32 i = 0; i < NS; i++) {
        for (u32 j = 0; j < NS; j++) {
            u32 k = (i * 7 + j * 3 + 1) % NS;
            f_pair(b2f(SF[i]), b2f(SF[j]), b2f(SF[k]));
            d_pair(b2d(SD[i]), b2d(SD[j]), b2d(SD[k]));
        }
    }
    R(0xf10a7ull);
}
