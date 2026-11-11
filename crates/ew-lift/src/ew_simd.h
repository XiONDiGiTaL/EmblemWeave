
/* ---- Advanced SIMD ------------------------------------------------------
 *
 * Written from the Arm pseudocode independently of the Rust reference in
 * ew-lift/src/simd.rs, so a differential run compares two readings of the
 * architecture. Default FPCR: round to nearest even, no flush to zero; NaNs
 * propagate by the Arm priority rule and created NaNs are the Arm default NaN.
 * Operation numbers match the Rust enums' discriminants.
 * Hosts are little-endian (x86-64 and AArch64), which the lane access assumes.
 */

typedef struct { ew_u8 b[16]; } ew_vr;

static inline ew_vr ew_vr_get(Cpu *cpu, unsigned r)
{
    ew_vr v;
    memcpy(v.b, &cpu->v[r & 31u][0], 8);
    memcpy(v.b + 8, &cpu->v[r & 31u][1], 8);
    return v;
}

/* Keep `bytes` bytes, zero the rest. */
static inline void ew_vr_put(Cpu *cpu, unsigned r, ew_vr v, unsigned bytes)
{
    unsigned i;
    for (i = bytes; i < 16u; i++) v.b[i] = 0;
    memcpy(&cpu->v[r & 31u][0], v.b, 8);
    memcpy(&cpu->v[r & 31u][1], v.b + 8, 8);
}

static inline ew_u64 ew_lane(const ew_vr *v, unsigned i, unsigned es)
{
    ew_u64 x = 0;
    unsigned k;
    for (k = 0; k < es; k++) x |= (ew_u64)v->b[i * es + k] << (8u * k);
    return x;
}

static inline void ew_lane_set(ew_vr *v, unsigned i, unsigned es, ew_u64 x)
{
    unsigned k;
    for (k = 0; k < es; k++) v->b[i * es + k] = (ew_u8)(x >> (8u * k));
}

static inline ew_u64 ew_emask(unsigned es)
{
    return es >= 8u ? ~(ew_u64)0 : (((ew_u64)1 << (8u * es)) - 1u);
}

static inline ew_i64 ew_esx(ew_u64 v, unsigned es)
{
    unsigned s = 64u - 8u * es;
    return (ew_i64)(v << s) >> s;
}

static inline ew_u64 ew_gpr(Cpu *cpu, unsigned r)
{
    return r == 31u ? 0 : cpu->x[r];
}

/* -- floating point at both widths. The template is expanded for float and
 *    double; F is the type, U the bit type, S the suffix. */
#define EW_FP_TEMPLATE(F, U, S, QB, DN, SIGN)                                 \
static inline F ew_f##S(ew_u64 b) { U u = (U)b; F f; memcpy(&f, &u, sizeof f); return f; } \
static inline ew_u64 ew_b##S(F f) { U u; memcpy(&u, &f, sizeof u); return (ew_u64)u; } \
static inline int ew_isnan##S(F f) { return f != f; }                         \
static inline int ew_snan##S(F f) { return f != f && (ew_b##S(f) & (QB)) == 0; } \
static inline F ew_quiet##S(F f) { return ew_f##S(ew_b##S(f) | (QB)); }       \
static inline F ew_dnan##S(void) { return ew_f##S(DN); }                      \
static inline int ew_nan2##S(F a, F b, F *out)                                \
{                                                                             \
    if (ew_snan##S(a)) { *out = ew_quiet##S(a); return 1; }                   \
    if (ew_snan##S(b)) { *out = ew_quiet##S(b); return 1; }                   \
    if (ew_isnan##S(a)) { *out = a; return 1; }                               \
    if (ew_isnan##S(b)) { *out = b; return 1; }                               \
    return 0;                                                                 \
}                                                                             \
static inline int ew_nan3##S(F a, F b, F c, F *out)                           \
{                                                                             \
    if (ew_snan##S(a)) { *out = ew_quiet##S(a); return 1; }                   \
    if (ew_snan##S(b)) { *out = ew_quiet##S(b); return 1; }                   \
    if (ew_snan##S(c)) { *out = ew_quiet##S(c); return 1; }                   \
    if (ew_isnan##S(a)) { *out = a; return 1; }                               \
    if (ew_isnan##S(b)) { *out = b; return 1; }                               \
    if (ew_isnan##S(c)) { *out = c; return 1; }                               \
    return 0;                                                                 \
}                                                                             \
static inline F ew_fix##S(F r) { return ew_isnan##S(r) ? ew_dnan##S() : r; } \
static inline F ew_neg##S(F x) { return ew_f##S(ew_b##S(x) ^ (SIGN)); }       \
static inline F ew_abs##S(F x) { return ew_f##S(ew_b##S(x) & ~(ew_u64)(SIGN)); } \
static inline int ew_infzero##S(F a, F b)                                     \
{                                                                             \
    return (isinf(a) && b == 0) || (a == 0 && isinf(b));                      \
}                                                                             \
static inline F ew_max##S(F a, F b)                                           \
{                                                                             \
    F n;                                                                      \
    if (ew_nan2##S(a, b, &n)) return n;                                       \
    if (a == b) return (a == 0 && !(signbit(a) && signbit(b))) ? (F)0 : a;    \
    return a > b ? a : b;                                                     \
}                                                                             \
static inline F ew_min##S(F a, F b)                                           \
{                                                                             \
    F n;                                                                      \
    if (ew_nan2##S(a, b, &n)) return n;                                       \
    if (a == b) return (a == 0 && (signbit(a) || signbit(b))) ? -(F)0 : a;    \
    return a < b ? a : b;                                                     \
}                                                                             \
static inline int ew_qonly##S(F x) { return ew_isnan##S(x) && !ew_snan##S(x); } \
static inline F ew_maxnm##S(F a, F b)                                         \
{                                                                             \
    if (ew_qonly##S(a) && !ew_qonly##S(b)) a = -(F)INFINITY;                  \
    else if (ew_qonly##S(b) && !ew_qonly##S(a)) b = -(F)INFINITY;             \
    return ew_max##S(a, b);                                                   \
}                                                                             \
static inline F ew_minnm##S(F a, F b)                                         \
{                                                                             \
    if (ew_qonly##S(a) && !ew_qonly##S(b)) a = (F)INFINITY;                   \
    else if (ew_qonly##S(b) && !ew_qonly##S(a)) b = (F)INFINITY;              \
    return ew_min##S(a, b);                                                   \
}                                                                             \
static inline F ew_step##S(F a, F b, F k, F scale)                            \
{                                                                             \
    F n;                                                                      \
    a = ew_neg##S(a);                                                         \
    if (ew_nan2##S(a, b, &n)) return n;                                       \
    if (ew_infzero##S(a, b)) return k * scale;                                \
    if (isinf(a) || isinf(b))                                                 \
        return (signbit(a) != signbit(b)) ? -(F)INFINITY : (F)INFINITY;       \
    return ew_fix##S(fma##S##_(a, b, k) * scale);                             \
}                                                                             \
static inline ew_u64 ew_fp3##S(unsigned op, F x, F y, F z)                    \
{                                                                             \
    F n;                                                                      \
    ew_u64 t = (sizeof(F) == 4) ? 0xFFFFFFFFull : ~(ew_u64)0;                 \
    switch (op) {                                                             \
    case 64: case 83: return ew_b##S(ew_nan2##S(x, y, &n) ? n : ew_fix##S(x + y)); \
    case 65: return ew_b##S(ew_nan2##S(x, y, &n) ? n : ew_fix##S(x - y));     \
    case 66: return ew_b##S(ew_nan2##S(x, y, &n) ? n : ew_fix##S(x * y));     \
    case 67: return ew_b##S(ew_nan2##S(x, y, &n) ? n : ew_fix##S(x / y));     \
    case 68: case 84: return ew_b##S(ew_max##S(x, y));                        \
    case 69: case 85: return ew_b##S(ew_min##S(x, y));                        \
    case 70: case 86: return ew_b##S(ew_maxnm##S(x, y));                      \
    case 71: case 87: return ew_b##S(ew_minnm##S(x, y));                      \
    case 72: case 73: {                                                       \
        F a = (op == 73) ? ew_neg##S(x) : x;                                  \
        if (ew_qonly##S(z) && ew_infzero##S(a, y)) return ew_b##S(ew_dnan##S()); \
        if (ew_nan3##S(z, a, y, &n)) return ew_b##S(n);                       \
        return ew_b##S(ew_fix##S(fma##S##_(a, y, z)));                        \
    }                                                                         \
    case 74: {                                                                \
        F d = ew_nan2##S(x, y, &n) ? n : ew_fix##S(x - y);                    \
        return ew_b##S(ew_abs##S(d));                                         \
    }                                                                         \
    case 75:                                                                  \
        if (ew_nan2##S(x, y, &n)) return ew_b##S(n);                          \
        if (ew_infzero##S(x, y))                                              \
            return ew_b##S((signbit(x) != signbit(y)) ? (F)-2.0 : (F)2.0);    \
        return ew_b##S(ew_fix##S(x * y));                                     \
    case 76: return (x == y) ? t : 0;                                         \
    case 77: return (x >= y) ? t : 0;                                         \
    case 78: return (x > y) ? t : 0;                                          \
    case 79: return (ew_abs##S(x) >= ew_abs##S(y)) ? t : 0;                   \
    case 80: return (ew_abs##S(x) > ew_abs##S(y)) ? t : 0;                    \
    case 81: return ew_b##S(ew_step##S(x, y, (F)2.0, (F)1.0));                \
    case 82: return ew_b##S(ew_step##S(x, y, (F)3.0, (F)0.5));                \
    default: return 0;                                                        \
    }                                                                         \
}

#define fma32_(a, b, c) fmaf((a), (b), (c))
#define fma64_(a, b, c) fma((a), (b), (c))

EW_FP_TEMPLATE(float, ew_u32, 32, 0x00400000u, 0x7FC00000u, 0x80000000u)
EW_FP_TEMPLATE(double, ew_u64, 64, 0x0008000000000000ull, 0x7FF8000000000000ull, 0x8000000000000000ull)

static inline ew_u64 ew_fp3(unsigned op, unsigned es, ew_u64 a, ew_u64 b, ew_u64 d)
{
    if (es == 4u) return ew_fp332(op, ew_f32(a), ew_f32(b), ew_f32(d));
    return ew_fp364(op, ew_f64(a), ew_f64(b), ew_f64(d));
}

static inline ew_u64 ew_int3(unsigned op, unsigned es, ew_u64 a, ew_u64 b, ew_u64 d)
{
    ew_u64 m = ew_emask(es), t = m, r;
    unsigned bits = 8u * es;
    ew_i64 sa, sb;
    a &= m; b &= m; d &= m;
    sa = ew_esx(a, es); sb = ew_esx(b, es);
    switch (op) {
    case 0: case 27: r = a + b; break;
    case 1:  r = a - b; break;
    case 2:  r = a * b; break;
    case 3:  r = d + a * b; break;
    case 4:  r = d - a * b; break;
    case 5:  r = a & b; break;
    case 6:  r = a & ~b; break;
    case 7:  r = a | b; break;
    case 8:  r = a | ~b; break;
    case 9:  r = a ^ b; break;
    case 10: r = (d & a) | (~d & b); break;
    case 11: r = (d & ~b) | (a & b); break;
    case 12: r = (d & b) | (a & ~b); break;
    case 13: r = (a == b) ? t : 0; break;
    case 14: r = (sa > sb) ? t : 0; break;
    case 15: r = (sa >= sb) ? t : 0; break;
    case 16: r = (a > b) ? t : 0; break;
    case 17: r = (a >= b) ? t : 0; break;
    case 18: r = (a & b) ? t : 0; break;
    case 19: case 28: r = (sa >= sb) ? a : b; break;
    case 20: case 29: r = (sa <= sb) ? a : b; break;
    case 21: case 30: r = (a >= b) ? a : b; break;
    case 22: case 31: r = (a <= b) ? a : b; break;
    case 23: r = (sa > sb) ? (ew_u64)(sa - sb) : (ew_u64)(sb - sa); break;
    case 24: r = (a > b) ? a - b : b - a; break;
    case 25: case 26: {
        int sh = (int)(signed char)(ew_u8)b;
        if (sh >= 0) {
            r = ((unsigned)sh >= bits) ? 0 : a << sh;
        } else {
            unsigned n = (unsigned)(-sh);
            if (op == 25) r = (ew_u64)(sa >> (n > 63u ? 63u : n));
            else r = (n >= bits) ? 0 : a >> n;
        }
        break;
    }
    default: r = 0; break;
    }
    return r & m;
}

static inline ew_u64 ew_v3op(unsigned op, unsigned es, ew_u64 a, ew_u64 b, ew_u64 d)
{
    return op >= 64u ? ew_fp3(op, es, a, b, d) : ew_int3(op, es, a, b, d);
}

static inline double ew_rnd(double x, unsigned mode)
{
    switch (mode) {
    case 0: return nearbyint(x);
    case 1: return ceil(x);
    case 2: return floor(x);
    case 3: return trunc(x);
    default: return round(x);
    }
}

static inline ew_u64 ew_vfp_to_int(unsigned es, ew_u64 v, unsigned mode, int uns, unsigned fbits)
{
    double x = (es == 4u) ? (double)ew_f32(v) : ew_f64(v);
    double r;
    if (x != x) return 0;
    r = ew_rnd(ldexp(x, (int)fbits), mode);
    if (es == 4u) {
        if (uns) {
            if (r <= 0) return 0;
            if (r >= 4294967295.0) return 0xFFFFFFFFull;
            return (ew_u64)(ew_u32)r;
        }
        if (r >= 2147483647.0) return 0x7FFFFFFFull;
        if (r <= -2147483648.0) return 0x80000000ull;
        return (ew_u64)(ew_u32)(ew_i32)r;
    }
    if (uns) {
        if (r <= 0) return 0;
        if (r >= 18446744073709551616.0) return ~(ew_u64)0;
        return (ew_u64)r;
    }
    if (r >= 9223372036854775808.0) return 0x7FFFFFFFFFFFFFFFull;
    if (r <= -9223372036854775808.0) return 0x8000000000000000ull;
    return (ew_u64)(ew_i64)r;
}

static inline ew_u64 ew_vint_to_fp(unsigned es, ew_u64 v, int uns, unsigned fbits)
{
    if (es == 4u) {
        double x = uns ? (double)(ew_u32)v : (double)(ew_i32)(ew_u32)v;
        return ew_b32((float)ldexp(x, -(int)fbits));
    }
    if (fbits == 0u) return ew_b64(uns ? (double)v : (double)(ew_i64)v);
    return ew_b64(ldexp(uns ? (double)v : (double)(ew_i64)v, -(int)fbits));
}

static inline ew_u64 ew_fround(unsigned es, ew_u64 a, unsigned mode)
{
    if (es == 4u) {
        float f = ew_f32(a);
        if (f != f) return ew_b32(ew_quiet32(f));
        return ew_b32((float)ew_rnd((double)f, mode));
    } else {
        double f = ew_f64(a);
        if (f != f) return ew_b64(ew_quiet64(f));
        return ew_b64(ew_rnd(f, mode));
    }
}

/* Arm FPRecipEstimate / FPRSqrtEstimate (round to nearest, no flush to zero),
 * written from the architecture pseudocode independently of the Rust model. */
static inline void ew_est_fields(unsigned es, ew_u64 a, ew_u64 *sign, long long *exp, ew_u64 *frac)
{
    if (es == 4u) {
        a &= 0xFFFFFFFFull;
        *sign = a >> 31; *exp = (long long)((a >> 23) & 0xFF); *frac = (a & 0x7FFFFFull) << 29;
    } else {
        *sign = a >> 63; *exp = (long long)((a >> 52) & 0x7FF); *frac = a & 0xFFFFFFFFFFFFFull;
    }
}

static inline int ew_est_special(unsigned es, ew_u64 a, int rsqrt, ew_u64 *out)
{
    ew_u64 sign, frac; long long exp;
    ew_u64 maxe = es == 4u ? 0xFFu : 0x7FFu;
    ew_u64 inf = es == 4u ? 0x7F800000ull : 0x7FF0000000000000ull;
    ew_u64 qbit = es == 4u ? 0x400000ull : (1ull << 51);
    ew_u64 dnan = es == 4u ? 0x7FC00000ull : 0x7FF8000000000000ull;
    ew_u64 bits = es == 4u ? (a & 0xFFFFFFFFull) : a;
    ew_u64 sbit;
    ew_est_fields(es, a, &sign, &exp, &frac);
    sbit = sign << (es * 8u - 1u);
    if ((ew_u64)exp == maxe && frac) { *out = bits | qbit; return 1; }
    if (exp == 0 && !frac) { *out = sbit | inf; return 1; }
    if ((ew_u64)exp == maxe) { *out = rsqrt ? (sign ? dnan : 0) : sbit; return 1; }
    if (rsqrt && sign) { *out = dnan; return 1; }
    if (!rsqrt && exp == 0 && (frac >> 50) == 0) { *out = sbit | inf; return 1; }
    return 0;
}

static inline ew_u64 ew_frecpe(unsigned es, ew_u64 a)
{
    ew_u64 sign, frac, out, est, fraction, scaled, b; long long exp, rexp;
    if (ew_est_special(es, a, 0, &out)) return out;
    ew_est_fields(es, a, &sign, &exp, &frac);
    if (exp == 0) {
        if (!((frac >> 51) & 1)) { exp = -1; frac = (frac << 2) & 0xFFFFFFFFFFFFFull; }
        else frac = (frac << 1) & 0xFFFFFFFFFFFFFull;
    }
    scaled = 256u | ((frac >> 44) & 0xFF);
    rexp = es == 4u ? 253 - exp : 2045 - exp;
    scaled = scaled * 2 + 1;
    b = (1ull << 19) / scaled;
    est = (b + 1) / 2;
    fraction = (est & 0xFF) << 44;
    if (rexp == 0) fraction = (1ull << 51) | (fraction >> 1);
    else if (rexp == -1) { fraction = (1ull << 50) | (fraction >> 2); rexp = 0; }
    if (es == 4u) return (sign << 31) | (((ew_u64)rexp & 0xFF) << 23) | (fraction >> 29);
    return (sign << 63) | (((ew_u64)rexp & 0x7FF) << 52) | fraction;
}

static inline ew_u64 ew_frsqrte(unsigned es, ew_u64 a)
{
    ew_u64 sign, frac, out, scaled, b, est; long long exp, rexp;
    if (ew_est_special(es, a, 1, &out)) return out;
    ew_est_fields(es, a, &sign, &exp, &frac);
    if (exp == 0) {
        while (!((frac >> 51) & 1)) { frac = (frac << 1) & 0xFFFFFFFFFFFFFull; exp--; }
        frac = (frac << 1) & 0xFFFFFFFFFFFFFull;
    }
    if ((exp & 1) == 0) scaled = 256u | ((frac >> 44) & 0xFF);
    else scaled = 128u | ((frac >> 45) & 0x7F);
    if (scaled < 256) scaled = scaled * 2 + 1;
    else { scaled = (scaled >> 1) << 1; scaled = (scaled + 1) * 2; }
    b = 512;
    while (scaled * (b + 1) * (b + 1) < (1ull << 28)) b++;
    est = ((b + 1) / 2) & 0xFF;
    if (es == 4u) { rexp = (380 - exp) / 2; return (((ew_u64)rexp & 0xFF) << 23) | (est << 15); }
    rexp = (3068 - exp) / 2;
    return (((ew_u64)rexp & 0x7FF) << 52) | (est << 44);
}

static inline ew_u64 ew_v2op(unsigned op, unsigned es, ew_u64 a)
{
    ew_u64 m = ew_emask(es), t = m;
    unsigned bits = 8u * es;
    ew_i64 s = ew_esx(a & m, es);
    a &= m;
    switch (op) {
    case 3: { unsigned c = 0; ew_u8 x = (ew_u8)a; while (x) { c += x & 1u; x >>= 1; } return c; }
    case 4: return ~a & m;
    case 5: { ew_u8 x = (ew_u8)a, r = 0; int i; for (i = 0; i < 8; i++) if (x & (1u << i)) r |= (ew_u8)(0x80u >> i); return r; }
    case 6: { unsigned n = 0; while (n < bits && !(a & ((ew_u64)1 << (bits - 1u - n)))) n++; return n; }
    case 7: { ew_u64 v = (s < 0) ? (~a & m) : a; unsigned n = 0; while (n < bits && !(v & ((ew_u64)1 << (bits - 1u - n)))) n++; return n - 1u; }
    case 8: return (s < 0 ? (ew_u64)0 - (ew_u64)s : (ew_u64)s) & m;
    case 9: return ((ew_u64)0 - (ew_u64)s) & m;
    case 10: return s > 0 ? t : 0;
    case 11: return s >= 0 ? t : 0;
    case 12: return a == 0 ? t : 0;
    case 13: return s <= 0 ? t : 0;
    case 14: return s < 0 ? t : 0;
    case 93: return ew_frecpe(es, a);
    case 94: return ew_frsqrte(es, a);
    case 64: return es == 4u ? ew_b32(ew_abs32(ew_f32(a))) : ew_b64(ew_abs64(ew_f64(a)));
    case 65: return es == 4u ? ew_b32(ew_neg32(ew_f32(a))) : ew_b64(ew_neg64(ew_f64(a)));
    case 66:
        if (es == 4u) {
            float f = ew_f32(a);
            if (f != f) return ew_b32(ew_quiet32(f));
            if (f < 0) return 0x7FC00000ull;
            return ew_b32(sqrtf(f));
        } else {
            double f = ew_f64(a);
            if (f != f) return ew_b64(ew_quiet64(f));
            if (f < 0) return 0x7FF8000000000000ull;
            return ew_b64(sqrt(f));
        }
    case 67: case 72: case 73: return ew_fround(es, a, 0);
    case 68: return ew_fround(es, a, 1);
    case 69: return ew_fround(es, a, 2);
    case 70: return ew_fround(es, a, 3);
    case 71: return ew_fround(es, a, 4);
    case 74: return ew_vfp_to_int(es, a, 0, 0, 0);
    case 75: return ew_vfp_to_int(es, a, 1, 0, 0);
    case 76: return ew_vfp_to_int(es, a, 2, 0, 0);
    case 77: return ew_vfp_to_int(es, a, 3, 0, 0);
    case 78: return ew_vfp_to_int(es, a, 4, 0, 0);
    case 79: return ew_vfp_to_int(es, a, 0, 1, 0);
    case 80: return ew_vfp_to_int(es, a, 1, 1, 0);
    case 81: return ew_vfp_to_int(es, a, 2, 1, 0);
    case 82: return ew_vfp_to_int(es, a, 3, 1, 0);
    case 83: return ew_vfp_to_int(es, a, 4, 1, 0);
    case 84: return ew_vint_to_fp(es, a, 0, 0);
    case 85: return ew_vint_to_fp(es, a, 1, 0);
    case 86: case 87: case 88: case 89: case 90: {
        double x = (es == 4u) ? (double)ew_f32(a) : ew_f64(a);
        int r;
        if (op == 86) r = x > 0; else if (op == 87) r = x >= 0;
        else if (op == 88) r = x == 0; else if (op == 89) r = x <= 0; else r = x < 0;
        return r ? t : 0;
    }
    default: return 0;
    }
}

static inline ew_u64 ew_d2s_bits(ew_u64 a)
{
    double x = ew_f64(a);
    if (x != x) return ((a >> 63) << 31) | 0x7FC00000ull | ((a >> 29) & 0x003FFFFFull);
    return ew_b32((float)x);
}

static inline ew_u64 ew_s2d_bits(ew_u64 a)
{
    float x = ew_f32(a);
    if (x != x) return (((a >> 31) & 1u) << 63) | 0x7FF8000000000000ull | ((a & 0x003FFFFFull) << 29);
    return ew_b64((double)x);
}

/* -- per-form entry points, one per SimdOp variant -- */

static inline void ew_v_dup_elem(Cpu *cpu, unsigned rd, unsigned rn, unsigned es, unsigned idx, int q)
{
    ew_vr n = ew_vr_get(cpu, rn), r;
    unsigned w = q ? 16u : 8u, i;
    ew_u64 e = ew_lane(&n, idx, es);
    memset(&r, 0, sizeof r);
    for (i = 0; i < w / es; i++) ew_lane_set(&r, i, es, e);
    ew_vr_put(cpu, rd, r, w);
}

static inline void ew_v_dup_gen(Cpu *cpu, unsigned rd, unsigned rn, unsigned es, int q)
{
    ew_vr r;
    unsigned w = q ? 16u : 8u, i;
    ew_u64 e = ew_gpr(cpu, rn) & ew_emask(es);
    memset(&r, 0, sizeof r);
    for (i = 0; i < w / es; i++) ew_lane_set(&r, i, es, e);
    ew_vr_put(cpu, rd, r, w);
}

static inline void ew_v_ins_gen(Cpu *cpu, unsigned rd, unsigned rn, unsigned es, unsigned idx)
{
    ew_vr r = ew_vr_get(cpu, rd);
    ew_lane_set(&r, idx, es, ew_gpr(cpu, rn) & ew_emask(es));
    ew_vr_put(cpu, rd, r, 16);
}

static inline void ew_v_ins_elem(Cpu *cpu, unsigned rd, unsigned rn, unsigned es, unsigned dst, unsigned src)
{
    ew_vr n = ew_vr_get(cpu, rn), r = ew_vr_get(cpu, rd);
    ew_lane_set(&r, dst, es, ew_lane(&n, src, es));
    ew_vr_put(cpu, rd, r, 16);
}

static inline void ew_v_to_gp(Cpu *cpu, unsigned rd, unsigned rn, unsigned es, unsigned idx, int sgn, int is64)
{
    ew_vr n = ew_vr_get(cpu, rn);
    ew_u64 e = ew_lane(&n, idx, es);
    if (sgn) e = (ew_u64)ew_esx(e, es);
    if (!is64) e &= 0xFFFFFFFFull;
    if (rd != 31u) cpu->x[rd] = e;
}

static inline void ew_v_dup_scalar(Cpu *cpu, unsigned rd, unsigned rn, unsigned es, unsigned idx)
{
    ew_vr n = ew_vr_get(cpu, rn), r;
    memset(&r, 0, sizeof r);
    ew_lane_set(&r, 0, es, ew_lane(&n, idx, es));
    ew_vr_put(cpu, rd, r, es);
}

static inline void ew_v_ext(Cpu *cpu, unsigned rd, unsigned rn, unsigned rm, unsigned pos, int q)
{
    ew_vr n = ew_vr_get(cpu, rn), m = ew_vr_get(cpu, rm), r;
    ew_u8 cat[32];
    unsigned w = q ? 16u : 8u;
    memcpy(cat, n.b, w);
    memcpy(cat + w, m.b, w);
    memset(&r, 0, sizeof r);
    memcpy(r.b, cat + pos, w);
    ew_vr_put(cpu, rd, r, w);
}

static inline void ew_v_imm(Cpu *cpu, unsigned rd, ew_u64 imm, unsigned kind, int q)
{
    ew_vr r = ew_vr_get(cpu, rd);
    unsigned w = q ? 16u : 8u, i;
    for (i = 0; i < w / 8u; i++) {
        ew_u64 d = ew_lane(&r, i, 8);
        ew_lane_set(&r, i, 8, kind == 0 ? imm : kind == 1 ? (d | imm) : (d & ~imm));
    }
    ew_vr_put(cpu, rd, r, w);
}

static inline int ew_v3_pairwise(unsigned op)
{
    return (op >= 27u && op <= 31u) || (op >= 83u && op <= 87u);
}

static inline void ew_v_three(Cpu *cpu, unsigned rd, unsigned rn, unsigned rm, unsigned op, unsigned es, int q, int scalar)
{
    ew_vr n = ew_vr_get(cpu, rn), m = ew_vr_get(cpu, rm), d = ew_vr_get(cpu, rd), r;
    unsigned w = scalar ? es : (q ? 16u : 8u), lanes = w / es, i;
    memset(&r, 0, sizeof r);
    if (ew_v3_pairwise(op)) {
        unsigned h = lanes / 2u;
        for (i = 0; i < h; i++) {
            ew_lane_set(&r, i, es, ew_v3op(op, es, ew_lane(&n, 2u * i, es), ew_lane(&n, 2u * i + 1u, es), 0));
            ew_lane_set(&r, h + i, es, ew_v3op(op, es, ew_lane(&m, 2u * i, es), ew_lane(&m, 2u * i + 1u, es), 0));
        }
    } else {
        for (i = 0; i < lanes; i++)
            ew_lane_set(&r, i, es, ew_v3op(op, es, ew_lane(&n, i, es), ew_lane(&m, i, es), ew_lane(&d, i, es)));
    }
    ew_vr_put(cpu, rd, r, w);
}

static inline void ew_v_pair(Cpu *cpu, unsigned rd, unsigned rn, unsigned op, unsigned es)
{
    ew_vr n = ew_vr_get(cpu, rn), r;
    memset(&r, 0, sizeof r);
    ew_lane_set(&r, 0, es, ew_v3op(op, es, ew_lane(&n, 0, es), ew_lane(&n, 1, es), 0));
    ew_vr_put(cpu, rd, r, es);
}

static inline void ew_v_two(Cpu *cpu, unsigned rd, unsigned rn, unsigned op, unsigned es, int q, int scalar)
{
    ew_vr n = ew_vr_get(cpu, rn), r;
    unsigned i;
    memset(&r, 0, sizeof r);
    if (op <= 2u) { /* rev16, rev32, rev64 */
        unsigned c = op == 0u ? 2u : op == 1u ? 4u : 8u, per = c / es, w = q ? 16u : 8u;
        for (i = 0; i < w / es; i++)
            ew_lane_set(&r, i, es, ew_lane(&n, (i / per) * per + (per - 1u - i % per), es));
        ew_vr_put(cpu, rd, r, w);
    } else if (op == 15u || op == 91u) { /* xtn, fcvtn: es is the narrow lane */
        unsigned lanes = 8u / es, base = q ? lanes : 0u;
        if (q) r = ew_vr_get(cpu, rd);
        for (i = 0; i < lanes; i++) {
            ew_u64 wide = ew_lane(&n, i, es * 2u);
            ew_lane_set(&r, base + i, es, op == 91u ? ew_d2s_bits(wide) : (wide & ew_emask(es)));
        }
        ew_vr_put(cpu, rd, r, q ? 16u : 8u);
    } else if (op == 92u) { /* fcvtl */
        unsigned base = q ? 2u : 0u;
        for (i = 0; i < 2u; i++) ew_lane_set(&r, i, 8, ew_s2d_bits(ew_lane(&n, base + i, 4)));
        ew_vr_put(cpu, rd, r, 16);
    } else if (op == 16u || op == 17u) { /* saddlp, uaddlp */
        unsigned h = es / 2u, w = q ? 16u : 8u;
        for (i = 0; i < w / es; i++) {
            ew_u64 x = ew_lane(&n, 2u * i, h), y = ew_lane(&n, 2u * i + 1u, h), v;
            v = (op == 16u) ? (ew_u64)(ew_esx(x, h) + ew_esx(y, h)) : x + y;
            ew_lane_set(&r, i, es, v & ew_emask(es));
        }
        ew_vr_put(cpu, rd, r, w);
    } else {
        unsigned w = scalar ? es : (q ? 16u : 8u);
        for (i = 0; i < w / es; i++) ew_lane_set(&r, i, es, ew_v2op(op, es, ew_lane(&n, i, es)));
        ew_vr_put(cpu, rd, r, w);
    }
}

static inline void ew_v_shift(Cpu *cpu, unsigned rd, unsigned rn, unsigned op, unsigned es, unsigned sh, int q, int scalar)
{
    ew_vr n = ew_vr_get(cpu, rn), d = ew_vr_get(cpu, rd), r;
    ew_u64 m = ew_emask(es);
    unsigned i;
    memset(&r, 0, sizeof r);
    if (op == 7u) { /* shrn */
        unsigned lanes = 8u / es, base = q ? lanes : 0u;
        if (q) r = d;
        for (i = 0; i < lanes; i++) ew_lane_set(&r, base + i, es, (ew_lane(&n, i, es * 2u) >> sh) & m);
        ew_vr_put(cpu, rd, r, q ? 16u : 8u);
    } else if (op == 8u || op == 9u) { /* sshll, ushll */
        unsigned h = es / 2u, lanes = 16u / es, base = q ? lanes : 0u;
        for (i = 0; i < lanes; i++) {
            ew_u64 x = ew_lane(&n, base + i, h);
            if (op == 8u) x = (ew_u64)ew_esx(x, h);
            ew_lane_set(&r, i, es, (x << sh) & m);
        }
        ew_vr_put(cpu, rd, r, 16);
    } else {
        unsigned w = scalar ? es : (q ? 16u : 8u), bits = 8u * es;
        for (i = 0; i < w / es; i++) {
            ew_u64 a = ew_lane(&n, i, es), old = ew_lane(&d, i, es), v;
            ew_u64 sshr = (ew_u64)(ew_esx(a, es) >> (sh > 63u ? 63u : sh)) & m;
            ew_u64 ushr = sh >= bits ? 0 : a >> sh;
            switch (op) {
            case 0: v = sshr; break;
            case 1: v = ushr; break;
            case 2: v = (old + sshr) & m; break;
            case 3: v = (old + ushr) & m; break;
            case 4: v = (a << sh) & m; break;
            case 5: v = sh >= bits ? old : ((old & ~(m >> sh)) | ushr); break;
            case 6: { ew_u64 k = (m << sh) & m; v = (old & ~k) | ((a << sh) & k); break; }
            case 10: v = ew_vint_to_fp(es, a, 0, sh); break;
            case 11: v = ew_vint_to_fp(es, a, 1, sh); break;
            case 12: v = ew_vfp_to_int(es, a, 3, 0, sh); break;
            case 13: v = ew_vfp_to_int(es, a, 3, 1, sh); break;
            default: v = 0; break;
            }
            ew_lane_set(&r, i, es, v);
        }
        ew_vr_put(cpu, rd, r, w);
    }
}

static inline void ew_v_long(Cpu *cpu, unsigned rd, unsigned rn, unsigned rm, unsigned op, int sgn, unsigned es, int q)
{
    ew_vr n = ew_vr_get(cpu, rn), m = ew_vr_get(cpu, rm), d = ew_vr_get(cpu, rd), r;
    unsigned h = es / 2u, lanes = 16u / es, base = q ? lanes : 0u, i;
    memset(&r, 0, sizeof r);
    for (i = 0; i < lanes; i++) {
        ew_u64 a, b, v, old = ew_lane(&d, i, es);
        if (op == 1u || op == 3u) a = ew_lane(&n, i, es);
        else { a = ew_lane(&n, base + i, h); if (sgn) a = (ew_u64)ew_esx(a, h); }
        b = ew_lane(&m, base + i, h);
        if (sgn) b = (ew_u64)ew_esx(b, h);
        switch (op) {
        case 0: case 1: v = a + b; break;
        case 2: case 3: v = a - b; break;
        case 4:
            if (sgn) v = ((ew_i64)a > (ew_i64)b) ? a - b : b - a;
            else v = a > b ? a - b : b - a;
            break;
        case 5: v = old + a * b; break;
        case 6: v = old - a * b; break;
        default: v = a * b; break;
        }
        ew_lane_set(&r, i, es, v & ew_emask(es));
    }
    ew_vr_put(cpu, rd, r, 16);
}

static inline void ew_v_perm(Cpu *cpu, unsigned rd, unsigned rn, unsigned rm, unsigned op, unsigned es, int q)
{
    ew_vr n = ew_vr_get(cpu, rn), m = ew_vr_get(cpu, rm), r;
    unsigned w = q ? 16u : 8u, lanes = w / es, h = lanes / 2u, i;
    memset(&r, 0, sizeof r);
    for (i = 0; i < lanes; i++) {
        ew_u64 v;
        unsigned k;
        switch (op) {
        case 0: k = 2u * i; v = k < lanes ? ew_lane(&n, k, es) : ew_lane(&m, k - lanes, es); break;
        case 1: k = 2u * i + 1u; v = k < lanes ? ew_lane(&n, k, es) : ew_lane(&m, k - lanes, es); break;
        case 2: v = (i % 2u == 0u) ? ew_lane(&n, i, es) : ew_lane(&m, i - 1u, es); break;
        case 3: v = (i % 2u == 0u) ? ew_lane(&n, i + 1u, es) : ew_lane(&m, i, es); break;
        case 4: v = (i % 2u == 0u) ? ew_lane(&n, i / 2u, es) : ew_lane(&m, i / 2u, es); break;
        default: v = (i % 2u == 0u) ? ew_lane(&n, h + i / 2u, es) : ew_lane(&m, h + i / 2u, es); break;
        }
        ew_lane_set(&r, i, es, v);
    }
    ew_vr_put(cpu, rd, r, w);
}

static inline void ew_v_elem(Cpu *cpu, unsigned rd, unsigned rn, unsigned rm, unsigned idx, unsigned op, unsigned es, int q, int scalar)
{
    ew_vr n = ew_vr_get(cpu, rn), mr = ew_vr_get(cpu, rm), d = ew_vr_get(cpu, rd), r;
    ew_u64 e = ew_lane(&mr, idx, es), m = ew_emask(es);
    unsigned i;
    memset(&r, 0, sizeof r);
    if (op >= 3u && op <= 6u) { /* smull, umull, smlal, umlal */
        unsigned wd = es * 2u, lanes = 8u / es, base = q ? lanes : 0u;
        int sgn = (op == 3u || op == 5u);
        ew_u64 ee = sgn ? (ew_u64)ew_esx(e, es) : e;
        for (i = 0; i < lanes; i++) {
            ew_u64 a = ew_lane(&n, base + i, es), p;
            if (sgn) a = (ew_u64)ew_esx(a, es);
            p = a * ee;
            if (op >= 5u) p += ew_lane(&d, i, wd);
            ew_lane_set(&r, i, wd, p & ew_emask(wd));
        }
        ew_vr_put(cpu, rd, r, 16);
        return;
    }
    {
        unsigned w = scalar ? es : (q ? 16u : 8u);
        for (i = 0; i < w / es; i++) {
            ew_u64 a = ew_lane(&n, i, es), old = ew_lane(&d, i, es), v;
            switch (op) {
            case 0: v = (a * e) & m; break;
            case 1: v = (old + a * e) & m; break;
            case 2: v = (old - a * e) & m; break;
            case 64: v = ew_fp3(66, es, a, e, old); break;
            case 65: v = ew_fp3(72, es, a, e, old); break;
            case 66: v = ew_fp3(73, es, a, e, old); break;
            default: v = ew_fp3(75, es, a, e, old); break;
            }
            ew_lane_set(&r, i, es, v);
        }
        ew_vr_put(cpu, rd, r, w);
    }
}

static inline ew_u64 ew_reduce(const ew_u64 *v, unsigned count, unsigned op, unsigned es)
{
    unsigned h;
    if (count == 1u) return v[0];
    h = count / 2u;
    return ew_fp3(op, es, ew_reduce(v, h, op, es), ew_reduce(v + h, count - h, op, es), 0);
}

static inline void ew_v_across(Cpu *cpu, unsigned rd, unsigned rn, unsigned op, unsigned es, int q)
{
    ew_vr n = ew_vr_get(cpu, rn), r;
    unsigned w = q ? 16u : 8u, lanes = w / es, i, out = es;
    ew_u64 vals[16], acc;
    memset(&r, 0, sizeof r);
    for (i = 0; i < lanes; i++) vals[i] = ew_lane(&n, i, es);
    acc = vals[0];
    switch (op) {
    case 0: for (i = 1; i < lanes; i++) acc += vals[i]; acc &= ew_emask(es); break;
    case 1: for (i = 1; i < lanes; i++) if (ew_esx(vals[i], es) > ew_esx(acc, es)) acc = vals[i]; break;
    case 2: for (i = 1; i < lanes; i++) if (ew_esx(vals[i], es) < ew_esx(acc, es)) acc = vals[i]; break;
    case 3: for (i = 1; i < lanes; i++) if (vals[i] > acc) acc = vals[i]; break;
    case 4: for (i = 1; i < lanes; i++) if (vals[i] < acc) acc = vals[i]; break;
    case 5: { ew_i64 s = 0; for (i = 0; i < lanes; i++) s += ew_esx(vals[i], es); out = es * 2u; acc = (ew_u64)s & ew_emask(out); break; }
    case 6: { ew_u64 s = 0; for (i = 0; i < lanes; i++) s += vals[i]; out = es * 2u; acc = s & ew_emask(out); break; }
    case 64: acc = ew_reduce(vals, lanes, 68, es); break;
    case 65: acc = ew_reduce(vals, lanes, 69, es); break;
    case 66: acc = ew_reduce(vals, lanes, 70, es); break;
    default: acc = ew_reduce(vals, lanes, 71, es); break;
    }
    ew_lane_set(&r, 0, out, acc);
    ew_vr_put(cpu, rd, r, out);
}

static inline void ew_v_tbl(Cpu *cpu, unsigned rd, unsigned rn, unsigned rm, unsigned len, int tbx, int q)
{
    ew_u8 table[64];
    ew_vr m = ew_vr_get(cpu, rm), d = ew_vr_get(cpu, rd), r;
    unsigned w = q ? 16u : 8u, k, i;
    for (k = 0; k < len; k++) { ew_vr t = ew_vr_get(cpu, (rn + k) & 31u); memcpy(table + 16u * k, t.b, 16); }
    memset(&r, 0, sizeof r);
    for (i = 0; i < w; i++) {
        unsigned idx = m.b[i];
        r.b[i] = idx < 16u * len ? table[idx] : (tbx ? d.b[i] : 0);
    }
    ew_vr_put(cpu, rd, r, w);
}

/* shape: 0 multiple structures, 1 single lane, 2 replicate.
 * post: 0 none, 1 immediate, 2 register. */
static inline void ew_v_mem(Cpu *cpu, unsigned rt, unsigned rn, unsigned regs, unsigned selem,
                            unsigned es, int q, int load, unsigned shape, unsigned idx,
                            unsigned post, ew_u64 postv)
{
    ew_u64 base = rn == 31u ? cpu->sp : cpu->x[rn], addr = base;
    unsigned w = q ? 16u : 8u, s, e, k;
    if (shape == 0u) {
        ew_vr rr[4];
        unsigned rpt = regs / selem, r;
        for (k = 0; k < regs; k++) rr[k] = ew_vr_get(cpu, (rt + k) & 31u);
        for (r = 0; r < rpt; r++)
            for (e = 0; e < w / es; e++)
                for (s = 0; s < selem; s++) {
                    if (load) ew_lane_set(&rr[r + s], e, es, ew_load(cpu, addr, es));
                    else ew_store(cpu, addr, ew_lane(&rr[r + s], e, es), es);
                    addr += es;
                }
        if (load) for (k = 0; k < regs; k++) ew_vr_put(cpu, (rt + k) & 31u, rr[k], w);
    } else if (shape == 1u) {
        for (s = 0; s < selem; s++) {
            unsigned reg = (rt + s) & 31u;
            ew_vr v = ew_vr_get(cpu, reg);
            if (load) { ew_lane_set(&v, idx, es, ew_load(cpu, addr, es)); ew_vr_put(cpu, reg, v, 16); }
            else ew_store(cpu, addr, ew_lane(&v, idx, es), es);
            addr += es;
        }
    } else {
        for (s = 0; s < selem; s++) {
            ew_vr v;
            ew_u64 x = ew_load(cpu, addr, es);
            memset(&v, 0, sizeof v);
            for (e = 0; e < w / es; e++) ew_lane_set(&v, e, es, x);
            ew_vr_put(cpu, (rt + s) & 31u, v, w);
            addr += es;
        }
    }
    if (post) {
        ew_u64 nv = base + (post == 1u ? postv : ew_gpr(cpu, (unsigned)postv));
        if (rn == 31u) cpu->sp = nv; else cpu->x[rn] = nv;
    }
}
