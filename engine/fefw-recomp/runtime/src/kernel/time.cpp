// SPDX-License-Identifier: GPL-3.0-or-later
// System tick and high-resolution sleep. Pattern from smo-recomp runtime/os/os_threads.c sleep_ns and SuperMarioWonderRecomp kernel.cpp system_tick.
// The host calls live behind nxrt/platform.h (docs/LINUX.md, phase 2): the tick is the platform clock turned into console ticks, the wait is
// the platform timer, and this file keeps only the policy that is the console's own.
#include "nxrt/kernel.h"
#include "nxrt/game.h"
#include "nxrt/platform.h"
#include "nxrt/tt.h"
#include "kernel_internal.h"

namespace nxrt::kernel {

uint64_t tick_hz() {
    static const uint64_t hz = game::info().platform == game::Platform::Switch2 ? kTickHzSwitch2 : kTickHzSwitch1;
    return hz;
}

uint64_t system_tick() {
    if (uint64_t vt; tt::virtual_tick(&vt)) return vt;      // deterministic mode: the virtual clock (test tooling)
    const uint64_t freq = platform::clock_hz();
    const uint64_t v = platform::clock_ticks();
    const uint64_t hz = tick_hz();
    return (v / freq) * hz + (v % freq) * hz / freq;     // split so the multiplication cannot overflow
}

// SleepThread takes nanoseconds and the console timer is good to a few microseconds. The platform wait is already a high-resolution timer
// (a waitable timer on Windows, nanosleep on POSIX); what is left here is the console's own shape: a zero or negative span yields, and a
// short span is spent yielding rather than sleeping.
namespace {
void sleep_once(int64_t ns) {
    if (ns <= 0) {
        yield_host();
        return;
    }
    // Short sleeps (polls such as Pokopia's GPU-fence wait, 100 us a round) overshoot to about half a millisecond on the timer: yield until the
    // deadline instead. Longer ones keep the timer (no busy waiting while a thread idles).
    if (ns <= 200000) {
        const int64_t ticks = ns * static_cast<int64_t>(platform::clock_hz()) / 1000000000;
        const int64_t start = static_cast<int64_t>(platform::clock_ticks());
        do {
            yield_host();
        } while (static_cast<int64_t>(platform::clock_ticks()) - start < ticks);
        return;
    }
    platform::sleep_ns(ns);
}
}  // namespace

// On the thread set_blocked_hook names, a sleep longer than 10 ms is cut into 10 ms slices with the hook run between them, as in the kernel
// waits: Pokopia's main thread sleeps in SleepThread while the software keyboard applet is open (the freeze report's main stack).
void sleep_ns(int64_t ns) {
    if (tt::virtual_sleep(ns)) return;                      // deterministic mode: time advances, the thread only yields
    constexpr int64_t kSlice = 10000000;
    void (*fn)() = ns > kSlice ? detail::blocked_hook_here() : nullptr;
    if (!fn) return sleep_once(ns);
    const int64_t until = platform::now_ns() + ns;
    for (int64_t rem = ns; rem > 0; rem = until - platform::now_ns()) {
        if (rem <= kSlice) return sleep_once(rem);
        sleep_once(kSlice);
        fn();
    }
}

}  // namespace nxrt::kernel
