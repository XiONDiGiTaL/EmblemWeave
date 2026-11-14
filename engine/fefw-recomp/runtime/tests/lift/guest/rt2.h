/* SPDX-License-Identifier: GPL-3.0-or-later
 * Guest side of the runtime's C++ exception tests (tests/lift): a miniature libc++abi, freestanding, single-threaded.
 *
 * It follows LLVM libc++abi (cxa_exception.cpp, Apache-2.0 with LLVM exception, written from its behaviour and the Itanium C++ ABI):
 * the same __cxa_exception header layout (LP64, 0x80 bytes, _Unwind_Exception at +0x60, thrown object at +0x80), the same
 * handlerCount / caught-exception-stack protocol in __cxa_begin_catch / __cxa_end_catch / __cxa_rethrow, and the same
 * contract with the unwinder: __cxa_throw calls _Unwind_RaiseException(&header->unwindHeader) and, if that RETURNS (no handler),
 * calls __cxa_begin_catch and std::terminate(); the personality routine (here: the runtime) leaves adjustedPtr in the header and
 * __cxa_begin_catch returns it.
 *
 * Three functions are only stubs in the program; the test driver (tests/lift/driver.cpp) puts entry hooks on them
 * (hooks.h; the lifter builds them with `entryhook` site lines, tests/tools/make_sites.py):
 *   _Unwind_RaiseException(ue)    -> nxrt::eh::hook_unwind_raise_exception
 *   _Unwind_Resume(ue)            -> nxrt::eh::hook_unwind_resume
 *   nx_host_terminate()           -> the driver's terminate handler (log, guest backtrace, exit code)
 * Programs include this header AFTER nothing else (it includes t.h) and use results[] for everything they want compared.
 * tests/lift/native/rt2.h is the hosted stand-in with the host's own exceptions: same program text, expected results.
 *
 * State is per thread (__thread), like libc++abi's __cxa_get_globals: programs may run on several guest threads.
 *
 * Not available in a program: libc, libstdc++/libc++ headers, new/delete, typeid, dynamic_cast, global constructors. Use
 * `static` plain-data globals and placement of objects on the stack.
 */
#ifndef NX_RT2_H
#define NX_RT2_H
#include "t.h"

namespace std { [[noreturn]] void terminate() noexcept; }

extern "C" {

/* ---- typeinfo vtables and the typeinfos libc++abi provides -------------------------------------------------------------------- */
/* The compiler's typeinfos of user types point at (these symbols + 16). The runtime tells the kind of a typeinfo by that address. */
void* _ZTVN10__cxxabiv117__class_type_infoE[8];
void* _ZTVN10__cxxabiv120__si_class_type_infoE[8];
void* _ZTVN10__cxxabiv121__vmi_class_type_infoE[8];
void* _ZTVN10__cxxabiv119__pointer_type_infoE[8];
void* _ZTVN10__cxxabiv123__fundamental_type_infoE[8];
void* _ZTVN10__cxxabiv116__enum_type_infoE[8];
void* _ZTVN10__cxxabiv117__array_type_infoE[8];
void* _ZTVN10__cxxabiv120__function_type_infoE[8];
void* _ZTVN10__cxxabiv129__pointer_to_member_type_infoE[8];

struct NxTypeInfo { const void* vptr; const char* name; };
struct NxPointerTypeInfo { const void* vptr; const char* name; unsigned flags; const void* pointee; };
#define NX_FUND_TI(mangled, nm) NxTypeInfo _ZTI##mangled = { _ZTVN10__cxxabiv123__fundamental_type_infoE + 2, nm };
NX_FUND_TI(v, "v") NX_FUND_TI(Dn, "Dn") NX_FUND_TI(b, "b") NX_FUND_TI(w, "w") NX_FUND_TI(c, "c") NX_FUND_TI(a, "a") NX_FUND_TI(h, "h")
NX_FUND_TI(s, "s") NX_FUND_TI(t, "t") NX_FUND_TI(i, "i") NX_FUND_TI(j, "j") NX_FUND_TI(l, "l") NX_FUND_TI(m, "m") NX_FUND_TI(x, "x")
NX_FUND_TI(y, "y") NX_FUND_TI(f, "f") NX_FUND_TI(d, "d") NX_FUND_TI(Di, "Di") NX_FUND_TI(Ds, "Ds")
#define NX_PTR_TI(mangled, nm, fl, pointee) NxPointerTypeInfo _ZTI##mangled = { _ZTVN10__cxxabiv119__pointer_type_infoE + 2, nm, fl, &pointee };
NX_PTR_TI(Pv, "Pv", 0, _ZTIv) NX_PTR_TI(PKv, "PKv", 1, _ZTIv) NX_PTR_TI(Pc, "Pc", 0, _ZTIc) NX_PTR_TI(PKc, "PKc", 1, _ZTIc)
NX_PTR_TI(Pi, "Pi", 0, _ZTIi) NX_PTR_TI(PKi, "PKi", 1, _ZTIi) NX_PTR_TI(Pl, "Pl", 0, _ZTIl) NX_PTR_TI(Pj, "Pj", 0, _ZTIj)

/* ---- the unwinder interface ---------------------------------------------------------------------------------------------------- */
struct NxUnwindException {                        /* _Unwind_Exception, LP64 */
    unsigned long long exception_class;
    void (*exception_cleanup)(int, NxUnwindException*);
    unsigned long long private_1, private_2;
} __attribute__((aligned(16)));

/* Stubs: the runtime replaces them by entry hooks (see the file comment). _URC_END_OF_STACK = 5 is what a failed raise answers.
 * optnone: the compiler must not learn what the stub does (it would delete the call of a function that "only returns 5"). */
#define NX_OPAQUE __attribute__((noinline, optnone))
NX_OPAQUE int _Unwind_RaiseException(NxUnwindException* ue) { (void)ue; return 5; }
NX_OPAQUE __attribute__((noreturn)) void _Unwind_Resume(NxUnwindException* ue) { (void)ue; for (;;) {} }
NX_OPAQUE __attribute__((noreturn)) void nx_host_terminate(void) { for (;;) {} }
int __gxx_personality_v0(int, int, unsigned long long, void*, void*) { return 0; }   /* named by the FDEs, never called */

/* ---- __cxa_exception ----------------------------------------------------------------------------------------------------------- */
struct NxCxaException {
    void* reserve;
    unsigned long referenceCount;
    void* exceptionType;
    void (*exceptionDestructor)(void*);
    void (*unexpectedHandler)();
    void (*terminateHandler)();
    NxCxaException* nextException;
    int handlerCount;
    int handlerSwitchValue;
    const unsigned char* actionRecord;
    const unsigned char* languageSpecificData;
    void* catchTemp;
    void* adjustedPtr;
    NxUnwindException unwindHeader;
};
static_assert(sizeof(NxCxaException) == 0x80, "the runtime assumes libc++abi's LP64 __cxa_exception layout");
static_assert(__builtin_offsetof(NxCxaException, unwindHeader) == 0x60, "unwindHeader at 0x60");

static const unsigned long long kNxClangClass = 0x434C4E47432B2B00ull;    /* "CLNGC++\0"; low byte 1 = dependent */

struct NxEhGlobals { NxCxaException* caughtExceptions; unsigned uncaughtExceptions; };
#define NX_TLS __thread        /* per guest thread: each thread has its own caught-exception stack and exception arena (libc++abi: __cxa_get_globals) */
static NX_TLS NxEhGlobals nx_globals;

/* ---- exception memory: first-fit over a static arena with exact-size free blocks ---------------------------------------------- */
struct NxBlock { unsigned long size; NxBlock* next; };
static NX_TLS unsigned char nx_arena[1 << 17] __attribute__((aligned(16)));
static NX_TLS unsigned long nx_arena_top;
static NX_TLS NxBlock* nx_free_list;

static inline NxCxaException* nx_header_of(void* thrown_object) { return (NxCxaException*)thrown_object - 1; }
static inline NxCxaException* nx_header_of_unwind(NxUnwindException* ue) { return (NxCxaException*)((char*)ue - __builtin_offsetof(NxCxaException, unwindHeader)); }

__attribute__((noinline)) void* __cxa_allocate_exception(unsigned long thrown_size) {
    unsigned long total = (sizeof(NxCxaException) + thrown_size + 15) & ~15ul;
    char* block = 0;
    for (NxBlock** pp = &nx_free_list; *pp; pp = &(*pp)->next)
        if ((*pp)->size == total) { block = (char*)*pp; *pp = (*pp)->next; for (unsigned long i = 0; i < total; i++) block[i] = 0; break; }
    if (!block) {
        block = (char*)nx_arena + nx_arena_top;                           /* the arena starts zeroed */
        nx_arena_top += total;
        if (nx_arena_top > sizeof nx_arena) for (;;) {}                    /* arena exhausted: the program leaks too many exceptions */
    }
    ((NxCxaException*)block)->reserve = (void*)total;                      /* the header's padding word keeps the block size */
    return block + sizeof(NxCxaException);
}
__attribute__((noinline)) void __cxa_free_exception(void* thrown_object) {
    NxCxaException* h = nx_header_of(thrown_object);
    NxBlock* b = (NxBlock*)h;
    b->size = (unsigned long)h->reserve;
    b->next = nx_free_list;
    nx_free_list = b;
}

/* ---- reference counting and destruction ---------------------------------------------------------------------------------------- */
static void nx_exception_cleanup(int reason, NxUnwindException* ue);
__attribute__((noinline)) void __cxa_decrement_exception_refcount(void* thrown_object) {
    NxCxaException* h = nx_header_of(thrown_object);
    if (--h->referenceCount == 0) {
        if (h->exceptionDestructor) h->exceptionDestructor(thrown_object);
        __cxa_free_exception(thrown_object);
    }
}
static void nx_exception_cleanup(int reason, NxUnwindException* ue) {
    NxCxaException* h = nx_header_of_unwind(ue);
    (void)reason;
    __cxa_decrement_exception_refcount(h + 1);
}

/* ---- throw, catch, rethrow ----------------------------------------------------------------------------------------------------- */
__attribute__((noinline)) void* __cxa_begin_catch(void* unwind_arg) {
    NxUnwindException* ue = (NxUnwindException*)unwind_arg;
    NxCxaException* h = nx_header_of_unwind(ue);
    if ((ue->exception_class & 0xFFFFFFFFFFFFFF00ull) != kNxClangClass) return (void*)(ue + 1);          /* foreign: not used by the tests */
    h->handlerCount = h->handlerCount < 0 ? -h->handlerCount + 1 : h->handlerCount + 1;
    if (h != nx_globals.caughtExceptions) {
        h->nextException = nx_globals.caughtExceptions;
        nx_globals.caughtExceptions = h;
    }
    nx_globals.uncaughtExceptions -= 1;
    return h->adjustedPtr;
}
__attribute__((noinline)) void* __cxa_get_exception_ptr(void* unwind_arg) {
    return nx_header_of_unwind((NxUnwindException*)unwind_arg)->adjustedPtr;
}
__attribute__((noinline)) void __cxa_end_catch(void) {
    NxCxaException* h = nx_globals.caughtExceptions;
    if (!h) return;
    if (h->handlerCount < 0) {                         /* rethrown: count up toward zero; the exception lives on elsewhere */
        if (++h->handlerCount == 0) nx_globals.caughtExceptions = h->nextException;
    } else if (--h->handlerCount == 0) {
        nx_globals.caughtExceptions = h->nextException;
        __cxa_decrement_exception_refcount(h + 1);
    }
}
/* optnone: in libc++abi these live in another translation unit, so a caller must assume they unwind. Here the compiler would see the body,
 * infer nounwind and delete every landing pad. */
[[noreturn]] NX_OPAQUE void __cxa_throw(void* thrown_object, void* tinfo, void (*dest)(void*)) {
    NxCxaException* h = nx_header_of(thrown_object);
    h->referenceCount = 1;
    h->exceptionType = tinfo;
    h->exceptionDestructor = dest;
    h->unwindHeader.exception_class = kNxClangClass;
    h->unwindHeader.exception_cleanup = nx_exception_cleanup;
    nx_globals.uncaughtExceptions += 1;
    _Unwind_RaiseException(&h->unwindHeader);
    __cxa_begin_catch(&h->unwindHeader);                /* no handler: libc++abi marks the exception caught and terminates */
    std::terminate();
}
[[noreturn]] NX_OPAQUE void __cxa_rethrow(void) {
    NxCxaException* h = nx_globals.caughtExceptions;
    if (!h) std::terminate();
    h->handlerCount = -h->handlerCount;                 /* mark it rethrown */
    nx_globals.uncaughtExceptions += 1;
    _Unwind_RaiseException(&h->unwindHeader);
    __cxa_begin_catch(&h->unwindHeader);
    std::terminate();
}
void __cxa_call_unexpected(void*) { std::terminate(); }
void __cxa_pure_virtual(void) { std::terminate(); }
}

namespace std { [[noreturn]] void terminate() noexcept { nx_host_terminate(); } }
/* the deleting destructors of virtual classes name these; a program never calls them (no heap) */
void operator delete(void*) noexcept { std::terminate(); }
void operator delete(void*, unsigned long) noexcept { std::terminate(); }
#endif
