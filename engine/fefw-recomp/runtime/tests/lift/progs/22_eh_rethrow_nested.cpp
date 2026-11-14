// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_rethrow>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <_Unwind_Resume>
/* 22_eh_rethrow_nested: throw; (rethrow), exceptions thrown while another is in flight, a try nested in a handler, a new
 * exception thrown from a handler, a throw caught inside a destructor during unwinding, a rethrow stress loop and a
 * function-try-block on a constructor.
 *
 * What the standard predicts per case (C++17 [except.throw], [except.handle], [class.dtor]); the expectations were written
 * before the native run and are checked against it:
 *  1  throw; rethrows the SAME exception object: the inner handler's change to e.hits is seen by the outer handler and the
 *     address is identical. throw; makes no copy. Expected 1510 = code 5 * 100 + hits 1 * 10 + copies 0 + same-object 1000.
 *  2  throw; from catch(...) keeps the dynamic type, so the outer catch(ErrA&) matches. Events: the catch(...) handler, then
 *     the exception object is destroyed when the outer handler exits. Expected 7003; events 0x2a, 0x703.
 *  3  Rethrow through three frames. Each frame's guard is built on entry and destroyed when the frame is left. A frame's
 *     handler is inside that frame, so it runs before the guard is destroyed. Order: ctor 1, 2, 3; handler 3; ~guard 3;
 *     handler 2; ~guard 2; handler 1; ~guard 1; outer handler; exception object destroyed at the end of the outer handler.
 *     Expected 3 and 11 events.
 *  4  catch (ErrA e) { throw e; } throws a NEW exception copy-constructed from the handler parameter (a named object: no
 *     elision), so exactly one copy-constructor call. throw; makes no copy at all. Expected 31 (code 3 * 10 + 1 copy) and 30.
 *  5  A try/catch nested in a handler: the inner int is handled inside the outer handler; the outer object stays alive and
 *     usable; throw; in that nested scope rethrows the OUTER object (same address). It is destroyed once, when its handler
 *     exits. Expected 51091 (inner 104 + 5005 = 5109, * 10 + same 1) and 1 destruction.
 *  6  An exception thrown from a handler (not a rethrow) leaves that handler by unwinding, so the original exception object
 *     is destroyed BEFORE the outer handler starts. Expected 6; events 0x602 (inner handler), 0x702 (destroyed), 0x900.
 *  7  An int and an ErrC are thrown and caught inside two destructors that run while ErrA unwinds. Neither escapes its
 *     destructor, so the ErrA reaches its handler unchanged. Destructors run in reverse construction order. Expected 102;
 *     events 0xc0a (~Scrub2, tag 3, v+w=3+7), 0xa02 (~Scrub1, tag 2), 0xb02 (outer handler), 0x702 (ErrA destroyed).
 *  8  100 iterations of throw / catch / throw; / catch over three exception classes. Each result is compared with a value
 *     predicted without exceptions (mismatch count must be 0); the ErrA destructor runs once per ErrA iteration (i % 3 == 0,
 *     34 times). Expected mismatches 0, destructions 34.
 *  9  Function-try-block on a constructor: a member constructor throws; the fully constructed member is destroyed before the
 *     handler runs; the handler's exception is rethrown automatically and caught outside. For k = 2 nothing throws: the
 *     body runs, then members are destroyed in reverse order. Expected 503 with 3 events; 2 with 5 events.
 */
#include "rt2.h"

static u64 acc;          /* order-sensitive hash of every recorded value */
static u64 nrec;

static void mix(u64 v) { acc = (acc ^ v) * 0x100000001b3ull; acc ^= acc >> 29; }
static void rec(u64 v) { mix(v); if (nrec < NRES - 2) results[nrec++] = v; }

/* ordered event log: destructors and handlers note themselves here; each case records its own sequence */
static u64 g_ev[32];
static u64 g_nev;
static void ev(u64 t) { if (g_nev < 32) g_ev[g_nev++] = t; }
static void evreset(void) { g_nev = 0; }
static void rec_ev(void) { rec(g_nev); for (u64 i = 0; i < g_nev; i++) rec(g_ev[i]); }

static u64 g_dtor;       /* ErrA destructors run */
static u64 g_copy;       /* ErrA copy constructors run */
static void *g_addr;     /* address of the exception object seen by the inner handler of case 1 */

struct ErrA {            /* data members plus observable copy and destruction */
    int code;
    int hits;
    ErrA(int c) : code(c), hits(0) {}
    ErrA(const ErrA &o) : code(o.code), hits(o.hits) { g_copy++; }
    ~ErrA() { g_dtor++; ev(0x700 + (u64)code); }
};
struct ErrC { u64 v; u64 w; };
struct ErrD { u64 a; u64 b; };
struct ErrF { double x; int n; };

static NOINLINE void raise_a(int k) { throw ErrA(k); }

/* case 1: same object through a rethrow */
static NOINLINE void c1_inner(int k) {
    try { raise_a(k); }
    catch (ErrA &e) { e.hits += 1; g_addr = (void *)&e; g_copy = 0; throw; }
}
static NOINLINE u64 c1(int k) {
    try { c1_inner(k); return 0; }
    catch (ErrA &e) {
        return (u64)e.code * 100 + (u64)e.hits * 10 + g_copy + (((void *)&e == g_addr) ? 1000 : 0);
    }
}

/* case 2: rethrow from catch(...) still reaches the typed outer handler */
static NOINLINE void c2_mid(int k) {
    try { raise_a(k); }
    catch (...) { ev(0x2a); throw; }
}
static NOINLINE u64 c2(int k) {
    try { c2_mid(k); return 0; }
    catch (ErrA &e) { return 7000 + (u64)e.code; }
    catch (...) { return 9999; }
}

/* case 3: three frames, each with a handler that rethrows and a RAII guard */
struct Guard {
    int id;
    Guard(int i) : id(i) { ev(0x100 + (u64)i); }
    ~Guard() { ev(0x200 + (u64)id); }
};
static NOINLINE void c3_l3(int k) {
    Guard g(3);
    try { raise_a(k); }
    catch (ErrA &e) { ev(0x303); e.hits++; throw; }
}
static NOINLINE void c3_l2(int k) {
    Guard g(2);
    try { c3_l3(k); }
    catch (ErrA &e) { ev(0x302); e.hits++; throw; }
}
static NOINLINE void c3_l1(int k) {
    Guard g(1);
    try { c3_l2(k); }
    catch (ErrA &e) { ev(0x301); e.hits++; throw; }
}
static NOINLINE u64 c3(int k) {
    try { c3_l1(k); return 0; }
    catch (ErrA &e) { ev(0x400); return (u64)e.hits; }
}

/* case 4: catch by value and throw e (a new exception, one copy) versus throw; (none) */
static NOINLINE void c4_raise(int k) { throw ErrA(k); }
static NOINLINE u64 c4_copy_throw(int k) {
    try {
        try { c4_raise(k); return 0; }
        catch (ErrA e) { g_copy = 0; throw e; }
    } catch (ErrA &e) { return (u64)e.code * 10 + g_copy; }
}
static NOINLINE u64 c4_rethrow(int k) {
    try {
        try { c4_raise(k); return 0; }
        catch (ErrA &e) { g_copy = 0; throw; }
    } catch (ErrA &e) { return (u64)e.code * 10 + g_copy; }
}

/* case 5: a try inside a handler; the outer exception survives and throw; in the nested scope rethrows it */
static NOINLINE u64 c5(int k) {
    try { raise_a(k); return 0; }
    catch (ErrA &outer) {
        u64 r = 0;
        try { throw (int)(k + 100); }
        catch (int v) { r += (u64)v; }
        outer.hits += 5;
        u64 same = 0;
        try { throw; }
        catch (int) { r += 1000000; }
        catch (ErrA &again) { same = ((void *)&again == (void *)&outer) ? 1 : 0; r += 5000 + (u64)again.hits; }
        return r * 10 + same;
    }
}

/* case 6: a new exception leaves the handler of the first one; the first is destroyed before the outer handler runs */
static NOINLINE u64 c6(int k) {
    try {
        try { raise_a(k); return 0; }
        catch (ErrA &e) { ev(0x600 + (u64)e.code); throw (int)(e.code * 3); }
    } catch (int v) { ev(0x900); return (u64)v; }
}

/* case 7: exceptions thrown and caught inside destructors that run while ErrA unwinds */
struct Scrub1 {
    int tag;
    Scrub1(int t) : tag(t) {}
    NOINLINE ~Scrub1() { try { throw tag; } catch (int v) { ev(0xa00 + (u64)v); } }
};
struct Scrub2 {
    u64 tag;
    Scrub2(u64 t) : tag(t) {}
    NOINLINE ~Scrub2() { try { throw ErrC{tag, 7}; } catch (ErrC &c) { ev(0xc00 + c.v + c.w); } }
};
static NOINLINE u64 c7(int k) {
    try {
        Scrub1 s1(k);
        Scrub2 s2((u64)k + 1);
        raise_a(k);
        return 0;
    } catch (ErrA &e) { ev(0xb00 + (u64)e.code); return 100 + (u64)e.code; }
}

/* case 8: rethrow stress over three exception classes */
static NOINLINE void raise_kind(u64 i) {
    switch (i % 3) {
    case 0: throw ErrA((int)i);
    case 1: throw ErrD{i, i * 3};
    default: throw ErrF{(double)i * 0.5, (int)i};
    }
}
static NOINLINE void pass_once(u64 i) {
    try { raise_kind(i); }
    catch (ErrA &e) { e.hits++; throw; }
    catch (ErrD &d) { d.a++; throw; }
    catch (ErrF &f) { f.n++; throw; }
}
static NOINLINE u64 c8_one(u64 i) {
    try { pass_once(i); return 0; }
    catch (ErrA &e) { return 1000 + (u64)e.code + (u64)e.hits * 7; }
    catch (ErrD &d) { return 2000 + d.a * 3 + d.b; }
    catch (ErrF &f) { return 3000 + (u64)(s64)(f.x * 4.0) + (u64)f.n; }
}
static u64 pred_one(u64 i) {                    /* the same value computed without any exception */
    switch (i % 3) {
    case 0: return 1000 + i + 7;                /* code i, hits 1 */
    case 1: return 2000 + 3 * (i + 1) + 3 * i;  /* a = i + 1, b = 3 i */
    default: return 3000 + 2 * i + (i + 1);     /* x * 4 = 2 i, n = i + 1 */
    }
}

/* case 9: function-try-block on a constructor */
struct Mem {
    int id;
    Mem(int i) : id(i) { ev(0x1000 + (u64)i); }
    ~Mem() { ev(0x2000 + (u64)id); }
};
struct MemBad {
    int id;
    NOINLINE MemBad(int i) : id(i) { if (i & 1) throw i; ev(0x1100 + (u64)i); }
    ~MemBad() { ev(0x2100 + (u64)id); }
};
struct FtbObj {
    Mem a;
    MemBad b;
    int r;
    NOINLINE FtbObj(int k) try : a(k), b(k), r(k) { ev(0x3000); }
    catch (int v) { ev(0x4000 + (u64)v); }   /* rethrown automatically at the end of this handler */
};
static NOINLINE u64 c9(int k) {
    try { FtbObj o(k); return (u64)o.r; }
    catch (int v) { return 500 + (u64)v; }
}

extern "C" void test_main(void) {
    acc = 0x9e3779b97f4a7c15ull;
    nrec = 0;
    g_nev = 0;
    g_dtor = 0;
    g_copy = 0;
    g_addr = 0;
    try {
        rec(c1(5));                                              /* 1510 */
        evreset(); rec(c2(3)); rec_ev();                         /* 7003; 0x2a 0x703 */
        evreset(); rec(c3(3)); rec_ev();                         /* 3; 11 events */
        rec(c4_copy_throw(3));                                   /* 31 */
        rec(c4_rethrow(3));                                      /* 30 */
        { u64 d0 = g_dtor; rec(c5(4)); rec(g_dtor - d0); }       /* 51091; 1 */
        evreset(); rec(c6(2)); rec_ev();                         /* 6; 0x602 0x702 0x900 */
        evreset(); rec(c7(2)); rec_ev();                         /* 102; 0xc0a 0xa02 0xb02 0x702 */
        { u64 sum = 0, mism = 0, d0 = g_dtor;
          for (u64 i = 0; i < 100; i++) {
              u64 got = c8_one(i);
              sum = sum * 31 + got;
              if (got != pred_one(i)) mism++;
          }
          rec(sum); rec(mism); rec(g_dtor - d0); }              /* sum; 0; 34 */
        evreset(); rec(c9(3)); rec_ev();                         /* 503; 0x1003 0x2003 0x4003 */
        evreset(); rec(c9(2)); rec_ev();                         /* 2; 0x1002 0x1102 0x3000 0x2102 0x2002 */
    } catch (...) {
        rec(0xdeadull);
    }
    results[NRES - 1] = acc;
}
