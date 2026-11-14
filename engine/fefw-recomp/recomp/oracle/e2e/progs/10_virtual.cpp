// FLAGS:
// EXPECT-INSN: \bblr\s+x\d+
// EXPECT-INSN: \bbr\s+x\d+
/* C++ virtual dispatch and indirect calls: vtables, multiple inheritance (adjustor thunks), arrays of base pointers,
 * function-pointer tables, pointer-to-member-function calls, callbacks and a hand-written closure.
 * No RTTI, no exceptions, no libc. Objects are placement-constructed at run time (the harness runs no constructors)
 * and reached through volatile pointers so the compiler cannot devirtualise. */
#include "t.h"

/* placement new (the harness has no <new>) */
inline void *operator new(unsigned long, void *p) { return p; }

static u64 ri, hsh;
static inline void put(u64 v) {
    hsh = (hsh ^ v) * 0x100000001b3ull + 0x9e37ull;
    if (ri < 400) results[ri++] = v;
}

static volatile u64 g_seed = 0x0123456789abcdefull;

struct Base {
    u64 bv;
    Base(u64 v) : bv(v) {}
    virtual u64 f(u64 x) { return x * 3 + bv; }
    virtual u64 g(u64 x) { return (x ^ bv) << 7; }
    virtual u64 h(u64 a, u64 b) { return a + b * bv; }
};
struct DerA : Base {
    u64 ka;
    DerA(u64 v, u64 k) : Base(v), ka(k) {}
    u64 f(u64 x) override { return x + ka; }
    u64 g(u64 x) override { return Base::g(x) + ka; }
};
struct DerB : Base {
    u64 kb;
    DerB(u64 v, u64 k) : Base(v), kb(k) {}
    u64 f(u64 x) override { return (x << 1) ^ kb; }
    u64 h(u64 a, u64 b) override { return a * kb - b; }
};
struct Side {
    u64 sv;
    Side(u64 v) : sv(v) {}
    virtual u64 side(u64 x) { return x * 11 + sv; }
    virtual u64 side2(u64 x) { return x ^ 0x55; }
};
/* second base at a non-zero offset: calls through Side* go through an adjustor thunk */
struct MI : Base, Side {
    u64 mi;
    MI(u64 v, u64 m) : Base(v), Side(v + m), mi(m) {}
    u64 f(u64 x) override { return Base::f(x) ^ mi; }
    u64 side(u64 x) override { return x + mi; }
};

typedef u64 (*fn1)(u64);
typedef u64 (Base::*mfn1)(u64);
typedef u64 (Base::*mfn2)(u64, u64);

struct Closure { u64 (*fn)(void *env, u64 x); void *env; };

NOINLINE static u64 k0(u64 x) { return x * 5 + 1; }
NOINLINE static u64 k1(u64 x) { return x ^ (x >> 9); }
NOINLINE static u64 k2(u64 x) { return (x << 3) | 7; }
NOINLINE static u64 k3(u64 x) { return x * 0x9e3779b97f4a7c15ull; }
NOINLINE static u64 k4(u64 x) { return ~x + 3; }
NOINLINE static u64 k5(u64 x) { return (x >> 1) - 11; }
NOINLINE static u64 k6(u64 x) { return x * x + 5; }
NOINLINE static u64 k7(u64 x) { return x ^ 0xa5a5a5a5ull; }
static fn1 const ftab[8] = { k0, k1, k2, k3, k4, k5, k6, k7 };
static mfn1 const mtab[2] = { &Base::f, &Base::g };
static mfn2 const mh = &Base::h;

NOINLINE static u64 op_add(u64 a, u64 b) { return a + b * 0x9e3779b97f4a7c15ull; }
NOINLINE static u64 op_xor(u64 a, u64 b) { return (a ^ b) * 0xff51afd7ed558ccdull; }

NOINLINE static u64 cl_add(void *env, u64 x) { return x + *(u64 *)env; }
NOINLINE static u64 cl_mul(void *env, u64 x) { return x * (*(u64 *)env | 1); }

NOINLINE static u64 call_base_f(Base *b, u64 x) { return b->f(x); }          /* tail call through vtable: br */
NOINLINE static u64 call_base_f_ret(Base *b, u64 x) { u64 r = b->f(x); return r + 1; } /* blr */
NOINLINE static u64 invoke(const Closure &c, u64 x) { return c.fn(c.env, x); }  /* tail call through pointer: br */
NOINLINE static u64 fold(u64 (*f)(u64, u64), const u64 *a, int n) {           /* blr per element */
    u64 acc = 0;
    for (int i = 0; i < n; i++) acc = f(acc, a[i]);
    return acc;
}
NOINLINE static u64 chain(Base *volatile *objs, int n, u64 x) {
    u64 acc = x;
    for (int i = 0; i < n; i++) {
        Base *b = objs[i];
        acc = b->f(acc) ^ b->h(acc, (u64)i);
    }
    return acc;
}

alignas(16) static unsigned char pool[4096];
static Base *volatile g_objs[8];
static Side *volatile g_sides[2];
static u64 g_env[8];

extern "C" void test_main(void) {
    u64 s = g_seed;

    u64 p0 = lcg(&s);
    g_objs[0] = new (pool + 0) Base(p0);
    u64 p1 = lcg(&s), p2 = lcg(&s);
    g_objs[1] = new (pool + 64) DerA(p1, p2);
    u64 p3 = lcg(&s), p4 = lcg(&s);
    g_objs[2] = new (pool + 128) DerB(p3, p4);
    u64 p5 = lcg(&s), p6 = lcg(&s);
    MI *mi = new (pool + 192) MI(p5, p6);
    g_objs[3] = mi;
    g_sides[0] = mi;                       /* MI* -> Side*: adjusted pointer */
    u64 p7 = lcg(&s), p8 = lcg(&s);
    g_objs[4] = new (pool + 256) DerA(p7, p8);
    u64 p9 = lcg(&s), pa = lcg(&s);
    g_objs[5] = new (pool + 320) DerB(p9, pa);
    u64 pb = lcg(&s);
    g_objs[6] = new (pool + 384) Base(pb);
    u64 pc = lcg(&s), pd = lcg(&s);
    g_objs[7] = new (pool + 448) DerA(pc, pd);
    u64 pe = lcg(&s);
    g_sides[1] = new (pool + 512) Side(pe);

    for (int r = 0; r < 96; r++) {
        u64 k = lcg(&s);
        u64 x = lcg(&s);
        Base *b = g_objs[k & 7];
        put(b->f(x));                        /* vtable call through a base pointer */
        put(b->g(x));
        put(b->h(x, k));
        put(call_base_f(b, x));
        put(call_base_f_ret(b, x));
        put(ftab[k % 8](x));                 /* function-pointer table indexed by data */
        put((b->*mtab[k & 1])(x));           /* pointer to virtual member function */
        put((b->*mh)(x, k));
        u64 arr[4] = { x, k, x * 3, k ^ 9 };
        put(fold(((k >> 4) & 1) ? op_add : op_xor, arr, 4));
        g_env[k & 7] = x;
        Closure c = { ((k >> 5) & 1) ? cl_add : cl_mul, (void *)&g_env[k & 7] };
        put(invoke(c, x));
        Side *sd = g_sides[k & 1];
        put(sd->side(x));                    /* second-base calls, one through a thunk */
        put(sd->side2(x));
        put(chain(g_objs, 8, x));
    }
    put(g_objs[1]->bv);
    put(g_sides[0]->side(1));
    results[ri++] = hsh;
}
