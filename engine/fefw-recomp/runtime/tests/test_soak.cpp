// SPDX-License-Identifier: GPL-3.0-or-later
// S34: gauges, the soak sampler, and the per-thread cache of the indirect-call dispatch.
#include <windows.h>

#include <atomic>
#include <chrono>
#include <cstdio>
#include <fstream>
#include <sstream>
#include <string>
#include <thread>

#include "nxrt/config.h"
#include "nxrt/diag.h"
#include "nxrt/dispatch.h"
#include "nxrt/gauge.h"
#include "test_framework.h"

NXTEST_GROUP(soak)

namespace {

int64_t find_gauge(const char* name, bool* found = nullptr) {
    for (auto& kv : nxrt::gauge_read_all())
        if (kv.first == name) {
            if (found) *found = true;
            return kv.second;
        }
    if (found) *found = false;
    return -1;
}

// SEH helper (no C++ objects in the frame): the exception code a guest call to `addr` raises, or 0 when it returns.
DWORD call_and_catch(uint64_t addr) {
    ::Ctx c{};
    __try {
        nxrt::dispatch_call(&c, addr);
    } __except (GetExceptionCode() == 0xE0000001u || GetExceptionCode() == EXCEPTION_ACCESS_VIOLATION ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        return GetExceptionCode();
    }
    return 0;
}

std::atomic<int> g_a_calls{0}, g_b_calls{0};
void fn_a(::Ctx*) { g_a_calls++; }
void fn_b(::Ctx*) { g_b_calls++; }

}  // namespace

NXTEST(gauge_callback_and_counter) {
    int64_t v = 5;
    nxrt::gauge_register("test.callback", [&] { return v; });
    {
        nxrt::GaugeCounter c("test.counter");
        c.add(3);
        ++c;
        --c;
        c.sub(1);
        bool f = false;
        CHECK_EQ(find_gauge("test.counter", &f), 2);
        CHECK(f);
        v = 9;
        CHECK_EQ(find_gauge("test.callback"), 9);
    }
    bool f = true;
    find_gauge("test.counter", &f);
    CHECK(!f);                                       // a counter removes its gauge when it is destroyed
    nxrt::gauge_unregister("test.callback");
    find_gauge("test.callback", &f);
    CHECK(!f);
}

NXTEST(gauge_register_again_replaces) {
    nxrt::gauge_register("test.same", [] { return int64_t(1); });
    nxrt::gauge_register("test.same", [] { return int64_t(2); });
    CHECK_EQ(find_gauge("test.same"), 2);
    nxrt::gauge_unregister("test.same");
}

NXTEST(soak_sampler_writes_json_lines) {
    char tmp[MAX_PATH];
    GetTempPathA(sizeof tmp, tmp);
    const std::string path = std::string(tmp) + "nxrt_soak_test_" + std::to_string(GetCurrentProcessId()) + ".jsonl";
    std::remove(path.c_str());
    nxrt::config_set("diag", "soak_csv", path.c_str());
    nxrt::config_set("diag", "soak_interval_s", "0.5");
    nxrt::GaugeCounter c("test.sampled");
    c.add(7);
    nxrt::soak_start();
    for (int i = 0; i < 20; i++) {         // a few presented frames, 5 ms apart, for the window statistics
        nxrt::diag_frame_tick();
        std::this_thread::sleep_for(std::chrono::milliseconds(5));
    }
    std::this_thread::sleep_for(std::chrono::milliseconds(2600));
    nxrt::soak_stop();
    nxrt::config_set("diag", "soak_csv", "");
    std::ifstream f(path);
    std::string line, last;
    int n = 0;
    while (std::getline(f, line)) {
        if (!line.empty()) { last = line; n++; }
    }
    f.close();
    std::remove(path.c_str());
    CHECK(n >= 2);
    CHECK(last.find("\"test.sampled\":7") != std::string::npos);
    CHECK(last.find("\"ws\":") != std::string::npos);
    CHECK(last.find("\"handles\":") != std::string::npos);
    CHECK(last.find("\"ft_p50\":") != std::string::npos);
    CHECK(last.front() == '{' && last.back() == '}');
}

NXTEST(dispatch_cache_follows_a_replaced_function) {
    constexpr uint64_t kAddr = 0x7123450000ull;
    nxrt::fn_register(kAddr, &fn_a);
    ::Ctx c{};
    g_a_calls = g_b_calls = 0;
    for (int i = 0; i < 5; i++) nxrt::dispatch_call(&c, kAddr);     // the first call fills this thread's cache, the rest are hits
    CHECK_EQ(g_a_calls.load(), 5);
    nxrt::fn_register(kAddr, &fn_b);                                // replaced: the cache must not keep calling the old one
    for (int i = 0; i < 3; i++) nxrt::dispatch_call(&c, kAddr);
    CHECK_EQ(g_a_calls.load(), 5);
    CHECK_EQ(g_b_calls.load(), 3);
}

NXTEST(dispatch_cache_other_thread_sees_the_replacement) {
    constexpr uint64_t kAddr = 0x7123460000ull;
    nxrt::fn_register(kAddr, &fn_a);
    g_a_calls = g_b_calls = 0;
    std::atomic<int> phase{0};
    std::thread t([&] {
        ::Ctx c{};
        for (int i = 0; i < 4; i++) nxrt::dispatch_call(&c, kAddr);
        phase = 1;
        while (phase != 2) std::this_thread::yield();
        for (int i = 0; i < 4; i++) nxrt::dispatch_call(&c, kAddr);
    });
    while (phase != 1) std::this_thread::yield();
    nxrt::fn_register(kAddr, &fn_b);
    phase = 2;
    t.join();
    CHECK_EQ(g_a_calls.load(), 4);
    CHECK_EQ(g_b_calls.load(), 4);
}

NXTEST(dispatch_cache_colliding_addresses) {
    // two addresses that map to the same cache slot ((a >> 2) ^ (a >> 12), low 10 bits) and are called alternately
    constexpr uint64_t kA = 0x7123470000ull;
    uint64_t b = kA + (1ull << 14);                                 // same low bits of both terms
    nxrt::fn_register(kA, &fn_a);
    nxrt::fn_register(b, &fn_b);
    ::Ctx c{};
    g_a_calls = g_b_calls = 0;
    for (int i = 0; i < 6; i++) {
        nxrt::dispatch_call(&c, kA);
        nxrt::dispatch_call(&c, b);
    }
    CHECK_EQ(g_a_calls.load(), 6);
    CHECK_EQ(g_b_calls.load(), 6);
}

NXTEST(sampling_profiler_writes_folded_stacks) {
    char tmp[MAX_PATH];
    GetTempPathA(sizeof tmp, tmp);
    const std::string path = std::string(tmp) + "nxrt_prof_test_" + std::to_string(GetCurrentProcessId()) + ".folded";
    std::remove(path.c_str());
    nxrt::config_set("prof", "sample_hz", "200");
    nxrt::config_set("prof", "sample_out", path.c_str());
    std::atomic<bool> stop{false}, up{false};
    std::thread worker([&] {
        nxrt::diag_register_thread("soak-test-spin", nullptr);
        up = true;
        volatile uint64_t x = 0;
        while (!stop) x += 1;                         // running user code: its samples are not [wait]
        nxrt::diag_unregister_thread();
    });
    while (!up) std::this_thread::yield();
    nxrt::prof_sampler_label("scene-x");
    nxrt::prof_sampler_start();
    std::this_thread::sleep_for(std::chrono::milliseconds(700));
    nxrt::prof_sampler_stop();
    stop = true;
    worker.join();
    nxrt::config_set("prof", "sample_hz", "0");
    std::ifstream f(path);
    std::string line;
    int running = 0, all = 0;
    while (std::getline(f, line)) {
        if (line.empty() || line[0] == '#') continue;
        if (line.compare(0, 8, "scene-x;") != 0 || line.find(";soak-test-spin;") == std::string::npos) continue;
        all++;
        if (line.find(";[wait] ") == std::string::npos) running++;
    }
    f.close();
    std::remove(path.c_str());
    CHECK(all >= 1);
    CHECK(running >= 1);
}

NXTEST(dispatch_guest_call_to_zero_traps) {
    // An empty cache entry is {0, nullptr}: a call to address 0 must not "hit" it (that would jump to host address 0, an access violation)
    // but reach the guest trap (0xE0000001). Twice: the second call runs on a cache the first one has touched.
    CHECK_EQ(call_and_catch(0), 0xE0000001u);
    CHECK_EQ(call_and_catch(0), 0xE0000001u);
}

NXTEST(diag_threads_dup_handles_survive_unregister) {
    std::atomic<int> phase{0};
    std::thread t([&] {
        nxrt::diag_register_thread("dup-test", nullptr);
        phase = 1;
        while (phase != 2) std::this_thread::yield();
        nxrt::diag_unregister_thread();
        phase = 3;
    });
    while (phase != 1) std::this_thread::yield();
    std::vector<nxrt::ThreadRecord> recs = nxrt::diag_threads_dup();
    phase = 2;
    while (phase != 3) std::this_thread::yield();   // the thread has unregistered (and closed the registry's handle)
    bool found = false;
    for (auto& r : recs)
        if (r.name == "dup-test") {
            found = true;
            CHECK(r.host_handle != nullptr);
            CHECK_EQ(GetThreadId(static_cast<HANDLE>(r.host_handle)), r.host_tid);   // still a valid handle to that thread
        }
    CHECK(found);
    nxrt::diag_threads_release(recs);
    CHECK(recs.empty());
    t.join();
}
