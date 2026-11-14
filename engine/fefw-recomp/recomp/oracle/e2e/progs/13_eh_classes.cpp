// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <_Unwind_Resume>
/* 13_eh_classes: C++ exceptions of user class types. Several catch clauses on one try, catch(...) fallback for builtin
 * types, nested try blocks, a throw from inside a handler, rethrow with `throw;` into an outer handler, RAII destructor
 * order through five frames, constructor failure with member cleanup, temporaries, and a try inside a loop.
 * Only types without base classes are thrown; catch clauses use exact types. No std::, no new/delete, no globals with
 * constructors. */
#include "rt.h"

static u64 acc;          /* order-sensitive hash of every note and every recorded value */
static u64 nrec;

static void mix(u64 v) { acc = (acc ^ v) * 0x100000001b3ull; acc ^= acc >> 29; }
static void note(u64 tag) { mix(tag); }
static void rec(u64 v) { mix(v); if (nrec < NRES - 2) results[nrec++] = v; }

struct ErrCode { int code; int line; };
struct ErrWide { u64 big; u8 tag; };
struct ErrMix  { s16 a; s16 b; float f; };
struct ErrDbl  { double d; s32 n; };
struct ErrArr  { u64 w[4]; };

struct Tracker {
    u64 tag;
    Tracker(u64 t) : tag(t) { note(0x100 + t); }
    ~Tracker() { note(0x200 + tag); }
};

struct Pair {                  /* members built a, b; destroyed b, a; ~Pair runs before them */
    Tracker a;
    Tracker b;
    int n;
    Pair(int k) : a((u64)k * 2 + 1), b((u64)k * 2 + 2), n(k) { note(0x300 + (u64)k); }
    ~Pair() { note(0x400 + (u64)n); }
};

struct Risky {                 /* ctor throws after a and b are built: they are destroyed, ~Risky does not run */
    Tracker a;
    Tracker b;
    Risky(u64 k) : a(k), b(k + 1) { if (k == 3) throw (int)k; note(0x700 + k); }
    ~Risky() { note(0x800); }
};

struct Temp {
    u64 v;
    Temp(u64 x) : v(x) { note(0x500 + x); }
    ~Temp() { note(0x600 + v); }
};

static NOINLINE void raise_code(int k) {
    Tracker t((u64)k);                 /* destroyed while unwinding */
    switch (k & 3) {
    case 0: throw ErrCode{k, 10};
    case 1: throw ErrWide{0x1122334455667788ull ^ (u64)k, (u8)(k + 1)};
    case 2: throw ErrMix{(s16)(-k), (s16)(k * 3), (float)k * 0.5f};
    default: throw ErrDbl{(double)k * 1.25, (s32)k};
    }
}

/* several catch clauses on one try: the exact type selects the handler */
static NOINLINE u64 select_catch(int k) {
    try {
        raise_code(k);
        return 0;
    } catch (ErrCode &e) {
        return 1000 + (u64)e.code * 16 + (u64)e.line;
    } catch (ErrWide w) {
        return 2000000 + (w.big & 0xffffff) * 4 + w.tag;
    } catch (const ErrMix &m) {
        return 3000 + (u64)(s64)m.a * 2 + (u64)(s64)m.b + (u64)(s64)(m.f * 4.0f);
    } catch (ErrDbl d) {
        return 4000 + (u64)(s64)(d.d * 8.0) + (u64)(s64)d.n;
    }
    return 0xffff;
}

/* builtin types: int, long, const char*, unsigned int (only catch(...)) */
static NOINLINE u64 other_throw(int k) {
    try {
        if (k == 4) throw 42;
        if (k & 1) throw (long)(k * 11);
        if (k & 2) throw "text";
        throw (unsigned int)(k + 5);
    } catch (int v) {
        return 1 + (u64)v;
    } catch (long l) {
        return 500 + (u64)l;
    } catch (const char *s) {
        return 900 + (u64)s[0] + (u64)s[3];
    } catch (...) {
        return 7000 + (u64)k;
    }
}

/* nested try: the inner handler throws a new exception; the outer try catches it, catch(...) takes the rest */
static NOINLINE u64 nested(int k) {
    u64 r = 0;
    try {
        try {
            raise_code(k);
            r = 1;
        } catch (ErrCode &e) {
            r = 2 + (u64)e.code;
            if (e.code == 4) throw (int)77;       /* new exception from inside a handler */
            r += 10;
        }
        r += 100;
    } catch (int v) {
        r = r * 1000 + (u64)v;
    } catch (...) {
        r = 99999;                                 /* ErrWide, ErrMix, ErrDbl pass the inner try */
    }
    return r;
}

/* rethrow: the inner handler modifies the caught object and throws it on with `throw;` */
static NOINLINE void rethrow_inner(int k) {
    try {
        raise_code(k);
    } catch (ErrWide &w) {
        note(0x900 + w.tag);
        w.tag = (u8)(w.tag + 1);
        throw;                                     /* same object goes on to the outer handler */
    }
}

static NOINLINE u64 rethrow_outer(int k) {
    try {
        rethrow_inner(k);
        return 0;
    } catch (ErrCode &e) {
        return 10 + (u64)e.code;
    } catch (ErrWide &w) {
        return 2000 + (u64)w.tag;
    } catch (...) {
        return 3;
    }
}

/* five frames, each with a RAII object; the throw comes out of the innermost one */
static NOINLINE u64 f5(u64 k) { Pair p((int)k); if (k > 2) throw ErrDbl{(double)k, (s32)k}; return (u64)p.n; }
static NOINLINE u64 f4(u64 k) { Tracker t(40 + k); return f5(k) + 1; }
static NOINLINE u64 f3(u64 k) { Tracker t(30 + k); return f4(k) + 1; }
static NOINLINE u64 f2(u64 k) { Tracker t(20 + k); return f3(k) + 1; }
static NOINLINE u64 f1(u64 k) { Tracker t(10 + k); return f2(k) + 1; }

/* two temporaries are alive in the full expression that calls a throwing function */
static NOINLINE u64 use_temps(u64 x) {
    if (x & 2) throw ErrCode{(int)x, 1};
    return x * 3;
}
static NOINLINE u64 temps_case(u64 k) {
    try {
        u64 r = use_temps(Temp(k).v + Temp(k + 1).v);
        return r;
    } catch (ErrCode &e) {
        return 600 + (u64)e.code;
    }
}

/* constructor throws after two members are constructed */
static NOINLINE u64 risky(u64 k) {
    try {
        Risky r(k);
        return r.a.tag + r.b.tag;
    } catch (int v) {
        return 4000 + (u64)v;
    }
}

/* a try inside a loop: some iterations throw and are caught, some do not; continue and break leave the try normally */
static NOINLINE u64 loop_try(u64 n) {
    u64 s = 0;
    for (u64 i = 0; i < n; i++) {
        try {
            if (i == 7) continue;
            if (i == 9) break;
            if (i % 5 == 4) throw (long)i;
            if (i % 3 == 1) throw ErrCode{(int)i, 7};
            s += i;
        } catch (ErrCode &e) {
            s += 1000 + (u64)e.code;
        } catch (long l) {
            s ^= (u64)l << 3;
        }
    }
    return s;
}

/* function-try-block */
static NOINLINE int ftb(int k) try {
    if (k & 1) throw k;
    return k * 2;
} catch (int v) {
    return 300 + v;
}

/* aggregate with an array member */
static NOINLINE void throw_arr(u64 s) { throw ErrArr{{s, s * 3, s ^ 0x55, ~s}}; }
static NOINLINE u64 arr_catch(u64 s) {
    try {
        throw_arr(s);
        return 0;
    } catch (ErrArr a) {
        return a.w[0] + a.w[1] * 2 + a.w[2] * 3 + a.w[3] * 5;
    }
}

extern "C" void test_main(void) {
    acc = 0x9e3779b97f4a7c15ull;
    nrec = 0;
    try {
        for (int k = 0; k < 8; k++) rec(select_catch(k));                       /* 8 */
        for (int k = 0; k < 8; k++) rec(other_throw(k));                        /* 8 */
        for (int k = 0; k < 8; k++) rec(nested(k));                             /* 8 */
        for (int k = 0; k < 8; k++) rec(rethrow_outer(k));                      /* 8 */
        for (int k = 0; k < 6; k++) {                                            /* 6 */
            try { rec(f1((u64)k)); }
            catch (ErrDbl &d) { rec(5000 + (u64)(s64)d.n); }
        }
        for (u64 k = 0; k < 6; k++) rec(temps_case(k));                         /* 6 */
        for (u64 k = 0; k < 5; k++) rec(risky(k));                              /* 5 */
        rec(loop_try(12));                                                       /* 1 */
        rec(loop_try(3));                                                        /* 1 */
        for (int k = 0; k < 4; k++) rec((u64)ftb(k));                           /* 4 */
        for (u64 s = 1; s < 5; s++) rec(arr_catch(s));                          /* 4 */
        u64 seed = 77;
        for (int i = 0; i < 16; i++) rec(select_catch((int)(lcg(&seed) % 8)));  /* 16 */
        rec(nrec);
    } catch (...) {
        rec(0xdeadull);
    }
    results[NRES - 1] = acc;
}
