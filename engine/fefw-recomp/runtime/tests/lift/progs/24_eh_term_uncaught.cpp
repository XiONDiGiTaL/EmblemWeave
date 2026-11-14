// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
/* 24_eh_term_uncaught: six caught exceptions (the recorded prefix), then an exception that no handler catches. It leaves
 * test_main through a chain of four NOINLINE frames and must end in the terminate handler. The native build (NX_NATIVE) skips
 * the final call and returns normally. results[NRES-2] holds the prefix length, written before the terminating action.
 * The RAII destructors record into dtor_log, never into results: an uncaught exception may or may not unwind first. */
#include "rt2.h"

struct Base { int b; virtual ~Base() {} };
struct Derived : Base { int d; };
struct Pair { int a; int b; };
struct Fatal { int code; unsigned long long tag; };

static u64 nrec;
static NOINLINE void rec(u64 v) { results[nrec++] = v; }

static u64 dtor_log[16];
static unsigned dtor_n;
struct DtorMark {
    u64 v;
    ~DtorMark() { dtor_log[dtor_n++] = v; }
};

static NOINLINE void throw_int(int k) { throw k; }
static NOINLINE void throw_pair(int a, int b) { Pair p; p.a = a; p.b = b; throw p; }
static NOINLINE void throw_derived(int k) { Derived x; x.b = k; x.d = k * 2; throw x; }
static NOINLINE void throw_cstr(const char *s) { throw s; }
static NOINLINE void throw_long(long v) { throw v; }
static NOINLINE void throw_base_ptr(Base *p) { throw p; }

static NOINLINE void terminate_thrower(void) {
    DtorMark m{4};
    throw Fatal{77, 0xF47A1ull};
}
static NOINLINE void terminate_depth3(void) { DtorMark m{3}; terminate_thrower(); }
static NOINLINE void terminate_depth2(void) { DtorMark m{2}; terminate_depth3(); }
static NOINLINE void terminate_depth1(void) { DtorMark m{1}; terminate_depth2(); }

extern "C" void test_main(void) {
    try { throw_int(11); } catch (int v) { rec(1000 + v); }
    try { throw_pair(3, 4); } catch (Pair &p) { rec(2000 + p.a * 10 + p.b); }
    try { throw_derived(5); } catch (Base &b) { rec(3000 + b.b); }
    try { throw_cstr("hello"); } catch (const char *s) { rec(4000 + (unsigned char)s[0]); }
    try { throw_long(7); } catch (...) { rec(5000); }
    Derived dd; dd.b = 9; dd.d = 1;
    try { throw_base_ptr(&dd); } catch (Base *p) { rec(6000 + p->b); }

    results[NRES - 2] = nrec;   /* prefix length, read before the terminating action */
#ifndef NX_NATIVE
    terminate_depth1();
#endif
}
