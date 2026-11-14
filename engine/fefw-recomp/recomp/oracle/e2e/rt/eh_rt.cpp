// SPDX-License-Identifier: GPL-3.0-or-later
// C++ exception runtime of the end-to-end oracle (test code; the game's runtime has its own, built on the same tables).
//
// The guest program (oracle/e2e/progs/rt.h) calls two host services through fixed addresses, which call_indirect hands
// to eh_host_call: raise(object, typeinfo) and resume(object). This file unwinds the GUEST stack with the NxEh tables
// the lifter wrote into the module descriptor (ctx.h: CFI rows, LSDA call sites, actions, types), picks the landing pad
// like the Itanium personality routine does (cleanup pads are entered, a catch clause matches an identical typeinfo
// pointer or catch(...)), restores the callee-saved registers of that frame and resumes there.
//
// Resuming: a function that has a catch clause (nx_catch_call, see ctx.h) is entered through nx_catch_call, which keeps a
// setjmp record keyed by the guest SP at entry (= the function's CFA): the unwinder longjmps to it and calls
// body(c, landing pad). A function with only cleanup pads is resumed by calling its u_<entry>(c, pad) directly from the
// unwinder; the cleanup ends in _Unwind_Resume, which unwinds further. (After a catch the host stack above the catching
// function's record is discarded by the longjmp.)
#include <stdio.h>
#include <stdlib.h>
#include <setjmp.h>
#include <string.h>
#include <vector>
#include "eh_rt.h"

static const NxModule *g_mod;
static const uint64_t HOST_BASE = 0x7110000000ull;

struct CatchRec {
    jmp_buf jb;
    uint64_t cfa;               // guest SP at entry of the function
    volatile uint64_t pad;
};
static std::vector<CatchRec *> g_catch;
static uint64_t g_cur_type;      // typeinfo of the exception in flight (_Unwind_Resume does not pass it)

void eh_rt_init(const NxModule *m) { g_mod = m; g_catch.clear(); }

extern "C" void nx_catch_call(Ctx *c, void (*body)(Ctx *, uint64_t)) {
    CatchRec rec;
    rec.cfa = c->x[31];
    rec.pad = 0;
    g_catch.push_back(&rec);
    volatile uint64_t pad = 0;
    for (;;) {
        if (setjmp(rec.jb) == 0) {
            body(c, pad);
            break;
        }
        pad = rec.pad;                       // landed here by an exception: run the handler
    }
    for (size_t i = g_catch.size(); i-- > 0;)
        if (g_catch[i] == &rec) { g_catch.resize(i); break; }
}

struct Regs {
    uint64_t pc, sp;
    uint64_t x[31];
    uint64_t d[8];                           // low halves of v8..v15
};

static const NxFde *find_fde(uint64_t pc) {
    const NxEh *eh = g_mod->eh;
    int lo = 0, hi = eh->nfdes - 1;
    while (lo <= hi) {
        int mid = (lo + hi) / 2;
        const NxFde *f = &eh->fdes[mid];
        if (pc < f->start) hi = mid - 1; else if (pc >= f->end) lo = mid + 1; else return f;
    }
    return nullptr;
}

static void fail(const char *what, uint64_t pc) { fprintf(stderr, "eh_rt: %s at %llx\n", what, (unsigned long long)pc); exit(20); }

// One unwind step: the CFA of the frame `r` is in, and the state of its caller.
static uint64_t step(const NxFde *f, Regs *r) {
    const NxEh *eh = g_mod->eh;
    uint64_t pcm1 = r->pc - 1;
    const NxCfiRow *row = nullptr;
    for (uint32_t i = 0; i < f->row_count; i++) {
        const NxCfiRow *q = &eh->rows[f->row_first + i];
        if (q->pc <= pcm1) row = q;
    }
    if (!row) fail("no CFI row", r->pc);
    uint64_t cfa = (row->cfa_reg == 31 ? r->sp : r->x[row->cfa_reg]) + (int64_t)row->cfa_off;
    Regs n = *r;
    bool ra = false;
    for (int i = 0; i < row->nsaved; i++) {
        const NxCfiSaved *s = &eh->saved[row->saved_first + i];
        uint64_t v = *(uint64_t *)(uintptr_t)(cfa + (int64_t)s->off);
        if (s->reg <= 30) { n.x[s->reg] = v; if (s->reg == 30) ra = true; }
        else if (s->reg >= 72 && s->reg < 80) n.d[s->reg - 72] = v;
    }
    (void)ra;
    n.sp = cfa;
    n.pc = n.x[30];
    *r = n;
    return cfa;
}

static bool pick_pad(const NxFde *f, uint64_t pc, uint64_t type, uint64_t *pad, int64_t *selector) {
    const NxEh *eh = g_mod->eh;
    uint64_t pcm1 = pc - 1;
    for (uint32_t i = 0; i < f->site_count; i++) {
        const NxCallSite *s = &eh->sites[f->site_first + i];
        if (pcm1 < s->start || pcm1 >= s->start + s->len) continue;
        if (!s->pad) return false;                      // covered, but no landing pad: the exception goes on
        uint32_t a = s->action;
        if (!a) { *pad = s->pad; *selector = 0; return true; }      // cleanup only
        while (a) {
            const NxAction *act = &eh->actions[a - 1];
            if (act->filter == 0) { *pad = s->pad; *selector = 0; return true; }
            if (act->filter > 0) {
                uint64_t t = eh->types[f->type_first + act->filter - 1];
                if (t == 0 || t == type) { *pad = s->pad; *selector = act->filter; return true; }
            }
            a = act->next;
        }
        return false;
    }
    return false;
}

[[noreturn]] static void land(Ctx *c, const NxFde *f, const Regs &st, uint64_t cfa_of_f, uint64_t pad, int64_t sel, uint64_t obj) {
    for (int i = 19; i <= 30; i++) c->x[i] = st.x[i];
    c->x[31] = st.sp;
    for (int i = 0; i < 8; i++) { c->v[8 + i].d[0] = st.d[i]; c->v[8 + i].d[1] = 0; }
    c->x[0] = obj;
    c->x[1] = (uint64_t)sel;
    for (size_t i = g_catch.size(); i-- > 0;) {
        if (g_catch[i]->cfa == cfa_of_f) {
            g_catch[i]->pad = pad;
            g_catch.resize(i + 1);
            longjmp(g_catch[i]->jb, 1);
        }
    }
    // a function without a catch record: only cleanup pads (or terminate-only pads) are resumed this way
    for (int i = 0; i < g_mod->nresume; i++)
        if (g_mod->resume[i].pad == pad) { g_mod->resume[i].fn(c, pad); fail("cleanup pad returned", pad); }
    (void)f;
    fail("landing pad without a resume entry", pad);
}

bool eh_host_call(Ctx *c, uint64_t addr) {
    if (addr < HOST_BASE || addr >= HOST_BASE + 0x1000) return false;
    int svc = (int)((addr - HOST_BASE) / 16);
    uint64_t obj = c->x[0], type = c->x[1];
    if (!g_mod->eh) fail("module has no unwind tables", addr);
    Regs r;
    r.pc = c->x[30];
    r.sp = c->x[31];
    for (int i = 0; i < 31; i++) r.x[i] = c->x[i];
    for (int i = 0; i < 8; i++) r.d[i] = c->v[8 + i].d[0];
    if (svc != 0 && svc != 1) fail("unknown host service", addr);
    if (svc == 0) g_cur_type = type; else type = g_cur_type;
    // frame 0 is the runtime stub that made this call (__cxa_throw / _Unwind_Resume): unwind out of it
    const NxFde *f = find_fde(r.pc - 1);
    if (!f) fail("no FDE for the stub", r.pc);
    step(f, &r);
    for (int depth = 0; depth < 10000; depth++) {
        if (r.pc == 0) fail("unwound off the stack (no handler)", 0);
        f = find_fde(r.pc - 1);
        if (!f) fail("no FDE for a frame", r.pc);
        Regs here = r;
        uint64_t pad = 0;
        int64_t sel = 0;
        bool found = pick_pad(f, r.pc, type, &pad, &sel);
        Regs next = r;
        uint64_t cfa = step(f, &next);
        if (found) land(c, f, here, cfa, pad, sel, obj);
        r = next;
    }
    fail("unwind depth exceeded", 0);
    return true;
}
