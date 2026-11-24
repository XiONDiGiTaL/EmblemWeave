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
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

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

// Starts this exe as `--child <mode>` and waits for it: a stop deadlock must fail the parent after the timeout instead of
// hanging the whole test binary.
bool run_child_until(const char* mode, DWORD timeout_ms, DWORD& exit_code) {
    char exe[MAX_PATH * 4] = {};
    if (!GetModuleFileNameA(nullptr, exe, sizeof exe)) return false;
    const std::string cmd = std::string("\"") + exe + "\" --child " + mode;
    std::vector<char> cmdline(cmd.begin(), cmd.end());
    cmdline.push_back('\0');
    STARTUPINFOA si{};
    si.cb = sizeof si;
    PROCESS_INFORMATION pi{};
    if (!CreateProcessA(exe, cmdline.data(), nullptr, nullptr, FALSE, 0, nullptr, nullptr, &si, &pi)) return false;
    const DWORD waited = WaitForSingleObject(pi.hProcess, timeout_ms);
    if (waited == WAIT_OBJECT_0) GetExitCodeProcess(pi.hProcess, &exit_code);
    else TerminateProcess(pi.hProcess, 1);
    CloseHandle(pi.hThread);
    CloseHandle(pi.hProcess);
    return waited == WAIT_OBJECT_0;
}

std::atomic<bool> g_cb_entered{false};
std::atomic<bool> g_cb_release{false};
std::atomic<bool> g_cb_stopped{false};

void on_fire_concurrent_stop(const char*, void*) {
    g_cb_entered.store(true);
    while (!g_cb_release.load()) std::this_thread::sleep_for(milliseconds(1));
    nxrt::watchdog_stop();
    g_cb_stopped.store(true);
}

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

NXTEST_CHILD(watchdog_concurrent_stop) {
    nxrt::config_clear();
    nxrt::watchdog_stop();
    g_cb_entered.store(false);
    g_cb_release.store(false);
    g_cb_stopped.store(false);
    nxrt::WatchdogOptions wo;
    wo.seconds = 0.02;
    wo.poll_ms = 1;
    wo.report_path = "";
    wo.on_fire = on_fire_concurrent_stop;
    nxrt::watchdog_start(wo);
    if (!wait_for([] { return g_cb_entered.load(); }, 3000)) return 2;

    std::atomic<bool> caller_done{false};
    std::thread caller([&] {
        nxrt::watchdog_stop();
        caller_done.store(true);
    });
    std::this_thread::sleep_for(milliseconds(100));   // the caller is now joining the callback
    g_cb_release.store(true);
    if (!wait_for([&] { return caller_done.load(); }, 3000)) {   // old code deadlocked here
        caller.detach();
        return 3;
    }
    caller.join();
    if (!g_cb_stopped.load()) return 4;
    return 0;
}

NXTEST(concurrent_stop_from_callback_and_caller_does_not_deadlock) {
    DWORD code = 1;
    const bool finished = run_child_until("watchdog_concurrent_stop", 8000, code);
    CHECK_MSG(finished, "concurrent watchdog_stop deadlocked (child did not exit)");
    CHECK_EQ(code, 0u);
}

NXTEST(dump_latency_scales_with_thread_count) {
    // Baseline for freeze-dump latency: registered busy threads make SuspendThread/GetThreadContext/stack capture measurable.
    // Times are printed so a regression can be compared with the recorded audit numbers instead of guessed at.
    for (const int count : {1, 8, 32}) {
        std::atomic<bool> stop{false};
        std::atomic<int> ready{0};
        std::vector<std::thread> workers;
        workers.reserve(static_cast<size_t>(count));
        for (int i = 0; i < count; i++) {
            workers.emplace_back([&stop, &ready, i] {
                const std::string name = "dumpbench" + std::to_string(i);
                nxrt::diag_register_thread(name.c_str(), nullptr);
                ready.fetch_add(1, std::memory_order_release);
                while (!stop.load(std::memory_order_relaxed)) _mm_pause();
                nxrt::diag_unregister_thread();
            });
        }
        CHECK_MSG(wait_for([&] { return ready.load(std::memory_order_acquire) == count; }, 5000), "workers did not register");

        nxrt::ThreadDumpTiming worst{};
        std::string dump;
        for (int round = 0; round < 3; round++) {
            nxrt::ThreadDumpTiming t;
            dump = nxrt::diag_thread_dump(&t);
            CHECK_MSG(t.threads >= static_cast<size_t>(count), "dump missed registered worker threads");
            CHECK(t.collect_ms >= t.snapshot_ms);
            CHECK(t.total_ms >= t.collect_ms);
            CHECK(t.format_ms >= 0.0);
            CHECK(t.total_ms < 2000.0);   // a dump that takes seconds would itself look like a freeze
            if (t.total_ms > worst.total_ms) worst = t;
        }
        CHECK_MSG(has(dump, "dumpbench"), "thread dump does not name a registered worker");
        std::printf("freeze dump: %2d threads: total %.3f ms (collect %.3f: snapshot %.3f suspend %.3f context %.3f stack %.3f resume %.3f, format %.3f)\n",
                    count, worst.total_ms, worst.collect_ms, worst.snapshot_ms, worst.suspend_ms, worst.context_ms, worst.stack_ms,
                    worst.resume_ms, worst.format_ms);

        stop.store(true);
        for (auto& t : workers) t.join();
    }
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

// ---- batched suspension safety -------------------------------------------------------------------------------------------------
// One dump suspends every registered thread before it reads any context. These cases pin down what the optimisation must not break: a
// thread's own suspend count, resume accounting, threads leaving the registry mid-dump, and two dumps at once.

NXTEST(every_suspension_is_released_and_accounted) {
    static const char* const kNames[] = {"resume-0", "resume-1", "resume-2", "resume-3"};
    std::vector<std::unique_ptr<Worker>> workers;
    for (const char* n : kNames) {
        workers.emplace_back(new Worker(n));
        workers.back()->spin = true;
        workers.back()->start();
    }
    for (auto& w : workers) CHECK(wait_for([&] { return w->ready.load(); }, 2000));

    for (int round = 0; round < 5; round++) {
        nxrt::ThreadDumpTiming t;
        const std::string d = nxrt::diag_thread_dump(&t);
        CHECK_MSG(!d.empty(), "empty thread dump");
        CHECK_MSG(t.suspended >= workers.size(), "the dump did not suspend every registered worker");
        CHECK_EQ(t.resumed, t.suspended);   // every suspension is undone, even when a later one fails
        CHECK_MSG(has(d, "thread resume-0 tid="), "a registered worker is missing from the dump");
    }
    for (auto& w : workers) w->join();
}

NXTEST(dump_preserves_a_threads_own_suspend_count) {
    // SuspendThread counts: a thread already suspended once (a debugger, or an earlier dump that was interrupted) must still be
    // suspended once after the dump, neither running early nor double-suspended.
    std::atomic<DWORD> tid{0};
    std::atomic<bool> about_to_suspend{false};
    std::atomic<bool> ran_after{false};
    std::thread th([&] {
        nxrt::diag_register_thread("self-suspended", nullptr);
        tid.store(GetCurrentThreadId(), std::memory_order_release);
        about_to_suspend.store(true, std::memory_order_release);
        SuspendThread(GetCurrentThread());   // one count of our own
        ran_after.store(true, std::memory_order_release);
        nxrt::diag_unregister_thread();
    });
    CHECK(wait_for([&] { return about_to_suspend.load(); }, 2000));
    CHECK(wait_for([&] { return tid.load() != 0; }, 2000));
    std::this_thread::sleep_for(milliseconds(50));
    CHECK_MSG(!ran_after.load(), "the thread did not stay suspended by its own SuspendThread");

    const std::string d = nxrt::diag_thread_dump();
    CHECK_MSG(has(d, "thread self-suspended tid="), "the self-suspended thread is missing from the dump");

    HANDLE h = OpenThread(THREAD_SUSPEND_RESUME | THREAD_QUERY_INFORMATION, FALSE, tid.load());
    REQUIRE(h != nullptr);
    const DWORD remaining = ResumeThread(h);   // 1: the dump restored the thread's own count instead of leaving 0 or 2
    CloseHandle(h);
    CHECK_MSG(remaining == 1, "the dump did not restore the thread's own suspend count");
    CHECK(wait_for([&] { return ran_after.load(); }, 2000));
    th.join();
}

NXTEST(dump_survives_threads_leaving_the_registry_mid_dump) {
    // Registration and removal race the enumeration: a thread can unregister (and exit) between the snapshot and its suspension.
    std::atomic<bool> stop{false};
    std::atomic<int> churned{0};
    std::thread churn([&] {
        while (!stop.load(std::memory_order_relaxed)) {
            std::thread t([&] {
                nxrt::diag_register_thread("churn", nullptr);
                Sleep(1);
                nxrt::diag_unregister_thread();
            });
            t.join();
            churned.fetch_add(1, std::memory_order_relaxed);
        }
    });
    CHECK_MSG(wait_for([&] { return churned.load() >= 3; }, 2000), "the churn thread did not start");
    for (int i = 0; i < 25; i++) {
        nxrt::ThreadDumpTiming t;
        const std::string d = nxrt::diag_thread_dump(&t);
        CHECK_MSG(!d.empty(), "empty thread dump while threads were leaving the registry");
        CHECK_EQ(t.resumed, t.suspended);
    }
    stop.store(true);
    churn.join();
    CHECK(churned.load() >= 3);
}

// Two dumpers, each suspending the other, would never run their resume loops: the child must exit instead of hanging.
// The dumpers register too: a dump only suspends registry members, so the mutual suspension this guards against needs dumper threads that
// are registered - main calling the fault dump while a registered render thread dumps, for example.
NXTEST_CHILD(watchdog_concurrent_dump) {
    std::atomic<bool> stop{false};
    std::atomic<int> ready{0};
    std::vector<std::thread> workers;
    for (int i = 0; i < 6; i++)
        workers.emplace_back([&] {
            nxrt::diag_register_thread("dump-concurrent", nullptr);
            ready.fetch_add(1);
            while (!stop.load(std::memory_order_relaxed)) _mm_pause();
            nxrt::diag_unregister_thread();
        });
    while (ready.load() != 6) std::this_thread::sleep_for(milliseconds(1));
    std::atomic<int> done{0}, bad{0};
    std::vector<std::thread> dumpers;
    for (int i = 0; i < 4; i++)
        dumpers.emplace_back([&] {
            nxrt::diag_register_thread("dump-caller", nullptr);
            for (int k = 0; k < 25; k++) {
                const std::string d = nxrt::diag_thread_dump();
                if (d.empty()) bad.fetch_add(1);
            }
            nxrt::diag_unregister_thread();
            done.fetch_add(1);
        });
    for (auto& t : dumpers) t.join();
    stop.store(true);
    for (auto& t : workers) t.join();
    return done.load() == 4 && bad.load() == 0 ? 0 : 5;
}

NXTEST(concurrent_dumps_do_not_deadlock) {
    DWORD code = 1;
    const bool finished = run_child_until("watchdog_concurrent_dump", 20000, code);
    CHECK_MSG(finished, "concurrent thread dumps deadlocked (child did not exit)");
    CHECK_EQ(code, 0u);
}

// A thread that registered and then exited without unregistering: its duplicated handle is still valid, SuspendThread is not.
NXTEST_CHILD(watchdog_dump_unsuspendable) {
    std::thread done([] { nxrt::diag_register_thread("gone-thread", nullptr); });
    done.join();
    nxrt::ThreadDumpTiming t;
    const std::string d = nxrt::diag_thread_dump(&t);
    if (!has(d, "thread gone-thread tid=")) return 2;
    if (!has(d, "(could not suspend)")) return 3;
    if (t.suspended != 0 || t.resumed != 0) return 4;   // nothing was suspended, so nothing needed resuming
    return 0;
}

NXTEST(a_thread_that_cannot_be_suspended_is_reported) {
    DWORD code = 1;
    const bool finished = run_child_until("watchdog_dump_unsuspendable", 8000, code);
    CHECK_MSG(finished, "the dump hung on a thread that had exited");
    CHECK_EQ(code, 0u);
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
