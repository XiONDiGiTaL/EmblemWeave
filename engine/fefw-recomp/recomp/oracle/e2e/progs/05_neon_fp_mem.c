// FLAGS:
// EXPECT-INSN: \bfadd\s+v\d+
// EXPECT-INSN: \bfmul\s+v\d+
// EXPECT-INSN: \bfdiv\s+v\d+
// EXPECT-INSN: \bfmla\b
// EXPECT-INSN: \bfsqrt\s+v
// EXPECT-INSN: \bfrecpe\b|\bfrecps\b
// EXPECT-INSN: \bfrsqrte\b|\bfrsqrts\b
// EXPECT-INSN: \bfmax\s+v|\bfmaxnm\s+v
// EXPECT-INSN: \bfaddp\b
// EXPECT-INSN: \bfcvtz[su]\s+v|\bscvtf\s+v|\bucvtf\s+v
// EXPECT-INSN: \bfrint[a-z]\s+v
// EXPECT-INSN: \bfcmeq\b|\bfcmgt\b|\bfcmge\b
// EXPECT-INSN: \bld1\b
// EXPECT-INSN: \bst1\b
// EXPECT-INSN: \bld2\b
// EXPECT-INSN: \bld3\b
// EXPECT-INSN: \bld4\b
// EXPECT-INSN: \bst2\b|\bst3\b|\bst4\b
// EXPECT-INSN: \bld1r\b|\bld2r\b|\bld3r\b|\bld4r\b
#include "t.h"
#include <arm_neon.h>

/* FP AdvSIMD and vector memory access. Results are raw bits (every value is sunk as its bit pattern).
 * FP inputs come from fbytes[] (special patterns first, then lcg bits), memory tests from mem[] (1 KiB). */

static u64 ri, kk;
static u64 hacc = 0x9e3779b97f4a7c15ull;
static u8 fbytes[1024] __attribute__((aligned(16)));
static u8 mem[1024] __attribute__((aligned(16)));
static volatile u64 vseed = 0x1B873593CC9E2D51ull;
typedef float f32;   /* only for sizeof() in the memory templates; intrinsics are named by token (vld1q_f32) */
typedef double f64;

/* qNaN, payload NaN, sNaN, neg qNaN, denormals, 1.0, -1.0, 0.5, 2.5, 3.5, +-1e30, +-0, +-inf */
static const u32 sp32[16] = {0x00000000u, 0x80000000u, 0x7f800000u, 0xff800000u, 0x7fc00000u, 0x7fc12345u,
                             0x7f800001u, 0xffc00000u, 0x00000001u, 0x007fffffu, 0x3f800000u, 0xbf800000u,
                             0x3f000000u, 0x40200000u, 0x40600000u, 0x7149f2cau};
static const u64 sp64[16] = {0x0000000000000000ull, 0x8000000000000000ull, 0x7ff0000000000000ull, 0xfff0000000000000ull,
                             0x7ff8000000000000ull, 0x7ff8000000012345ull, 0x7ff0000000000001ull, 0xfff8000000000000ull,
                             0x0000000000000001ull, 0x000fffffffffffffull, 0x3ff0000000000000ull, 0xbff0000000000000ull,
                             0x3fe0000000000000ull, 0x4004000000000000ull, 0x400c000000000000ull, 0x46293e5939a08ceaull};

#define QF(S, k) vld1q_##S((const void *)(fbytes + ((((u64)(k)) & 63) << 4)))
#define DF(S, k) vld1_##S((const void *)(fbytes + ((((u64)(k)) & 63) << 3)))
#define FF(v, k) __builtin_memcpy(fbytes + ((((u64)(k)) & 63) << 4), &(v), sizeof(v))

/* At -O0 clang copies the 64-byte uint8x16x4_t structs with a memcpy call; there is no libc, so provide a byte loop.
 * optnone keeps the optimiser from turning this loop back into a call to memcpy. */
__attribute__((optnone, noinline)) void *memcpy(void *d, const void *s, size_t n) {
    u8 *dd = d; const u8 *ss = s;
    for (size_t i = 0; i < n; i++) dd[i] = ss[i];
    return d;
}

NOINLINE static void sink(const void *p, u32 n) {
    const u8 *b = p;
    for (u32 i = 0; i < n; i += 8) {
        u64 w;
        __builtin_memcpy(&w, b + i, 8);
        if (ri < NRES - 1) results[ri++] = w;
        hacc = (hacc ^ w) * 0x100000001b3ull + (hacc >> 29);
    }
}
#define SV(v) sink(&(v), sizeof(v))
#define SU(x) do { u64 t_ = (u64)(x); sink(&t_, 8); } while (0)
#define SF(x) do { u64 t_ = (u64)f2b((float)(x)); sink(&t_, 8); } while (0)
#define SD(x) do { u64 t_ = d2b((double)(x)); sink(&t_, 8); } while (0)

NOINLINE static void fill_inputs(void) {
    u64 s = vseed;
    for (int i = 0; i < 16; i++) {
        __builtin_memcpy(fbytes + 4 * i, &sp32[i], 4);       /* f32 specials in bytes 0..63 */
        __builtin_memcpy(fbytes + 512 + 8 * i, &sp64[i], 8); /* f64 specials in bytes 512..639 */
    }
    for (int i = 128; i < 1024; i += 8) {
        if (i >= 512 && i < 640) continue;
        u64 z = lcg(&s);
        __builtin_memcpy(fbytes + i, &z, 8);
    }
    for (int i = 0; i < 1024; i += 8) {
        u64 z = lcg(&s);
        __builtin_memcpy(mem + i, &z, 8);
    }
}

/* ---- FP templates: S is f32 (float32x2/4) or f64 (float64x1/2) ---- */

#define FB2(op, S) do { u64 kb = kk; kk += 4; \
    __auto_type a = QF(S, kb); __auto_type b = QF(S, kb + 1); __auto_type r = op##q_##S(a, b); SV(r); FF(r, kb + 2); \
    __auto_type da = DF(S, kb); __auto_type db = DF(S, kb + 1); __auto_type dr = op##_##S(da, db); SV(dr); FF(dr, kb + 3); } while (0)

#define FT3(op, S) do { u64 kb = kk; kk += 5; \
    __auto_type a = QF(S, kb); __auto_type b = QF(S, kb + 1); __auto_type c = QF(S, kb + 2); \
    __auto_type r = op##q_##S(a, b, c); SV(r); FF(r, kb + 3); \
    __auto_type da = DF(S, kb); __auto_type db = DF(S, kb + 1); __auto_type dc = DF(S, kb + 2); \
    __auto_type dr = op##_##S(da, db, dc); SV(dr); FF(dr, kb + 4); } while (0)

#define FU1(op, S) do { u64 kb = kk; kk += 3; \
    __auto_type a = QF(S, kb); __auto_type r = op##q_##S(a); SV(r); FF(r, kb + 1); \
    __auto_type da = DF(S, kb); __auto_type dr = op##_##S(da); SV(dr); FF(dr, kb + 2); } while (0)

/* the same set, quad-only (for f64 pairwise ops that have no d form) */
#define FQ2(op, S) do { u64 kb = kk; kk += 2; \
    __auto_type a = QF(S, kb); __auto_type b = QF(S, kb + 1); __auto_type r = op##q_##S(a, b); SV(r); FF(r, kb + 2); } while (0)

/* lane forms: multiply / fused multiply-add by a lane of b */
#define FLANE(S, LQL, LDL) do { u64 kb = kk; kk += 3; \
    __auto_type a = QF(S, kb); __auto_type b = QF(S, kb + 1); __auto_type da = DF(S, kb); __auto_type db = DF(S, kb + 1); \
    __auto_type r1 = vmulq_laneq_##S(a, b, LQL); SV(r1); __auto_type r2 = vmul_laneq_##S(da, b, LQL); SV(r2); \
    __auto_type r3 = vmulq_lane_##S(a, db, LDL); SV(r3); __auto_type r4 = vfmaq_laneq_##S(a, a, b, LQL); SV(r4); \
    __auto_type r5 = vfma_lane_##S(da, da, db, LDL); SV(r5); __auto_type r6 = vfmaq_lane_##S(a, b, db, LDL); SV(r6); \
    FF(r4, kb + 2); } while (0)

/* scalar reductions: value (raw bits) */
#define REDF32(S) do { u64 kb = kk; kk += 2; __auto_type a = QF(f32, kb); __auto_type da = DF(f32, kb); \
    SF(vaddvq_f32(a)); SF(vmaxvq_f32(a)); SF(vminvq_f32(a)); SF(vmaxnmvq_f32(a)); SF(vminnmvq_f32(a)); SF(vaddv_f32(da)); \
    SF(vmaxv_f32(da)); SF(vminv_f32(da)); SF(vget_lane_f32(da, 1)); SF(vgetq_lane_f32(a, 3)); \
    __auto_type dp = vdup_n_f32(vget_lane_f32(da, 0)); SV(dp); __auto_type qp = vdupq_n_f32(vgetq_lane_f32(a, 2)); SV(qp); \
    } while (0)

/* ---- parts ---- */

NOINLINE static void part_fp32(void) {
    FB2(vadd, f32); FB2(vsub, f32); FB2(vmul, f32); FB2(vdiv, f32); FB2(vabd, f32);
    FB2(vmax, f32); FB2(vmin, f32); FB2(vmaxnm, f32); FB2(vminnm, f32); FB2(vmulx, f32);
    FB2(vpadd, f32); FB2(vpmax, f32); FB2(vpmin, f32); FB2(vpmaxnm, f32); FB2(vpminnm, f32);
    FB2(vrecps, f32); FB2(vrsqrts, f32);
    FB2(vcage, f32); FB2(vcagt, f32); FB2(vceq, f32); FB2(vcgt, f32); FB2(vcge, f32); FB2(vclt, f32); FB2(vcle, f32);
    FT3(vfma, f32); FT3(vfms, f32); FT3(vmla, f32); FT3(vmls, f32);
    FU1(vabs, f32); FU1(vneg, f32); FU1(vsqrt, f32); FU1(vrecpe, f32); FU1(vrsqrte, f32);
    FU1(vrnd, f32); FU1(vrndn, f32); FU1(vrndm, f32); FU1(vrndp, f32); FU1(vrnda, f32); FU1(vrndx, f32); FU1(vrndi, f32);
    FU1(vceqz, f32); FU1(vcgtz, f32); FU1(vcgez, f32); FU1(vcltz, f32); FU1(vclez, f32);
    FLANE(f32, 3, 1);
    REDF32(f32);
}

NOINLINE static void part_fp64(void) {
    FB2(vadd, f64); FB2(vsub, f64); FB2(vmul, f64); FB2(vdiv, f64); FB2(vabd, f64);
    FB2(vmax, f64); FB2(vmin, f64); FB2(vmaxnm, f64); FB2(vminnm, f64); FB2(vmulx, f64);
    FQ2(vpadd, f64); FQ2(vpmax, f64); FQ2(vpmin, f64); FQ2(vpmaxnm, f64); FQ2(vpminnm, f64);
    FQ2(vrecps, f64); FQ2(vrsqrts, f64);
    FB2(vcage, f64); FB2(vcagt, f64); FB2(vceq, f64); FB2(vcgt, f64); FB2(vcge, f64); FB2(vclt, f64); FB2(vcle, f64);
    FT3(vfma, f64); FT3(vfms, f64);
    FU1(vabs, f64); FU1(vneg, f64); FU1(vsqrt, f64); FU1(vrecpe, f64); FU1(vrsqrte, f64);
    FU1(vrnd, f64); FU1(vrndn, f64); FU1(vrndm, f64); FU1(vrndp, f64); FU1(vrnda, f64); FU1(vrndx, f64); FU1(vrndi, f64);
    FU1(vceqz, f64); FU1(vcgtz, f64); FU1(vcgez, f64); FU1(vcltz, f64); FU1(vclez, f64);
    {   /* pairwise and across-vector f64 reductions, lane forms on 2-lane q */
        u64 kb = kk; kk += 4;
        __auto_type a = QF(f64, kb); __auto_type b = QF(f64, kb + 1);
        __auto_type pd = vpaddd_f64(a); SD(pd);
        SD(vaddvq_f64(a)); SD(vmaxvq_f64(a)); SD(vminvq_f64(a)); SD(vmaxnmvq_f64(a));
        SD(vgetq_lane_f64(b, 1)); SD(vget_lane_f64(DF(f64, kb + 2), 0));
        __auto_type m1 = vmulq_laneq_f64(a, b, 1); SV(m1);
        __auto_type m2 = vfmaq_laneq_f64(a, a, b, 0); SV(m2); FF(m2, kb + 3);
        __auto_type dq = vdupq_n_f64(vgetq_lane_f64(b, 0)); SV(dq);
    }
}

/* conversions: f32 <-> s32/u32 (incl. rounding variants and fixed point), f64 <-> s64/u64, f32 <-> f64 widen/narrow */
NOINLINE static void part_cvt(void) {
    u64 kb = kk; kk += 8;
    __auto_type a = QF(f32, kb), b = QF(f32, kb + 1);
    __auto_type da = DF(f32, kb), db = DF(f32, kb + 1);
    __auto_type c1 = vcvtq_s32_f32(a); SV(c1);
    __auto_type c2 = vcvtq_u32_f32(a); SV(c2);
    __auto_type c3 = vcvt_s32_f32(da); SV(c3);
    __auto_type c4 = vcvt_u32_f32(db); SV(c4);
    __auto_type c5 = vcvtq_f32_s32(vcvtq_s32_f32(b)); SV(c5);
    __auto_type c6 = vcvtq_f32_u32(vcvtq_u32_f32(a)); SV(c6);
    __auto_type c7 = vcvt_f32_s32(vcvt_s32_f32(da)); SV(c7);
    __auto_type n1 = vcvtnq_s32_f32(a); SV(n1);
    __auto_type n2 = vcvtaq_s32_f32(a); SV(n2);
    __auto_type n3 = vcvtmq_s32_f32(a); SV(n3);
    __auto_type n4 = vcvtpq_s32_f32(a); SV(n4);
    __auto_type n5 = vcvtnq_u32_f32(b); SV(n5);
    __auto_type n6 = vcvtmq_u32_f32(b); SV(n6);
    __auto_type f1 = vcvtq_n_s32_f32(a, 3); SV(f1);
    __auto_type f2 = vcvtq_n_u32_f32(a, 7); SV(f2);
    __auto_type f3 = vcvtq_n_f32_s32(vcvtq_s32_f32(a), 5); SV(f3);
    __auto_type f4 = vcvt_n_f32_u32(vcvt_u32_f32(da), 9); SV(f4);
    __auto_type f5 = vcvt_n_s32_f32(da, 11); SV(f5);
    __auto_type w1 = vcvt_f64_f32(da); SV(w1);
    __auto_type w2 = vcvt_high_f64_f32(a); SV(w2);
    __auto_type w3 = vcvt_f32_f64(vcvtq_f64_s64(vcvtq_s64_f64(QF(f64, kb + 2)))); SV(w3);
    __auto_type w4 = vcvt_high_f32_f64(vcvt_f32_f64(QF(f64, kb + 3)), QF(f64, kb + 3)); SV(w4); FF(w4, kb + 4);

    u64 kd = kk; kk += 8;
    __auto_type x = QF(f64, kd), y = QF(f64, kd + 1);
    __auto_type dx = DF(f64, kd);
    __auto_type e1 = vcvtq_s64_f64(x); SV(e1);
    __auto_type e2 = vcvtq_u64_f64(x); SV(e2);
    __auto_type e3 = vcvt_s64_f64(dx); SV(e3);
    __auto_type e4 = vcvtq_f64_s64(e1); SV(e4);
    __auto_type e5 = vcvtq_f64_u64(e2); SV(e5);
    __auto_type e6 = vcvt_f64_s64(vcvt_s64_f64(dx)); SV(e6);
    __auto_type m1 = vcvtnq_s64_f64(y); SV(m1);
    __auto_type m2 = vcvtaq_s64_f64(y); SV(m2);
    __auto_type m3 = vcvtmq_s64_f64(y); SV(m3);
    __auto_type m4 = vcvtpq_u64_f64(y); SV(m4);
    __auto_type g1 = vcvtq_n_s64_f64(x, 13); SV(g1);
    __auto_type g2 = vcvtq_n_u64_f64(y, 17); SV(g2);
    __auto_type g3 = vcvtq_n_f64_s64(e1, 19); SV(g3);
    __auto_type g4 = vcvtq_n_f64_u64(e2, 23); SV(g4); FF(g4, kd + 2);
    __auto_type h1 = vcvtnq_u64_f64(x); SV(h1);
    __auto_type h2 = vrndq_f64(x); SV(h2);
    FF(h2, kd + 3);
}

/* rounding to integral: all frint forms on f32 and f64, plus the fp to int with the same modes */
NOINLINE static void part_round(void) {
    u64 kb = kk; kk += 6;
    __auto_type a = QF(f32, kb), b = QF(f32, kb + 1);
    __auto_type r1 = vrndq_f32(a); SV(r1);
    __auto_type r2 = vrndnq_f32(b); SV(r2);
    __auto_type r3 = vrndaq_f32(a); SV(r3);
    __auto_type r4 = vrndmq_f32(b); SV(r4);
    __auto_type r5 = vrndpq_f32(a); SV(r5);
    __auto_type r6 = vrndxq_f32(b); SV(r6);
    __auto_type r7 = vrndq_f64(QF(f64, kb + 2)); SV(r7);
    __auto_type r8 = vrndnq_f64(QF(f64, kb + 3)); SV(r8);
    __auto_type r9 = vrndaq_f64(QF(f64, kb + 4)); SV(r9);
    FF(r9, kb + 5);
}

/* ---- memory: vld1/vst1 (x1..x4, d and q), ld2/3/4 + st2/3/4, dup (ld1r..ld4r), lanes ---- */

#define MEM_LOOP_A(S, T) do { \
    const u8 *sp = mem; u8 *dp = mem + 512; \
    for (int i = 0; i < 2; i++) { \
        __auto_type a = vld1q_##S((const void *)sp); sp += 16; SV(a); \
        __auto_type b = vld1_##S((const void *)sp); sp += 8; SV(b); \
        __auto_type c = vld1q_##S##_x2((const void *)sp); sp += 32; SV(c); \
        __auto_type e = vld1_##S##_x3((const void *)sp); sp += 24; SV(e); \
        __auto_type f = vld1q_##S##_x4((const void *)sp); sp += 64; SV(f); \
        vst1q_##S((void *)dp, a); dp += 16; \
        vst1_##S((void *)dp, b); dp += 8; \
        vst1q_##S##_x2((void *)dp, c); dp += 32; \
        vst1_##S##_x3((void *)dp, e); dp += 24; \
        vst1q_##S##_x4((void *)dp, f); dp += 64; \
    } \
} while (0)

#define MEM_LOOP_B(S, T) do { \
    const u8 *sp = mem; u8 *dp = mem + 512; \
    for (int i = 0; i < 2; i++) { \
        __auto_type g = vld2q_##S((const void *)sp); sp += 32; SV(g); \
        __auto_type h = vld3_##S((const void *)sp); sp += 24; SV(h); \
        __auto_type k = vld4q_##S((const void *)sp); sp += 64; SV(k); \
        __auto_type dq = vld2_##S((const void *)sp); sp += 16; SV(dq); \
        __auto_type d4 = vld4_##S((const void *)sp); sp += 32; SV(d4); \
        vst2q_##S((void *)dp, g); dp += 32; \
        vst3_##S((void *)dp, h); dp += 24; \
        vst4q_##S((void *)dp, k); dp += 64; \
        vst2_##S((void *)dp, dq); dp += 16; \
        vst4_##S((void *)dp, d4); dp += 32; \
    } \
} while (0)

#define MEM_LOOP_C(S, T) do { \
    const u8 *sp = mem; u8 *dp = mem + 512; \
    for (int i = 0; i < 2; i++) { \
        __auto_type rq = vld1q_dup_##S((const void *)sp); sp += sizeof(T); SV(rq); \
        __auto_type rd = vld1_dup_##S((const void *)sp); sp += sizeof(T); SV(rd); \
        __auto_type r2 = vld2_dup_##S((const void *)sp); sp += 2 * sizeof(T); SV(r2); \
        __auto_type r2q = vld2q_dup_##S((const void *)sp); sp += 2 * sizeof(T); SV(r2q); \
        __auto_type r3 = vld3_dup_##S((const void *)sp); sp += 3 * sizeof(T); SV(r3); \
        __auto_type r3q = vld3q_dup_##S((const void *)sp); sp += 3 * sizeof(T); SV(r3q); \
        __auto_type r4 = vld4_dup_##S((const void *)sp); sp += 4 * sizeof(T); SV(r4); \
        __auto_type r4q = vld4q_dup_##S((const void *)sp); sp += 4 * sizeof(T); SV(r4q); \
        vst1q_##S((void *)dp, rq); dp += 16; \
        vst2q_##S((void *)dp, r2q); dp += 32; \
        vst4_##S((void *)dp, r4); dp += 32; \
    } \
} while (0)

#define MEM_LOOP_D(S, T, LD, LQ) do { \
    const u8 *sp = mem; u8 *dp = mem + 512; \
    __auto_type a = vld1q_##S((const void *)sp); __auto_type da = vld1_##S((const void *)sp); \
    for (int i = 0; i < 2; i++) { \
        a = vld1q_lane_##S((const void *)sp, a, LQ); sp += sizeof(T); SV(a); \
        da = vld1_lane_##S((const void *)sp, da, LD); sp += sizeof(T); SV(da); \
        __auto_type two = vld2q_##S((const void *)sp); \
        two = vld2q_lane_##S((const void *)sp, two, LQ); sp += 2 * sizeof(T); SV(two); \
        __auto_type dtwo = vld2_##S((const void *)sp); \
        dtwo = vld2_lane_##S((const void *)sp, dtwo, LD); sp += 2 * sizeof(T); SV(dtwo); \
        __auto_type three = vld3q_##S((const void *)sp); \
        three = vld3q_lane_##S((const void *)sp, three, LQ); sp += 3 * sizeof(T); SV(three); \
        __auto_type dthree = vld3_##S((const void *)sp); \
        dthree = vld3_lane_##S((const void *)sp, dthree, LD); sp += 3 * sizeof(T); SV(dthree); \
        __auto_type four = vld4q_##S((const void *)sp); \
        four = vld4q_lane_##S((const void *)sp, four, LQ); sp += 4 * sizeof(T); SV(four); \
        __auto_type dfour = vld4_##S((const void *)sp); \
        dfour = vld4_lane_##S((const void *)sp, dfour, LD); sp += 4 * sizeof(T); SV(dfour); \
        vst1q_lane_##S((void *)dp, a, LQ); dp += sizeof(T); \
        vst1_lane_##S((void *)dp, da, LD); dp += sizeof(T); \
        vst2q_lane_##S((void *)dp, two, LQ); dp += 2 * sizeof(T); \
        vst2_lane_##S((void *)dp, dtwo, LD); dp += 2 * sizeof(T); \
        vst3q_lane_##S((void *)dp, three, LQ); dp += 3 * sizeof(T); \
        vst3_lane_##S((void *)dp, dthree, LD); dp += 3 * sizeof(T); \
        vst4q_lane_##S((void *)dp, four, LQ); dp += 4 * sizeof(T); \
        vst4_lane_##S((void *)dp, dfour, LD); dp += 4 * sizeof(T); \
    } \
} while (0)

NOINLINE static void mem_u8(void)  { MEM_LOOP_A(u8, u8);  MEM_LOOP_B(u8, u8);  MEM_LOOP_C(u8, u8);  MEM_LOOP_D(u8, u8, 1, 3); }
NOINLINE static void mem_u16(void) { MEM_LOOP_A(u16, u16); MEM_LOOP_B(u16, u16); MEM_LOOP_C(u16, u16); MEM_LOOP_D(u16, u16, 1, 5); }
NOINLINE static void mem_u32(void) { MEM_LOOP_A(u32, u32); MEM_LOOP_B(u32, u32); MEM_LOOP_C(u32, u32); MEM_LOOP_D(u32, u32, 1, 3); }
NOINLINE static void mem_u64(void) { MEM_LOOP_A(u64, u64); MEM_LOOP_B(u64, u64); MEM_LOOP_C(u64, u64); MEM_LOOP_D(u64, u64, 0, 1); }
NOINLINE static void mem_f32(void) { MEM_LOOP_A(f32, f32); MEM_LOOP_B(f32, f32); MEM_LOOP_C(f32, f32); MEM_LOOP_D(f32, f32, 1, 3); }

void test_main(void) {
    fill_inputs();
    part_fp32();
    part_fp64();
    part_cvt();
    part_round();
    mem_u8();
    mem_u16();
    mem_u32();
    mem_u64();
    mem_f32();
    results[NRES - 1] = hacc;
}
