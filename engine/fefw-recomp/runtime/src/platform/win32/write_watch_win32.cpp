// SPDX-License-Identifier: GPL-3.0-or-later
// Page write tracking by page protection (include/nxrt/write_watch.h). Four bitmaps per region, one bit per 4 KiB page:
//   prot    the page is read-only because of us (set by arm, cleared by whoever makes it writable again)
//   valid   armed and not written since (the owner's copy of the page is current)
//   written the CPU wrote it since the owner last asked (take_written)
//   ban     never arm (another view of the page exists)
// The fault handler and before_os_write may run on any thread at any time; they only use atomics, platform::protect and the region's
// read-write lock: whoever makes pages writable holds it shared, arm() and region_destroy() hold it exclusive, so a page's bit
// and its protection change together as seen by everybody else (no page ends up read-only without its bit, or armed and writable). Regions are
// published in stable, dynamically appended slot blocks. Readers count themselves in g_busy while looking at a region;
// region_destroy takes the region out of the table, then waits for that count to drop.
#include "nxrt/write_watch.h"

#include <algorithm>
#include <atomic>
#include <cstdio>
#include <map>
#include <memory>
#include <mutex>
#include <thread>
#include <utility>

#include <unordered_map>
#include <vector>

#if defined(_MSC_VER)
#include <intrin.h>
#pragma intrinsic(_BitScanForward64)
#endif

#include "nxrt/config.h"
#include "nxrt/gauge.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"
#include "nxrt/platform.h"

namespace nxrt::mem::ww {

static inline unsigned long ctzll_portable(uint64_t v) {
#if defined(_MSC_VER)
    unsigned long index = 0;
    if (!_BitScanForward64(&index, v)) return 64;   // v == 0 (the callers never pass 0)
    return index;
#else
    return v ? static_cast<unsigned long>(__builtin_ctzll(v)) : 64;   // v == 0
#endif
}

struct Region {
    const char* owner = "unspecified"; // static lifetime name supplied by owner
    uint64_t base = 0;          // address of page 0
    uint64_t npages = 0;
    uint64_t nwords = 0;
    std::unique_ptr<std::atomic<uint64_t>[]> prot, valid, written, ban;
    std::unique_ptr<std::atomic<uint64_t>[]> guard;     // no-access because of guard() (a subset of prot)
    std::atomic<Region*>* slot = nullptr;
    nxrt::platform::RWLock* rwlock = nullptr;
    std::unique_ptr<std::atomic<uint16_t>[]> releasing;   // per page: releasers that saw it armed and have not finished its protect (release_page)
};

// Protection belongs to the physical page, not a Region. Arming must exclude
// transitions in every overlapping region, including destruction and unguarding.
// Lock order: this lock, then Region::rwlock. Guard callbacks run outside both.
nxrt::platform::RWLock* g_transitions = nullptr;

struct SharedLock {
    explicit SharedLock(Region* r) : r_(r) {
        nxrt::platform::rwlock_lock_shared(g_transitions);
        nxrt::platform::rwlock_lock_shared(r_->rwlock);
    }
    ~SharedLock() {
        nxrt::platform::rwlock_unlock_shared(r_->rwlock);
        nxrt::platform::rwlock_unlock_shared(g_transitions);
    }
    Region* r_;
};

struct ExclusiveLock {
    explicit ExclusiveLock(Region* r, bool arming = false) : r_(r), arming_(arming) {
        if (arming_) nxrt::platform::rwlock_lock_exclusive(g_transitions);
        else nxrt::platform::rwlock_lock_shared(g_transitions);
        nxrt::platform::rwlock_lock_exclusive(r_->rwlock);
    }
    ~ExclusiveLock() {
        nxrt::platform::rwlock_unlock_exclusive(r_->rwlock);
        if (arming_) nxrt::platform::rwlock_unlock_exclusive(g_transitions);
        else nxrt::platform::rwlock_unlock_shared(g_transitions);
    }
    Region* r_;
    bool arming_;
};

namespace {

constexpr const char* kTag = "mem.ww";
// Blocks never move or disappear while fault handlers can run. Growth allocates only
// on region_create, under g_mu; readers need neither allocation nor a table lock.
// Empty slots are reused, bounding table storage by peak live regions (2 KiB/block).
constexpr int kSlotsPerBlock = 256;
struct SlotBlock {
    std::atomic<Region*> slots[kSlotsPerBlock]{};
    std::atomic<int> hi{0};
    std::atomic<SlotBlock*> next{nullptr};
};
SlotBlock g_slots;

// Caller participates in g_busy until done using the returned Region pointers.
struct RegionCursor {
    SlotBlock* block = &g_slots;
    int index = 0;
    int hi = block->hi.load(std::memory_order_seq_cst);
    Region* next() {
        while (block) {
            while (index < hi)
                if (Region* r = block->slots[index++].load(std::memory_order_seq_cst)) return r;
            block = block->next.load(std::memory_order_acquire);
            index = 0;
            if (block) hi = block->hi.load(std::memory_order_seq_cst);
        }
        return nullptr;
    }
};
std::atomic<int> g_busy{0};            // fault handlers and os-write calls looking at the table
std::atomic<int> g_live{0};
std::atomic<uint64_t> g_generation{0};
std::atomic<uint64_t> g_faults{0};
std::atomic<uint64_t> g_write_seq{0};   // bumped after any region marks a page written (write_seq)
std::atomic<uint64_t> g_waits{0};      // rounds a fault waited for another releaser (statistics, tests)
std::atomic<uint64_t> g_guard_faults{0};   // accesses that hit a guarded page
std::atomic<GuardHandler> g_guard_fn{nullptr};
std::mutex g_mu;                       // create / destroy

// Operating-system writes in progress (OsWriteScope): arm() leaves these ranges alone, so a page cannot become read-only between the
// release in before_os_write and the end of the ReadFile that fills it. When every slot is taken, g_os_overflow stops all arming.
constexpr int kOsSlots = 32;
struct OsRange { std::atomic<uint64_t> lo{0}, hi{0}; };
OsRange g_os[kOsSlots];
std::atomic<int> g_os_overflow{0};

bool os_write_overlaps(uint64_t lo, uint64_t hi) {
    if (g_os_overflow.load(std::memory_order_seq_cst)) return true;
    for (OsRange& o : g_os) {
        const uint64_t h = o.hi.load(std::memory_order_seq_cst);
        if (h && o.lo.load(std::memory_order_seq_cst) < hi && lo < h) return true;
    }
    return false;
}

// Import freezes (ImportFreeze): ranges whose CPU writes wait for a Vulkan import of the same pages. g_scope_mu orders them against
// OsWriteScope: a file read never starts in a frozen range, and a freeze never starts over a file read in progress.
constexpr int kFreezeSlots = 16;
OsRange g_frz[kFreezeSlots];
std::atomic<int> g_frz_n{0};
std::mutex g_scope_mu;

std::atomic<uint64_t> g_freeze_table_hits{0};   // ImportFreeze constructions that found their runs from the page table (no VirtualQuery)

bool freeze_overlaps(uint64_t lo, uint64_t hi) {
    if (!g_frz_n.load(std::memory_order_seq_cst)) return false;
    for (OsRange& f : g_frz) {
        const uint64_t h = f.hi.load(std::memory_order_seq_cst);
        if (h && f.lo.load(std::memory_order_seq_cst) < hi && lo < h) return true;
    }
    return false;
}

uint64_t bit(uint64_t page) { return uint64_t{1} << (page & 63); }

bool test(const std::atomic<uint64_t>* bm, uint64_t page) { return (bm[page >> 6].load(std::memory_order_acquire) & bit(page)) != 0; }

// Sets or clears the bits of pages [first, first + count) word by word.
void set_bits(std::atomic<uint64_t>* bm, uint64_t first, uint64_t count, bool on) {
    uint64_t p = first;
    const uint64_t end = first + count;
    while (p < end) {
        const uint64_t w = p >> 6;
        const uint64_t lo = p & 63;
        const uint64_t hi = std::min<uint64_t>(64, lo + (end - p));
        const uint64_t mask = (hi - lo == 64) ? ~uint64_t{0} : (((uint64_t{1} << (hi - lo)) - 1) << lo);
        if (on) bm[w].fetch_or(mask, std::memory_order_acq_rel);
        else bm[w].fetch_and(~mask, std::memory_order_acq_rel);
        p += hi - lo;
    }
}

bool all_set(const std::atomic<uint64_t>* bm, uint64_t first, uint64_t count) {
    uint64_t p = first;
    const uint64_t end = first + count;
    while (p < end) {
        const uint64_t w = p >> 6;
        const uint64_t lo = p & 63;
        const uint64_t hi = std::min<uint64_t>(64, lo + (end - p));
        const uint64_t mask = (hi - lo == 64) ? ~uint64_t{0} : (((uint64_t{1} << (hi - lo)) - 1) << lo);
        if ((bm[w].load(std::memory_order_acquire) & mask) != mask) return false;
        p += hi - lo;
    }
    return true;
}

bool any_set(const std::atomic<uint64_t>* bm, uint64_t first, uint64_t count) {
    uint64_t p = first;
    const uint64_t end = first + count;
    while (p < end) {
        const uint64_t w = p >> 6;
        const uint64_t lo = p & 63;
        const uint64_t hi = std::min<uint64_t>(64, lo + (end - p));
        const uint64_t mask = (hi - lo == 64) ? ~uint64_t{0} : (((uint64_t{1} << (hi - lo)) - 1) << lo);
        if (bm[w].load(std::memory_order_acquire) & mask) return true;
        p += hi - lo;
    }
    return false;
}

uint32_t page_protect(uint64_t addr) {
    return nxrt::platform::page_protect(addr);
}

// A fault can be delivered after its last owner has gone out of the table.
// Retain only a bit per retired heap page (384 KiB for the maximum heap), never
// Region pointers. Retry only if the *current* protection permits the access;
// foreign read-only/no-access/decommitted pages still pass to the crash handler.
std::atomic<uint64_t> g_retired[kHeapMax / kPage / 64]{};
bool retired_retry(uint64_t addr, int kind) {
    if (addr < kHeapBase || addr - kHeapBase >= kHeapMax || !test(g_retired, (addr - kHeapBase) / kPage)) return false;
    const uint32_t pr = page_protect(addr);
    // Check for guard page equivalent (not directly available on POSIX, skip)
    const uint32_t p = pr & 0xFF;
    // On Windows: PAGE_READWRITE=0x04, PAGE_EXECUTE_READWRITE=0x40
    // On POSIX: nxrt::platform::PROT_READ|nxrt::platform::PROT_WRITE = 3
    if (p == 0x04 || p == 0x40 || p == (nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE)) return true;
    // On Windows: PAGE_READONLY=0x02, PAGE_EXECUTE_READ=0x20
    // On POSIX: nxrt::platform::PROT_READ = 1
    return kind == 0 && (p == 0x02 || p == 0x20 || p == nxrt::platform::PROT_READ);
}

// A final rejection must not sample a replacement arm halfway through its
// protection change (its generation is published before releasing this lock).
int final_retry(uint64_t addr, int kind, uint64_t generation) {
    nxrt::platform::rwlock_lock_shared(g_transitions);
    const bool accessible = retired_retry(addr, kind);
    const bool changed = g_generation.load(std::memory_order_seq_cst) != generation;
    nxrt::platform::rwlock_unlock_shared(g_transitions);
    return accessible ? 1 : changed ? -1 : 0;
}

// A write fault on a heap page that is read-write by the time this handler looks, with no region claiming it: the page was read-only
// only for a moment, nearly always an import freeze (ImportFreeze) that ended between the fault and freeze_overlaps in on_fault (the
// destructor gives the pages back and only then clears its slot, so the freeze is gone before the handler sees it). The write can simply
// run again. retired_retry does the same for retired pages only. Seen on an Arc B390 in the ObeliskChamber fork (its issue 81, fixed in
// its PR 93): a guest write to the first page of a 4 KiB pool being imported crashed with the page read-write in the report and no owner.
// Bounded per thread and page, so a fault that keeps coming back still reports.
std::atomic<uint64_t> g_late_retries{0};
bool heap_write_landed_late(uint64_t addr) {
    if (addr < kHeapBase || addr - kHeapBase >= kHeapMax) return false;
    thread_local uint64_t t_page = 0;
    thread_local int t_count = 0;
    const uint64_t pg = addr & ~(kPage - 1);
    if (pg != t_page) {
        t_page = pg;
        t_count = 0;
    }
    if (++t_count > 8) return false;
    const uint32_t pr = page_protect(addr);
    const uint32_t p = pr & 0xFF;
    if (p != 0x04 && p != 0x40 && p != (nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE)) return false;
    if (g_late_retries.fetch_add(1, std::memory_order_relaxed) == 0)
        NXLOG_W(kTag, "write fault at %llx on a page that is read-write again (an import freeze ended meanwhile): the write runs again",
                static_cast<unsigned long long>(addr));
    return true;
}

// One page of a region becomes writable and counts as written. True when the page is ours to handle (the faulting write may retry).
// The caller holds the region's lock shared.
bool release_page(Region* r, uint64_t page) {
    const uint64_t b = bit(page);
    const uint64_t w = page >> 6;
    // Announce the release before the bit goes: whoever finds the bit already clear can then tell "somebody is making this page writable"
    // (the count is up from before the bit went until after the protect) from "nobody is": a foreign read-only page.
    std::atomic<uint16_t>& rel = r->releasing[page];
    const bool armed = (r->prot[w].load(std::memory_order_acquire) & b) != 0;
    if (armed) rel.fetch_add(1, std::memory_order_seq_cst);
    const uint64_t old = r->prot[w].fetch_and(~b, std::memory_order_acq_rel);
    r->valid[w].fetch_and(~b, std::memory_order_acq_rel);
    r->written[w].fetch_or(b, std::memory_order_acq_rel);
    g_write_seq.fetch_add(1, std::memory_order_seq_cst);
    const uint64_t addr = r->base + page * kPage;
    if (old & b) {
        bool ok = true;
        // On Windows, we need to check the old protection. On POSIX, we just set RW.
        // Use platform::protect with writable=true
        if (!nxrt::platform::protect(reinterpret_cast<void*>(addr), kPage, nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE)) {
            ok = false;
        }
        rel.fetch_sub(1, std::memory_order_seq_cst);
        return ok;
    }
    if (armed) rel.fetch_sub(1, std::memory_order_seq_cst);   // lost the race for the bit: not a releaser (and must not make others wait for it)
    // Not armed (any more): another releaser (also holding the lock shared) may be making it writable right now. Wait for it only while
    // such a release is in progress; with none, a read-only page is somebody else's and the fault goes on at once. The count is read before
    // the protection: a releaser that cleared the bit before our fetch_and has counted itself in before that, so a zero count means every
    // such release has finished and its effect is visible in the protection we read next.
    // The wait has no round limit: a releaser that is counted in will finish (it only has its protect left), but it may be descheduled
    // for a long time on a busy machine, and giving up early would pass a fault on that is ours (seen as an unhandled access violation).
    for (int i = 0;; ++i) {
        const uint16_t busy = rel.load(std::memory_order_seq_cst);
        const uint32_t pr = page_protect(addr) & 0xFF;
        if (pr == 0x04 || pr == 0x40 || pr == (nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE)) return true;
        if (pr != 0x02 && pr != 0x20 && pr != nxrt::platform::PROT_READ) return false;
        if (busy == 0) return false;
        g_waits.fetch_add(1, std::memory_order_relaxed);
        if (i < 50) nxrt::platform::yield_cpu();
        else if (i < 1000) nxrt::platform::sleep_ns(0);
        else nxrt::platform::sleep_ns(1000000);  // 1 ms
    }
}

// Coalesces contiguous armed pages ahead of a sequential write fault into a single VirtualProtect call.
// Caller holds SharedLock(r) and g_busy > 0. Returns the number of extra pages released starting at `start_page`.
uint64_t release_pages_ahead(Region* r, uint64_t start_page, uint64_t max_count) {
    if (start_page >= r->npages || max_count == 0) return 0;
    const uint64_t limit = std::min(r->npages, start_page + max_count);
    uint64_t cand = 0;
    for (uint64_t p = start_page; p < limit; ++p) {
        if (!test(r->prot.get(), p) || test(r->guard.get(), p)) break;
        const uint64_t a = r->base + p * kPage;
        if (freeze_overlaps(a, a + kPage)) break;
        bool other_armed = false;
        for (RegionCursor cursor; Region* o = cursor.next();) {
            if (o == r || a < o->base || a >= o->base + o->npages * kPage) continue;
            const uint64_t opg = (a - o->base) / kPage;
            if (test(o->prot.get(), opg) || test(o->guard.get(), opg)) { other_armed = true; break; }
        }
        if (other_armed) break;
        ++cand;
    }
    if (cand == 0) return 0;
    uint64_t claimed = 0;
    for (uint64_t i = 0; i < cand; ++i) {
        const uint64_t p = start_page + i;
        const uint64_t b = bit(p);
        const uint64_t w = p >> 6;
        std::atomic<uint16_t>& rel = r->releasing[p];
        rel.fetch_add(1, std::memory_order_seq_cst);
        const uint64_t old = r->prot[w].fetch_and(~b, std::memory_order_acq_rel);
        if (!(old & b)) {
            rel.fetch_sub(1, std::memory_order_seq_cst);
            break;
        }
        r->valid[w].fetch_and(~b, std::memory_order_acq_rel);
        r->written[w].fetch_or(b, std::memory_order_seq_cst);
        ++claimed;
    }
    if (claimed > 0) {
        g_write_seq.fetch_add(1, std::memory_order_seq_cst);
        nxrt::platform::protect(reinterpret_cast<void*>(r->base + start_page * kPage), static_cast<std::size_t>(claimed * kPage),
                               nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE);
        for (uint64_t i = 0; i < claimed; ++i) r->releasing[start_page + i].fetch_sub(1, std::memory_order_seq_cst);
    }
    return claimed;
}

// [diag] ww_stats (env NXRT_DIAG_WW_STATS=1): faults per region and per 64 KiB block, the 10 busiest blocks logged every 10 s.
std::mutex g_fs_lock;
std::map<std::pair<uint64_t, uint64_t>, uint64_t> g_fs_blocks;   // (region base, 64 KiB block address) -> faults
std::unordered_map<uint64_t, uint64_t> g_fs_regions;     // region base -> faults
uint64_t g_fs_since = 0;
void fault_stat(const Region* r, uint64_t addr) {
    static const bool on = config_get_bool("diag", "ww_stats", false);
    if (!on) return;
    std::lock_guard<std::mutex> lk(g_fs_lock);
    g_fs_blocks[{r->base, addr & ~0xffffull}]++;
    g_fs_regions[r->base]++;
    const uint64_t now = nxrt::platform::tick_count_ms();
    if (!g_fs_since) g_fs_since = now;
    if (now - g_fs_since < 10000) return;
    const double secs = static_cast<double>(now - g_fs_since) / 1000.0;
    std::vector<std::pair<std::pair<uint64_t, uint64_t>, uint64_t>> v(g_fs_blocks.begin(), g_fs_blocks.end());
    std::sort(v.begin(), v.end(), [](const auto& a, const auto& b) { return a.second > b.second; });
    for (const auto& [base, n] : g_fs_regions) NXLOG_I("ww", "faults over %.1f s: region %llx: %.0f/s", secs, static_cast<unsigned long long>(base), n / secs);
    for (size_t i = 0; i < v.size() && i < 10; ++i)
        NXLOG_I("ww", "  region %llx block %llx: %.0f faults/s", static_cast<unsigned long long>(v[i].first.first), static_cast<unsigned long long>(v[i].first.second),
                v[i].second / secs);
    g_fs_blocks.clear();
    g_fs_regions.clear();
    g_fs_since = now;
}

// The guarded pages among [first, first + count) become read-only (armed) again. The caller holds the region's lock exclusive.
void unguard_locked(Region* r, uint64_t first, uint64_t count) {
    for (uint64_t p = first; p < first + count;) {
        if (!test(r->guard.get(), p)) { ++p; continue; }
        uint64_t q = p;
        while (q < first + count && test(r->guard.get(), q)) ++q;
        // protection first, then the bit: a fault that finds the bit clear must find the page accessible (the read path takes no lock)
        nxrt::platform::protect(reinterpret_cast<void*>(r->base + p * kPage), static_cast<std::size_t>((q - p) * kPage), nxrt::platform::PROT_READ);  // read-only
        set_bits(r->guard.get(), p, q - p, false);
        p = q;
    }
}

bool guarded_at(uint64_t addr) {
    bool any = false;
    g_busy.fetch_add(1, std::memory_order_seq_cst);
    for (RegionCursor cursor; Region* r = cursor.next();) {
        if (any) break;
        if (addr < r->base || addr >= r->base + r->npages * kPage) continue;
        any = test(r->guard.get(), (addr - r->base) / kPage);
    }
    g_busy.fetch_sub(1, std::memory_order_seq_cst);
    return any;
}

void drop_guard(uint64_t addr, uint64_t size);   // below

// Platform-independent fault handler. Returns true if the fault was handled (page made accessible),
// false if the fault should continue to the default handler (crash).
bool on_fault_handler(uint64_t addr, int kind) {
    // A write into a range an import has frozen (ImportFreeze): wait for the import, then let the write run again (the page is
    // read-write again, or armed and handled below on the next fault).
    if (kind == 1 && freeze_overlaps(addr, addr + 1)) {
        for (int i = 0; freeze_overlaps(addr, addr + 1); ++i) {
            if (i < 64) nxrt::platform::yield_cpu();
            else nxrt::platform::sleep_ns(i < 1000 ? 0 : 1000000);
        }
        return true;  // fault handled, retry the write
    }
retry:
    const uint64_t generation = g_generation.load(std::memory_order_seq_cst);
    nxrt::HitchTimer ht(nxrt::kHpFaults);      // the hitch line: time in this handler (a wait for arm's exclusive lock included)
    // A guarded page (read or write): the owner makes it current first (outside every lock: it may wait for the GPU). A write then faults
    // again on the read-only page and is tracked as usual.
    if (guarded_at(addr)) {
        g_guard_faults.fetch_add(1, std::memory_order_relaxed);
        if (GuardHandler fn = g_guard_fn.load(std::memory_order_acquire)) fn(addr & ~(kPage - 1));
        drop_guard(addr & ~(kPage - 1), kPage);   // still guarded (no handler, or it found nothing): read-only again, as it is
        return true;
    }
    if (kind != 1) {
        // a guard dropped by another thread since the access: the page is readable again, let the read retry
        const uint32_t pr = page_protect(addr) & 0xff;
        if (pr == 0x02 || pr == 0x04 || pr == 0x20 || pr == 0x40 || pr == nxrt::platform::PROT_READ || pr == (nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE)) {
            bool in_region = false;
            g_busy.fetch_add(1, std::memory_order_seq_cst);
            for (RegionCursor cursor; Region* r = cursor.next();) {
                if (addr >= r->base && addr < r->base + r->npages * kPage) { in_region = true; break; }
            }
            g_busy.fetch_sub(1, std::memory_order_seq_cst);
            if (in_region) return true;
        }
        // Check for no-access (PAGE_NOACCESS = 0x01, nxrt::platform::PROT_NONE = 0)
        if ((pr == 0x01 || pr == 0) && guarded_at(addr)) {    // dropped and guarded again since the check above
            g_guard_faults.fetch_add(1, std::memory_order_relaxed);
            if (GuardHandler fn = g_guard_fn.load(std::memory_order_acquire)) fn(addr & ~(kPage - 1));
            drop_guard(addr & ~(kPage - 1), kPage);
            return true;
        }
        const int decision = final_retry(addr, kind, generation);
        if (decision == 1) return true;
        if (decision == -1) goto retry;
        // A read fault on a no-access page is restored only while an import freeze covers it: the freeze made the page
        // no-access for the Vulkan import and the reader must not see a fault the guest did not cause. Every other
        // no-access page is a real fault (a guest decommit, a stack guard, a page the guest made inaccessible) and
        // goes to the crash handler. Rescuing any no-access page in the heap range used to hide those bugs and leave
        // host protection disagreeing with the page table (issue 77); the write path lost the same fallbacks in
        // e9c0ccc9, this is the read half.
        if (kind != 1 && (pr == 0x01 || pr == 0) && freeze_overlaps(addr, addr + 1)) {
            NXLOG_W(kTag, "read fault on freeze page %llx (was protection %x): readable again",
                    static_cast<unsigned long long>(addr), pr);
            if (nxrt::platform::protect(reinterpret_cast<void*>(addr & ~(kPage - 1)), kPage,
                                        nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE))
                return true;
            NXLOG_E(kTag, "read fault: protect failed for freeze page %llx", static_cast<unsigned long long>(addr));
        }
        return false;  // not handled
    }
    bool ours = false, guarded_now = false;
    g_busy.fetch_add(1, std::memory_order_seq_cst);
    // Pass 0 releases the page in the regions that armed it (usually one). Only when none did (another thread released it meanwhile) does
    // pass 1 ask release_page's slow path in every region.
    for (int pass = 0; pass < 2 && !ours && !guarded_now; ++pass) {
        for (RegionCursor cursor; Region* r = cursor.next();) {
            if (addr < r->base || addr >= r->base + r->npages * kPage) continue;
            SharedLock lk(r);
            const uint64_t pg = (addr - r->base) / kPage;
            // guard() (exclusive lock) may have guarded the page since guarded_at above: under the shared lock the bit is final. Releasing a
            // guarded page cleared its armed bit and left it no-access with nobody to claim the fault (a crash in Dagsion, notes/pacing.md).
            if (test(r->guard.get(), pg)) { guarded_now = true; continue; }
            if (pass == 0 && !test(r->prot.get(), pg)) continue;
            const bool mine = release_page(r, pg);
            if (mine) {
                fault_stat(r, addr);
                if (pass == 0) {
                    // Sequential bulk-write coalescing: after 2+ consecutive sequential page faults on the same region,
                    // release a small contiguous run (8-16 pages = 32-64 KiB) in one VirtualProtect call.
                    thread_local const Region* tl_seq_r = nullptr;
                    thread_local uint64_t tl_seq_next_pg = ~0ull;
                    thread_local uint32_t tl_seq_run = 0;
                    if (r == tl_seq_r && pg == tl_seq_next_pg) {
                        ++tl_seq_run;
                        uint64_t extra = 0;
                        if (tl_seq_run >= 2) extra = release_pages_ahead(r, pg + 1, tl_seq_run >= 6 ? 15 : 7);
                        tl_seq_next_pg = pg + 1 + extra;
                    } else {
                        tl_seq_r = r;
                        tl_seq_next_pg = pg + 1;
                        tl_seq_run = 0;
                    }
                }
            }
            ours |= mine;
        }
    }
    if (guarded_now) {     // the guard fault as above; the write then faults again on the armed page and is released as usual
        g_busy.fetch_sub(1, std::memory_order_seq_cst);
        g_guard_faults.fetch_add(1, std::memory_order_relaxed);
        if (GuardHandler fn = g_guard_fn.load(std::memory_order_acquire)) fn(addr & ~(kPage - 1));
        drop_guard(addr & ~(kPage - 1), kPage);
        return true;
    }
    if (!ours) {
        const int decision = final_retry(addr, kind, generation);
        ours = decision == 1;
        if (decision == -1) {
            g_busy.fetch_sub(1, std::memory_order_seq_cst);
            goto retry; // a replacement region may now own the protection
        }
    }
    if (!ours && heap_write_landed_late(addr)) {   // an import freeze ended between the fault and freeze_overlaps: run the write again
        g_busy.fetch_sub(1, std::memory_order_seq_cst);
        return true;
    }
    if (!ours) {   // a write fault in a tracked region that no region claims: what each one holds for the page (the crash report follows)
        const uint32_t pr = page_protect(addr);
        NXLOG_E(kTag, "FAULT DEBUG: kind=%d addr=%llx pr=%x in_heap=%d heap_base=%llx heap_max=%llx",
                kind, static_cast<unsigned long long>(addr), pr,
                (addr >= kHeapBase && addr < kHeapBase + kHeapMax) ? 1 : 0,
                static_cast<unsigned long long>(kHeapBase), static_cast<unsigned long long>(kHeapMax));
        for (RegionCursor cursor; Region* r = cursor.next();) {
            if (addr < r->base || addr >= r->base + r->npages * kPage) continue;
            const uint64_t pg = (addr - r->base) / kPage;
            NXLOG_E(kTag, "unclaimed write fault at %llx (protection %x): region %llx (+%llu pages) prot %d valid %d guard %d ban %d releasing %d",
                    static_cast<unsigned long long>(addr), pr, static_cast<unsigned long long>(r->base),
                    static_cast<unsigned long long>(r->npages), test(r->prot.get(), pg), test(r->valid.get(), pg), test(r->guard.get(), pg),
                    test(r->ban.get(), pg), static_cast<int>(r->releasing[pg].load()));
        }
        // No broad fallback: a foreign read-only page, a guarded page, or a decommitted (NOACCESS) page must reach
        // the crash handler. A page this tracker retired is retried only while its current protection permits the
        // access (retired_retry), and an import freeze that has ended is handled by heap_write_landed_late above.
    }
    g_busy.fetch_sub(1, std::memory_order_seq_cst);
    if (!ours) return false;
    g_faults.fetch_add(1, std::memory_order_relaxed);
    return true;
}

// Calls fn(region, first page, count) for the part of [addr, addr + size) inside every published region.
template <class Fn>
void for_regions(uint64_t addr, uint64_t size, Fn&& fn) {
    if (g_live.load(std::memory_order_acquire) == 0 || size == 0) return;
    g_busy.fetch_add(1, std::memory_order_seq_cst);
    for (RegionCursor cursor; Region* r = cursor.next();) {
        const uint64_t lo = std::max(addr, r->base), end = std::min(addr + size, r->base + r->npages * kPage);
        if (lo >= end) continue;
        const uint64_t first = (lo - r->base) / kPage, last = (end - 1 - r->base) / kPage;
        SharedLock lk(r);
        fn(r, first, last - first + 1);
    }
    g_busy.fetch_sub(1, std::memory_order_seq_cst);
}

void drop_guard(uint64_t addr, uint64_t size) {
    if (g_live.load(std::memory_order_acquire) == 0 || size == 0) return;
    g_busy.fetch_add(1, std::memory_order_seq_cst);
    for (RegionCursor cursor; Region* r = cursor.next();) {
        const uint64_t lo = std::max(addr, r->base), end = std::min(addr + size, r->base + r->npages * kPage);
        if (lo >= end) continue;
        const uint64_t first = (lo - r->base) / kPage, last = (end - 1 - r->base) / kPage;
        if (!any_set(r->guard.get(), first, last - first + 1)) continue;
        ExclusiveLock lk(r);
        unguard_locked(r, first, last - first + 1);
    }
    g_busy.fetch_sub(1, std::memory_order_seq_cst);
}

struct Gauges {
    Gauges() {
        nxrt::gauge_register("mem.ww.faults", [] { return static_cast<int64_t>(g_faults.load(std::memory_order_relaxed)); });
        nxrt::gauge_register("mem.ww.regions", [] { return static_cast<int64_t>(g_live.load(std::memory_order_relaxed)); });
    }
    ~Gauges() {
        nxrt::gauge_unregister("mem.ww.faults");
        nxrt::gauge_unregister("mem.ww.regions");
    }
};
Gauges g_gauges;

}  // namespace

Region* region_create(uint64_t addr, uint64_t size, const char* owner) {
    if (!size || !heap_exists()) return nullptr;
    const uint64_t start = addr & ~(kPage - 1), end = (addr + size + kPage - 1) & ~(kPage - 1);
    if (start < kHeapBase || end > kHeapBase + heap_reserved()) return nullptr;
    auto r = std::make_unique<Region>();
    r->owner = owner ? owner : "unspecified";
    r->base = start;
    r->npages = (end - start) / kPage;
    r->nwords = (r->npages + 63) / 64;
    for (auto* bm : {&r->prot, &r->valid, &r->written, &r->ban, &r->guard}) {
        bm->reset(new std::atomic<uint64_t>[r->nwords]);
        for (uint64_t i = 0; i < r->nwords; ++i) (*bm)[i].store(0, std::memory_order_relaxed);
    }
    r->releasing.reset(new std::atomic<uint16_t>[r->npages]);
    for (uint64_t i = 0; i < r->npages; ++i) r->releasing[i].store(0, std::memory_order_relaxed);
    r->rwlock = nxrt::platform::rwlock_create();
    {
        std::lock_guard<std::mutex> lk(g_mu);
        if (!g_transitions) g_transitions = nxrt::platform::rwlock_create();
        // Every NVN pool region installs the same process-wide handler. Log the
        // first installation only: at one pair per pool import this was 27% of
        // a session's log and two log calls in a startup-critical path.
        static bool handler_announced = false;
        if (!handler_announced) NXLOG_I(kTag, "region_create: installing page fault handler");
        if (!nxrt::platform::page_fault_handler_install(on_fault_handler)) {
            NXLOG_W(kTag, "page_fault_handler_install failed: no write tracking");
            return nullptr;
        }
        if (!handler_announced) {
            NXLOG_I(kTag, "region_create: page fault handler installed successfully");
            handler_announced = true;
        }
        SlotBlock* block = &g_slots;
        for (;;) {
            int slot = 0;
            while (slot < kSlotsPerBlock && block->slots[slot].load(std::memory_order_seq_cst)) ++slot;
            if (slot < kSlotsPerBlock) {
                r->slot = &block->slots[slot];
                r->slot->store(r.get(), std::memory_order_seq_cst);
                if (slot >= block->hi.load()) block->hi.store(slot + 1, std::memory_order_seq_cst);
                break;
            }
            SlotBlock* next = block->next.load(std::memory_order_acquire);
            if (!next) {
                next = new SlotBlock;
                block->next.store(next, std::memory_order_release);
            }
            block = next;
        }
        g_live.fetch_add(1, std::memory_order_seq_cst);
        g_generation.fetch_add(1, std::memory_order_seq_cst);
    }
    // Pages that another view shows can never be tracked. Published first, so an alias made from now on bans them through ban_section.
    std::vector<std::pair<uint64_t, uint64_t>> views;
    heap_alias_ranges(views);
    for (const auto& v : views) {
        const uint64_t lo = std::max(start, kHeapBase + v.first), hi = std::min(end, kHeapBase + v.first + v.second);
        if (lo < hi) set_bits(r->ban.get(), (lo - start) / kPage, (hi - lo) / kPage, true);
    }
    return r.release();
}

void region_destroy(Region* r) {
    if (!r) return;
    // Writable again first (faults on armed pages wait for the lock and then find the page writable), then out of the table.
    {
    ExclusiveLock lk(r);
    // Every page this region protects: armed (read-only) or guarded (no access). A guarded page can have lost its armed bit (release_page
    // on a page that guard() took again after the caller dropped its guard, as before_os_write does outside the lock): skipped here, it
    // stayed no-access after the region was gone, with nobody to claim the fault when the memory was used again.
    const auto held = [r](uint64_t i) { return test(r->prot.get(), i) || test(r->guard.get(), i); };
    for (uint64_t p = 0; p < r->npages;) {
        if (!held(p)) { ++p; continue; }
        uint64_t q = p;
        while (q < r->npages && held(q)) ++q;
        set_bits(g_retired, (r->base - kHeapBase) / kPage + p, q - p, true);
        set_bits(r->prot.get(), p, q - p, false);
        set_bits(r->guard.get(), p, q - p, false);
        nxrt::platform::protect(reinterpret_cast<void*>(r->base + p * kPage), static_cast<std::size_t>((q - p) * kPage), nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE);  // writable
        p = q;
    }
    }
    {
        std::lock_guard<std::mutex> lk(g_mu);
        r->slot->store(nullptr, std::memory_order_seq_cst);
        g_live.fetch_sub(1, std::memory_order_seq_cst);
        g_generation.fetch_add(1, std::memory_order_seq_cst);
        // Do not remove the page fault handler here. It must stay installed to handle
        // faults on retired pages (pages that were tracked by now-destroyed regions).
        // The handler correctly handles the case when g_live == 0 by checking
        // retired_retry() and heap_write_landed_late().
        // The handler is installed on first region_create and stays for the process lifetime.
    }
    while (g_busy.load(std::memory_order_seq_cst) != 0) std::this_thread::yield();
    nxrt::platform::rwlock_destroy(r->rwlock);
    delete r;
}

std::string describe_page(uint64_t addr) {
    std::string out;
    g_busy.fetch_add(1, std::memory_order_seq_cst);
    unsigned matches = 0;
    for (RegionCursor cursor; Region* r = cursor.next();) {
        if (addr < r->base || addr - r->base >= r->npages * kPage) continue;
        const uint64_t pg = (addr - r->base) / kPage;
        char line[256];
        std::snprintf(line, sizeof line, "  write-watch owner=%s base=0x%llx pages=%llu page=%llu prot=%d valid=%d guard=%d ban=%d releasing=%u\n",
            r->owner, (unsigned long long)r->base, (unsigned long long)r->npages, (unsigned long long)pg,
            test(r->prot.get(), pg), test(r->valid.get(), pg), test(r->guard.get(), pg), test(r->ban.get(), pg),
            (unsigned)r->releasing[pg].load());
        out += line;
        ++matches;
    }
    g_busy.fetch_sub(1, std::memory_order_seq_cst);
    if (!matches) out += "  write-watch owner=none\n";
    if (addr >= kHeapBase && addr - kHeapBase < kHeapMax && test(g_retired, (addr - kHeapBase) / kPage))
        out += "  write-watch retired-page history=1\n";
    return out;
}

uint64_t region_base(const Region* r) { return r ? r->base : 0; }
uint64_t region_pages(const Region* r) { return r ? r->npages : 0; }

bool arm(Region* r, uint64_t first, uint64_t count) {
    if (!r || !count || first >= r->npages || count > r->npages - first) return false;
    nxrt::HitchTimer ht(nxrt::kHpWwArm);
    const uint64_t a = r->base + first * kPage, n = count * kPage;
    // The heap's page table says whether every page is committed read-write (memory.h heap_rw_pages). A page_query walk over the heap
    // view costs a walk of every page of the run of identical pages after the address, under the exclusive
    // lock that the game's write faults wait for. The table
    // is read before the lock (memory.cpp takes its lock first and region locks inside it); a change since shows in the epoch.
    const uint64_t epoch = heap_epoch();
    const int table = heap_rw_pages(a, n);
    ExclusiveLock lk(r, true);
    if (any_set(r->ban.get(), first, count)) return false;
    // A file read (or a mapping change in memory.cpp) into these pages is in progress: leave them alone. Its scope is published before it
    // releases pages under the shared lock, so either this sees it, or its release waits for this arm and undoes it.
    if (os_write_overlaps(a, a + n) || freeze_overlaps(a, a + n)) return false;
    if (table == 1 && heap_epoch() == epoch) {
        // what the walk below checks besides the table: no page read-guarded here, or read-only because another region armed it
        if (any_set(r->guard.get(), first, count)) return false;
        bool other = false;
        g_busy.fetch_add(1, std::memory_order_seq_cst);
        for (RegionCursor cursor; Region* o = cursor.next();) {
            if (other) break;
            if (o == r) continue;
            const uint64_t lo = std::max(a, o->base), end = std::min(a + n, o->base + o->npages * kPage);
            if (lo < end) other = any_set(o->prot.get(), (lo - o->base) / kPage, (end - lo) / kPage);
        }
        g_busy.fetch_sub(1, std::memory_order_seq_cst);
        if (other) return false;
    } else if (table == 0) {
        return false;
    } else {
        // every page committed read-write (the guest may have changed a protection; a page it made read-only is not ours to take)
        for (uint64_t cur = a; cur < a + n;) {
            nxrt::platform::PageState state;
            nxrt::platform::PageProtect protect;
            uint64_t region_base, region_size;
            if (!nxrt::platform::page_query(cur, &state, &protect, &region_base, &region_size) || state != nxrt::platform::PageState::Commit) return false;
            const uint32_t pr = static_cast<uint32_t>(protect) & 0xff;
            const bool already = pr == 0x02 && all_set(r->prot.get(), (cur - r->base) / kPage,
                                                                (std::min<uint64_t>(a + n, region_base + region_size) - cur) / kPage);
            if ((pr != 0x04 && !already) || (pr & 0x100)) return false;
            cur = std::min<uint64_t>(a + n, region_base + region_size);
        }
    }
    // Under the exclusive lock no fault is handled until both bits and protection are in place; a write that faults meanwhile waits and
    // then releases the page. A write that lands before the protection does not fault, and the owner's copy (made later) still sees it.
    set_bits(r->prot.get(), first, count, true);
    set_bits(r->valid.get(), first, count, true);
    if (!nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(n), nxrt::platform::PROT_READ)) {  // read-only
        set_bits(r->valid.get(), first, count, false);
        set_bits(r->prot.get(), first, count, false);
        return false;
    }
    g_generation.fetch_add(1, std::memory_order_seq_cst);
    return true;
}

bool all_valid(const Region* r, uint64_t first, uint64_t count) {
    if (!r || !count || first >= r->npages || count > r->npages - first) return false;
    return all_set(r->valid.get(), first, count);
}

bool page_valid(const Region* r, uint64_t page) { return r && page < r->npages && test(r->valid.get(), page); }
const std::atomic<uint64_t>* valid_bitmap(const Region* r) { return r ? r->valid.get() : nullptr; }
bool any_valid(const Region* r, uint64_t first, uint64_t count) {
    if (!r || !count || first >= r->npages) return false;
    const uint64_t end = first + std::min(count, r->npages - first);
    for (uint64_t p = first; p < end;) {
        const uint64_t lo = p & 63;
        const uint64_t hi = std::min<uint64_t>(64, lo + (end - p));
        const uint64_t mask = (hi - lo == 64) ? ~uint64_t{0} : (((uint64_t{1} << (hi - lo)) - 1) << lo);
        if (r->valid[p >> 6].load(std::memory_order_acquire) & mask) return true;
        p += hi - lo;
    }
    return false;
}
bool page_banned(const Region* r, uint64_t page) { return !r || page >= r->npages || test(r->ban.get(), page); }

void invalidate(Region* r, uint64_t first, uint64_t count) {
    if (!r || first >= r->npages) return;
    count = std::min(count, r->npages - first);
    if (count) set_bits(r->valid.get(), first, count, false);
}

void set_guard_handler(GuardHandler fn) { g_guard_fn.store(fn, std::memory_order_release); }

bool guard(Region* r, uint64_t first, uint64_t count) {
    if (!r || !count || first >= r->npages || count > r->npages - first) return false;
    ExclusiveLock lk(r);
    if (!all_set(r->prot.get(), first, count) || any_set(r->ban.get(), first, count)) return false;
    const uint64_t a = r->base + first * kPage, n = count * kPage;
    if (os_write_overlaps(a, a + n) || freeze_overlaps(a, a + n)) return false;
    set_bits(r->guard.get(), first, count, true);
    // Use platform::protect with writable=false for read-only, then we need no-access
    // On Windows: PAGE_NOACCESS, on POSIX: nxrt::platform::PROT_NONE
    if (!nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(n), nxrt::platform::PROT_NONE)) {
        // Try to revert to read-only
        nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(n), nxrt::platform::PROT_READ);
        set_bits(r->guard.get(), first, count, false);
        return false;
    }
    return true;
}

void unguard(Region* r, uint64_t first, uint64_t count) {
    if (!r || first >= r->npages) return;
    count = std::min(count, r->npages - first);
    if (!count || !any_set(r->guard.get(), first, count)) return;
    ExclusiveLock lk(r);
    unguard_locked(r, first, count);
}

bool all_armed(const Region* r, uint64_t first, uint64_t count) {
    if (!r || !count || first >= r->npages || count > r->npages - first) return false;
    return all_set(r->prot.get(), first, count);
}

bool all_valid_or_armed(const Region* r, uint64_t first, uint64_t count, const std::atomic<uint64_t>* alt) {
    if (!r || !alt || !count || first >= r->npages || count > r->npages - first) return false;
    uint64_t p = first;
    const uint64_t end = first + count;
    while (p < end) {
        const uint64_t w = p >> 6;
        const uint64_t lo = p & 63;
        const uint64_t hi = std::min<uint64_t>(64, lo + (end - p));
        const uint64_t mask = (hi - lo == 64) ? ~uint64_t{0} : (((uint64_t{1} << (hi - lo)) - 1) << lo);
        const uint64_t ok = r->valid[w].load(std::memory_order_acquire) |
                            (r->prot[w].load(std::memory_order_acquire) & alt[w].load(std::memory_order_acquire));
        if ((ok & mask) != mask) return false;
        p += hi - lo;
    }
    return true;
}

uint64_t guard_faults() { return g_guard_faults.load(std::memory_order_relaxed); }

uint64_t write_seq() { return g_write_seq.load(std::memory_order_seq_cst); }

uint64_t count_valid(const Region* r) {
    if (!r) return 0;
    uint64_t n = 0;
    for (uint64_t w = 0; w < r->nwords; ++w) n += static_cast<uint64_t>(__builtin_popcountll(r->valid[w].load(std::memory_order_relaxed)));
    return n;
}

void take_written(Region* r, std::vector<uint64_t>& out) {
    if (!r) return;
    for (uint64_t w = 0; w < r->nwords; ++w) {
        if (!r->written[w].load(std::memory_order_relaxed)) continue;
        uint64_t v = r->written[w].exchange(0, std::memory_order_acq_rel);
        while (v) {
            const unsigned long b = static_cast<unsigned long>(ctzll_portable(v));
            v &= v - 1;
            const uint64_t page = w * 64 + b;
            if (page < r->npages) out.push_back(page);
        }
    }
}

namespace {
// An OS-call slot over [p, p + n) (OsWriteScope, OsReadScope): arm() and guard() leave the range alone while it is held.
int os_slot_acquire(const void* p, uint64_t n) {
    const uint64_t lo = reinterpret_cast<uint64_t>(p) & ~(kPage - 1), hi = (reinterpret_cast<uint64_t>(p) + n + kPage - 1) & ~(kPage - 1);
    for (int i = 0; i < kOsSlots; ++i) {
        uint64_t expect = 0;
        if (g_os[i].hi.load(std::memory_order_relaxed) == 0 && g_os[i].lo.compare_exchange_strong(expect, lo | 1, std::memory_order_seq_cst)) {
            g_os[i].lo.store(lo, std::memory_order_seq_cst);
            g_os[i].hi.store(hi, std::memory_order_seq_cst);
            return i;
        }
    }
    g_os_overflow.fetch_add(1, std::memory_order_seq_cst);
    return kOsSlots;
}

void os_slot_release(int slot) {
    if (slot < 0) return;
    if (slot == kOsSlots) {
        g_os_overflow.fetch_sub(1, std::memory_order_seq_cst);
        return;
    }
    g_os[slot].hi.store(0, std::memory_order_seq_cst);
    g_os[slot].lo.store(0, std::memory_order_seq_cst);
}
}  // namespace

OsWriteScope::OsWriteScope(const void* p, uint64_t n) {
    if (!n) return;
    const uint64_t lo = reinterpret_cast<uint64_t>(p) & ~(kPage - 1), hi = (reinterpret_cast<uint64_t>(p) + n + kPage - 1) & ~(kPage - 1);
    for (int i = 0;; ++i) {      // the operating system's write would fail on a page an import froze: wait for the import
        {
            std::lock_guard<std::mutex> lk(g_scope_mu);
            if (!freeze_overlaps(lo, hi)) {
                slot_ = os_slot_acquire(p, n);
                break;
            }
        }
        nxrt::platform::sleep_ns(i < 1000 ? 0 : 1000000);
    }
    before_os_write(p, n);
}

ImportFreeze::ImportFreeze(const void* p, uint64_t n) {
    if (!n || !heap_exists()) return;
    const uint64_t lo = reinterpret_cast<uint64_t>(p) & ~(kPage - 1), hi = (reinterpret_cast<uint64_t>(p) + n + kPage - 1) & ~(kPage - 1);
    // g_transitions is initialized in region_create, but ImportFreeze can run before any region exists (e.g., NVN pool import).
    // Ensure it's created here if needed. Also install the page fault handler so that writes to frozen pages are caught.
    // NVN pool imports can occur on non-heap addresses (when pool_view = false), so we don't restrict to heap range.
    {
        std::lock_guard<std::mutex> lk(g_mu);
        if (!g_transitions) g_transitions = nxrt::platform::rwlock_create();
        if (!g_transitions) return;
        // Install the fault handler if not already installed. This must happen before any ImportFreeze
        // makes pages read-only, otherwise writes to those pages will crash.
        nxrt::platform::page_fault_handler_install(on_fault_handler);
    }
    for (int i = 0; slot_ < 0; ++i) {   // after the file reads in progress there; then no new one starts until the end
        {
            std::lock_guard<std::mutex> lk(g_scope_mu);
            if (!os_write_overlaps(lo, hi)) {
                for (int k = 0; k < kFreezeSlots; ++k) {
                    uint64_t expect = 0;
                    if (g_frz[k].hi.load(std::memory_order_relaxed) == 0 && g_frz[k].lo.compare_exchange_strong(expect, lo | 1, std::memory_order_seq_cst)) {
                        g_frz[k].lo.store(lo, std::memory_order_seq_cst);
                        g_frz[k].hi.store(hi, std::memory_order_seq_cst);
                        g_frz_n.fetch_add(1, std::memory_order_seq_cst);
                        slot_ = k;
                        break;
                    }
                }
            }
        }
        if (slot_ < 0) nxrt::platform::sleep_ns(i < 1000 ? 0 : 1000000);
    }
    // [memory] freeze_table (default on): when the heap's page table says every page of the range is committed read-write (the guest's own
    // protection), the read-write runs are the pages no region has armed or guarded (our read-only pages, by their bits), and no page_query
    // is needed. Each page_query on the heap's section view walks the whole run of identical pages after the address.
    const bool use_table = config_get_bool("memory", "freeze_table", true);   // read per freeze (imports are rare): the tests switch it
    const uint64_t epoch = use_table ? heap_epoch() : 0;
    const int table = use_table ? heap_rw_pages(lo, hi - lo) : -1;
    // Exclusive against arm() and region destruction: the read-write runs found here are ours until the destructor gives them back.
    nxrt::platform::rwlock_lock_exclusive(g_transitions);
    bool done = false;
    if (table == 1 && heap_epoch() == epoch) {
        bool other = false;
        for (int k = 0; k < kFreezeSlots && !other; ++k) {
            if (k == slot_) continue;
            const uint64_t h = g_frz[k].hi.load(std::memory_order_seq_cst);
            other = h && g_frz[k].lo.load(std::memory_order_seq_cst) < hi && lo < h;
        }
        if (!other) {
            const uint64_t np = (hi - lo) / kPage;
            std::vector<uint64_t> ours((np + 63) / 64, 0);   // pages read-only (or no-access) because a region armed or guarded them
            g_busy.fetch_add(1, std::memory_order_seq_cst);
            for (RegionCursor cursor; Region* r = cursor.next();) {
                const uint64_t rlo = std::max(lo, r->base), rhi = std::min(hi, r->base + r->npages * kPage);
                for (uint64_t a = rlo; a < rhi; a += kPage)
                    if (test(r->prot.get(), (a - r->base) / kPage)) {
                        const uint64_t q = (a - lo) / kPage;
                        ours[q >> 6] |= uint64_t{1} << (q & 63);
                    }
            }
            g_busy.fetch_sub(1, std::memory_order_seq_cst);
            for (uint64_t q = 0; q < np;) {
                while (q < np && (ours[q >> 6] >> (q & 63) & 1)) ++q;
                const uint64_t s = q;
                while (q < np && !(ours[q >> 6] >> (q & 63) & 1)) ++q;
                if (q > s) {
                    const uint64_t a = lo + s * kPage, e = lo + q * kPage;
                    if (nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(e - a), nxrt::platform::PROT_READ)) runs_.emplace_back(a, e);
                }
            }
            done = true;
            g_freeze_table_hits.fetch_add(1, std::memory_order_relaxed);
        }
    }
    for (uint64_t a = lo; !done && a < hi;) {
        nxrt::platform::PageState state;
        nxrt::platform::PageProtect protect;
        uint64_t region_base, region_size;
        if (!nxrt::platform::page_query(a, &state, &protect, &region_base, &region_size)) break;
        const uint64_t end = std::min<uint64_t>(hi, region_base + region_size);
        const uint32_t pr = static_cast<uint32_t>(protect) & 0xff;
        if (state == nxrt::platform::PageState::Commit && pr == 0x04 && !(pr & 0x100) &&
            nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(end - a), nxrt::platform::PROT_READ))
            runs_.emplace_back(a, end);
        a = end;
    }
    nxrt::platform::rwlock_unlock_exclusive(g_transitions);
}

uint64_t freeze_table_hits() { return g_freeze_table_hits.load(std::memory_order_relaxed); }

bool any_guarded(uint64_t lo, uint64_t hi) {
    bool any = false;
    g_busy.fetch_add(1, std::memory_order_seq_cst);
    for (RegionCursor cursor; Region* r = cursor.next();) {
        if (any) break;
        const uint64_t a = std::max(lo, r->base), e = std::min(hi, r->base + r->npages * kPage);
        if (a < e) any = any_set(r->guard.get(), (a - r->base) / kPage, (e - a + kPage - 1) / kPage);
    }
    g_busy.fetch_sub(1, std::memory_order_seq_cst);
    return any;
}

ImportFreeze::~ImportFreeze() {
    if (slot_ < 0) return;
    nxrt::platform::rwlock_lock_exclusive(g_transitions);
    for (const auto& r : runs_) {
        nxrt::platform::protect(reinterpret_cast<void*>(r.first), static_cast<std::size_t>(r.second - r.first), nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE);
    }
    nxrt::platform::rwlock_unlock_exclusive(g_transitions);
    g_frz[slot_].hi.store(0, std::memory_order_seq_cst);
    g_frz[slot_].lo.store(0, std::memory_order_seq_cst);
    g_frz_n.fetch_sub(1, std::memory_order_seq_cst);
}

OsWriteScope::~OsWriteScope() { os_slot_release(slot_); }

OsReadScope::OsReadScope(const void* p, uint64_t n) {
    if (!n) return;
    slot_ = os_slot_acquire(p, n);   // first: guard() cannot take a page of the range back between the fetch and the OS call
    before_os_read(p, n);
}

OsReadScope::~OsReadScope() { os_slot_release(slot_); }

void before_os_read(const void* p, uint64_t n) {
    // guarded pages only: their owner makes them current and they become read-only (armed) again, which the OS can read. Armed pages
    // stay armed: a read does not change them.
    if (!n || g_live.load(std::memory_order_acquire) == 0) return;
    const uint64_t lo = reinterpret_cast<uint64_t>(p) & ~(kPage - 1), hi = reinterpret_cast<uint64_t>(p) + n;
    for (uint64_t a = lo; a < hi; a += kPage)
        if (guarded_at(a)) {
            if (GuardHandler fn = g_guard_fn.load(std::memory_order_acquire)) fn(a);
            drop_guard(a, kPage);
        }
}

void before_os_write(const void* p, uint64_t n) {
    // guarded pages first: their owner makes them current (the write may cover only part of a page), outside the region locks
    if (n && g_live.load(std::memory_order_acquire) != 0) {
        const uint64_t lo = reinterpret_cast<uint64_t>(p) & ~(kPage - 1), hi = reinterpret_cast<uint64_t>(p) + n;
        for (uint64_t a = lo; a < hi; a += kPage)
            if (guarded_at(a)) {
                if (GuardHandler fn = g_guard_fn.load(std::memory_order_acquire)) fn(a);
                drop_guard(a, kPage);
            }
    }
    for_regions(reinterpret_cast<uint64_t>(p), n, [](Region* r, uint64_t first, uint64_t count) {
        for (uint64_t q = first; q < first + count; ++q)
            if (test(r->prot.get(), q)) release_page(r, q);
    });
}

void forget(uint64_t addr, uint64_t size, bool ban) {
    for_regions(addr, size, [ban](Region* r, uint64_t first, uint64_t count) {
        if (ban) set_bits(r->ban.get(), first, count, true);
        if (!any_set(r->prot.get(), first, count) && !any_set(r->valid.get(), first, count)) return;
        for (uint64_t q = first; q < first + count; ++q)           // guarded pages: read-only first (the owner's newer copy is dropped)
            if (test(r->guard.get(), q)) {              // protection first, then the bit (see unguard_locked)
                nxrt::platform::protect(reinterpret_cast<void*>(r->base + q * kPage), kPage, nxrt::platform::PROT_READ);  // read-only
                r->guard[q >> 6].fetch_and(~bit(q), std::memory_order_acq_rel);
            }
        for (uint64_t q = first; q < first + count; ++q)           // armed pages go back to read-write (the caller may then set its own)
            if (test(r->prot.get(), q)) release_page(r, q);
        set_bits(r->valid.get(), first, count, false);
        set_bits(r->written.get(), first, count, true);
        g_write_seq.fetch_add(1, std::memory_order_seq_cst);
    });
}

void ban_section(uint64_t offset, uint64_t size) { forget(kHeapBase + offset, size, true); }

uint64_t faults() { return g_faults.load(std::memory_order_relaxed); }
uint64_t fault_waits() { return g_waits.load(std::memory_order_relaxed); }

bool is_retired_page(uint64_t addr) {
    if (addr < kHeapBase || addr - kHeapBase >= kHeapMax) return false;
    return test(g_retired, (addr - kHeapBase) / kPage);
}

void clear_retired_page(uint64_t addr) {
    if (addr < kHeapBase || addr - kHeapBase >= kHeapMax) return;
    set_bits(g_retired, (addr - kHeapBase) / kPage, 1, false);
}

}  // namespace nxrt::mem::ww
