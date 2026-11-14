// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
#include "rt.h"

struct Guard {
    u64 *counter;
    int id;
    Guard(u64 *c, int i) : counter(c), id(i) { *counter = *counter * 31 + i; }
    ~Guard() { *counter = *counter * 37 + id + 100; }
};

static NOINLINE void thrower(int v) {
    if (v > 2) throw v * 7;
}
static NOINLINE int middle(int v, u64 *c) {
    Guard g(c, v);
    thrower(v);
    return v + 1;
}
extern "C" void test_main(void) {
    u64 c = 1;
    for (int v = 0; v < 6; v++) {
        try {
            results[v] = middle(v, &c);
        } catch (int e) {
            results[v] = 1000 + e;
        }
    }
    results[10] = c;
}
