// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
/* 19_eh_probe: smallest program of the rt2.h family: one class hierarchy, a derived-to-base catch, a pointer catch, catch(...). */
#include "rt2.h"

struct Base { int b; virtual ~Base() {} };
struct Derived : Base { int d; };

static NOINLINE void throw_derived(int k) { Derived x; x.b = k; x.d = k * 2; throw x; }
static NOINLINE void throw_ptr(Base *p) { throw p; }

extern "C" void test_main(void) {
    int n = 0;
    try { throw_derived(5); } catch (Base &b) { results[n++] = 100 + b.b; }
    Derived dd; dd.b = 9; dd.d = 1;
    try { throw_ptr(&dd); } catch (Base *p) { results[n++] = 200 + p->b; }
    try { throw 7L; } catch (...) { results[n++] = 300; }
    results[n++] = 4242;
    results[NRES - 1] = n;
}
