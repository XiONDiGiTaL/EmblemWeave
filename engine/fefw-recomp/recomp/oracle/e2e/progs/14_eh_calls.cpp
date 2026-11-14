// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
// EXPECT-INSN: \bblr\s+x\d+
/* 14_eh_calls: exceptions crossing different kinds of calls: a virtual call through a base pointer, a call through a
 * function pointer with a tail call behind it, a member function called from a member, a 20-level recursion with an RAII
 * guard per level, and an initialisation-style function called from test_main. Each part also has inputs that do not
 * throw, so both paths are compared. Freestanding: no std::, no new/delete, no globals with constructors. */
#include "rt.h"

struct Fail { u64 site; int depth; };
struct Code { int c; };

static u64 acc;
static u64 nrec;
static u64 depth_acc;

static void mix(u64 v) { acc = (acc ^ v) * 0x100000001b3ull; acc ^= acc >> 29; }
static void rec(u64 v) { mix(v); if (nrec < NRES - 2) results[nrec++] = v; }

/* A. virtual call through a base pointer; the overrides throw */
struct Shape {
    u64 id;
    Shape(u64 i) : id(i) {}
    virtual u64 measure(u64 x) { return x + id; }
};
struct Circle : Shape {
    Circle(u64 i) : Shape(i) {}
    u64 measure(u64 x) override {
        if (x % 7 == 3) throw Fail{x, 1};
        return x * 3 + id;
    }
};
struct Square : Shape {
    Square(u64 i) : Shape(i) {}
    u64 measure(u64 x) override {
        if (x > 100) throw Code{(int)x};
        return x * x + id;
    }
};
static NOINLINE u64 through_base(Shape *s, u64 x) { return s->measure(x); }

/* B. function pointer; tail_caller reaches tail_callee by a jump, so its frame is gone when the throw happens */
typedef u64 (*fn_t)(u64);
static NOINLINE u64 fn_ok(u64 x) { return x ^ 0x55; }
static NOINLINE u64 fn_throw(u64 x) { if (x & 4) throw Fail{x, 2}; return x + 9; }
static NOINLINE u64 tail_callee(u64 x) { if (x == 6) throw Code{(int)x}; return x * x; }
static NOINLINE u64 tail_caller(u64 x) { return tail_callee(x + 1); }
static fn_t fn_table[3] = { fn_ok, fn_throw, tail_caller };
static NOINLINE u64 call_via(fn_t f, u64 x) { return f(x) + 1; }

/* C. member function called from a member function (neither is noexcept) */
struct Engine {
    u64 fuel;
    __attribute__((noinline)) u64 burn(u64 x) {
        if (x > fuel) throw Fail{x, 3};
        fuel -= x;
        return fuel;
    }
    __attribute__((noinline)) u64 drive(u64 n) {
        u64 t = 0;
        for (u64 i = 0; i < n; i++) t += burn(i * 7);
        return t;
    }
};

/* D. recursion 20 levels deep, one RAII guard per level; the guards record into depth_acc in order */
struct DepthGuard {
    u64 level;
    DepthGuard(u64 l) : level(l) { depth_acc = depth_acc * 131 + l; }
    ~DepthGuard() { depth_acc = depth_acc * 137 + level + 1000; }
};
static NOINLINE u64 recurse(u64 level, u64 limit, u64 throw_at) {
    DepthGuard g(level);
    if (level == limit) return 0;
    if (level == throw_at) throw Fail{level, (int)level};
    return recurse(level + 1, limit, throw_at) + level;
}

/* E. initialisation-style function: fills a global table, may fail half way through */
static u64 init_table[8];
static NOINLINE u64 init_tables(u64 seed) {
    for (u64 i = 0; i < 8; i++) {
        init_table[i] = lcg(&seed) >> 40;
        if (i == 5 && seed % 3 == 0) throw Code{(int)i};
    }
    return init_table[7];
}

extern "C" void test_main(void) {
    acc = 0x2545f4914f6cdd1dull;
    nrec = 0;
    depth_acc = 7;
    try {
        /* A: virtual dispatch through a base pointer */
        Circle c(5);
        Square sq(9);
        Shape *arr[2] = { &c, &sq };
        for (u64 k = 0; k < 24; k++) {                                            /* 24 */
            try {
                rec(through_base(arr[k & 1], k * 6));
            } catch (Fail &f) {
                rec(0x100 + f.site + (u64)(s64)f.depth);
            } catch (Code &cd) {
                rec(0x200 + (u64)(s64)cd.c);
            }
        }

        /* B: function pointer, and a tail call inside it */
        for (u64 k = 0; k < 12; k++) {                                            /* 12 */
            try {
                rec(call_via(fn_table[k % 3], k));
            } catch (Fail &f) {
                rec(0x400 + f.site);
            } catch (Code &cd) {
                rec(0x500 + (u64)(s64)cd.c);
            }
        }

        /* C: member function called from a member function */
        Engine e;
        for (u64 n = 0; n < 6; n++) {                                             /* 6 */
            e.fuel = 40 + n * 10;
            try {
                rec(e.drive(n + 4));
            } catch (Fail &f) {
                rec(0x600 + f.site * 4 + (u64)(s64)f.depth + e.fuel);
            }
        }

        /* D: 20 levels with a guard each; the throw happens at level 3, 7, 12 and 19 */
        rec(recurse(0, 20, ~0ull));                                               /* 1, no throw */
        for (u64 j = 0; j < 4; j++) {
            u64 at = j == 0 ? 3 : j == 1 ? 7 : j == 2 ? 12 : 19;
            try {
                rec(recurse(0, 25, at));
            } catch (Fail &f) {
                rec(0x700 + f.site);
                rec((u64)(s64)f.depth);
            }                                                                     /* 8 */
        }

        /* E: initialisation-style function called from test_main */
        for (u64 s = 0; s < 6; s++) {                                             /* 6 */
            try {
                rec(init_tables(s + 1));
            } catch (Code &cd) {
                rec(0x800 + (u64)(s64)cd.c + init_table[0]);
            }
        }
        for (u64 i = 0; i < 8; i++) rec(init_table[i]);                           /* 8 */
    } catch (...) {
        rec(0xdeadull);
    }
    rec(depth_acc);
    results[NRES - 1] = acc;
}
