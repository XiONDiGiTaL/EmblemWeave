// SPDX-License-Identifier: GPL-3.0-or-later
// Background render preparation thread pool. Runs descriptor set construction in parallel.
#include "nvn_render_prep.h"

#include <windows.h>

// Forward declaration for the gpu::build_set function defined in nvn_replay.cpp
namespace nxrt::nvn::be::gpu {
VkDescriptorSet build_set(const ProgLayout& pl, uint32_t set);
}

#include <algorithm>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <deque>
#include <memory>
#include <mutex>
#include <thread>
#include <vector>

#include "nxrt/config.h"
#include "nxrt/ftrace.h"
#include "nxrt/gauge.h"
#include "nxrt/log.h"
#include "nvn_pipeline.h"

namespace nxrt::nvn::be::renderprep {
namespace {

constexpr const char* kTag = "nvn.renderprep";

struct Job {
    SetBuildWork* work = nullptr;
    std::shared_ptr<std::atomic<bool>> done;
};

struct State {
    std::mutex mu;
    std::condition_variable cv_work;
    std::deque<Job> q;
    std::atomic<bool> shutdown{false};
    std::vector<std::thread> workers;
    Stats st{};
    int num_threads = 0;
};

State* g_state = nullptr;
std::once_flag g_once;
std::atomic<int> g_enabled{-1};

State& S() { return *g_state; }

void worker_main() {
    log_set_thread_name("nvn-renderprep");

    State& s = S();
    std::unique_lock<std::mutex> lk(s.mu);
    for (;;) {
        if (s.q.empty()) {
            if (s.shutdown.load(std::memory_order_acquire)) break;
            s.cv_work.wait(lk);
            continue;
        }
        Job job = std::move(s.q.front());
        s.q.pop_front();
        lk.unlock();

        // Build the descriptor set
        if (job.work) {
            job.work->result = gpu::build_set(*job.work->pl, job.work->set);
            job.work->cache_hit = (job.work->result != VK_NULL_HANDLE);  // Will be checked against cache in build_set
        }
        job.done->store(true, std::memory_order_release);
        s.st.sets_built_async++;

        lk.lock();
    }
}

void init_once() {
    const bool on = nxrt::config_get_bool("nvn", "render_prep", false);  // Default OFF until thread-safety fixed
    if (!on) {
        g_enabled.store(0);
        NXLOG_I(kTag, "background render preparation is off");
        return;
    }

    g_state = new State();
    const unsigned hw = std::max(1u, std::thread::hardware_concurrency());
    int n = static_cast<int>(nxrt::config_get_int("nvn", "render_prep_threads", 0));
    if (n <= 0) n = static_cast<int>(std::clamp<unsigned>(hw / 4, 1, 4));  // Conservative: 1/4 of cores, max 4
    NXLOG_I(kTag, "background render preparation: %d threads", n);

    for (int i = 0; i < n; ++i) {
        g_state->workers.emplace_back(worker_main);
    }
    g_state->num_threads = n;
    g_enabled.store(1);
}

}  // namespace

void init() {
    std::call_once(g_once, init_once);
}

void shutdown() {
    std::call_once(g_once, [] {});  // Ensure init_once ran
    if (g_state) {
        g_state->shutdown.store(true, std::memory_order_release);
        g_state->cv_work.notify_all();
        for (auto& t : g_state->workers) {
            if (t.joinable()) t.join();
        }
        delete g_state;
        g_state = nullptr;
    }
    g_enabled.store(0);
}

bool enabled() {
    int v = g_enabled.load(std::memory_order_acquire);
    if (v < 0) {
        init();
        v = g_enabled.load(std::memory_order_acquire);
    }
    return v == 1;
}

bool build_set_async(SetBuildWork& work) {
    if (!enabled()) return false;

    State& s = S();
    auto done = std::make_shared<std::atomic<bool>>(false);
    {
        std::lock_guard<std::mutex> lk(s.mu);
        if (s.shutdown.load(std::memory_order_acquire)) return false;
        s.q.push_back({&work, done});
        s.cv_work.notify_one();
    }
    // Wait for completion (for now, we wait synchronously; can be made fully async later)
    while (!done->load(std::memory_order_acquire)) {
        std::this_thread::yield();
    }
    return true;
}

void wait_idle() {
    if (!enabled()) return;
    State& s = S();
    std::unique_lock<std::mutex> lk(s.mu);
    s.cv_work.wait(lk, [&] { return s.q.empty(); });
}

Stats stats() {
    if (!enabled()) return Stats{};
    return S().st;
}

}  // namespace nxrt::nvn::be::renderprep