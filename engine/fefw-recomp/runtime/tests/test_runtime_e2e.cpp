// SPDX-License-Identifier: GPL-3.0-or-later
// End-to-end: runtime bring-up, the fixture module mapped and linked, lifted-code stand-ins calling the ABI functions the generated code
// would call (call_indirect, host_import, guest_svc, guest_mrs, stx_diag), a guest thread started through SVCs, an IPC round trip to
// sm: and a stub service, the unknown-SVC report, and the end-of-run summary.
#include <windows.h>

#include <atomic>
#include <cstring>

#include "nxrt/diag.h"
#include "nxrt/dispatch.h"
#include "nxrt/hipc.h"
#include "nxrt/ipc.h"
#include "nxrt/kernel.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"
#include "nxrt/module.h"
#include "nxrt/runtime.h"
#include "nxrt/svc.h"
#include "test_framework.h"
NXTEST_GROUP(runtime_e2e)

using namespace nxrt;

namespace {

constexpr uint64_t kFnA = 0x2000, kFnB = 0x2008, kFnC = 0x2010, kGotAdd = 0x5208;

std::atomic<int> g_thread_ran{0};
std::atomic<uint64_t> g_thread_arg{0};
std::atomic<uint64_t> g_import_result{0};
std::atomic<uint32_t> g_cmif_result{0xFFFFFFFF};
std::atomic<uint32_t> g_session_handle{0};
std::atomic<int> g_main_ok{0};
long g_data_import = 0;
uint64_t g_base = 0;

void host_hle_add(::Ctx* c) { c->x[0] = static_cast<uint32_t>(c->x[0] + c->x[1]); }

// Stand-in for a lifted function that makes an import call through the generated-code ABI.
void lifted_fn_a(::Ctx* c) {
    c->fn = g_base + kFnA;                 // the lifted prologue records the function it is in
    c->x[0] = 40;
    c->x[1] = 2;
    host_import(c, NX_IMPORT_ID(0, 0));    // module index 0, dyn_syms[0] = hle_add
    g_import_result = c->x[0];
    c->x[30] = 0x1234;
    // indirect call through the table, as BLR does
    ::Ctx c2{};
    c2.x[0] = 5;
    call_indirect(&c2, g_base + kFnB);
    g_import_result = g_import_result * 1000 + c2.x[0];
}
void lifted_fn_b(::Ctx* c) { c->x[0] = c->x[0] * 2 + 1; }

// A guest thread entry: uses SVCs as the recompiled SDK would.
void thread_entry(::Ctx* c) {
    g_thread_arg = c->x[0];
    c->x[0] = 0;
    guest_svc(c, 0x1E);                                  // GetSystemTick
    uint64_t tick = c->x[0];
    c->x[0] = static_cast<uint64_t>(-1);
    c->x[0] = 1000000;                                   // SleepThread(1 ms)
    guest_svc(c, 0x0B);
    c->x[0] = 0;
    guest_svc(c, 0x1E);
    if (c->x[0] > tick && kernel::current_thread() && kernel::current_thread()->name == "e2e-thread") g_thread_ran = 1;
    c->x[0] = guest_mrs(c, 0x1234) + 7;                  // not modelled system register: 0
    c->x[0] = 0;
    for (int i = 0; i < 3; i++) stx_diag(c, 0x1000, 1, 2, 3, 32);
    c->x[0] = 0x7777;
    guest_svc(c, 0x38);                                  // an SVC nobody implements: logged, 0x4201
    g_main_ok = c->x[0] == kernel::kNotImplemented ? 1 : 0;
}

uint32_t send(::Ctx* c, uint32_t handle, hipc::Message& m, uint32_t* buf) {
    std::memset(buf, 0, 0x100);
    size_t n = hipc::build(m, buf, 0x100 / 4);
    if (!n) return 0xDEAD;
    c->tpidrro = reinterpret_cast<uint64_t>(buf);
    c->x[0] = handle;
    svc_dispatch(c, 0x21);
    return static_cast<uint32_t>(c->x[0]);
}

hipc::Message cmif_request(uint32_t method, const void* payload, size_t n) {
    hipc::Message m;
    m.type = 4;
    uint32_t hdr[4] = {0x49434653u, 0, method, 0};
    m.raw.assign(reinterpret_cast<uint8_t*>(hdr), reinterpret_cast<uint8_t*>(hdr) + 16);
    m.raw.insert(m.raw.end(), static_cast<const uint8_t*>(payload), static_cast<const uint8_t*>(payload) + n);
    return m;
}

}  // namespace

NXTEST(bring_up_load_run_ipc_shutdown) {
    modules_unload_all();
    hle_clear();
    kernel::kernel_reset();
    ipc::registry_reset();
    mem::shutdown();
    svc_reset_counters();

    InitOptions io;
    io.ini_path = "";
    io.crash_handler = false;
    CHECK(init(io));
    CHECK(initialised());

    // module: map + link the fixture, register stand-ins as lifted code, provide the imports
    module_provide_data("hle_data_import", reinterpret_cast<uint64_t>(&g_data_import));
    hle_register("hle_add", host_hle_add);
    Module* m = module_load_file(NXRT_TEST_DATA_DIR "/fixture_aarch64.elf");
    REQUIRE(m);
    CHECK(module_link_all());
    g_base = m->base();
    FnEntry fns[] = {{m->base() + kFnA, lifted_fn_a}, {m->base() + kFnB, lifted_fn_b}};
    static DynSym syms[] = {{"hle_add", 0, 0}};
    NxModule nx = {"fixture", 0u, m->base(), m->base() + 0x3000, fns, 2, syms, 1, nullptr, 0, nullptr};
    CHECK_EQ(module_register_nxmodule(m, &nx), 2u);

    // 1. lifted code calling an import and an indirect call
    ::Ctx c{};
    call_indirect(&c, m->base() + kFnA);
    CHECK_EQ(g_import_result.load(), 42u * 1000 + 11u);     // hle_add(40, 2) = 42; fn_b(5) = 11

    // 2. a guest thread created and started through SVCs
    kernel::main_thread_adopt(0x10000);
    fn_register(0x7100500000, thread_entry);
    const uint64_t stack = mem::alloc_kernel(0x10000);
    ::Ctx s{};
    s.x[1] = 0x7100500000;
    s.x[2] = 0xABCDEF;
    s.x[3] = stack + 0x10000;
    s.x[4] = 44;
    s.x[5] = static_cast<uint64_t>(-2);
    svc_dispatch(&s, 0x08);                                   // CreateThread
    REQUIRE(s.x[0] == 0);
    uint32_t th = static_cast<uint32_t>(s.x[1]);
    kernel::handle_as<kernel::KThread>(th)->name = "e2e-thread";
    s.x[0] = th;
    svc_dispatch(&s, 0x09);                                   // StartThread
    CHECK_EQ(s.x[0], 0u);
    uint32_t handles[1] = {th};
    s.x[1] = reinterpret_cast<uint64_t>(handles);
    s.x[2] = 1;
    s.x[3] = static_cast<uint64_t>(10'000'000'000ll);
    svc_dispatch(&s, 0x18);                                   // WaitSynchronization on the thread: returns when it exits
    CHECK_EQ(s.x[0], 0u);
    CHECK_EQ(g_thread_arg.load(), 0xABCDEFu);
    CHECK_EQ(g_thread_ran.load(), 1);
    CHECK_EQ(g_main_ok.load(), 1);
    CHECK(svc_unknown_total() >= 1);
    kernel::main_thread_release();

    // 3. IPC: connect to sm:, register the client, get a service that has no implementation (stub), call a command on it
    alignas(16) static uint32_t buf[0x40];
    const char name[] = "sm:";
    s.x[1] = reinterpret_cast<uint64_t>(name);
    svc_dispatch(&s, 0x1F);
    REQUIRE(s.x[0] == 0);
    uint32_t sm = static_cast<uint32_t>(s.x[1]);
    uint64_t zero = 0;
    hipc::Message reg = cmif_request(0, &zero, 8);
    reg.has_pid = true;
    CHECK_EQ(send(&s, sm, reg, buf), 0u);
    char svcname[8] = "e2e:svc";
    hipc::Message get = cmif_request(1, svcname, 8);
    CHECK_EQ(send(&s, sm, get, buf), 0u);
    hipc::Message rsp;
    REQUIRE(hipc::parse(buf, 0x40, rsp));
    REQUIRE(rsp.move.size() == 1);
    uint32_t stub_session = rsp.move[0];
    hipc::Message cmd = cmif_request(77, nullptr, 0);
    CHECK_EQ(send(&s, stub_session, cmd, buf), 0u);
    REQUIRE(hipc::parse(buf, 0x40, rsp));
    uint32_t cmif_result = 0;
    REQUIRE(rsp.raw.size() >= 16);
    std::memcpy(&cmif_result, rsp.raw.data() + 8, 4);
    CHECK_EQ(cmif_result, kernel::kStubService);

    // 4. the summary names what was missing
    std::string sum = summary();
    CHECK(sum.find("UNKNOWN") == std::string::npos);       // the summary lists "0x38" rather than the log word
    CHECK(sum.find("0x38") != std::string::npos);
    CHECK(sum.find("e2e:svc") != std::string::npos);
    CHECK(sum.find("hle: ") != std::string::npos);
    shutdown();
    CHECK(!initialised());
    modules_unload_all();
    hle_clear();
    kernel::kernel_reset();
    ipc::registry_reset();
    mem::shutdown();
}
