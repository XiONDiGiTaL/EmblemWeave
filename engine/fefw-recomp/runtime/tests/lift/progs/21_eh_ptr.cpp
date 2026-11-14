// FLAGS: -fexceptions -funwind-tables -frtti
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_throw>
// EXPECT-INSN: \bbl\s+0x[0-9a-f]+ <__cxa_begin_catch>
/* 21_eh_ptr: catching POINTER types. A thrown pointer is matched against each handler's pointer type by the pointer conversions
 * the standard allows, and the caught pointer carries the adjusted address.
 *
 * What ISO C++ requires, case by case (written before the program was run):
 *  [except.throw]p3   the exception object has the operand's type after array-to-pointer conversion: throw "text" is const char*.
 *  [except.handle]p3  a handler of type cv T* matches a thrown pointer E if E converts to cv T* by a standard pointer conversion
 *                     (derived-to-base, to void*) or by a qualification conversion. A handler T* also matches a thrown nullptr_t.
 *                     A function pointer does not convert to void*, so catch(void*) does not catch a thrown void(*)().
 *  [conv.ptr]p2       derived-to-base conversion moves the address to the base subobject (second base and virtual base too);
 *                     a null pointer stays null.
 *  [except.handle]p3  handlers are tried in order; the first match runs; no match falls through to catch(...).
 *
 * Per case, the expected recorded values:
 *  1  Derived* -> Base* matches (b=11, same address); -> const Base* matches (11); -> void* matches (same address);
 *     -> const void* matches. Base* -> Derived* does not match (catch(...) runs). With catch(void*) listed before catch(Base*),
 *     the void* handler wins for a Derived*.
 *  2  int* -> const int* matches (31). const int* -> int* does not match: the const int* handler runs (32).
 *     "text" (const char*): catch(char*) does not match, catch(const char*) runs ('t', 't').
 *  3  M* (struct M : P, Q) -> Q* is adjusted to the Q subobject (q=102, address &g_m.q); -> P* (p=101, same address); -> M* (m=103).
 *     Polymorphic MV : PV, QV: MV* -> QV* adjusted, virtual call qv() gives 1202; MV* -> PV* gives pv()=201.
 *  4  D* (L : virtual V) -> V* adjusted through the virtual base offset (v=301); -> L* (l=302); -> D* (d=303).
 *  5  (Derived*)nullptr -> Base* handler runs with a null pointer; (MV*)nullptr -> QV* handler runs with a null pointer
 *     (throw nullptr is dropped, see the comment at case 5).
 *  6  int** -> int** exact (41). (int** -> const int* const* is dropped, see the comment at case 6.)
 *  7  void(*)() -> void* does not match (skipped); void(*)() handler runs and calling the caught pointer sets the hook flag to 1.
 *  8  M* to a static global, thrown through two frames: Q* is adjusted (q=802, same address); the object is readable after unwinding.
 */
#include "rt2.h"

static u64 acc;          /* order-sensitive hash of every recorded value */
static u64 nrec;

static void mix(u64 v) { acc = (acc ^ v) * 0x100000001b3ull; acc ^= acc >> 29; }
static void rec(u64 v) { mix(v); if (nrec < NRES - 2) results[nrec++] = v; }

/* ---- case 1: single inheritance ----------------------------------------------------------------------------------------------- */
struct Base { int b; };
struct Derived : Base { int d; };
static Derived g_der;    /* static storage: the object outlives every throw */
static Base g_base;

static NOINLINE void throw_derived(Derived *p) { throw p; }
static NOINLINE void throw_base(Base *p) { throw p; }

static NOINLINE void case1(void) {
    g_der.b = 11; g_der.d = 22; g_base.b = 13;
    try { throw_derived(&g_der); }
    catch (Base *p) { rec(0x101); rec((u64)p->b); rec(p == &g_der); }
    try { throw_derived(&g_der); }
    catch (const Base *p) { rec(0x102); rec((u64)p->b); }
    try { throw_derived(&g_der); }
    catch (void *p) { rec(0x103); rec(p == (void *)&g_der); }
    try { throw_derived(&g_der); }
    catch (const void *p) { rec(0x104); rec(p == (const void *)&g_der); }
    try { throw_base(&g_base); }                  /* no downcast: Derived* handler must not match */
    catch (Derived *p) { rec(0x1ee); rec((u64)p->d); }
    catch (...) { rec(0x1ff); }
    try { throw_derived(&g_der); }                /* void* is first in order and matches a Derived* */
    catch (void *p) { rec(0x105); rec(p == (void *)&g_der); }
    catch (Base *p) { rec(0x106); rec((u64)p->b); }
}

/* ---- case 2: qualification conversions and string literals ---------------------------------------------------------------------- */
static int g_i = 31;
static const int g_ci = 32;

static NOINLINE void throw_int_ptr(int *p) { throw p; }
static NOINLINE void throw_cint_ptr(const int *p) { throw p; }
static NOINLINE void throw_str(void) { throw "text"; }

static NOINLINE void case2(void) {
    try { throw_int_ptr(&g_i); }
    catch (const int *p) { rec(0x201); rec((u64)*p); }
    try { throw_cint_ptr(&g_ci); }
    catch (int *p) { rec(0x202); rec((u64)*p); }         /* must NOT run */
    catch (const int *p) { rec(0x203); rec((u64)*p); }
    try { throw_str(); }
    catch (char *s) { rec(0x204); rec((u64)s[0]); }     /* must NOT run: the thrown type is const char* */
    catch (const char *s) { rec(0x205); rec((u64)s[0]); rec((u64)s[3]); }
}

/* ---- case 3: multiple inheritance, pointer adjustment ----------------------------------------------------------------------- */
struct P { int p; };
struct Q { int q; };
struct M : P, Q { int m; };
static M g_m;
static NOINLINE void throw_m(M *p) { throw p; }

struct PV { int p; virtual int pv(void) const { return p; } };
struct QV { int q; virtual int qv(void) const { return q + 1000; } };
struct MV : PV, QV { int m; };
static NOINLINE void throw_mv(MV *p) { throw p; }

static NOINLINE void case3(void) {
    g_m.p = 101; g_m.q = 102; g_m.m = 103;
    try { throw_m(&g_m); }
    catch (Q *q) { rec(0x301); rec((u64)q->q); rec(q == (Q *)&g_m); }
    try { throw_m(&g_m); }
    catch (P *p) { rec(0x302); rec((u64)p->p); rec(p == (P *)&g_m); }
    try { throw_m(&g_m); }
    catch (M *m) { rec(0x303); rec((u64)m->m); }

    MV mv; mv.p = 201; mv.q = 202; mv.m = 203;
    try { throw_mv(&mv); }
    catch (QV *qv) { rec(0x304); rec((u64)qv->qv()); rec((u64)qv->q); rec(qv == (QV *)&mv); }
    try { throw_mv(&mv); }
    catch (PV *pv) { rec(0x305); rec((u64)pv->pv()); rec(pv == (PV *)&mv); }
}

/* ---- case 4: virtual base, pointer adjustment through the vbase offset -------------------------------------------------------- */
struct V { int v; };
struct L : virtual V { int l; };
struct D : L { int d; };
static NOINLINE void throw_d(D *p) { throw p; }

static NOINLINE void case4(void) {
    D d; d.v = 301; d.l = 302; d.d = 303;
    try { throw_d(&d); }
    catch (V *v) { rec(0x401); rec((u64)v->v); rec(v == (V *)&d); }
    try { throw_d(&d); }
    catch (L *l) { rec(0x402); rec((u64)l->l); rec(l == (L *)&d); }
    try { throw_d(&d); }
    catch (D *x) { rec(0x403); rec((u64)x->d); }
}

/* ---- case 5: null pointers and nullptr_t ----------------------------------------------------------------------------------- */
static NOINLINE void throw_null_derived(void) { throw (Derived *)nullptr; }
static NOINLINE void throw_null_mv(void) { throw (MV *)nullptr; }
/* throw nullptr (std::nullptr_t) is NOT in this program: the standard and the Itanium ABI say a pointer handler matches it, but the
 * native reference (clang-cl, MSVC ABI) lets it escape every pointer handler, so it cannot supply the expected output. */

static NOINLINE void case5(void) {
    try { throw_null_derived(); }
    catch (Base *p) { rec(0x501); rec(p == nullptr); }
    try { throw_null_mv(); }
    catch (QV *p) { rec(0x502); rec(p == nullptr); }
}

/* ---- case 6: pointer to pointer --------------------------------------------------------------------------------------------- */
static int g_i2 = 41;
static int *g_pi = &g_i2;
static NOINLINE void throw_pp(int **pp) { throw pp; }

/* int** -> const int* const* (a qualification conversion, which the standard says matches) is NOT in this program: the native
 * reference (clang-cl, MSVC ABI) lets it escape, so it cannot supply the expected output. */
static NOINLINE void case6(void) {
    try { throw_pp(&g_pi); }
    catch (int **pp) { rec(0x601); rec((u64)**pp); }
}

/* ---- case 7: function pointer is not caught by void* ------------------------------------------------------------------------ */
static int g_hook;
static void hook_fn(void) { g_hook += 1; }
static NOINLINE void throw_fn(void) { throw &hook_fn; }

static NOINLINE void case7(void) {
    try { throw_fn(); }
    catch (void *p) { rec(0x701); rec(p == (void *)0); }   /* must NOT run */
    catch (void (*fp)()) { rec(0x702); g_hook = 0; fp(); rec((u64)g_hook); }
}

/* ---- case 8: a pointer to a static global, thrown through two frames -------------------------------------------------------- */
static NOINLINE void deep2(M *p) { throw p; }
static NOINLINE void deep1(M *p) { deep2(p); }

static NOINLINE void case8(void) {
    g_m.q = 802;
    try { deep1(&g_m); }
    catch (Q *q) { rec(0x801); rec((u64)q->q); rec(q == (Q *)&g_m); }
}

extern "C" void test_main(void) {
    acc = 0x9e3779b97f4a7c15ull;
    nrec = 0;
    try {
        case1();
        case2();
        case3();
        case4();
        case5();
        case6();
        case7();
        case8();
        rec(nrec);
    } catch (...) {
        rec(0xdeadull);
    }
    results[NRES - 1] = acc;
}
