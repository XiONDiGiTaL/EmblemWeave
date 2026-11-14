// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <_Unwind_Resume>
/* 26_eh_threads: exceptions on several guest threads at once. thread_main(idx) is the work of one thread: 400 throw/catch round trips
 * through 3 to 7 frames with an RAII guard each, exception types E1 / E2 (derived from E1) / E3 / int, handlers by base class and
 * catch(...). Each thread works on its own stack variables and writes results[idx*4 .. idx*4+3]. The lifted run starts NTHREADS
 * runtime guest threads at thread_main (driver option --threads); the expected output is the same function run one after the other
 * on the host. test_main itself does nothing in the lifted run. */
#include "rt2.h"

#define NTHREADS 8

struct E1 { u64 a; };
struct E2 : E1 { u64 b; };
struct E3 { u32 c[3]; };

struct G {
    u64* acc;
    u64 id;
    G(u64* a, u64 i) : acc(a), id(i) { *acc = *acc * 31 + i; }
    ~G() { *acc = *acc * 37 + id + 100; }
};

static NOINLINE u64 level(u64* acc, u64 k, u64 depth) {
    G g(acc, depth);
    if (depth == 0) {
        switch (k % 4) {
        case 0: throw E1{k};
        case 1: { E2 e; e.a = k; e.b = k * 3; throw e; }
        case 2: { E3 e; e.c[0] = (u32)k; e.c[1] = (u32)(k >> 3); e.c[2] = 7; throw e; }
        default: throw (int)k;
        }
    }
    return level(acc, k, depth - 1) + depth;
}

static NOINLINE u64 attempt(u64* acc, u64 k) {
    try {
        return level(acc, k, 3 + k % 5);
    } catch (E1& e) {
        return 1000 + e.a;
    } catch (E3& e) {
        return 2000 + e.c[0] + e.c[2];
    } catch (...) {
        return 3000 + k;
    }
}

extern "C" void thread_main(u64 idx) {
    u64 acc = 0x1234 + idx, s = 99 + idx * 7, sum = 0;
    for (u64 i = 0; i < 400; i++) {
        u64 k = lcg(&s) % 1000;
        sum += attempt(&acc, k);
    }
    results[idx * 4 + 0] = sum;
    results[idx * 4 + 1] = acc;
    results[idx * 4 + 2] = idx + 1;
    results[idx * 4 + 3] = 0xabc00 + idx;
}

extern "C" void test_main(void) {
#ifdef NX_NATIVE
    for (u64 i = 0; i < NTHREADS; i++) thread_main(i);
#endif
}
