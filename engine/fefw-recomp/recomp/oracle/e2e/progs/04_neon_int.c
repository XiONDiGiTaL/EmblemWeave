// FLAGS:
// EXPECT-INSN: \badd\s+v\d+\.\d+[bhsd]
// EXPECT-INSN: \bmul\s+v\d+
// EXPECT-INSN: \bmla\s+v\d+
// EXPECT-INSN: \bsmull\b|\bumull\b
// EXPECT-INSN: \babd\b|\buabd\b|\bsabd\b
// EXPECT-INSN: \bshl\s+v\d+
// EXPECT-INSN: \bsshr\b|\bushr\b
// EXPECT-INSN: \bsqadd\b|\buqadd\b
// EXPECT-INSN: \bsqshrn\b|\buqshrn\b|\bsqrshrn\b
// EXPECT-INSN: \bxtn\b
// EXPECT-INSN: \bcnt\s+v
// EXPECT-INSN: \bclz\s+v
// EXPECT-INSN: \bext\s+v
// EXPECT-INSN: \bzip1\b
// EXPECT-INSN: \buzp1\b
// EXPECT-INSN: \btrn1\b
// EXPECT-INSN: \btbl\s+v
// EXPECT-INSN: \bbsl\s+v|\bbit\s+v|\bbif\s+v
// EXPECT-INSN: \bcmeq\b|\bcmgt\b|\bcmhi\b
// EXPECT-INSN: \baddv\b|\bsmaxv\b|\bumaxv\b
// EXPECT-INSN: \brev64\b|\brev32\b|\brev16\b
// EXPECT-INSN: \bpmull\b
// EXPECT-INSN: \bsqdmulh\b|\bsqrdmulh\b
// EXPECT-INSN: \bsshll\b|\bushll\b|\bsxtl\b|\buxtl\b
#include "t.h"
#include <arm_neon.h>

/* Integer AdvSIMD coverage. Inputs come from inb[] (filled at run time from a volatile seed, so clang cannot fold).
 * Every result is sunk into results[] (first 511 values) and folded into hacc, which goes into results[NRES-1].
 * Each op's output is written back into inb[] (FQ) so later ops consume it: values chain through the whole program. */

static u64 ri, kk;
static u64 hacc = 0x9e3779b97f4a7c15ull;
static u8 inb[1024] __attribute__((aligned(16)));
static u64 edge[16] = {0, 1, ~0ull, 0x80ull, 0x7full, 0xffull, 0x8000ull, 0x7fffull, 0xffffull,
                       0x80000000ull, 0x7fffffffull, 0xffffffffull, 0x8000000000000000ull,
                       0x7fffffffffffffffull, 0x0123456789abcdefull, 0xfedcba9876543210ull};
static volatile u64 vseed = 0x2545F4914F6CDD1Dull;

#define QL(S, k) vld1q_##S((const void *)(inb + ((((u64)(k)) & 63) << 4)))
#define DL(S, k) vld1_##S((const void *)(inb + ((((u64)(k)) & 63) << 3)))
#define FQ(v, k) __builtin_memcpy(inb + ((((u64)(k)) & 63) << 4), &(v), sizeof(v))

NOINLINE static void sink(const void *p, u32 n) {
    u64 w[2] = {0, 0};
    if (n == 16) __builtin_memcpy(w, p, 16);   /* constant sizes only: no libc memcpy call */
    else __builtin_memcpy(w, p, 8);
    for (u32 i = 0; i < n / 8; i++) {
        if (ri < NRES - 1) results[ri++] = w[i];
        hacc = (hacc ^ w[i]) * 0x100000001b3ull + (hacc >> 29);
    }
}
#define SV(v) sink(&(v), sizeof(v))
#define SU(x) do { u64 t_ = (u64)(x); sink(&t_, 8); } while (0)

NOINLINE static void fill_inputs(void) {
    u64 s = vseed;
    for (int i = 0; i < 16; i++) __builtin_memcpy(inb + 8 * i, &edge[i], 8);
    for (int i = 128; i < 1024; i += 8) {
        u64 z = lcg(&s);
        __builtin_memcpy(inb + i, &z, 8);
    }
}

/* ---- op templates (d = 64-bit vector, q = 128-bit vector) ---- */

/* binary, same type */
#define MB2(op, S) do { u64 kb = kk; kk += 4; \
    __auto_type a = QL(S, kb), b = QL(S, kb + 1); __auto_type r = op##q_##S(a, b); SV(r); FQ(r, kb + 2); \
    __auto_type da = DL(S, kb), db = DL(S, kb + 1); __auto_type dr = op##_##S(da, db); SV(dr); FQ(dr, kb + 3); } while (0)

/* binary, shift-count vector of the signed type SS */
#define MB2S(op, S, SS) do { u64 kb = kk; kk += 4; \
    __auto_type a = QL(S, kb); __auto_type b = QL(SS, kb + 1); __auto_type r = op##q_##S(a, b); SV(r); FQ(r, kb + 2); \
    __auto_type da = DL(S, kb); __auto_type db = DL(SS, kb + 1); __auto_type dr = op##_##S(da, db); SV(dr); FQ(dr, kb + 3); } while (0)

/* ternary (accumulate forms) */
#define MT3(op, S) do { u64 kb = kk; kk += 5; \
    __auto_type a = QL(S, kb), b = QL(S, kb + 1), c = QL(S, kb + 2); __auto_type r = op##q_##S(a, b, c); SV(r); FQ(r, kb + 3); \
    __auto_type da = DL(S, kb), db = DL(S, kb + 1), dc = DL(S, kb + 2); __auto_type dr = op##_##S(da, db, dc); SV(dr); FQ(dr, kb + 4); } while (0)

/* unary */
#define MU1(op, S) do { u64 kb = kk; kk += 3; \
    __auto_type a = QL(S, kb); __auto_type r = op##q_##S(a); SV(r); FQ(r, kb + 1); \
    __auto_type da = DL(S, kb); __auto_type dr = op##_##S(da); SV(dr); FQ(dr, kb + 2); } while (0)

/* pairwise add-long and accumulate-long (vpaddl / vpadal) */
#define MPADAL(S) do { u64 kb = kk; kk += 4; \
    __auto_type a = QL(S, kb), b = QL(S, kb + 1); __auto_type w = vpaddlq_##S(a); w = vpadalq_##S(w, b); SV(w); \
    __auto_type da = DL(S, kb), db = DL(S, kb + 1); __auto_type dw = vpaddl_##S(da); dw = vpadal_##S(dw, db); SV(dw); FQ(dw, kb + 2); } while (0)

/* across-vector reductions */
#define MRED(S) do { u64 kb = kk; kk += 2; __auto_type a = QL(S, kb); __auto_type da = DL(S, kb); \
    SU(vaddvq_##S(a)); SU(vmaxvq_##S(a)); SU(vminvq_##S(a)); SU(vaddv_##S(da)); SU(vmaxv_##S(da)); SU(vminv_##S(da)); \
    SU(vaddlvq_##S(a)); SU(vaddlv_##S(da)); } while (0)

/* widening: d x d -> q, and q x q -> q (high halves), accumulate, absolute difference long, widen-add/sub, vmovl */
#define MWIDE(S) do { u64 kb = kk; kk += 6; \
    __auto_type a = DL(S, kb), b = DL(S, kb + 1); __auto_type r = vmull_##S(a, b); SV(r); FQ(r, kb + 2); \
    __auto_type q1 = QL(S, kb + 3), q2 = QL(S, kb + 4); __auto_type h = vmull_high_##S(q1, q2); SV(h); \
    __auto_type m = vmlal_##S(r, a, b); SV(m); __auto_type ms = vmlsl_##S(h, a, b); SV(ms); \
    __auto_type ab = vabdl_##S(a, b); SV(ab); __auto_type ad = vaddl_##S(a, b); SV(ad); \
    __auto_type sl = vsubl_##S(a, b); SV(sl); __auto_type aw = vaddw_##S(r, b); SV(aw); \
    __auto_type mv = vmovl_##S(a); SV(mv); FQ(mv, kb + 5); } while (0)

/* narrowing: q -> d (vmovn, vqmovn) */
#define MNAR(S) do { u64 kb = kk; kk += 4; \
    __auto_type q1 = QL(S, kb), q2 = QL(S, kb + 1); __auto_type n1 = vmovn_##S(q1); SV(n1); \
    __auto_type n2 = vqmovn_##S(q2); SV(n2); FQ(n2, kb + 2); } while (0)
#define MNARUN(S) do { u64 kb = kk; kk += 2; __auto_type q1 = QL(S, kb); \
    __auto_type n1 = vqmovun_##S(q1); SV(n1); FQ(n1, kb + 1); } while (0)

/* immediate shifts, all forms, q and d */
#define MSHI(S, N) do { u64 kb = kk; kk += 6; \
    __auto_type a = QL(S, kb), b = QL(S, kb + 1); \
    __auto_type r1 = vshlq_n_##S(a, N); SV(r1); __auto_type r2 = vshrq_n_##S(a, N); SV(r2); \
    __auto_type r3 = vsraq_n_##S(a, b, N); SV(r3); __auto_type r4 = vrshrq_n_##S(a, N); SV(r4); \
    __auto_type r5 = vrsraq_n_##S(a, b, N); SV(r5); __auto_type r6 = vsliq_n_##S(a, b, N); SV(r6); \
    __auto_type r7 = vsriq_n_##S(a, b, N); SV(r7); __auto_type r8 = vqshlq_n_##S(a, N); SV(r8); \
    __auto_type da = DL(S, kb), db = DL(S, kb + 1); \
    __auto_type d1 = vshl_n_##S(da, N); SV(d1); __auto_type d2 = vshr_n_##S(da, N); SV(d2); \
    __auto_type d3 = vsra_n_##S(da, db, N); SV(d3); __auto_type d4 = vrshr_n_##S(da, N); SV(d4); \
    __auto_type d5 = vrsra_n_##S(da, db, N); SV(d5); __auto_type d6 = vsli_n_##S(da, db, N); SV(d6); \
    __auto_type d7 = vsri_n_##S(da, db, N); SV(d7); __auto_type d8 = vqshl_n_##S(da, N); SV(d8); \
    FQ(r5, kb + 2); FQ(d7, kb + 3); } while (0)

/* narrowing immediate shifts: q -> d */
#define MSHN(S, N) do { u64 kb = kk; kk += 3; __auto_type q = QL(S, kb), q2 = QL(S, kb + 1); \
    __auto_type n1 = vshrn_n_##S(q, N); SV(n1); __auto_type n2 = vrshrn_n_##S(q2, N); SV(n2); \
    __auto_type n3 = vqshrn_n_##S(q, N); SV(n3); __auto_type n4 = vqrshrn_n_##S(q2, N); SV(n4); FQ(n4, kb + 2); } while (0)
#define MSHUN(S, N) do { u64 kb = kk; kk += 2; __auto_type q = QL(S, kb); \
    __auto_type n1 = vqshrun_n_##S(q, N); SV(n1); __auto_type n2 = vqrshrun_n_##S(q, N); SV(n2); FQ(n2, kb + 1); } while (0)

/* widening immediate shift: d -> q */
#define MSHLL(S, N) do { u64 kb = kk; kk += 2; __auto_type da = DL(S, kb); \
    __auto_type r = vshll_n_##S(da, N); SV(r); FQ(r, kb + 1); } while (0)

/* saturating doubling multiply long: d x d -> q, and high */
#define MDMULL(S) do { u64 kb = kk; kk += 4; __auto_type a = DL(S, kb), b = DL(S, kb + 1); \
    __auto_type r = vqdmull_##S(a, b); SV(r); __auto_type q1 = QL(S, kb + 2), q2 = QL(S, kb + 3); \
    __auto_type h = vqdmull_high_##S(q1, q2); SV(h); FQ(h, kb + 1); } while (0)

/* bit select: mask type MS, data type S */
#define MBSL(S, MS) do { u64 kb = kk; kk += 4; __auto_type m = QL(MS, kb); \
    __auto_type a = QL(S, kb + 1), b = QL(S, kb + 2); __auto_type r = vbslq_##S(m, a, b); SV(r); FQ(r, kb + 3); \
    __auto_type dm = DL(MS, kb); __auto_type da = DL(S, kb + 1), db = DL(S, kb + 2); \
    __auto_type dr = vbsl_##S(dm, da, db); SV(dr); } while (0)

/* lane / duplicate / get / set / create / combine / high-low / extract */
#define MLANE(S, LQL, LDL, E) do { u64 kb = kk; kk += 4; \
    __auto_type a = QL(S, kb), b = QL(S, kb + 1); __auto_type da = DL(S, kb), db = DL(S, kb + 1); \
    __auto_type r1 = vdupq_laneq_##S(a, LQL); SV(r1); __auto_type r2 = vdup_laneq_##S(a, LQL); SV(r2); \
    __auto_type r3 = vdupq_lane_##S(da, LDL); SV(r3); __auto_type r4 = vdup_lane_##S(da, LDL); SV(r4); \
    SU(vgetq_lane_##S(a, LQL)); SU(vget_lane_##S(da, LDL)); \
    __auto_type r5 = vsetq_lane_##S(vget_lane_##S(db, LDL), a, LQL); SV(r5); \
    __auto_type r6 = vset_lane_##S(vgetq_lane_##S(b, LQL), da, LDL); SV(r6); \
    __auto_type r7 = vdupq_n_##S(vgetq_lane_##S(b, LQL)); SV(r7); __auto_type r8 = vdup_n_##S(vget_lane_##S(db, LDL)); SV(r8); \
    __auto_type r9 = vcombine_##S(da, db); SV(r9); \
    __auto_type rh = vget_high_##S(a); SV(rh); __auto_type rl = vget_low_##S(a); SV(rl); \
    __auto_type rx = vext_##S(da, db, E); SV(rx); __auto_type rqx = vextq_##S(a, b, E); SV(rqx); \
    __auto_type rc = vcreate_##S(edge[kb & 15]); SV(rc); FQ(r9, kb + 2); } while (0)

/* groups */
#define GRP_A(S) \
    MB2(vadd, S); MB2(vsub, S); MB2(vabd, S); MB2(vmax, S); MB2(vmin, S); MB2(vqadd, S); MB2(vqsub, S); \
    MB2(vhadd, S); MB2(vrhadd, S); MB2(vhsub, S); MB2(vtst, S); MB2(vceq, S); MB2(vcgt, S); MB2(vcge, S); \
    MB2(vclt, S); MB2(vcle, S); MB2(vpadd, S); MB2(vpmax, S); MB2(vpmin, S); MB2(vmul, S); \
    MB2(vzip1, S); MB2(vzip2, S); MB2(vuzp1, S); MB2(vuzp2, S); MB2(vtrn1, S); MB2(vtrn2, S); \
    MB2(vand, S); MB2(vorr, S); MB2(veor, S); MB2(vbic, S); MB2(vorn, S); \
    MT3(vmla, S); MT3(vmls, S); MT3(vaba, S); \
    MU1(vmvn, S); MU1(vclz, S); MU1(vceqz, S); MPADAL(S); MRED(S); MWIDE(S)
#define GRP_SG(S) \
    MU1(vabs, S); MU1(vneg, S); MU1(vqabs, S); MU1(vqneg, S); \
    MU1(vcltz, S); MU1(vcgez, S); MU1(vcgtz, S); MU1(vclez, S);
#define GRP_SH(S, SS) \
    MB2S(vshl, S, SS); MB2S(vrshl, S, SS); MB2S(vqshl, S, SS); MB2S(vqrshl, S, SS);
#define GRP64(S) \
    MB2(vadd, S); MB2(vsub, S); MB2(vqadd, S); MB2(vqsub, S); MB2(vtst, S); MB2(vceq, S); MB2(vcgt, S); \
    MB2(vcge, S); MB2(vclt, S); MB2(vcle, S); MB2(vand, S); MB2(vorr, S); MB2(veor, S); MB2(vbic, S); MB2(vorn, S); \
    MB2S(vshl, S, s64); MB2S(vrshl, S, s64); MB2S(vqshl, S, s64); MB2S(vqrshl, S, s64); \
    MU1(vceqz, S); MRED64(S);

#define MRED64(S) do { u64 kb = kk; kk += 2; __auto_type a = QL(S, kb); SU(vaddvq_##S(a)); \
    } while (0)

/* ---- parts (NOINLINE so the body is compiled as its own function) ---- */

NOINLINE static void part_8(void) {
    GRP_A(s8); GRP_A(u8);
    GRP_SG(s8);
    GRP_SH(s8, s8); GRP_SH(u8, s8);
    MU1(vcls, s8); MU1(vcnt, u8); MU1(vcnt, s8); MU1(vrbit, u8); MU1(vrbit, s8);
    MU1(vrev16, u8); MU1(vrev16, s8); MU1(vrev32, u8); MU1(vrev32, s8); MU1(vrev64, u8); MU1(vrev64, s8);
    MLANE(s8, 3, 1, 3); MLANE(u8, 3, 1, 3);
    MSHI(s8, 3); MSHI(u8, 3);
    MSHLL(s8, 3); MSHLL(u8, 3);
    MBSL(s8, u8); MBSL(u8, u8);
}

NOINLINE static void part_16(void) {
    GRP_A(s16); GRP_A(u16);
    GRP_SG(s16);
    GRP_SH(s16, s16); GRP_SH(u16, s16);
    MU1(vcls, s16); MU1(vrev32, s16); MU1(vrev32, u16); MU1(vrev64, s16); MU1(vrev64, u16);
    MLANE(s16, 3, 1, 1); MLANE(u16, 3, 1, 1);
    MSHI(s16, 5); MSHI(u16, 5);
    MSHLL(s16, 5); MSHLL(u16, 5);
    MB2(vqdmulh, s16); MB2(vqrdmulh, s16);
    MDMULL(s16);
    MBSL(s16, u16); MBSL(u16, u16);
}

NOINLINE static void part_32(void) {
    GRP_A(s32); GRP_A(u32);
    GRP_SG(s32);
    GRP_SH(s32, s32); GRP_SH(u32, s32);
    MU1(vcls, s32); MU1(vrev64, s32); MU1(vrev64, u32);
    MLANE(s32, 3, 1, 1); MLANE(u32, 3, 1, 1);
    MSHI(s32, 9); MSHI(u32, 9);
    MSHLL(s32, 9); MSHLL(u32, 9);
    MB2(vqdmulh, s32); MB2(vqrdmulh, s32);
    MDMULL(s32);
    MBSL(s32, u32); MBSL(u32, u32);
}

NOINLINE static void part_64(void) {
    GRP64(s64); GRP64(u64);
    GRP_SG(s64);
    MSHI(s64, 17); MSHI(u64, 17);
    MLANE(s64, 1, 0, 0); MLANE(u64, 1, 0, 0);
    MBSL(s64, u64); MBSL(u64, u64);
    {   /* pairwise add of q-registers, 64-bit */
        u64 kb = kk; kk += 2;
        __auto_type a = QL(s64, kb), b = QL(s64, kb + 1);
        __auto_type pq = vpaddq_s64(a, b); SV(pq); FQ(pq, kb + 2);
        __auto_type pu = vpaddq_u64(a, b); SV(pu);
    }
}

NOINLINE static void part_narrow(void) {
    MNAR(s16); MNAR(u16); MNAR(s32); MNAR(u32); MNAR(s64); MNAR(u64);
    MNARUN(s16); MNARUN(s32); MNARUN(s64);
    MSHN(s16, 5); MSHN(u16, 5); MSHN(s32, 9); MSHN(u32, 9); MSHN(s64, 17); MSHN(u64, 17);
    MSHUN(s16, 5); MSHUN(s32, 9); MSHUN(s64, 17);
}

/* tables, 8-bit: d and q, 1..4 registers, plus the tbx (keep-fallback) forms */
NOINLINE static void part_tbl(void) {
    u64 kb = kk; kk += 16;
    uint8x8x2_t t2; t2.val[0] = DL(u8, kb); t2.val[1] = DL(u8, kb + 1);
    uint8x8x3_t t3; t3.val[0] = DL(u8, kb + 2); t3.val[1] = DL(u8, kb + 3); t3.val[2] = DL(u8, kb + 4);
    uint8x8x4_t t4; t4.val[0] = DL(u8, kb + 5); t4.val[1] = DL(u8, kb + 6); t4.val[2] = DL(u8, kb + 7); t4.val[3] = DL(u8, kb + 8);
    uint8x8_t ix = DL(u8, kb + 9), fb = DL(u8, kb + 10);
    uint8x16_t qa = QL(u8, kb + 11), qb = QL(u8, kb + 12), qi = QL(u8, kb + 14), qf = QL(u8, kb + 15);
    uint8x16x2_t q2; q2.val[0] = qa; q2.val[1] = qb;
    uint8x16x3_t q3; q3.val[0] = qa; q3.val[1] = qb; q3.val[2] = QL(u8, kb + 13);
    uint8x16x4_t q4; q4.val[0] = qa; q4.val[1] = qb; q4.val[2] = qi; q4.val[3] = qf;

    __auto_type r1 = vtbl1_u8(fb, ix); SV(r1);
    __auto_type r2 = vtbl2_u8(t2, ix); SV(r2);
    __auto_type r3 = vtbl3_u8(t3, ix); SV(r3);
    __auto_type r4 = vtbl4_u8(t4, ix); SV(r4);
    __auto_type x1 = vtbx1_u8(fb, t2.val[0], ix); SV(x1);
    __auto_type x2 = vtbx2_u8(fb, t2, ix); SV(x2);
    __auto_type x3 = vtbx3_u8(fb, t3, ix); SV(x3);
    __auto_type x4 = vtbx4_u8(fb, t4, ix); SV(x4);
    __auto_type y1 = vqtbl1q_u8(qa, qi); SV(y1);
    __auto_type y2 = vqtbl2q_u8(q2, qi); SV(y2);
    __auto_type y3 = vqtbl3q_u8(q3, qi); SV(y3);
    __auto_type y4 = vqtbl4q_u8(q4, qi); SV(y4);
    __auto_type z1 = vqtbx1q_u8(qf, qa, qi); SV(z1);
    __auto_type z2 = vqtbx2q_u8(qf, q2, qi); SV(z2);
    __auto_type z3 = vqtbx3q_u8(qf, q3, qi); SV(z3);
    __auto_type z4 = vqtbx4q_u8(qf, q4, qi); SV(z4);
    __auto_type w1 = vqtbl1_u8(qa, ix); SV(w1);
    FQ(y4, kb + 1); FQ(x4, kb + 2);
}

/* polynomial (carry-less) 8-bit multiply, d and q-high, plus vdup/vget of poly */
NOINLINE static void part_poly(void) {
    u64 kb = kk; kk += 4;
    __auto_type pa = DL(p8, kb), pb = DL(p8, kb + 1);
    __auto_type pr = vmull_p8(pa, pb); SV(pr);
    __auto_type qa = QL(p8, kb + 2), qb = QL(p8, kb + 3);
    __auto_type ph = vmull_high_p8(qa, qb); SV(ph);
    FQ(ph, kb + 1);
}

NOINLINE static void part_misc(void) {
    /* a few hand-picked edges through every width's max/min, to hit saturation */
    u64 kb = kk; kk += 4;
    __auto_type e8 = vqaddq_s8(QL(s8, kb), QL(s8, kb + 1)); SV(e8);
    __auto_type e16 = vqsubq_s16(QL(s16, kb), QL(s16, kb + 1)); SV(e16);
    __auto_type e32 = vqdmulhq_s32(QL(s32, kb), QL(s32, kb + 1)); SV(e32);
    __auto_type e64 = vqshlq_s64(QL(s64, kb), QL(s64, kb + 1)); SV(e64);
    FQ(e64, kb + 2);
}

void test_main(void) {
    fill_inputs();
    part_8();
    part_16();
    part_32();
    part_64();
    part_narrow();
    part_tbl();
    part_poly();
    part_misc();
    results[NRES - 1] = hacc;
}
