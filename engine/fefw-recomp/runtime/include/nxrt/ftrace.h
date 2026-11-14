// SPDX-License-Identifier: GPL-3.0-or-later
// Frame trace: a tiny zone recorder for frame-time work. Off by default (one relaxed load per zone). With [prof] trace = <file>
// (env NXRT_PROF_TRACE) every thread appends {start, duration, zone} events to a preallocated array and a writer thread flushes
// them to a text file once a second, so a run that ends by TerminateProcess loses nothing. tools/prof/ftrace_report.py turns the
// file into frame-time percentiles, stall lists and a Chrome trace.
//
//   NXFT("replay");                       // scope; recorded when it ends
//   NXFT_MIN("svc", 50000);               // scope; recorded only when it took at least 50 us
//   nxrt::ft::mark("frame");              // instant (zero duration)
#pragma once
#include <atomic>
#include <cstdint>

namespace nxrt::ft {

extern std::atomic<bool> g_on;
inline bool on() { return g_on.load(std::memory_order_relaxed); }

uint64_t now_ns();                         // monotonic, same base as FrameLimiter::now_ns
uint16_t zone_id(const char* name);        // registers a name once (static local at each site)
void event(uint16_t zone, uint64_t t0, uint64_t t1, uint32_t arg = 0);
void mark(const char* name, uint32_t arg = 0);
void frame();                              // the "frame" mark (one per presented frame); also feeds the stall sampler
void start_from_config();                  // reads [prof] trace / trace_events; a no-op when unset or already started
void stop();                               // final flush

struct Scope {
    uint16_t id;
    uint32_t arg;
    uint64_t min_ns;
    uint64_t t0;
    explicit Scope(uint16_t i, uint64_t min = 0, uint32_t a = 0) : id(i), arg(a), min_ns(min), t0(on() ? now_ns() : 0) {}
    void set_arg(uint32_t a) { arg = a; }
    ~Scope() {
        if (t0) {
            const uint64_t t1 = now_ns();
            if (t1 - t0 >= min_ns) event(id, t0, t1, arg);
        }
    }
};

}  // namespace nxrt::ft

#define NXFT_CAT2(a, b) a##b
#define NXFT_CAT(a, b) NXFT_CAT2(a, b)
#define NXFT(name) static const uint16_t NXFT_CAT(nxft_id_, __LINE__) = ::nxrt::ft::zone_id(name); ::nxrt::ft::Scope NXFT_CAT(nxft_s_, __LINE__)(NXFT_CAT(nxft_id_, __LINE__))
#define NXFT_MIN(name, min_ns) static const uint16_t NXFT_CAT(nxft_id_, __LINE__) = ::nxrt::ft::zone_id(name); ::nxrt::ft::Scope NXFT_CAT(nxft_s_, __LINE__)(NXFT_CAT(nxft_id_, __LINE__), (min_ns))
