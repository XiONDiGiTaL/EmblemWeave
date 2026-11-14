// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
// EXPECT-INSN: \bblr\s+x\d+
/* 23_eh_catchall_misc: catch(...) and exact-type catches of every builtin type and two enums; exceptions crossing virtual
 * calls, function pointers, a tail call, constructors and a 40-level recursion; by-value catch with copy/destructor counters;
 * exception object lifetime; a 500-iteration stress loop.
 *
 * Expected outcome per case (the native run must print exactly these; results[] words in order):
 *  1a ladder: the thrown type selects its own exact handler. Handler index (int 0, unsigned 1, long 2, unsigned long 3,
 *     long long 4, unsigned long long 5, short 6, unsigned short 7, char 8, signed char 9, unsigned char 10, bool 11,
 *     float 12, double 13, E1 14, E2 15) for thrown k = 0..15:
 *       11 8 9 10 6 7 0 1 2 3 4 5 12 13 14 15
 *  1b catch(...) alone: records k back for k = 0..15 (0 1 2 ... 15).
 *  2  class ladder (Ca, Cb typed; catch(...) last), k = 0..3: Ca -> 1+41 = 42, Cb -> 100+77 = 177,
 *     Cc (k=2) and int (k=3) -> catch(...) -> 202, 203.
 *  3  virtual dispatch through VBase* array (VA, VB, VC), caught at the call site, k = 0..11, x = 2k+1:
 *       7 16 3005 1007 34 2 31 2015 24 1019 2021 30
 *     function pointers, k = 0..8 (third slot is a tail call into a throwing callee):
 *       86 19 10 87 22 7006 84 5007 82
 *  4  constructor throws: single object fails at its body -> member b then a destroyed, ~Pair2 not run: 4000;
 *     no failure: 1; array Pair2[3] whose 2nd element's constructor throws: element 0 and the partial element 1
 *     members are destroyed: 5001.
 *  5  recursion of 41 frames (levels 0..40), one guard per frame, throw at level 40 caught at the top:
 *       140 (100+40), 0 guards live, max 41 guards live at once.
 *  6  by-value catch of Heavy (copy ctor and dtor counted): value 10, dtors-copies = 1 (identical with or without
 *     copy elision of the throw operand), live objects = 0.
 *  7  exception object lifetime: handler marks 1, its dtor marks 2 at end of the catch clause, the statement after the
 *     try marks 3: order 123, destructor count 1, value 42.
 *  8  stress: 500 iterations, kind = lcg % 5 chooses int / SSmall / SDer (caught as SBase&) / const char* / double
 *     (caught by catch(...)); records the checksum and the per-kind hit counts (they sum to 500).
 */
#include "rt2.h"

static u64 acc;
static u64 nrec;

static void mix(u64 v) { acc = (acc ^ v) * 0x100000001b3ull; acc ^= acc >> 29; }
static void note(u64 tag) { mix(tag); }
static void rec(u64 v) { mix(v); if (nrec < NRES - 2) results[nrec++] = v; }

enum E1 { a, b };
enum class E2 : int { x, y };

struct Ca { int v; };
struct Cb { u64 w; };
struct Cc { double d; };

/* ---- 1. builtin types and enumerations ------------------------------------------------------------------------------------ */
NOINLINE static void throw_type(int k) {
    switch (k) {
    case 0: throw true;
    case 1: throw (char)'A';
    case 2: throw (signed char)-5;
    case 3: throw (unsigned char)200;
    case 4: throw (short)-300;
    case 5: throw (unsigned short)60000;
    case 6: throw (int)-7;
    case 7: throw 8u;
    case 8: throw 9L;
    case 9: throw 10UL;
    case 10: throw 11LL;
    case 11: throw 12ULL;
    case 12: throw 1.5f;
    case 13: throw 2.25;
    case 14: throw E1::b;
    case 15: throw E2::y;
    default: return;
    }
}

/* one handler per exact type; the int handler comes before short so a wrong (promoting) match would show */
NOINLINE static int ladder(int k) {
    try {
        throw_type(k);
    } catch (int) { return 0;
    } catch (unsigned) { return 1;
    } catch (long) { return 2;
    } catch (unsigned long) { return 3;
    } catch (long long) { return 4;
    } catch (unsigned long long) { return 5;
    } catch (short) { return 6;
    } catch (unsigned short) { return 7;
    } catch (char) { return 8;
    } catch (signed char) { return 9;
    } catch (unsigned char) { return 10;
    } catch (bool) { return 11;
    } catch (float) { return 12;
    } catch (double) { return 13;
    } catch (E1) { return 14;
    } catch (E2) { return 15;
    }
    return -1;
}

NOINLINE static int catch_all(int k) {
    try {
        throw_type(k);
    } catch (...) {
        return k;
    }
    return -1;
}

/* ---- 2. catch(...) after typed class handlers ------------------------------------------------------------------------------ */
NOINLINE static void throw_class(int k) {
    if (k == 0) throw Ca{41};
    if (k == 1) throw Cb{77};
    if (k == 2) throw Cc{2.5};
    throw (int)k;
}

NOINLINE static u64 class_ladder(int k) {
    try {
        throw_class(k);
        return 0;
    } catch (Ca &c) {
        return 1 + (u64)(s64)c.v;
    } catch (Cb cb) {
        return 100 + cb.w;
    } catch (...) {
        return 200 + (u64)(s64)k;
    }
}

/* ---- 3. virtual calls, function pointers, tail call ------------------------------------------------------------------------- */
struct VBase {
    u64 id;
    VBase(u64 i) : id(i) {}
    virtual u64 op(u64 x) { return x + id; }
};
struct VA : VBase {
    VA(u64 i) : VBase(i) {}
    __attribute__((noinline)) u64 op(u64 x) override { if (x % 4 == 3) throw Ca{(int)x}; return x * 2 + id; }
};
struct VB : VBase {
    VB(u64 i) : VBase(i) {}
    __attribute__((noinline)) u64 op(u64 x) override { if (x > 10) throw (int)x; return x * 3 + id; }
};
struct VC : VBase {
    VC(u64 i) : VBase(i) {}
    __attribute__((noinline)) u64 op(u64 x) override { if (x == 5) throw Cb{x}; return x ^ id; }
};

NOINLINE static u64 virt_site(VBase *p, u64 x) {
    try {
        return p->op(x);
    } catch (Ca &c) {
        return 1000 + (u64)(s64)c.v;
    } catch (int v) {
        return 2000 + (u64)(s64)v;
    } catch (Cb &w) {
        return 3000 + w.w;
    }
}

typedef u64 (*fp_t)(u64);
NOINLINE static u64 fp_ok(u64 x) { return x ^ 0x55; }
NOINLINE static u64 fp_bad(u64 x) { if (x & 2) throw Cc{(double)x}; return x + 17; }
NOINLINE static u64 tc_inner(u64 x) { if (x == 6) throw 6L; return x * x; }
NOINLINE static u64 tc_outer(u64 x) { return tc_inner(x + 1); }     /* tail-call shape: return g(x); */
static fp_t fp_table[3] = { fp_ok, fp_bad, tc_outer };

NOINLINE static u64 call_fp(fp_t f, u64 x) {
    try {
        return f(x) + 1;
    } catch (Cc &c) {
        return 5000 + (u64)(s64)c.d;
    } catch (long l) {
        return 7000 + (u64)l;
    }
}

/* ---- 4. constructor throws ---------------------------------------------------------------------------------------------------- */
static u64 mk_seq;               /* number of Pair2 constructions started */
static int mk_fail_at = -1;      /* index of the Pair2 whose constructor body throws */

struct Mark {
    u64 tag;
    Mark(u64 t) : tag(t) { note(0x100 + t); }
    ~Mark() { note(0x200 + tag); }
};
struct Pair2 {
    Mark a;
    Mark b;
    __attribute__((noinline)) Pair2() : a(100 + mk_seq * 2), b(101 + mk_seq * 2) {
        u64 me = mk_seq++;
        if ((int)me == mk_fail_at) throw (int)me;
        note(0x300 + me);
    }
    ~Pair2() { note(0x400); }
};

NOINLINE static u64 ctor_single(int fail_at) {
    mk_fail_at = fail_at;
    mk_seq = 0;
    try {
        Pair2 p;
        return 1;
    } catch (int v) {
        return 4000 + (u64)(s64)v;
    }
}

NOINLINE static u64 ctor_array(void) {
    mk_fail_at = 1;
    mk_seq = 0;
    try {
        Pair2 arr[3];
        return 1;
    } catch (int v) {
        return 5000 + (u64)(s64)v;
    }
}

/* ---- 5. recursion with one guard per frame ------------------------------------------------------------------------------------- */
static u64 live_guards;
static u64 max_live;
struct Guard {
    u64 lvl;
    Guard(u64 l) : lvl(l) { live_guards++; if (live_guards > max_live) max_live = live_guards; note(0x1000 + l); }
    ~Guard() { live_guards--; note(0x2000 + lvl); }
};
NOINLINE static u64 deep(u64 lvl, u64 limit) {
    Guard g(lvl);
    if (lvl == limit) throw Ca{(int)lvl};
    return deep(lvl + 1, limit) + 1;
}

/* ---- 6. by-value catch with copy constructor and destructor counters ---------------------------------------------------------- */
static u64 hv_live;
static u64 hv_copies;
static u64 hv_dtors;
struct Heavy {
    u64 v;
    Heavy(u64 x) : v(x) { hv_live++; }
    Heavy(const Heavy &o) : v(o.v) { hv_copies++; hv_live++; }
    ~Heavy() { hv_dtors++; hv_live--; }
};
NOINLINE static void throw_heavy(u64 x) { throw Heavy(x); }
NOINLINE static u64 heavy_case(u64 x) {
    try {
        throw_heavy(x);
        return 0;
    } catch (Heavy h) {
        return h.v + 1;
    }
}

/* ---- 7. exception object lifetime ---------------------------------------------------------------------------------------------- */
static u64 life_order;
static u64 life_dtors;
struct Life {
    u64 id;
    Life(u64 i) : id(i) {}
    ~Life() { life_dtors++; life_order = life_order * 10 + 2; }
};
NOINLINE static void throw_life(u64 i) { throw Life(i); }
NOINLINE static u64 life_case(u64 i) {
    u64 r = 0;
    try {
        throw_life(i);
    } catch (Life &l) {
        r = l.id;
        life_order = life_order * 10 + 1;             /* inside the handler */
    }
    life_order = life_order * 10 + 3;                 /* after the try statement */
    return r;
}

/* ---- 8. stress loop ------------------------------------------------------------------------------------------------------------ */
struct SSmall { u32 a; u16 b; };
struct SBase { u64 b; };
struct SDer : SBase { u64 d; };

NOINLINE static void stress_throw(u64 kind, u64 i) {
    switch (kind) {
    case 0: throw (int)(i & 0xffff);
    case 1: throw SSmall{(u32)i, (u16)(i * 3)};
    case 2: { SDer x; x.b = i * 7; x.d = i ^ 0x33; throw x; }
    case 3: throw "stress";
    default: throw (double)i * 0.5;
    }
}

NOINLINE static u64 stress_catch(u64 kind, u64 i) {
    try {
        stress_throw(kind, i);
        return 0;
    } catch (int v) {
        return 1 + (u64)(s64)v;
    } catch (SSmall &s) {
        return 100000 + s.a + s.b;
    } catch (SBase &sb) {
        return 200000 + sb.b;                          /* SDer caught through its base */
    } catch (const char *s) {
        return 300000 + (u64)s[0];
    } catch (...) {
        return 400000;                                 /* double */
    }
}

extern "C" void test_main(void) {
    acc = 0x9e3779b97f4a7c15ull;
    nrec = 0;

    /* 1 */
    for (int k = 0; k < 16; k++) rec((u64)(s64)ladder(k));                       /* 16 */
    for (int k = 0; k < 16; k++) rec((u64)(s64)catch_all(k));                    /* 16 */

    /* 2 */
    for (int k = 0; k < 4; k++) rec(class_ladder(k));                            /* 4 */

    /* 3 */
    VA va(5);
    VB vb(7);
    VC vc(9);
    VBase *varr[3] = { &va, &vb, &vc };
    for (u64 k = 0; k < 12; k++) rec(virt_site(varr[k % 3], k * 2 + 1));         /* 12 */
    for (u64 k = 0; k < 9; k++) rec(call_fp(fp_table[k % 3], k));                /* 9 */

    /* 4 */
    rec(ctor_single(0));                                                         /* 4000 */
    rec(ctor_single(-1));                                                        /* 1 */
    rec(ctor_array());                                                           /* 5001 */

    /* 5 */
    live_guards = 0;
    max_live = 0;
    u64 dr = 0;
    try {
        deep(0, 40);
        dr = 1;
    } catch (Ca &c) {
        dr = 100 + (u64)(s64)c.v;
    }
    rec(dr);                                                                     /* 140 */
    rec(live_guards);                                                            /* 0 */
    rec(max_live);                                                               /* 41 */

    /* 6 */
    hv_live = 0;
    hv_copies = 0;
    hv_dtors = 0;
    rec(heavy_case(9));                                                          /* 10 */
    rec((u64)(s64)(hv_dtors - hv_copies));                                       /* 1 */
    rec(hv_live);                                                                /* 0 */

    /* 7 */
    rec(life_case(42));                                                          /* 42 */
    rec(life_order);                                                             /* 123 */
    rec(life_dtors);                                                             /* 1 */

    /* 8 */
    u64 seed = 12345;
    u64 stress_sum = 0;
    u64 kind_hits[5] = { 0, 0, 0, 0, 0 };
    for (u64 i = 0; i < 500; i++) {
        u64 kind = lcg(&seed) % 5;
        kind_hits[kind]++;
        u64 r = stress_catch(kind, i);
        stress_sum += r;
        mix(r);
    }
    rec(stress_sum);
    for (int j = 0; j < 5; j++) rec(kind_hits[j]);                               /* 5 */

    results[NRES - 1] = acc;
}
