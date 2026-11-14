// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <_Unwind_Resume>
/* 20_eh_hier: C++ exceptions across class hierarchies. Every case throws with k = 7 and records what its handler saw.
 * pk(h, x, y, z) packs the handler id h (bits 48..63) and three 16-bit fields x, y, z. What the C++ standard requires:
 *  1 Chain A <- B <- C (A has a virtual destructor, so the chain is polymorphic). Thrown C, handlers listed C&, B&, A&:
 *    C& runs, the first matching handler (pk 3: a=7 b=107 c=207). Thrown B: B& (pk 2). Thrown A: A& (pk 1).
 *    Thrown C caught by B& (pk 21: a=7 b=107) and by const A& (pk 31: a=7): the reference sees the base subobject.
 *  2 Handler order: handlers listed A&, B&, C&, thrown C: A& is the first handler whose type is a base of C, so A& runs
 *    (pk 11: a=7). The same happens for thrown B and thrown A. C& is never reached.
 *  3 Multiple inheritance. M : P, Q (no virtuals): Q is a non-primary base at offset 8. Thrown M, caught by Q& (pk 41: q=17),
 *    by P& (pk 51: p=7), by M& (pk 61: p=7 q=17 m=27). Polymorphic MP : PP, QP with pt() overridden to 11 and qt() to 22.
 *    Thrown MP: Q& does not match, QP& runs (pk 42: q=17, qt() through the QP subobject = 22). PP& runs (pk 52: p=7,
 *    pt()=11). MP& runs (pk 62: 7, 17, 27).
 *  4 Virtual diamond D : L, R, both with virtual base V. Thrown D: V& (pk 71: v=7), L& (pk 72: v=7 l=8), R& (pk 73: v=7
 *    r=9), D& (pk 74: d=10, v=7, l*100+r=809). All four match; V is one shared subobject.
 *  5 Ambiguous base. DD : B1, B2, each derived from A0 without virtual. A0& does NOT match DD (A0 is an ambiguous base), so
 *    B1& runs (pk 82: a=7). With only A0& and catch(...) listed, catch(...) runs (pk 92).
 *  6 Access. Priv : private Pub, Prot : protected Pub, Pubd : public Pub. Only a public unambiguous base matches, so
 *    catch(Pub&) matches only Pubd (pk 101: x=7). Priv and Prot fall through to catch(...) (pk 102, kind 0 and 1).
 *  7 Value. throw Val(7) is a prvalue, so the exception object is built in place (no copy). catch(Val) copies once (pk 111:
 *    v=7, copies=1). catch(Val&) copies nothing (pk 112: copies=0). The global copy counter ends at 1.
 *  8 Unrelated. Ub has no relation to Ua. The inner catch(Ua&) does not match, the exception leaves the inner try and the
 *    outer catch(Ub&) runs (pk 123: w=7).
 * The expected words are also the values of results[0..27] (27 handler results, then nrec); results[511] is the hash.
 * All throws happen in throw_any, and every catching function calls it through call_thrower (a plain indirection).
 */
#include "rt2.h"

static u64 acc;          /* order-sensitive hash of every recorded value */
static u64 nrec;
static u64 copies;       /* bumped by Val's copy constructor */

static void mix(u64 v) { acc = (acc ^ v) * 0x100000001b3ull; acc ^= acc >> 29; }
static void rec(u64 v) { mix(v); if (nrec < NRES - 2) results[nrec++] = v; }
static u64 pk(u64 h, u64 x, u64 y, u64 z) { return (h << 48) | ((x & 0xffff) << 32) | ((y & 0xffff) << 16) | (z & 0xffff); }

/* ---- class hierarchies --------------------------------------------------------------------------------------------------- */
struct A { u64 a; virtual ~A() {} };                    /* 1, 2: single chain */
struct B : A { u64 b; };
struct C : B { u64 c; };

struct P { u64 p; };                                    /* 3: multiple inheritance, plain */
struct Q { u64 q; };
struct M : P, Q { u64 m; };
struct PP { u64 p; virtual u64 pt() { return 1; } };    /* 3: and polymorphic */
struct QP { u64 q; virtual u64 qt() { return 2; } };
struct MP : PP, QP { u64 m; u64 pt() override { return 11; } u64 qt() override { return 22; } };

struct V { u64 v; };                                    /* 4: virtual diamond */
struct L : virtual V { u64 l; };
struct R : virtual V { u64 r; };
struct D : L, R { u64 d; };

struct A0 { u64 a; };                                   /* 5: non-virtual diamond */
struct B1 : A0 {};
struct B2 : A0 {};
struct DD : B1, B2 { u64 d; };

struct Pub { u64 x; };                                  /* 6: access */
struct Priv : private Pub { u64 own; };
struct Prot : protected Pub { u64 own; };
struct Pubd : public Pub { u64 own; };

struct Val {                                            /* 7: copy counting */
    u64 v;
    Val(u64 x) : v(x) {}
    Val(const Val &o) : v(o.v) { copies++; }
};

struct Ua { u64 u; };                                   /* 8: unrelated */
struct Ub { u64 w; };

/* ---- the one thrower: code selects the variant (10+kind chain, 20+kind multiple inheritance, 30 diamond, 40 ambiguous,
 * 50+kind access, 60 value, 70 unrelated) ---------------------------------------------------------------------------------- */
static NOINLINE void throw_any(int code, u64 k) {
    switch (code) {
    case 10: { A x{}; x.a = k; throw x; }
    case 11: { B x{}; x.a = k; x.b = k + 100; throw x; }
    case 12: { C x{}; x.a = k; x.b = k + 100; x.c = k + 200; throw x; }
    case 20: { M x{}; x.p = k; x.q = k + 10; x.m = k + 20; throw x; }
    case 21: { MP y{}; y.p = k; y.q = k + 10; y.m = k + 20; throw y; }
    case 30: { D x{}; x.v = k; x.l = k + 1; x.r = k + 2; x.d = k + 3; throw x; }
    case 40: { DD x{}; x.B1::a = k; x.d = k; throw x; }
    case 50: { Priv y{}; y.own = k; throw y; }
    case 51: { Prot y{}; y.own = k; throw y; }
    case 52: { Pubd y{}; y.x = k; y.own = k; throw y; }
    case 60: throw Val(k);
    default: { Ub y{}; y.w = k; throw y; }
    }
}

typedef void (*Thrower)(int code, u64 k);
static NOINLINE void call_thrower(Thrower f, int code, u64 k) { f(code, k); }

/* ---- catching functions -------------------------------------------------------------------------------------------------- */
static NOINLINE u64 chain_catch(int mode, int kind, u64 k) {
    switch (mode) {
    case 0:                                       /* derived-first: C&, B&, A& */
        try { call_thrower(throw_any, 10 + kind, k); }
        catch (C &r) { return pk(3, r.a, r.b, r.c); }
        catch (B &r) { return pk(2, r.a, r.b, 0); }
        catch (A &r) { return pk(1, r.a, 0, 0); }
        break;
    case 1:                                       /* base-first: A&, B&, C& (case 2) */
        try { call_thrower(throw_any, 10 + kind, k); }
        catch (A &r) { return pk(11, r.a, 0, 0); }
        catch (B &r) { return pk(12, r.a, r.b, 0); }
        catch (C &r) { return pk(13, r.a, r.b, r.c); }
        break;
    case 2:                                       /* only B& */
        try { call_thrower(throw_any, 10 + kind, k); }
        catch (B &r) { return pk(21, r.a, r.b, 0); }
        break;
    case 3:                                       /* only const A& */
        try { call_thrower(throw_any, 10 + kind, k); }
        catch (const A &r) { return pk(31, r.a, 0, 0); }
        break;
    }
    return 0xffff;
}

static NOINLINE u64 mi_catch(int mode, int kind, u64 k) {
    switch (mode) {
    case 0:                                       /* Q& (non-primary base), then QP& */
        try { call_thrower(throw_any, 20 + kind, k); }
        catch (Q &r) { return pk(41, r.q, 0, 0); }
        catch (QP &r) { return pk(42, r.q, r.qt(), 0); }
        break;
    case 1:                                       /* P&, then PP& */
        try { call_thrower(throw_any, 20 + kind, k); }
        catch (P &r) { return pk(51, r.p, 0, 0); }
        catch (PP &r) { return pk(52, r.p, r.pt(), 0); }
        break;
    case 2:                                       /* M&, then MP& */
        try { call_thrower(throw_any, 20 + kind, k); }
        catch (M &r) { return pk(61, r.p, r.q, r.m); }
        catch (MP &r) { return pk(62, r.p, r.q, r.m); }
        break;
    }
    return 0xffff;
}

static NOINLINE u64 dia_catch(int mode, u64 k) {
    switch (mode) {
    case 0:
        try { call_thrower(throw_any, 30, k); }
        catch (V &r) { return pk(71, r.v, 0, 0); }
        break;
    case 1:
        try { call_thrower(throw_any, 30, k); }
        catch (L &r) { return pk(72, r.v, r.l, 0); }
        break;
    case 2:
        try { call_thrower(throw_any, 30, k); }
        catch (R &r) { return pk(73, r.v, r.r, 0); }
        break;
    case 3:
        try { call_thrower(throw_any, 30, k); }
        catch (D &r) { return pk(74, r.d, r.v, r.l * 100 + r.r); }
        break;
    }
    return 0xffff;
}

static NOINLINE u64 amb_catch(int mode, u64 k) {
    switch (mode) {
    case 0:                                       /* A0& listed first: ambiguous, no match; B1& matches */
        try { call_thrower(throw_any, 40, k); }
        catch (A0 &r) { return pk(81, r.a, 0, 0); }
        catch (B1 &r) { return pk(82, r.a, 0, 0); }
        break;
    case 1:                                       /* A0& only: no match, catch(...) runs */
        try { call_thrower(throw_any, 40, k); }
        catch (A0 &r) { return pk(91, r.a, 0, 0); }
        catch (...) { return pk(92, 0, 0, 0); }
        break;
    }
    return 0xffff;
}

static NOINLINE u64 access_catch(int kind, u64 k) {
    try { call_thrower(throw_any, 50 + kind, k); }
    catch (Pub &r) {
        return pk(101, r.x, 0, 0);
    } catch (...) {
        return pk(102, (u64)kind, 0, 0);
    }
    return 0xffff;
}

static NOINLINE u64 val_catch(int mode, u64 k) {
    u64 before = copies;
    switch (mode) {
    case 0:
        try { call_thrower(throw_any, 60, k); }
        catch (Val v) { return pk(111, v.v, copies - before, 0); }
        break;
    case 1:
        try { call_thrower(throw_any, 60, k); }
        catch (Val &r) { return pk(112, r.v, copies - before, 0); }
        break;
    }
    return 0xffff;
}

static NOINLINE u64 unrelated_catch(u64 k) {
    try {
        try { call_thrower(throw_any, 70, k); }
        catch (Ua &r) { return pk(121, r.u, 0, 0); }
    } catch (Ub &r) {
        return pk(123, r.w, 0, 0);
    }
    return 0xffff;
}

extern "C" void test_main(void) {
    acc = 0x9e3779b97f4a7c15ull;
    nrec = 0;
    copies = 0;
    const u64 k = 7;
    /* 1: chain, derived-first and other single handlers */
    rec(chain_catch(0, 2, k));          /* C thrown, C&: pk(3, 7, 107, 207) */
    rec(chain_catch(0, 1, k));          /* B thrown, B&: pk(2, 7, 107, 0) */
    rec(chain_catch(0, 0, k));          /* A thrown, A&: pk(1, 7, 0, 0) */
    rec(chain_catch(2, 2, k));          /* C thrown, B&: pk(21, 7, 107, 0) */
    rec(chain_catch(3, 2, k));          /* C thrown, const A&: pk(31, 7, 0, 0) */
    /* 2: handler order, A& listed before C& */
    rec(chain_catch(1, 2, k));          /* C thrown, A& first: pk(11, 7, 0, 0) */
    rec(chain_catch(1, 1, k));          /* B thrown, A& first: pk(11, 7, 0, 0) */
    rec(chain_catch(1, 0, k));          /* A thrown, A&: pk(11, 7, 0, 0) */
    /* 3: multiple inheritance */
    rec(mi_catch(0, 0, k));             /* M thrown, Q&: pk(41, 17, 0, 0) */
    rec(mi_catch(0, 1, k));             /* MP thrown, Q& no match, QP&: pk(42, 17, 22, 0) */
    rec(mi_catch(1, 0, k));             /* M thrown, P&: pk(51, 7, 0, 0) */
    rec(mi_catch(1, 1, k));             /* MP thrown, PP&: pk(52, 7, 11, 0) */
    rec(mi_catch(2, 0, k));             /* M thrown, M&: pk(61, 7, 17, 27) */
    rec(mi_catch(2, 1, k));             /* MP thrown, MP&: pk(62, 7, 17, 27) */
    /* 4: virtual diamond */
    rec(dia_catch(0, k));               /* V&: pk(71, 7, 0, 0) */
    rec(dia_catch(1, k));               /* L&: pk(72, 7, 8, 0) */
    rec(dia_catch(2, k));               /* R&: pk(73, 7, 9, 0) */
    rec(dia_catch(3, k));               /* D&: pk(74, 10, 7, 809) */
    /* 5: ambiguous base */
    rec(amb_catch(0, k));               /* A0& no match, B1&: pk(82, 7, 0, 0) */
    rec(amb_catch(1, k));               /* A0& no match, catch(...): pk(92, 0, 0, 0) */
    /* 6: access */
    rec(access_catch(0, k));            /* Priv: catch(...): pk(102, 0, 0, 0) */
    rec(access_catch(1, k));            /* Prot: catch(...): pk(102, 1, 0, 0) */
    rec(access_catch(2, k));            /* Pubd: Pub&: pk(101, 7, 0, 0) */
    /* 7: value copies */
    rec(val_catch(0, k));               /* catch(Val): pk(111, 7, 1, 0) */
    rec(val_catch(1, k));               /* catch(Val&): pk(112, 7, 0, 0) */
    rec(copies);                        /* global copy count: 1 */
    /* 8: unrelated */
    rec(unrelated_catch(k));            /* Ua& no match, Ub&: pk(123, 7, 0, 0) */
    rec(nrec);
    results[NRES - 1] = acc;
}
