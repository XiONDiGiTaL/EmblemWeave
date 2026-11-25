// SPDX-License-Identifier: GPL-3.0-or-later
// Fortune's Weave: the game's file cache. Offsets are in main of update 131072.
//
// The game's memory set-up (0x651d80) sizes its heaps from a table at 0x21a6778 of 32-byte entries {name, flag, size, create}; the File entry's
// size (0x21a67a8) is "the rest" of the budget (min(available - 128 MiB, 8664 MiB) - 3896 MiB = 4768 MiB on the PC, as on a console). It is not a
// heap of its own: the file cache allocates from the main heap and keeps the bytes it holds in a global (0x2206790, an ldaddal by the allocation
// wrapper at 0x657378), evicting least-recently-used files when less than 128 MiB of that budget is free, down to 512 MiB free.
//
// [fefw] file_cache_log = 0 | seconds  (seconds between "file cache" lines; 0: no lines)
// [fefw] file_cache_mb = 0 | 512..    (0: the game's own budget. Otherwise, once the set-up has written the File budget (`str x8, [x21, #0x28]`
//                                     at 0x651fcc), it is lowered to this many MiB when that is smaller. The cache reads the table entry each time
//                                     it checks its room (the nine readers in 0x632000..0x658000), so the new budget holds from then on; the
//                                     other heaps keep their sizes. This is an explicit tuning knob, not a fix for a long-session freeze)
#include "fefw_game.h"

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <thread>

#include "nxrt/config.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"

namespace nxrt::games::fefw {

namespace {

constexpr const char* kTag = "fefw.filecache";
constexpr uint64_t kFileBudget = 0x21a67a8;          // u64: the File entry's size in the heap table, written by the set-up
constexpr uint64_t kTableDefault = 4000ull << 20;    // the File entry in the file, before the set-up
constexpr uint64_t kFileUsed = 0x2206790;            // u64: bytes the file cache holds (GetSizeOf of each block)
constexpr uint64_t kBudgetStore = 0x651fcc;          // `str x8, [x21, #0x28]` (x21 = table + 8): the computed File budget is stored
constexpr uint32_t kBudgetStoreWord = 0xf90016a8u;   // checked: the table and the code are this build's
constexpr uint64_t kUsedAdd = 0x657378;              // `ldaddal x0, x9, [x8]` on kFileUsed in the allocation wrapper
constexpr uint32_t kUsedAddWord = 0xf8e00109u;

struct State {
    uint64_t base = 0;
    int64_t cap_mb = 0;
};
State& st() {
    static State s;
    return s;
}

bool word_is(uint64_t a, uint32_t w) {
    uint32_t v = 0;
    if (!mem::is_mapped(a, 4)) return false;
    std::memcpy(&v, reinterpret_cast<const void*>(a), 4);
    return v == w;
}

uint64_t rd64(uint64_t a) {
    uint64_t v = 0;
    if (mem::is_mapped(a, 8)) std::memcpy(&v, reinterpret_cast<const void*>(a), 8);
    return v;
}

// Waits for the set-up's budget (polled every 50 ms: the set-up runs once, early in nnMain), applies file_cache_mb, then logs every period_s
// seconds (0: no lines).
void cache_thread(int period_s) {
    State& s = st();
    const double mib = 1048576.0;
    // play mode starts before the game's modules are mapped (and LoaderAccess commits the image before copying segments):
    // wait until main's code words are actually populated, not just committed.
    for (int i = 0; i < 20 * 60 && !(word_is(s.base + kUsedAdd, kUsedAddWord) && word_is(s.base + kBudgetStore, kBudgetStoreWord)); ++i)
        std::this_thread::sleep_for(std::chrono::milliseconds(50));
    if (!word_is(s.base + kUsedAdd, kUsedAddWord) || !word_is(s.base + kBudgetStore, kBudgetStoreWord)) {
        uint32_t a = 0, b = 0;
        if (mem::is_mapped(s.base + kUsedAdd, 4)) std::memcpy(&a, reinterpret_cast<const void*>(s.base + kUsedAdd), 4);
        if (mem::is_mapped(s.base + kBudgetStore, 4)) std::memcpy(&b, reinterpret_cast<const void*>(s.base + kBudgetStore), 4);
        NXLOG_W(kTag, "main's code is not update 131072's at the file cache counter (0x%08x) or the budget store (0x%08x): no file cache lines%s",
                a, b, s.cap_mb ? " and [fefw] file_cache_mb is not applied" : "");
        return;
    }
    // The entry holds 0 (before data copy) or the table's default (4000 MiB) until the set-up replaces it with the computed budget;
    // after a minute with no change the default is taken as the budget.
    const uint64_t initial = kTableDefault;
    uint64_t budget = 0;
    for (int i = 0; i < 20 * 60 && ((budget = rd64(s.base + kFileBudget)) == 0 || budget == initial); ++i)
        std::this_thread::sleep_for(std::chrono::milliseconds(50));
    if (!budget) {
        NXLOG_W(kTag, "the file cache budget reads 0: nothing applied or logged");
        return;
    }
    const uint64_t cap = static_cast<uint64_t>(s.cap_mb) << 20;
    if (cap && cap < budget) {
        std::memcpy(reinterpret_cast<void*>(s.base + kFileBudget), &cap, 8);
        NXLOG_I(kTag, "file cache budget: %llu MiB instead of the game's %llu MiB ([fefw] file_cache_mb)", (unsigned long long)(cap >> 20),
                (unsigned long long)(budget >> 20));
    } else {
        NXLOG_I(kTag, "file cache budget: the game's %llu MiB%s", (unsigned long long)(budget >> 20),
                cap ? " ([fefw] file_cache_mb is not smaller)" : "");
    }
    uint64_t last = 0;
    while (period_s > 0) {
        std::this_thread::sleep_for(std::chrono::seconds(period_s));
        const uint64_t used = rd64(s.base + kFileUsed);
        budget = rd64(s.base + kFileBudget);
        const uint64_t free_b = budget > used ? budget - used : 0;
        NXLOG_I(kTag, "file cache: %.0f MiB in use of a %.0f MiB budget (%+.0f MiB in %d s, %.0f MiB free; the game evicts at under 128 MiB free, "
                "down to 512)", used / mib, budget / mib, (static_cast<double>(used) - static_cast<double>(last)) / mib, period_s, free_b / mib);
        last = used;
    }
}

}  // namespace

void filecache_install(uint64_t base) {
    State& s = st();
    if (s.base) return;
    s.base = base;
    s.cap_mb = config_get_int("fefw", "file_cache_mb", 0);
    if (s.cap_mb > 0 && s.cap_mb < 512) s.cap_mb = 512;
    if (s.cap_mb < 0) s.cap_mb = 0;
    int period = static_cast<int>(config_get_int("fefw", "file_cache_log", 0));
    if (period < 0) period = 0;
    if (period > 0 || s.cap_mb > 0) std::thread(cache_thread, period).detach();
}

}  // namespace nxrt::games::fefw
