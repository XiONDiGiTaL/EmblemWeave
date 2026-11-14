// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for the freeze watchdog and thread dump (nxrt/diag.h). Every case stops the watchdog it starts. Fire counts are
// compared as deltas because the counter is process-wide. The frame counter is process-wide too; the first case runs
// before any tick in this group, so it also covers a freeze before the first frame.
#include "test_framework.h"

#include "nxrt/abi.h"
#include "nxrt/config.h"
#include "nxrt/diag.h"
#include "nxrt/symbols.h"

#include <windows.h>
#include <intrin.h>

#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <functional>
#include <mutex>
#include <string>
#include <thread>

NXTEST_GROUP(watchdog)

using namespace std::chrono;

// A busy host function the dump must name (symbolised through the linker map).
extern "C" __declspec(noinline) void watchdog_test_spin(volatile bool* stop) {
    while (!*stop) _mm_pause();
}

namespace {

std::string exe_dir() {
    char b[4096];
    DWORD n = GetModuleFileNameA(nullptr, b, sizeof b);
    std::string p(b, n);
    size_t s = p.find_last_of("\\/");
    return s == std::string::npos ? std::string(".") : p.substr(0, s);
}

std::string temp_file(const char* tag) {
    char dir[MAX_PATH];
    DWORD n = GetTempPathA(MAX_PATH, dir);
    return std::string(dir, n) + "nxrt_wd_" + tag + ".txt";
}

std::string read_file(const std::string& path) {
    std::string out;
    FILE* f = std::fopen(path.c_str(), "rb");
    if (!f) return out;
    char buf[4096];
    size_t n;
    while ((n = std::fread(buf, 1, sizeof buf, f)) > 0) out.append(buf, n);
    std::fclose(f);
    return out;
}

bool file_exists(const std::string& p) { return GetFileAttributesA(p.c_str()) != INVALID_FILE_ATTRIBUTES; }

bool has(const std::string& s, const char* sub) { return s.find(sub) != std::string::npos; }

// Polls pred every 10 ms until it holds or `ms` elapse.
bool wait_for(const std::function<bool()>& pred, int ms) {
    auto end = steady_clock::now() + milliseconds(ms);
    while (!pred()) {
        if (steady_clock::now() >= end) return false;
        std::this_thread::sleep_for(milliseconds(10));
    }
    return true;
}

struct FireCapture {
    std::mutex m;
    std::string text;
    std::atomic<int> calls{0};
};

void capture_fire(const char* dump, void* user) {
    auto* f = static_cast<FireCapture*>(user);
    std::lock_guard<std::mutex> g(f->m);
    f->text = dump;
    f->calls++;
}

// A thread that runs guest code: registered with a fabricated Ctx (x30 = 0xAAAA0001). It either sleeps in a loop that
// counts (so the counter shows whether it keeps running) or spins inside watchdog_test_spin.
struct Worker {
    const char* name;
    bool spin = false;
    ::Ctx ctx{};
    volatile bool stop = false;
    std::atomic<bool> ready{false};
    std::atomic<uint64_t> counter{0};
    std::thread th;

    explicit Worker(const char* n) : name(n) {
        ctx.x[30] = 0xAAAA0001ull;
        ctx.x[29] = 0;
        ctx.x[31] = 0x71000000ull;
    }
    void start() { th = std::thread(&Worker::body, this); }
    void join() {
        stop = true;
        if (th.joinable()) th.join();
    }
    void body() {
        nxrt::diag_register_thread(name, &ctx, 0);
        ready = true;
        if (spin) {
            watchdog_test_spin(&stop);
        } else {
            while (!stop) {
                counter++;
                Sleep(1);
            }
        }
        nxrt::diag_unregister_thread();
    }
};

}  // namespace

// Starts the watchdog with a 1 s threshold, a 50 ms poll and no ticks: it fires once and stays quiet.
NXTEST(fires_once_on_freeze) {
    const std::string rep = temp_file("fires");
    std::remove(rep.c_str());
    const uint32_t base = nxrt::watchdog_fire_count();
    Worker a("worker-a"), b("worker-b");
    a.start();
    b.start();
    CHECK(wait_for([&] { return a.ready.load() && b.ready.load(); }, 2000));

    FireCapture cap;
    nxrt::WatchdogOptions opt;
    opt.seconds = 1.0;
    opt.poll_ms = 50;
    opt.report_path = rep.c_str();
    opt.on_fire = capture_fire;
    opt.user = &cap;
    nxrt::watchdog_start(opt);
    CHECK(nxrt::watchdog_running());
    CHECK(wait_for([&] { return cap.calls.load() >= 1; }, 6000));
    std::this_thread::sleep_for(seconds(2));
    CHECK_EQ(cap.calls.load(), 1);
    CHECK_EQ(nxrt::watchdog_fire_count() - base, 1u);
    nxrt::watchdog_stop();

    std::string text;
    {
        std::lock_guard<std::mutex> g(cap.m);
        text = cap.text;
    }
    CHECK_MSG(has(text, "FREEZE: no frame tick for"), "FREEZE header missing from the dump");
    CHECK_MSG(has(text, "worker-a"), "worker-a missing from the dump");
    CHECK_MSG(has(text, "worker-b"), "worker-b missing from the dump");
    CHECK_MSG(has(text, "lr=00000000aaaa0001"), "guest lr missing from the dump");
    CHECK_MSG(has(text, "#0"), "no host frame lines in the dump");
    CHECK_MSG(has(text, "last files opened"), "recent-files section missing");
    CHECK_MSG(file_exists(rep), "report file not written");
    CHECK_MSG(read_file(rep) == text, "report file differs from the on_fire text");
    a.join();
    b.join();
    std::remove(rep.c_str());
}

NXTEST(ticking_prevents_firing) {
    const uint32_t base = nxrt::watchdog_fire_count();
    FireCapture cap;
    nxrt::WatchdogOptions opt;
    opt.seconds = 1.0;
    opt.poll_ms = 50;
    opt.report_path = "";
    opt.on_fire = capture_fire;
    opt.user = &cap;
    const uint64_t f0 = nxrt::diag_frame_count();
    nxrt::watchdog_start(opt);
    const auto end = steady_clock::now() + seconds(3);
    while (steady_clock::now() < end) {
        nxrt::diag_frame_tick();
        std::this_thread::sleep_for(milliseconds(100));
    }
    CHECK_EQ(nxrt::watchdog_fire_count() - base, 0u);
    CHECK_EQ(cap.calls.load(), 0);
    CHECK(nxrt::diag_frame_count() >= f0 + 20);
    nxrt::watchdog_stop();
    CHECK(!nxrt::watchdog_running());
}

NXTEST(progress_heartbeats_prevent_firing_without_counting_frames) {
    const uint32_t base = nxrt::watchdog_fire_count();
    FireCapture cap;
    nxrt::WatchdogOptions opt;
    opt.seconds = 1.0;
    opt.poll_ms = 50;
    opt.report_path = "";
    opt.on_fire = capture_fire;
    opt.user = &cap;
    const uint64_t f0 = nxrt::diag_frame_count();
    nxrt::watchdog_start(opt);
    const auto end = steady_clock::now() + seconds(3);
    while (steady_clock::now() < end) {
        nxrt::diag_progress();                    // long work that moves (a texture transcode), not a presented frame
        std::this_thread::sleep_for(milliseconds(100));
    }
    CHECK_EQ(nxrt::watchdog_fire_count() - base, 0u);
    CHECK_EQ(cap.calls.load(), 0);
    CHECK_EQ(nxrt::diag_frame_count(), f0);
    nxrt::watchdog_stop();
}

NXTEST(rearms_after_a_tick) {
    FireCapture cap;
    nxrt::WatchdogOptions opt;
    opt.seconds = 1.0;
    opt.poll_ms = 50;
    opt.report_path = "";
    opt.on_fire = capture_fire;
    opt.user = &cap;
    nxrt::watchdog_start(opt);
    CHECK(wait_for([&] { return cap.calls.load() >= 1; }, 4000));
    nxrt::diag_frame_tick();
    CHECK(wait_for([&] { return cap.calls.load() >= 2; }, 4000));
    std::this_thread::sleep_for(milliseconds(300));
    CHECK_EQ(cap.calls.load(), 2);
    nxrt::watchdog_stop();
}

NXTEST(negative_seconds_disables) {
    const uint32_t base = nxrt::watchdog_fire_count();
    FireCapture cap;
    nxrt::WatchdogOptions opt;
    opt.seconds = -1.0;
    opt.poll_ms = 50;
    opt.report_path = "";
    opt.on_fire = capture_fire;
    opt.user = &cap;
    nxrt::watchdog_start(opt);
    CHECK(!nxrt::watchdog_running());
    std::this_thread::sleep_for(milliseconds(1500));
    CHECK_EQ(nxrt::watchdog_fire_count() - base, 0u);
    CHECK_EQ(cap.calls.load(), 0);
    nxrt::watchdog_stop();
}

NXTEST(stop_is_prompt) {
    nxrt::WatchdogOptions opt;
    opt.seconds = 60.0;
    opt.poll_ms = 250;
    opt.report_path = "";
    nxrt::watchdog_start(opt);
    CHECK(nxrt::watchdog_running());
    std::this_thread::sleep_for(milliseconds(300));
    const auto t0 = steady_clock::now();
    nxrt::watchdog_stop();
    const auto ms = duration_cast<milliseconds>(steady_clock::now() - t0).count();
    CHECK_MSG(ms < 500, "watchdog_stop took 500 ms or more");
    CHECK(!nxrt::watchdog_running());
}

NXTEST(threshold_from_config) {
    nxrt::config_set("diag", "watchdog_seconds", "1");
    const std::string rep = temp_file("config");
    std::remove(rep.c_str());
    const uint32_t base = nxrt::watchdog_fire_count();
    FireCapture cap;
    nxrt::WatchdogOptions opt;
    opt.seconds = 0;  // take the threshold from [diag] watchdog_seconds
    opt.poll_ms = 50;
    opt.report_path = rep.c_str();
    opt.on_fire = capture_fire;
    opt.user = &cap;
    nxrt::watchdog_start(opt);
    CHECK(nxrt::watchdog_running());
    CHECK(wait_for([&] { return cap.calls.load() >= 1; }, 5000));
    nxrt::watchdog_stop();
    CHECK_EQ(nxrt::watchdog_fire_count() - base, 1u);
    nxrt::config_clear();
    std::remove(rep.c_str());
}

NXTEST(thread_dump_lists_registered_threads_not_caller) {
    ::Ctx self_ctx{};
    nxrt::diag_register_thread("caller-thread", &self_ctx, 0);
    Worker a("worker-a"), b("worker-b");
    a.start();
    b.start();
    CHECK(wait_for([&] { return a.ready.load() && b.ready.load(); }, 2000));
    const std::string d = nxrt::diag_thread_dump();
    a.join();
    b.join();
    nxrt::diag_unregister_thread();
    CHECK_MSG(has(d, "thread worker-a tid="), "worker-a missing from the dump");
    CHECK_MSG(has(d, "thread worker-b tid="), "worker-b missing from the dump");
    CHECK_MSG(!has(d, "thread caller-thread"), "the calling thread was dumped");
    CHECK_MSG(has(d, "last files opened (oldest first):"), "recent-files section missing");
}

NXTEST(suspended_threads_keep_running) {
    Worker a("worker-a"), b("worker-b");
    a.start();
    b.start();
    CHECK(wait_for([&] { return a.ready.load() && b.ready.load(); }, 2000));
    std::this_thread::sleep_for(milliseconds(100));
    const std::string d = nxrt::diag_thread_dump();
    const uint64_t a1 = a.counter.load(), b1 = b.counter.load();
    std::this_thread::sleep_for(milliseconds(300));
    CHECK_MSG(!has(d, "(could not suspend)"), "a registered thread could not be suspended");
    CHECK_MSG(a.counter.load() > a1, "worker-a stopped running after the dump");
    CHECK_MSG(b.counter.load() > b1, "worker-b stopped running after the dump");
    a.join();
    b.join();
}

NXTEST(dump_names_symbolised_host_function) {
    Worker spin("worker-spin");
    spin.spin = true;
    spin.start();
    CHECK(wait_for([&] { return spin.ready.load(); }, 2000));
    std::this_thread::sleep_for(milliseconds(50));
    CHECK_MSG(nxrt::symbols_load_map() > 0, "no linker map next to the exe");
    const std::string d = nxrt::diag_thread_dump();
    spin.join();
    CHECK_MSG(has(d, "worker-spin"), "worker-spin missing from the dump");
    CHECK_MSG(has(d, "watchdog_test_spin"), "spinning host frame not symbolised");
}

NXTEST(dump_request_file_writes_dump) {
    const std::string req = exe_dir() + "\\dump_threads";
    const std::string rep = temp_file("request");
    std::remove(rep.c_str());
    FILE* f = std::fopen(req.c_str(), "wb");
    CHECK(f != nullptr);
    if (f) std::fclose(f);
    const uint32_t base = nxrt::watchdog_fire_count();
    FireCapture cap;
    nxrt::WatchdogOptions opt;
    opt.seconds = 60.0;
    opt.poll_ms = 50;
    opt.report_path = rep.c_str();
    opt.on_fire = capture_fire;
    opt.user = &cap;
    nxrt::watchdog_start(opt);
    CHECK_MSG(wait_for([&] { return !file_exists(req); }, 3000), "dump_threads request file was not consumed");
    CHECK_MSG(wait_for([&] { return has(read_file(rep), "THREAD DUMP REQUESTED"); }, 3000),
              "no THREAD DUMP REQUESTED in the report file");
    CHECK_EQ(nxrt::watchdog_fire_count() - base, 0u);
    CHECK_EQ(cap.calls.load(), 0);
    nxrt::watchdog_stop();
    std::remove(req.c_str());
    std::remove(rep.c_str());
}

NXTEST(second_start_is_ignored) {
    nxrt::WatchdogOptions opt;
    opt.seconds = 60.0;
    opt.poll_ms = 50;
    opt.report_path = "";
    nxrt::watchdog_start(opt);
    nxrt::watchdog_start(opt);
    CHECK(nxrt::watchdog_running());
    nxrt::watchdog_stop();
    CHECK(!nxrt::watchdog_running());
    nxrt::watchdog_stop();
}
