// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
/* 25_eh_term_noexcept: four caught exceptions (the recorded prefix), then an int that leaves a noexcept frame. The chain is
 * noexcept_depth1 -> noexcept_depth2 -> noexcept_barrier (noexcept) -> noexcept_thrower. The language requires terminate there,
 * and the compiler emits its call-terminate landing pad in noexcept_barrier. The native build (NX_NATIVE) skips the final call and
 * returns normally. results[NRES-2] holds the prefix length, written before the terminating action. */
#include "rt2.h"

struct Base { int b; virtual ~Base() {} };
struct Derived : Base { int d; };

static u64 nrec;
static NOINLINE void rec(u64 v) { results[nrec++] = v; }

static NOINLINE void throw_int(int k) { throw k; }
static NOINLINE void throw_derived(int k) { Derived x; x.b = k; x.d = k * 2; throw x; }
static NOINLINE void throw_cstr(const char *s) { throw s; }
static NOINLINE void throw_long(long v) { throw v; }

static NOINLINE void noexcept_thrower(void) { throw 42; }
static NOINLINE void noexcept_barrier(void) noexcept { noexcept_thrower(); }
static NOINLINE void noexcept_depth2(void) { noexcept_barrier(); }
static NOINLINE void noexcept_depth1(void) { noexcept_depth2(); }

extern "C" void test_main(void) {
    try { throw_int(21); } catch (int v) { rec(100 + v); }
    try { throw_derived(5); } catch (Base &b) { rec(200 + b.b); }
    try { throw_cstr("xyz"); } catch (const char *s) { rec(300 + (unsigned char)s[1]); }
    try { throw_long(7); } catch (...) { rec(400); }

    results[NRES - 2] = nrec;   /* prefix length, read before the terminating action */
#ifndef NX_NATIVE
    noexcept_depth1();
#endif
}
