// FLAGS: -ftls-model=local-exec
// EXPECT-INSN: mrs\s+x\d+,\s*TPIDR_EL0
// EXPECT-INSN: mrs\s+x\d+,\s*TPIDRRO_EL0
// EXPECT-INSN: msr\s+TPIDR_EL0
// EXPECT-INSN: mrs\s+x\d+,\s*FPCR
/* Thread-local storage as the console code uses it: __thread variables of several sizes/alignments, local-exec model,
 * TPIDR_EL0 / TPIDRRO_EL0 / FPCR / FPSR system registers. TLS blocks are NOT initialised by the harness, so every
 * __thread variable is written before it is read. */
#include "t.h"

typedef struct { u32 a; u16 b; u8 c[5]; u64 d; } tstruct;

__thread int ti;
__thread u64 tq;
__thread u8 tbytes[24];
__thread u16 thw[5];
__thread tstruct ts;
__thread u32 tal[4] __attribute__((aligned(64)));
__thread u8 tb;

/* 4 KiB buffer that becomes the thread pointer for the middle section */
static u64 tbuf[512] __attribute__((aligned(4096)));

static volatile u64 g_seed = 0x2468ace013579bdfull;
static u64 ri, hsh;

static inline void put(u64 v) {
    hsh = (hsh ^ v) * 0x100000001b3ull + 0x9e37ull;
    if (ri < 400) results[ri++] = v;
}

NOINLINE static u64 tls_fill(u64 seed) {
    u64 s = seed;
    ti = (int)lcg(&s);
    tq = lcg(&s);
    for (int i = 0; i < 24; i++) tbytes[i] = (u8)lcg(&s);
    for (int i = 0; i < 5; i++) thw[i] = (u16)lcg(&s);
    ts.a = (u32)lcg(&s);
    ts.b = (u16)lcg(&s);
    for (int i = 0; i < 5; i++) ts.c[i] = (u8)lcg(&s);
    ts.d = lcg(&s);
    for (int i = 0; i < 4; i++) tal[i] = (u32)lcg(&s);
    tb = (u8)lcg(&s);
    return tq ^ ts.d;
}

NOINLINE static void tls_dump(void) {
    put((u64)(s64)ti);
    put(tq);
    for (int i = 0; i < 24; i++) put(tbytes[i]);
    for (int i = 0; i < 5; i++) put(thw[i]);
    put(ts.a);
    put(ts.b);
    for (int i = 0; i < 5; i++) put(ts.c[i]);
    put(ts.d);
    for (int i = 0; i < 4; i++) put(tal[i]);
    put(tb);
}

/* pointer arithmetic from the thread pointer, loads and stores through it */
NOINLINE static void tp_access(void) {
    u8 *tp = (u8 *)__builtin_thread_pointer();
    u64 base = (u64)tp;
    u64 off_q = (u64)&tq - base;
    u64 off_ts = (u64)&ts - base;
    u64 off_tal = (u64)&tal[0] - base;
    u64 off_tb = (u64)&tb - base;
    put(off_q); put(off_ts); put(off_tal); put(off_tb);
    put(*(u64 *)(tp + off_q));
    put(*(u32 *)(tp + off_ts));
    put(*(u8 *)(tp + off_ts + offsetof(tstruct, c)));
    *(u64 *)(tp + off_q) = *(u64 *)(tp + off_q) * 3 + 1;
    *(u32 *)(tp + off_tal + 8) = 0xdeadbeefu;
    *(u8 *)(tp + off_tb) ^= 0x5au;
    put(tq); put(tal[2]); put(tb);
}

NOINLINE static u64 tp_now(void) { return (u64)__builtin_thread_pointer(); }
NOINLINE static u64 rsr_tp(void) { return __builtin_arm_rsr64("tpidr_el0"); }
NOINLINE static void wsr_tp(u64 v) { __builtin_arm_wsr64("tpidr_el0", v); }

void test_main(void) {
    u64 s = g_seed;

    /* original thread pointer (set by the harness) */
    u64 orig = rsr_tp();
    put(tp_now() == orig ? 1u : 0u);

    /* section 1: TLS in the harness block */
    put(tls_fill(s));
    tls_dump();
    tp_access();
    tls_dump();

    /* read-only TPIDRRO_EL0 and a few words at [tpidrro + k] */
    u64 ro = __builtin_arm_rsr64("tpidrro_el0");
    put(ro);
    for (int k = 0; k < 8; k++) put(((u64 *)ro)[k]);
    put(((u64 *)ro)[64]);
    put(((u64 *)ro)[127]);

    /* FP control and status registers */
    put(__builtin_arm_rsr64("fpcr"));
    put(__builtin_arm_rsr64("fpsr"));

    /* section 2: point TPIDR_EL0 at tbuf; the TLS variables now live in tbuf */
    wsr_tp((u64)tbuf);
    put(tp_now() - (u64)tbuf);
    put(tls_fill(s ^ 0x77));
    tls_dump();
    tp_access();
    tls_dump();
    for (int k = 0; k < 16; k++) put(tbuf[k]);

    /* section 3: restore the original thread pointer and use the TLS block again */
    wsr_tp(orig);
    put(rsr_tp() == orig ? 1u : 0u);
    put(tls_fill(s ^ 0x99));
    tls_dump();
    tp_access();

    results[ri++] = hsh;
}
