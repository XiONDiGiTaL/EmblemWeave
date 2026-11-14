/* SPDX-License-Identifier: GPL-3.0-or-later
 * Guest side of the end-to-end oracle's C++ exception support (EH tests only; include AFTER t.h).
 *
 * The test programs have no libc++abi. This header supplies, in the program itself, the few entry points the compiler's
 * exception code calls (__cxa_allocate_exception, __cxa_throw, __cxa_begin_catch, __cxa_end_catch, __cxa_rethrow,
 * _Unwind_Resume, the personality symbol, typeinfo objects), and routes the two operations that need the unwinder to HOST
 * SERVICES: a call through a function pointer at a fixed address (HOST_BASE + 16 * n). Under Unicorn the harness
 * (unicorn_side.py) intercepts that address; on x64 the lifted call lands in call_indirect, which hands it to
 * oracle/e2e/rt/eh_rt.cpp. Both unwind the GUEST stack from the .eh_frame data of the ELF: they walk the CFI of each frame,
 * look at its LSDA call sites, and resume at the landing pad with x0 = exception object, x1 = selector.
 *
 *   host service 0  raise(object, typeinfo)    start unwinding at the caller of __cxa_throw; does not return
 *   host service 1  resume(object)             continue unwinding after a cleanup landing pad (_Unwind_Resume)
 */
#ifndef NX_RT_H
#define NX_RT_H
#include "t.h"

#define HOST_BASE 0x7110000000ull
typedef u64 (*host_fn_t)(u64, u64, u64);
#define HOST_CALL(n, a, b, c) (((host_fn_t)(HOST_BASE + 16ull * (n)))((u64)(a), (u64)(b), (u64)(c)))

extern "C" {
/* typeinfo objects of the builtin types the tests throw (the compiler references them by these names) */
char _ZTIi[16], _ZTIj[16], _ZTIl[16], _ZTIm[16], _ZTIx[16], _ZTIy[16], _ZTIc[16], _ZTIh[16], _ZTIs[16], _ZTIt[16],
     _ZTIf[16], _ZTId[16], _ZTIPKc[16], _ZTIPc[16];
/* vtable the compiler's typeinfo of a user class points at */
char _ZTVN10__cxxabiv117__class_type_infoE[64], _ZTVN10__cxxabiv120__si_class_type_infoE[64], _ZTVN10__cxxabiv121__vmi_class_type_infoE[64];

static char eh_heap[1 << 16];
static u64 eh_heap_off;
static void *eh_caught;
static void *eh_caught_type;

__attribute__((noinline)) void *__cxa_allocate_exception(unsigned long size) {
    u64 p = (eh_heap_off + 15) & ~15ull;
    eh_heap_off = p + size + 16;
    return eh_heap + p;
}
void __cxa_free_exception(void *) {}
__attribute__((noreturn, noinline)) void __cxa_throw(void *obj, void *tinfo, void (*dtor)(void *)) {
    (void)dtor;
    eh_caught = obj;
    eh_caught_type = tinfo;
    HOST_CALL(0, obj, tinfo, 0);
    for (;;) {}                                   /* no handler found: the test has no business here */
}
__attribute__((noinline)) void *__cxa_begin_catch(void *obj) { eh_caught = obj; return obj; }
__attribute__((noinline)) void __cxa_end_catch(void) {}
__attribute__((noreturn, noinline)) void __cxa_rethrow(void) {
    HOST_CALL(0, eh_caught, eh_caught_type, 0);
    for (;;) {}
}
__attribute__((noreturn, noinline)) void _Unwind_Resume(void *obj) {
    HOST_CALL(1, obj, 0, 0);
    for (;;) {}
}
int __gxx_personality_v0(int, int, u64, void *, void *) { return 0; }      /* referenced by the FDEs, never called */
__attribute__((noreturn)) void _ZSt9terminatev(void) { for (;;) {} }
void __cxa_call_unexpected(void *) { for (;;) {} }
}
#endif
