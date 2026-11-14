// FLAGS:
// EXPECT-INSN: \badd\b
// EXPECT-INSN: \bmadd\b|\bmul\b
#include "t.h"
void test_main(void) {
    u64 s = 1;
    for (int i = 0; i < 16; i++) results[i] = lcg(&s) * 3 + i;
}
