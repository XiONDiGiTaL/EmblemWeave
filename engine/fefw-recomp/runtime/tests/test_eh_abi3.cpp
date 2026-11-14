// SPDX-License-Identifier: GPL-3.0-or-later
// ABI 3 unwinder features on the test_eh.cpp harness (hand-built NxModule tables, host code plays the guest bodies):
//   * exception specifications: NxAction filter < 0 selects the list NxEh.specs[-filter - 1]; an exception outside the list lands the
//     frame with that negative selector, one inside it unwinds on;
//   * imported typeinfo: NxEh.types[i] == 0 with NxEh.type_syms[i] >= 0 resolves the symbol by name through the registry;
//     type_syms[i] == -1 with types[i] == 0 is catch(...); an unresolved import matches nothing.
// Module 1 (index 30, base B3): Fmain [B3+0x100, B3+0x200) with a try around the call at B3+0x140 (pad B3+0x1a0), Fthrow [B3+0x200, B3+0x300).
// Module 2 (index 31, base B4): one dummy function and, in the import tests, the typeinfo symbol _ZTI1A.
// Frame layouts as in test_eh.cpp: Fmain CFA = sp + 32, x30 at CFA-8; Fthrow CFA = sp + 32, x30 at CFA-8, x20 at CFA-16, d8 at CFA-24.
#include <cstring>
#include <string>

#include "nxrt/dispatch.h"
#include "nxrt/eh.h"
#include "nxrt/hooks.h"
#include "nxrt/module.h"
#include "nxrt/nxmodule.h"
#include "test_framework.h"
NXTEST_GROUP(eh_abi3)

using namespace nxrt;

namespace {

constexpr uint64_t B3 = 0x7300000000ull;
constexpr uint64_t B4 = 0x7310000000ull;
constexpr uint64_t kPadMain = B3 + 0x1a0;

// typeinfos: kind Unknown (no vtables configured), identity by name
struct Ti { const void* vptr; const char* name; };
Ti g_ti_a = {nullptr, "1A"}, g_ti_b = {nullptr, "1B"};
const uint64_t kA = reinterpret_cast<uint64_t>(&g_ti_a), kB = reinterpret_cast<uint64_t>(&g_ti_b);

// Shared unwind rows of module 1 (identical in every test; the per-test parts are the actions, types, type_syms, specs and syms)
const NxCfiRow kRows[] = {
    {B3 + 0x100, 31, 1, 32, 0},     // Fmain: x30
    {B3 + 0x200, 31, 3, 32, 1},     // Fthrow: x30, x20, d8
};
const NxCfiSaved kSaved[] = {
    {30, -8},                                   // Fmain
    {30, -8}, {20, -16}, {72, -24},             // Fthrow
};
const NxCallSite kSites[] = {
    {B3 + 0x140, 0x10, kPadMain, 1},            // Fmain: action chain #1 (actions[0])
};
const NxFde kFdes[] = {
    {B3 + 0x100, B3 + 0x200, 0, 1, 0, 1, 0},    // Fmain: one row, one call site, types from 0
    {B3 + 0x200, B3 + 0x300, 1, 1, 0, 0, 0},    // Fthrow: no call sites
};
void fn_noop(::Ctx*) {}
const FnEntry kFns1[] = {{B3 + 0x100, fn_noop}, {B3 + 0x200, fn_noop}};
NxEh g_eh1;
NxModule g_mod1;

// Registers module 1 with the given per-test tables. The tables must outlive the test (static storage in the test body).
bool register_module1(const NxAction* actions, const uint64_t* types, const int32_t* type_syms, const uint32_t* specs, const DynSym* syms, int nsyms) {
    g_eh1 = NxEh{kFdes, 2, kRows, kSaved, kSites, actions, types, type_syms, specs};
    g_mod1 = NxModule{"eh_abi3_main", 30u, B3, B3 + 0x1000, kFns1, 2, syms, nsyms, nullptr, 0, &g_eh1};
    return NXRT_REGISTER_NXMODULE(nullptr, g_mod1).ok();
}

// Registers module 2, which defines _ZTI1A at the address of g_ti_a.
bool register_provider() {
    static const FnEntry fns2[] = {{B4 + 0x100, fn_noop}};
    static const DynSym syms2[] = {{"_ZTI1A", kA, 1}};
    static NxModule mod2 = {"eh_abi3_provider", 31u, B4, B4 + 0x1000, fns2, 1, syms2, 1, nullptr, 0, nullptr};
    return NXRT_REGISTER_NXMODULE(nullptr, mod2).ok();
}

// a guest stack in host memory
alignas(16) uint64_t g_stack[512];
uint64_t stack_top() { return reinterpret_cast<uint64_t>(g_stack + 512); }
void put(uint64_t addr, uint64_t v) { std::memcpy(reinterpret_cast<void*>(addr), &v, 8); }

// observations made by the host "guest code"
struct Seen {
    int body_calls = 0, handler_runs = 0;
    uint64_t pad = 0, x0 = 0, x1 = 0, x19 = 0, x20 = 0, d8 = 0, sp = 0;
    uint64_t sp_expected = 0;
    int thrown_kind = 0;           // what the next Fthrow throws: 0 type A, 1 type B, 2 untyped (foreign)
};
Seen g_seen;

nxrt::eh::Thrown make_thrown(uint64_t exc, uint64_t type) {
    nxrt::eh::Thrown t;
    t.exc = exc;
    t.type = type;
    t.object = exc;
    return t;
}

// Host stand-in for Fthrow: pushes its frame under the caller's call at ret_in_caller, clobbers x20 and d8, raises.
uint64_t guest_throw(::Ctx* c, uint64_t ret_in_caller, uint64_t exc, uint64_t type) {
    uint64_t cfa = c->x[31];                       // caller's sp at the call
    uint64_t sp = cfa - 32;
    put(sp + 32 - 8, ret_in_caller);               // x30
    put(sp + 32 - 16, c->x[20]);                   // x20 as the caller had it
    put(sp + 32 - 24, c->v[8].d[0]);               // d8 as the caller had it
    c->x[31] = sp;
    c->x[20] = 0xdead20;
    c->v[8].d[0] = 0xdead08;
    c->x[30] = B3 + 0x210;                         // return address inside Fthrow
    return nxrt::eh::raise(c, make_thrown(exc, type));
}

// Fmain body: pad 0 = normal entry, else the landing pad.
void fmain_body(::Ctx* c, uint64_t pad) {
    g_seen.body_calls++;
    if (pad) {
        g_seen.handler_runs++;
        g_seen.pad = pad;
        g_seen.x0 = c->x[0];
        g_seen.x1 = c->x[1];
        g_seen.x19 = c->x[19];
        g_seen.x20 = c->x[20];
        g_seen.d8 = c->v[8].d[0];
        g_seen.sp = c->x[31];
        return;
    }
    // prologue: sp -= 32, x30 saved at CFA-8 (return to nothing: the end of the guest stack)
    const uint64_t entry_sp = c->x[31];
    put(entry_sp - 8, 0);
    c->x[31] = entry_sp - 32;
    g_seen.sp_expected = entry_sp - 32;
    c->x[19] = 0x1919;
    c->x[20] = 0x2020;
    c->v[8].d[0] = 0x0808;
    c->x[30] = B3 + 0x144;                         // Fmain calls Fthrow at B3+0x140
    uint64_t type = g_seen.thrown_kind == 0 ? kA : g_seen.thrown_kind == 1 ? kB : 0;
    uint64_t r = guest_throw(c, B3 + 0x144, 0x7777, type);
    // returns only when nothing caught it
    g_seen.x0 = r;
}

void reset_world() {
    modules_unload_all();
    hle_clear();
    hooks_clear();
    nxrt::eh::set_typeinfo_vtables({});
    nxrt::eh::set_abi_autodetect();
    nxrt::eh::stats_reset();
    nxrt::eh::set_terminate_handler(nullptr);
    g_seen = Seen{};
    std::memset(g_stack, 0, sizeof g_stack);
}

constexpr uint64_t kSelectorMinusOne = static_cast<uint64_t>(static_cast<int64_t>(-1));

}  // namespace

NXTEST(spec_violation_lands_with_negative_selector) {
    reset_world();
    static const NxAction actions[] = {{-1, 0}};       // filter -1: NxEh.specs[0]
    static const uint64_t types[] = {kA};
    static const int32_t type_syms[] = {-1};
    static const uint32_t specs[] = {0};              // the empty list: throw()
    REQUIRE(register_module1(actions, types, type_syms, specs, nullptr, 0));
    ::Ctx c{};
    c.x[31] = stack_top();
    g_seen.thrown_kind = 0;                            // type A is not allowed by throw()
    nx_catch_call(&c, fmain_body);
    CHECK_EQ(g_seen.handler_runs, 1);
    CHECK_EQ(g_seen.pad, kPadMain);
    CHECK_EQ(g_seen.x0, 0x7777u);                      // the exception pointer
    CHECK_EQ(g_seen.x1, kSelectorMinusOne);            // selector -1: exception specification violated
    CHECK_EQ(nxrt::eh::stats().caught, 1u);
}

NXTEST(spec_allows_listed_type_and_unwinds_on) {
    reset_world();
    static const NxAction actions[] = {{-1, 0}};
    static const uint64_t types[] = {kA};
    static const int32_t type_syms[] = {-1};
    static const uint32_t specs[] = {1, 0};          // the list {1}: types[0] = A
    // type A is listed: the pad does not run, the exception reaches the bottom of the guest stack
    REQUIRE(register_module1(actions, types, type_syms, specs, nullptr, 0));
    ::Ctx c{};
    c.x[31] = stack_top();
    g_seen.thrown_kind = 0;
    nx_catch_call(&c, fmain_body);
    CHECK_EQ(g_seen.body_calls, 1);
    CHECK_EQ(g_seen.handler_runs, 0);
    CHECK_EQ(g_seen.x0, nxrt::eh::kUrcEndOfStack);     // no handler
    CHECK_EQ(nxrt::eh::stats().caught, 0u);
    CHECK_EQ(nxrt::eh::stats().uncaught, 1u);
    // type B is not listed: violation, the pad runs with the negative selector
    reset_world();
    REQUIRE(register_module1(actions, types, type_syms, specs, nullptr, 0));
    ::Ctx c2{};
    c2.x[31] = stack_top();
    g_seen.thrown_kind = 1;
    nx_catch_call(&c2, fmain_body);
    CHECK_EQ(g_seen.handler_runs, 1);
    CHECK_EQ(g_seen.pad, kPadMain);
    CHECK_EQ(g_seen.x1, kSelectorMinusOne);
}

NXTEST(spec_second_list_selected_by_index) {
    reset_world();
    // specs = {1, 0, 2, 0}: list 0 = {1} (A), list 1 = {2} (B). Filter -3 is index 2 = the second list. (An earlier expectation was filter -2 for
    // this list; by the ABI formula -2 is index 1, which is the empty list inside list 0, so -3 is the value that selects {2, 0}.)
    static const NxAction actions[] = {{-3, 0}};
    static const uint64_t types[] = {kA, kB};
    static const int32_t type_syms[] = {-1, -1};
    static const uint32_t specs[] = {1, 0, 2, 0};
    // throw A: not in {B}, so the spec is violated and the frame lands with -3
    REQUIRE(register_module1(actions, types, type_syms, specs, nullptr, 0));
    ::Ctx c{};
    c.x[31] = stack_top();
    g_seen.thrown_kind = 0;
    nx_catch_call(&c, fmain_body);
    CHECK_EQ(g_seen.handler_runs, 1);
    CHECK_EQ(g_seen.x1, static_cast<uint64_t>(static_cast<int64_t>(-3)));
    // throw B: listed, so no landing
    reset_world();
    REQUIRE(register_module1(actions, types, type_syms, specs, nullptr, 0));
    ::Ctx c2{};
    c2.x[31] = stack_top();
    g_seen.thrown_kind = 1;
    nx_catch_call(&c2, fmain_body);
    CHECK_EQ(g_seen.handler_runs, 0);
    CHECK_EQ(g_seen.x0, nxrt::eh::kUrcEndOfStack);
}

NXTEST(imported_typeinfo_resolves_by_name) {
    reset_world();
    // catch clause filter 1: types[0] = 0, type_syms[0] = 0 -> module 1's symbol 0, "_ZTI1A", which module 1 does not define
    static const NxAction actions[] = {{1, 0}};
    static const uint64_t types[] = {0};
    static const int32_t type_syms[] = {0};
    static const DynSym syms1[] = {{"_ZTI1A", 0, 0}};   // undefined here
    REQUIRE(register_module1(actions, types, type_syms, nullptr, syms1, 1));
    REQUIRE(register_provider());                        // module 2 defines _ZTI1A = kA
    uint64_t resolved = 0;
    CHECK(nxmodule_type_address(nxmodule_by_index(30), &kFdes[0], 1, &resolved));
    CHECK_EQ(resolved, kA);
    ::Ctx c{};
    c.x[31] = stack_top();
    g_seen.thrown_kind = 0;                              // type A
    nx_catch_call(&c, fmain_body);
    CHECK_EQ(g_seen.handler_runs, 1);
    CHECK_EQ(g_seen.pad, kPadMain);
    CHECK_EQ(g_seen.x0, 0x7777u);
    CHECK_EQ(g_seen.x1, 1u);                             // filter 1: the imported catch(A)
}

NXTEST(unresolved_imported_typeinfo_matches_nothing) {
    reset_world();
    static const NxAction actions[] = {{1, 0}};
    static const uint64_t types[] = {0};
    static const int32_t type_syms[] = {0};
    static const DynSym syms1[] = {{"_ZTI1A", 0, 0}};
    REQUIRE(register_module1(actions, types, type_syms, nullptr, syms1, 1));
    // module 2 is not registered: nothing defines _ZTI1A
    uint64_t resolved = 0xff;
    CHECK(!nxmodule_type_address(nxmodule_by_index(30), &kFdes[0], 1, &resolved));
    CHECK_EQ(resolved, 0u);
    ::Ctx c{};
    c.x[31] = stack_top();
    g_seen.thrown_kind = 0;                              // type A: the clause cannot match it
    nx_catch_call(&c, fmain_body);
    CHECK_EQ(g_seen.handler_runs, 0);
    CHECK_EQ(g_seen.x0, nxrt::eh::kUrcEndOfStack);
    CHECK_EQ(nxrt::eh::stats().caught, 0u);
    CHECK_EQ(nxrt::eh::stats().uncaught, 1u);
}

NXTEST(catch_all_slot_with_no_symbol_still_catches_everything) {
    reset_world();
    static const NxAction actions[] = {{1, 0}};
    static const uint64_t types[] = {0};
    static const int32_t type_syms[] = {-1};            // 0 with no symbol: catch(...)
    REQUIRE(register_module1(actions, types, type_syms, nullptr, nullptr, 0));
    ::Ctx c{};
    c.x[31] = stack_top();
    g_seen.thrown_kind = 1;                              // type B
    nx_catch_call(&c, fmain_body);
    CHECK_EQ(g_seen.handler_runs, 1);
    CHECK_EQ(g_seen.pad, kPadMain);
    CHECK_EQ(g_seen.x1, 1u);                             // filter 1 selects the catch(...) slot
}
