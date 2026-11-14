// SPDX-License-Identifier: GPL-3.0-or-later
// Integration driver: runs one lifted test program (recomp nxrecomp output linked into this exe as `nxmod_main`) on the REAL runtime:
// runtime bring-up, the program's AArch64 ELF mapped by the module loader, the module tables registered through the ABI-2 registry,
// the C++ unwinder and the site hooks attached, test_main run on a guest thread made by the kernel thread layer. The program's
// results[] array is then compared word by word with the expected output (Unicorn for the S1 programs, the host's own C++ runtime for
// the exception programs, see tests/lift/CMakeLists.txt).
//
//   liftrun --elf prog.elf --expected prog.expected [--mask i,j,..] [--ignore-last] [--log file]
//           [--expect-terminate --frames name,name]      the program must end in std::terminate; results[0..count) must equal the
//                                                         expected prefix (count = results[NRES-2]); the guest backtrace must name the frames
//           [--spawn-expect-exit N --log-contains text,text]   run the program in a child process and check its exit code and log
//           [--abi-mismatch]                                   first try to register the module with wrong ABI versions: all must be refused
//           [--threads N]                                      after test_main, run thread_main(0..N-1) on N guest threads at once
//
// Besides the results, a normal run checks that test_main returned with the callee-saved registers (x19..x29, d8..d15) and SP it was
// entered with: that is what the unwinder's register restoring has to get right.
#include <windows.h>

#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include "nxrt/diag.h"
#include "nxrt/dispatch.h"
#include "nxrt/eh.h"
#include "nxrt/hooks.h"
#include "nxrt/kernel.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"
#include "nxrt/module.h"
#include "nxrt/nxmodule.h"
#include "nxrt/runtime.h"

extern "C" const NxModule nxmod_main;

namespace {

constexpr int kNres = 512;                                 // results[] words (t.h NRES)
constexpr uint64_t kHostBase = 0x7110000000ull;             // host services of the S1 programs' rt.h (raise / resume)
constexpr uint64_t kHarnessEntry = 0x7120000000ull;         // guest address of the thread entry below (a host function)

struct Options {
    std::string elf, expected, log = "liftrun.log";
    std::vector<int> mask;
    bool ignore_last = false, expect_terminate = false, child = false, abi_mismatch = false;
    std::vector<std::string> frames, log_contains;
    int spawn_exit = -1;
    int threads = 0;                      // --threads N: after test_main, N guest threads run thread_main(idx) at the same time
};
Options g_opt;
uint64_t g_test_main = 0, g_results = 0, g_thread_main = 0;
std::atomic<int> g_thread_failures{0};
std::vector<uint64_t> g_expected;
int g_thread_status = -1;                                    // 0 = test_main returned and everything checked out
bool g_terminated = false;

std::vector<std::string> split(const std::string& s, char sep) {
    std::vector<std::string> v;
    size_t i = 0;
    while (i <= s.size()) {
        size_t j = s.find(sep, i);
        if (j == std::string::npos) j = s.size();
        if (j > i) v.push_back(s.substr(i, j - i));
        i = j + 1;
    }
    return v;
}

bool read_expected(const std::string& path) {
    FILE* f = std::fopen(path.c_str(), "rb");
    if (!f) { std::fprintf(stderr, "cannot open expected file %s\n", path.c_str()); return false; }
    g_expected.assign(kNres, 0);
    size_t n = std::fread(g_expected.data(), sizeof(uint64_t), kNres, f);
    std::fclose(f);
    if (n != static_cast<size_t>(kNres)) { std::fprintf(stderr, "expected file %s has %zu words, want %d\n", path.c_str(), n, kNres); return false; }
    return true;
}

bool masked(int i) {
    for (int m : g_opt.mask) if (m == i) return true;
    return g_opt.ignore_last && i == kNres - 1;
}

// Compares results[0..count) with the expected words; prints every mismatch (first 20).
bool compare_results(int count, const char* what) {
    const uint64_t* got = reinterpret_cast<const uint64_t*>(g_results);
    int bad = 0;
    for (int i = 0; i < count; i++) {
        if (masked(i) || got[i] == g_expected[i]) continue;
        if (bad++ < 20) std::fprintf(stderr, "%s: results[%d] = 0x%llx, expected 0x%llx\n", what, i, (unsigned long long)got[i], (unsigned long long)g_expected[i]);
    }
    if (bad) std::fprintf(stderr, "%s: %d of %d words differ\n", what, bad, count);
    return bad == 0;
}

// The terminate handler of --expect-terminate: runs inside the guest's std::terminate.
void on_terminate(::Ctx* c, const char* why) {
    g_terminated = true;
    bool ok = true;
    const uint64_t* got = reinterpret_cast<const uint64_t*>(g_results);
    int count = static_cast<int>(got[kNres - 2]);
    if (count < 0 || count > kNres - 2 || static_cast<uint64_t>(count) != g_expected[kNres - 2]) {
        std::fprintf(stderr, "terminate: recorded count %d, expected %llu\n", count, (unsigned long long)g_expected[kNres - 2]);
        ok = false;
    } else if (!compare_results(count, "terminate prefix")) {
        ok = false;
    }
    std::string bt = nxrt::eh::guest_backtrace(c);
    for (const std::string& f : g_opt.frames) {
        if (bt.find(f) == std::string::npos) { std::fprintf(stderr, "terminate: backtrace does not name %s:\n%s", f.c_str(), bt.c_str()); ok = false; }
    }
    std::printf("terminate reached (%s); backtrace:\n%s%s\n", why, bt.c_str(), ok ? "TERMINATE-OK" : "TERMINATE-FAILED");
    std::fflush(stdout);
    nxrt::log_flush();
    ExitProcess(ok ? 0 : 1);
}

int terminate_hook(::Ctx* c, uint64_t, void*) {
    nxrt::eh::terminate_process(c, "std::terminate called by the guest");
    return 1;
}

// Guest thread entry: sentinels in the callee-saved registers, run test_main, check them.
void harness_entry(::Ctx* c) {
    const uint64_t sentinel = 0x5eed000000000000ull;
    for (int i = 19; i <= 29; i++) c->x[i] = sentinel + static_cast<uint64_t>(i);
    for (int i = 0; i < 8; i++) { c->v[8 + i].d[0] = sentinel + 0x100 + static_cast<uint64_t>(i); c->v[8 + i].d[1] = 0; }
    c->x[30] = 0;
    const uint64_t sp_in = c->x[31];
    nxrt::dispatch_call(c, g_test_main);
    bool ok = true;
    for (int i = 19; i <= 29; i++)
        if (c->x[i] != sentinel + static_cast<uint64_t>(i)) { std::fprintf(stderr, "callee-saved x%d changed: 0x%llx\n", i, (unsigned long long)c->x[i]); ok = false; }
    for (int i = 0; i < 8; i++)
        if (c->v[8 + i].d[0] != sentinel + 0x100 + static_cast<uint64_t>(i)) { std::fprintf(stderr, "callee-saved d%d changed\n", 8 + i); ok = false; }
    if (c->x[31] != sp_in) { std::fprintf(stderr, "SP changed across test_main: 0x%llx -> 0x%llx\n", (unsigned long long)sp_in, (unsigned long long)c->x[31]); ok = false; }
    g_thread_status = ok ? 0 : 1;
}

// Entry of the extra threads (--threads): thread_main(idx) with the same callee-saved check.
void thread_entry(::Ctx* c) {
    const uint64_t idx = c->x[0];
    const uint64_t sentinel = 0x7a7a000000000000ull + (idx << 32);
    for (int i = 19; i <= 29; i++) c->x[i] = sentinel + static_cast<uint64_t>(i);
    c->x[30] = 0;
    const uint64_t sp_in = c->x[31];
    nxrt::dispatch_call(c, g_thread_main);
    bool ok = c->x[31] == sp_in;
    for (int i = 19; i <= 29; i++) ok = ok && c->x[i] == sentinel + static_cast<uint64_t>(i);
    if (!ok) { std::fprintf(stderr, "thread %llu: callee-saved registers or SP changed\n", (unsigned long long)idx); g_thread_failures++; }
}

int run() {
    nxrt::InitOptions io;
    io.ini_path = "";
    io.log_path = g_opt.log.c_str();
    io.watchdog = false;
    io.memguard = false;
    if (!nxrt::init(io)) { std::fprintf(stderr, "runtime init failed\n"); return 2; }
    nxrt::eh::register_unwind_hle();

    // the S1 programs' host services (progs/rt.h): raise(object, typeinfo) and resume(object)
    nxrt::fn_register(kHostBase + 0, nxrt::eh::legacy_raise);
    nxrt::fn_register(kHostBase + 16, nxrt::eh::legacy_resume);

    std::string err;
    nxrt::Module* m = nxrt::module_load_file(g_opt.elf.c_str(), {}, &err);
    if (!m) { std::fprintf(stderr, "cannot load %s: %s\n", g_opt.elf.c_str(), err.c_str()); return 2; }
    if (!nxrt::module_link_all()) { std::fprintf(stderr, "link failed\n"); return 2; }
    if (g_opt.abi_mismatch) {          // a module lifted against another ABI version must be refused and leave no trace
        for (uint32_t wrong : {nxrt::kAbiVersion - 1, nxrt::kAbiVersion + 1, 0u}) {
            nxrt::RegResult bad = nxrt::nxmodule_register(m, &nxmod_main, wrong);
            if (bad.status != nxrt::RegStatus::AbiMismatch || bad.functions != 0 || nxrt::nxmodule_by_index(nxmod_main.index) ||
                nxrt::fn_lookup(nxmod_main.fns[0].addr)) {
                std::fprintf(stderr, "ABI version %u was not refused cleanly (status %d, %zu functions)\n", wrong, static_cast<int>(bad.status), bad.functions);
                return 1;
            }
        }
        std::printf("ABI mismatch refused for versions %u, %u and 0; nothing registered\n", nxrt::kAbiVersion - 1, nxrt::kAbiVersion + 1);
    }
    nxrt::RegResult r = NXRT_REGISTER_NXMODULE(m, nxmod_main);
    if (!r.ok()) { std::fprintf(stderr, "module registration refused: %s\n", r.message.c_str()); return 2; }
    nxrt::eh::discover_typeinfo_vtables();

    g_test_main = nxrt::module_find_symbol("test_main");
    g_results = nxrt::module_find_symbol("results");
    if (!g_test_main || !g_results) { std::fprintf(stderr, "test_main / results symbols not found in %s\n", g_opt.elf.c_str()); return 2; }

    // entry hooks on the stubs of guest/rt2.h (the lifter built them with `entryhook` site lines)
    struct { const char* sym; nxrt::SiteHookFn fn; } hooks[] = {
        {"_Unwind_RaiseException", nxrt::eh::hook_unwind_raise_exception},
        {"_Unwind_Resume", nxrt::eh::hook_unwind_resume},
        {"nx_host_terminate", terminate_hook},
    };
    int attached = 0;
    for (auto& h : hooks) {
        uint64_t a = nxrt::module_find_symbol(h.sym);
        if (a && nxrt::hook_add_entry(a, h.fn)) attached++;
    }
    if (g_opt.expect_terminate) nxrt::eh::set_terminate_handler(on_terminate);
    std::printf("module %s: %zu functions, %d entry hooks attached\n", m->name().c_str(), (size_t)nxmod_main.nfns, attached);

    nxrt::fn_register(kHarnessEntry, harness_entry);
    constexpr uint64_t kStack = 8ull << 20;
    const uint64_t stack = nxrt::mem::alloc_kernel(kStack, nxrt::mem::kAllocGranularity);
    nxrt::kernel::ThreadParams p;
    p.name = "lift-test";
    p.entry = kHarnessEntry;
    p.stack_top = stack + kStack;
    uint32_t th = nxrt::kernel::thread_create(p);
    if (!th || nxrt::kernel::thread_start(th) != 0) { std::fprintf(stderr, "cannot start the guest thread\n"); return 2; }
    if (!nxrt::kernel::thread_join(th, 120000)) { std::fprintf(stderr, "guest thread did not finish in 120 s\n"); return 3; }

    if (g_opt.threads > 0) {
        g_thread_main = nxrt::module_find_symbol("thread_main");
        if (!g_thread_main) { std::fprintf(stderr, "no thread_main symbol for --threads\n"); return 2; }
        constexpr uint64_t kTStack = 1ull << 20;
        std::vector<uint32_t> handles;
        nxrt::fn_register(kHarnessEntry + 16, thread_entry);
        for (int i = 0; i < g_opt.threads; i++) {
            const uint64_t ts = nxrt::mem::alloc_kernel(kTStack, nxrt::mem::kAllocGranularity);
            nxrt::kernel::ThreadParams tp;
            tp.name = "lift-thread-" + std::to_string(i);
            tp.entry = kHarnessEntry + 16;
            tp.arg = static_cast<uint64_t>(i);
            tp.stack_top = ts + kTStack;
            uint32_t h = nxrt::kernel::thread_create(tp);
            if (!h) { std::fprintf(stderr, "cannot create thread %d\n", i); return 2; }
            handles.push_back(h);
        }
        for (uint32_t h : handles)
            if (nxrt::kernel::thread_start(h) != 0) { std::fprintf(stderr, "cannot start a thread\n"); return 2; }
        for (uint32_t h : handles)
            if (!nxrt::kernel::thread_join(h, 120000)) { std::fprintf(stderr, "a guest thread did not finish in 120 s\n"); return 3; }
        if (g_thread_failures) g_thread_status = 1;
    }
    if (g_opt.expect_terminate) { std::fprintf(stderr, "test_main returned, but the program had to end in std::terminate\n"); return 1; }
    bool ok = g_thread_status == 0;
    if (!compare_results(kNres, "results")) ok = false;
    nxrt::eh::Stats st = nxrt::eh::stats();
    std::printf("eh: %llu raised, %llu resumed, %llu caught, %llu cleanups, %llu uncaught, %llu frames walked, %llu catch-frame landings, %llu resume landings\n",
                (unsigned long long)st.raised, (unsigned long long)st.resumed, (unsigned long long)st.caught, (unsigned long long)st.cleanups,
                (unsigned long long)st.uncaught, (unsigned long long)st.frames_walked, (unsigned long long)st.catch_frame_landings,
                (unsigned long long)st.resume_fn_landings);
    std::printf("%s\n", ok ? "LIFT-PASS" : "LIFT-FAIL");
    nxrt::shutdown();
    return ok ? 0 : 1;
}

int spawn() {
    std::string cmd = std::string("\"") + __argv[0] + "\"";
    for (int i = 1; i < __argc; i++) {
        std::string a = __argv[i];
        if (a == "--spawn-expect-exit" || a == "--log-contains") { i++; continue; }
        cmd += " \"" + a + "\"";
    }
    cmd += " --child";
    DeleteFileA(g_opt.log.c_str());
    STARTUPINFOA si{};
    si.cb = sizeof si;
    PROCESS_INFORMATION pi{};
    if (!CreateProcessA(nullptr, cmd.data(), nullptr, nullptr, FALSE, 0, nullptr, nullptr, &si, &pi)) { std::fprintf(stderr, "cannot start the child: %lu\n", GetLastError()); return 2; }
    WaitForSingleObject(pi.hProcess, 120000);
    DWORD code = 0xdead;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    bool ok = static_cast<int>(code) == g_opt.spawn_exit;
    if (!ok) std::fprintf(stderr, "child exit code %d, expected %d\n", static_cast<int>(code), g_opt.spawn_exit);
    std::string text;
    if (FILE* f = std::fopen(g_opt.log.c_str(), "rb")) {
        char buf[4096];
        size_t n;
        while ((n = std::fread(buf, 1, sizeof buf, f)) > 0) text.append(buf, n);
        std::fclose(f);
    }
    for (const std::string& s : g_opt.log_contains)
        if (text.find(s) == std::string::npos) { std::fprintf(stderr, "child log does not contain \"%s\"\n", s.c_str()); ok = false; }
    std::printf("%s (child exit code %d)\n", ok ? "SPAWN-PASS" : "SPAWN-FAIL", static_cast<int>(code));
    return ok ? 0 : 1;
}

}  // namespace

int main(int argc, char** argv) {
    for (int i = 1; i < argc; i++) {
        std::string a = argv[i];
        auto next = [&]() -> std::string { return i + 1 < argc ? argv[++i] : ""; };
        if (a == "--elf") g_opt.elf = next();
        else if (a == "--expected") g_opt.expected = next();
        else if (a == "--log") g_opt.log = next();
        else if (a == "--mask") for (auto& s : split(next(), ',')) g_opt.mask.push_back(std::atoi(s.c_str()));
        else if (a == "--ignore-last") g_opt.ignore_last = true;
        else if (a == "--abi-mismatch") g_opt.abi_mismatch = true;
        else if (a == "--expect-terminate") g_opt.expect_terminate = true;
        else if (a == "--frames") g_opt.frames = split(next(), ',');
        else if (a == "--spawn-expect-exit") g_opt.spawn_exit = std::atoi(next().c_str());
        else if (a == "--log-contains") g_opt.log_contains = split(next(), ',');
        else if (a == "--child") g_opt.child = true;
        else if (a == "--threads") g_opt.threads = std::atoi(next().c_str());
        else { std::fprintf(stderr, "unknown argument %s\n", a.c_str()); return 2; }
    }
    if (g_opt.elf.empty() || g_opt.expected.empty()) { std::fprintf(stderr, "usage: liftrun --elf prog.elf --expected prog.expected [options]\n"); return 2; }
    if (!read_expected(g_opt.expected)) return 2;
    if (g_opt.spawn_exit >= 0 && !g_opt.child) return spawn();
    return run();
}
