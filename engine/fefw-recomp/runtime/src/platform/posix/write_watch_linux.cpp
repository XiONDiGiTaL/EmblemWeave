// SPDX-License-Identifier: GPL-3.0-or-later
// Linux write-watch implementation using userfaultfd + mprotect.
// Mirrors the Windows WriteWatch semantics exactly.
#include "nxrt/write_watch.h"

#include <algorithm>
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fcntl.h>
#include <linux/userfaultfd.h>
#include <map>
#include <memory>
#include <mutex>
#include <poll.h>
#include <pthread.h>
#include <sys/ioctl.h>
#include <sys/mman.h>
#include <sys/syscall.h>
#include <sys/uio.h>
#include <thread>
#include <unistd.h>
#include <unordered_map>
#include <vector>

#if defined(__GNUC__) || defined(__clang__)
static inline unsigned long ctzll_portable(uint64_t v) {
    return v ? static_cast<unsigned long>(__builtin_ctzll(v)) : 64;   // v == 0
}
#elif defined(_MSC_VER)
#include <intrin.h>
#pragma intrinsic(_BitScanForward64)
static inline unsigned long ctzll_portable(uint64_t v) {
    unsigned long index = 0;
    if (!_BitScanForward64(&index, v)) return 64;   // v == 0
    return index;
}
#else
#error "Unsupported compiler: no ctzll implementation"
#endif

#include "nxrt/config.h"
#include "nxrt/gauge.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"
#include "nxrt/platform.h"

namespace nxrt::mem::ww {

constexpr const char* kTag = "mem.ww";
constexpr uint64_t kPage = 4096;
constexpr uint64_t kHeapBase = 0x7100000000ull;  // From memory.h
constexpr uint64_t kHeapMax = 0x100000000ull;   // 4 GiB

struct Region {
    const char* owner = "unspecified";
    uint64_t base = 0;
    uint64_t npages = 0;
    uint64_t nwords = 0;
    std::unique_ptr<std::atomic<uint64_t>[]> prot, valid, written, ban;
    std::unique_ptr<std::atomic<uint64_t>[]> guard;
    std::atomic<Region*>* slot = nullptr;
    nxrt::platform::RWLock* rwlock = nullptr;
    std::unique_ptr<std::atomic<uint16_t>[]> releasing;
};

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

constexpr int kSlotsPerBlock = 256;
struct SlotBlock {
    std::atomic<Region*> slots[kSlotsPerBlock]{};
    std::atomic<int> hi{0};
    std::atomic<SlotBlock*> next{nullptr};
};
SlotBlock g_slots;

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
std::atomic<int> g_busy{0};
std::atomic<int> g_live{0};
std::atomic<uint64_t> g_generation{0};
std::atomic<uint64_t> g_faults{0};
std::atomic<uint64_t> g_waits{0};
std::atomic<uint64_t> g_guard_faults{0};
std::atomic<GuardHandler> g_guard_fn{nullptr};
std::mutex g_mu;

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

constexpr int kFreezeSlots = 16;
OsRange g_frz[kFreezeSlots];
std::atomic<int> g_frz_n{0};
std::mutex g_scope_mu;

std::atomic<uint64_t> g_freeze_table_hits{0};

bool freeze_overlaps(uint64_t lo, uint64_t hi) {
    if (!g_frz_n.load(std::memory_order_seq_cst)) return false;
    for (OsRange& f : g_frz) {
        const uint64_t h = f.hi.load(std::memory_order_seq_cst);
        if (h && f.lo.load(std::memory_order_seq_cst) < hi && lo < h) return true;
    }
    return false;
}

uint64_t bit(uint64_t page) { return uint64_t{1} << (page & 63); }

bool test(const std::atomic<uint64_t>* bm, uint64_t page) {
    return (bm[page >> 6].load(std::memory_order_acquire) & bit(page)) != 0;
}

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

std::atomic<uint64_t> g_retired[kHeapMax / kPage / 64]{};
bool retired_retry(uint64_t addr, int kind) {
    if (addr < kHeapBase || addr - kHeapBase >= kHeapMax || !test(g_retired, (addr - kHeapBase) / kPage)) return false;
    const uint32_t pr = page_protect(addr);
    const uint32_t p = pr & 0xFF;
    if (p == 0x04 || p == 0x40 || p == (0x1u | 0x2u)) return true;
    return kind == 0 && (p == 0x02 || p == 0x20 || p == 0x1u);
}

int final_retry(uint64_t addr, int kind, uint64_t generation) {
    nxrt::platform::rwlock_lock_shared(g_transitions);
    const bool accessible = retired_retry(addr, kind);
    const bool changed = g_generation.load(std::memory_order_seq_cst) != generation;
    nxrt::platform::rwlock_unlock_shared(g_transitions);
    return accessible ? 1 : changed ? -1 : 0;
}

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
    if (p != 0x04 && p != 0x40 && p != (0x1u | 0x2u)) return false;
    if (g_late_retries.fetch_add(1, std::memory_order_relaxed) == 0)
        NXLOG_W(kTag, "write fault at %llx on a page that is read-write again (an import freeze ended meanwhile): the write runs again",
                static_cast<unsigned long long>(addr));
    return true;
}

// ============================================================================
// Userfaultfd infrastructure
// ============================================================================

struct UffdRegion {
    Region* region = nullptr;
    uint64_t base = 0;
    uint64_t npages = 0;
    int uffd = -1;
    pthread_t thread = 0;
    std::atomic<bool> running{false};
};

static std::vector<UffdRegion*> g_uffd_regions;
static std::mutex g_uffd_mu;
static std::unordered_map<Region*, std::unique_ptr<UffdRegion>> g_uffd_map;
static std::mutex g_uffd_map_mu;

static UffdRegion* find_uffd_region(uint64_t addr) {
    std::lock_guard<std::mutex> lk(g_uffd_mu);
    for (UffdRegion* ur : g_uffd_regions) {
        if (addr >= ur->base && addr < ur->base + ur->npages * kPage) {
            return ur;
        }
    }
    return nullptr;
}

static UffdRegion* get_uffd_region(Region* r) {
    std::lock_guard<std::mutex> lk(g_uffd_map_mu);
    auto it = g_uffd_map.find(r);
    return it != g_uffd_map.end() ? it->second.get() : nullptr;
}

static void* uffd_handler_thread(void* arg) {
    UffdRegion* ur = static_cast<UffdRegion*>(arg);
    ur->running.store(true, std::memory_order_release);

    struct pollfd pfd{};
    pfd.fd = ur->uffd;
    pfd.events = POLLIN;

    while (ur->running.load(std::memory_order_acquire)) {
        int ret = poll(&pfd, 1, 100);
        if (ret < 0) {
            if (errno == EINTR) continue;
            NXLOG_E(kTag, "poll failed on uffd: %s", strerror(errno));
            break;
        }
        if (ret == 0) continue;

        if (!(pfd.revents & POLLIN)) continue;

        struct uffd_msg msg;
        ssize_t nread = read(ur->uffd, &msg, sizeof(msg));
        if (nread == 0) break;
        if (nread < 0) {
            if (errno == EAGAIN || errno == EINTR) continue;
            NXLOG_E(kTag, "read uffd_msg failed: %s", strerror(errno));
            break;
        }

        if (msg.event != UFFD_EVENT_PAGEFAULT) {
            NXLOG_W(kTag, "unexpected uffd event: %d", msg.event);
            continue;
        }

        uint64_t fault_addr = msg.arg.pagefault.address;
        uint64_t page_idx = (fault_addr - ur->base) / kPage;

        if (page_idx >= ur->npages) {
            NXLOG_W(kTag, "fault outside region: addr=%llx base=%llx npages=%llu",
                    static_cast<unsigned long long>(fault_addr),
                    static_cast<unsigned long long>(ur->base),
                    static_cast<unsigned long long>(ur->npages));
            struct uffdio_range range = {fault_addr & ~(kPage - 1), kPage};
            ioctl(ur->uffd, UFFDIO_CONTINUE, &range);
            continue;
        }

        const uint64_t w = page_idx >> 6;
        const uint64_t b = uint64_t{1} << (page_idx & 63);

        ur->region->written[w].fetch_or(b, std::memory_order_acq_rel);
        ur->region->valid[w].fetch_and(~b, std::memory_order_acq_rel);

        void* page_ptr = reinterpret_cast<void*>(fault_addr & ~(kPage - 1));
        if (mprotect(page_ptr, kPage, PROT_READ | PROT_WRITE) != 0) {
            NXLOG_E(kTag, "mprotect RW failed for page %llx: %s",
                    static_cast<unsigned long long>(fault_addr), strerror(errno));
        }

        struct uffdio_range range = {fault_addr & ~(kPage - 1), kPage};
        if (ioctl(ur->uffd, UFFDIO_CONTINUE, &range) != 0) {
            NXLOG_E(kTag, "UFFDIO_CONTINUE failed: %s", strerror(errno));
        }
    }
    return nullptr;
}

static bool uffd_region_init(UffdRegion* ur, Region* r, uint64_t first, uint64_t count) {
    ur->region = r;
    ur->base = r->base + first * kPage;
    ur->npages = count;

    ur->uffd = syscall(__NR_userfaultfd, UFFD_USER_MODE_ONLY | O_NONBLOCK);
    if (ur->uffd < 0) {
        NXLOG_E(kTag, "userfaultfd creation failed: %s", strerror(errno));
        return false;
    }

    struct uffdio_api api = {
        .api = UFFD_API,
        .features = UFFD_FEATURE_PAGEFAULT_FLAG_WP | UFFD_FEATURE_EVENT_FORK,
        .ioctls = 0
    };
    if (ioctl(ur->uffd, UFFDIO_API, &api) != 0) {
        NXLOG_E(kTag, "UFFDIO_API failed: %s", strerror(errno));
        close(ur->uffd);
        ur->uffd = -1;
        return false;
    }

    struct uffdio_register reg = {
        .range = {.start = ur->base, .len = count * kPage},
        .mode = UFFDIO_REGISTER_MODE_WP
    };
    if (ioctl(ur->uffd, UFFDIO_REGISTER, &reg) != 0) {
        NXLOG_E(kTag, "UFFDIO_REGISTER failed: %s", strerror(errno));
        close(ur->uffd);
        ur->uffd = -1;
        return false;
    }

    ur->running.store(true, std::memory_order_release);
    if (pthread_create(&ur->thread, nullptr, uffd_handler_thread, ur) != 0) {
        NXLOG_E(kTag, "pthread_create failed: %s", strerror(errno));
        struct uffdio_range range = {ur->base, count * kPage};
        ioctl(ur->uffd, UFFDIO_UNREGISTER, &range);
        close(ur->uffd);
        ur->uffd = -1;
        return false;
    }

    {
        std::lock_guard<std::mutex> lk(g_uffd_mu);
        g_uffd_regions.push_back(ur);
    }

    NXLOG_I(kTag, "uffd region created: base=%llx pages=%llu uffd=%d",
            static_cast<unsigned long long>(ur->base),
            static_cast<unsigned long long>(ur->npages),
            ur->uffd);
    return true;
}

static void uffd_region_destroy(UffdRegion* ur) {
    if (!ur) return;

    {
        std::lock_guard<std::mutex> lk(g_uffd_mu);
        auto it = std::find(g_uffd_regions.begin(), g_uffd_regions.end(), ur);
        if (it != g_uffd_regions.end()) g_uffd_regions.erase(it);
    }

    if (ur->uffd >= 0) {
        ur->running.store(false, std::memory_order_release);
        if (ur->thread) pthread_join(ur->thread, nullptr);

        struct uffdio_range range = {ur->base, ur->npages * kPage};
        ioctl(ur->uffd, UFFDIO_UNREGISTER, &range);
        close(ur->uffd);
        ur->uffd = -1;
    }

    NXLOG_I(kTag, "uffd region destroyed: base=%llx",
            static_cast<unsigned long long>(ur->base));
}

static UffdRegion* create_uffd_region(Region* r, uint64_t first, uint64_t count) {
    std::lock_guard<std::mutex> lk(g_uffd_map_mu);
    auto u = std::make_unique<UffdRegion>();
    if (uffd_region_init(u.get(), r, first, count)) {
        UffdRegion* ptr = u.get();
        g_uffd_map[r] = std::move(u);
        return ptr;
    }
    return nullptr;
}

static void destroy_uffd_region(Region* r) {
    std::lock_guard<std::mutex> lk(g_uffd_map_mu);
    auto it = g_uffd_map.find(r);
    if (it != g_uffd_map.end()) {
        uffd_region_destroy(it->second.get());
        g_uffd_map.erase(it);
    }
}

static void uffd_unregister_range(UffdRegion* ur, uint64_t addr, uint64_t size) {
    if (!ur || ur->uffd < 0) return;
    struct uffdio_range range = {addr & ~(kPage - 1), (size + kPage - 1) & ~(kPage - 1)};
    ioctl(ur->uffd, UFFDIO_UNREGISTER, &range);
}

static void uffd_register_range(UffdRegion* ur, uint64_t addr, uint64_t size) {
    if (!ur || ur->uffd < 0) return;
    struct uffdio_register reg = {
        .range = {.start = addr & ~(kPage - 1), .len = (size + kPage - 1) & ~(kPage - 1)},
        .mode = UFFDIO_REGISTER_MODE_WP
    };
    ioctl(ur->uffd, UFFDIO_REGISTER, &reg);
}

// ============================================================================
// Fault statistics
// ============================================================================

std::mutex g_fs_lock;
std::map<std::pair<uint64_t, uint64_t>, uint64_t> g_fs_blocks;
std::unordered_map<uint64_t, uint64_t> g_fs_regions;
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

void drop_guard(uint64_t addr, uint64_t size);

void unguard_locked(Region* r, uint64_t first, uint64_t count) {
    for (uint64_t p = first; p < first + count;) {
        if (!test(r->guard.get(), p)) { ++p; continue; }
        uint64_t q = p;
        while (q < first + count && test(r->guard.get(), q)) ++q;
        nxrt::platform::protect(reinterpret_cast<void*>(r->base + p * kPage), static_cast<std::size_t>((q - p) * kPage), 0x1u);
        set_bits(r->guard.get(), p, q - p, false);
        p = q;
    }
}

// Platform-independent fault handler - on Linux this is handled by userfaultfd threads
// This function is kept for compatibility but userfaultfd handles faults asynchronously
bool on_fault_handler(uint64_t addr, int kind) {
    // On Linux with userfaultfd, write faults are handled by the uffd threads.
    // This handler is only called for read faults on guarded pages or other edge cases.
    // The SIGSEGV handler in platform_posix.cpp will call this for any faults not handled by uffd.

    // A write into a range an import has frozen
    if (kind == 1 && freeze_overlaps(addr, addr + 1)) {
        for (int i = 0; freeze_overlaps(addr, addr + 1); ++i) {
            if (i < 64) nxrt::platform::yield_cpu();
            else nxrt::platform::sleep_ns(i < 1000 ? 0 : 1000000);
        }
        return true;
    }

    const uint64_t generation = g_generation.load(std::memory_order_seq_cst);
    nxrt::HitchTimer ht(nxrt::kHpFaults);

    if (guarded_at(addr)) {
        g_guard_faults.fetch_add(1, std::memory_order_relaxed);
        if (GuardHandler fn = g_guard_fn.load(std::memory_order_acquire)) fn(addr & ~(kPage - 1));
        drop_guard(addr & ~(kPage - 1), kPage);
        return true;
    }

    if (kind != 1) {
        const uint32_t pr = page_protect(addr) & 0xff;
        if (pr == 0x02 || pr == 0x04 || pr == 0x20 || pr == 0x40 || pr == 0x1u || pr == (0x1u | 0x2u)) {
            bool in_region = false;
            g_busy.fetch_add(1, std::memory_order_seq_cst);
            for (RegionCursor cursor; Region* r = cursor.next();) {
                if (addr >= r->base && addr < r->base + r->npages * kPage) { in_region = true; break; }
            }
            g_busy.fetch_sub(1, std::memory_order_seq_cst);
            if (in_region) return true;
        }
        if ((pr == 0x01 || pr == 0) && guarded_at(addr)) {
            g_guard_faults.fetch_add(1, std::memory_order_relaxed);
            if (GuardHandler fn = g_guard_fn.load(std::memory_order_acquire)) fn(addr & ~(kPage - 1));
            drop_guard(addr & ~(kPage - 1), kPage);
            return true;
        }
        const int decision = final_retry(addr, kind, generation);
        if (decision == 1) return true;
        if (decision == -1) {
            // Retry with new generation
            return on_fault_handler(addr, kind);
        }
        return false;
    }

    bool ours = false, guarded_now = false;
    g_busy.fetch_add(1, std::memory_order_seq_cst);
    for (int pass = 0; pass < 2 && !ours && !guarded_now; ++pass) {
        for (RegionCursor cursor; Region* r = cursor.next();) {
            if (addr < r->base || addr >= r->base + r->npages * kPage) continue;
            SharedLock lk(r);
            const uint64_t pg = (addr - r->base) / kPage;
            if (test(r->guard.get(), pg)) { guarded_now = true; continue; }
            if (pass == 0 && !test(r->prot.get(), pg)) continue;
            // On Linux, write faults on armed pages are handled by userfaultfd.
            // If we reach here, it means the page was armed but not registered with uffd,
            // or uffd missed it. Handle it manually.
            const uint64_t w = pg >> 6;
            const uint64_t b = uint64_t{1} << (pg & 63);
            const uint64_t old = r->prot[w].fetch_and(~b, std::memory_order_acq_rel);
            r->valid[w].fetch_and(~b, std::memory_order_acq_rel);
            r->written[w].fetch_or(b, std::memory_order_acq_rel);
            const uint64_t page_addr = r->base + pg * kPage;
            if (old & b) {
                nxrt::platform::protect(reinterpret_cast<void*>(page_addr), kPage, 0x1u | 0x2u);
                fault_stat(r, addr);
                ours = true;
            } else {
                // Lost race, wait for other releaser
                std::atomic<uint16_t>& rel = r->releasing[pg];
                for (int i = 0;; ++i) {
                    const uint16_t busy = rel.load(std::memory_order_seq_cst);
                    const uint32_t pr = page_protect(page_addr) & 0xFF;
                    if (pr == 0x04 || pr == 0x40 || pr == (0x1u | 0x2u)) { ours = true; break; }
                    if (pr != 0x02 && pr != 0x20 && pr != 0x1u) break;
                    if (busy == 0) break;
                    g_waits.fetch_add(1, std::memory_order_relaxed);
                    if (i < 50) nxrt::platform::yield_cpu();
                    else if (i < 1000) nxrt::platform::sleep_ns(0);
                    else nxrt::platform::sleep_ns(1000000);
                }
            }
        }
    }
    if (guarded_now) {
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
            return on_fault_handler(addr, kind);
        }
    }
    if (!ours && heap_write_landed_late(addr)) {
        g_busy.fetch_sub(1, std::memory_order_seq_cst);
        return true;
    }
    g_busy.fetch_sub(1, std::memory_order_seq_cst);
    if (!ours) return false;
    g_faults.fetch_add(1, std::memory_order_relaxed);
    return true;
}

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

// ============================================================================
// Public API implementations
// ============================================================================

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
    {
        ExclusiveLock lk(r);
        const auto held = [r](uint64_t i) { return test(r->prot.get(), i) || test(r->guard.get(), i); };
        for (uint64_t p = 0; p < r->npages;) {
            if (!held(p)) { ++p; continue; }
            uint64_t q = p;
            while (q < r->npages && held(q)) ++q;
            set_bits(g_retired, (r->base - kHeapBase) / kPage + p, q - p, true);
            set_bits(r->prot.get(), p, q - p, false);
            set_bits(r->guard.get(), p, q - p, false);
            nxrt::platform::protect(reinterpret_cast<void*>(r->base + p * kPage), static_cast<std::size_t>((q - p) * kPage), 0x1u | 0x2u);
            p = q;
        }
    }
    {
        std::lock_guard<std::mutex> lk(g_mu);
        r->slot->store(nullptr, std::memory_order_seq_cst);
        g_live.fetch_sub(1, std::memory_order_seq_cst);
        g_generation.fetch_add(1, std::memory_order_seq_cst);
    }
    while (g_busy.load(std::memory_order_seq_cst) != 0) std::this_thread::yield();
    destroy_uffd_region(r);
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
    const uint64_t epoch = heap_epoch();
    const int table = heap_rw_pages(a, n);
    ExclusiveLock lk(r, true);
    if (any_set(r->ban.get(), first, count)) return false;
    if (os_write_overlaps(a, a + n) || freeze_overlaps(a, a + n)) return false;
    if (table == 1 && heap_epoch() == epoch) {
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
    set_bits(r->prot.get(), first, count, true);
    set_bits(r->valid.get(), first, count, true);
    if (!nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(n), 0x1u)) {
        set_bits(r->valid.get(), first, count, false);
        set_bits(r->prot.get(), first, count, false);
        return false;
    }
    // Register with userfaultfd
    UffdRegion* ur = get_uffd_region(r);
    if (!ur) {
        ur = create_uffd_region(r, first, count);
        if (!ur) {
            nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(n), 0x1u | 0x2u);
            set_bits(r->valid.get(), first, count, false);
            set_bits(r->prot.get(), first, count, false);
            return false;
        }
    } else {
        uffd_register_range(ur, a, n);
    }
    g_generation.fetch_add(1, std::memory_order_seq_cst);
    return true;
}

bool all_valid(const Region* r, uint64_t first, uint64_t count) {
    if (!r || !count || first >= r->npages || count > r->npages - first) return false;
    return all_set(r->valid.get(), first, count);
}

bool page_valid(const Region* r, uint64_t page) { return r && page < r->npages && test(r->valid.get(), page); }

bool any_valid(const Region* r, uint64_t first, uint64_t count) {
    if (!r || !count || first >= r->npages) return false;
    const uint64_t end = first + std::min(count, r->npages - first);
    for (uint64_t p = first; p < end;) {
        const uint64_t w = p >> 6;
        const uint64_t lo = p & 63;
        const uint64_t hi = std::min<uint64_t>(64, lo + (end - p));
        const uint64_t mask = (hi - lo == 64) ? ~uint64_t{0} : (((uint64_t{1} << (hi - lo)) - 1) << lo);
        if (r->valid[w].load(std::memory_order_acquire) & mask) return true;
        p += hi - lo;
    }
    return false;
}

uint64_t count_valid(const Region* r) {
    if (!r) return 0;
    uint64_t n = 0;
    for (uint64_t w = 0; w < r->nwords; ++w) n += static_cast<uint64_t>(__builtin_popcountll(r->valid[w].load(std::memory_order_relaxed)));
    return n;
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

    // Unregister from userfaultfd
    UffdRegion* ur = get_uffd_region(r);
    if (ur) uffd_unregister_range(ur, a, n);

    set_bits(r->guard.get(), first, count, true);
    if (!nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(n), 0x0u)) {
        nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(n), 0x1u);
        set_bits(r->guard.get(), first, count, false);
        if (ur) uffd_register_range(ur, a, n);
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
    // Re-register with userfaultfd for write-protect
    UffdRegion* ur = get_uffd_region(r);
    if (ur) {
        uint64_t a = r->base + first * kPage;
        uint64_t n = count * kPage;
        uffd_register_range(ur, a, n);
    }
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
    for (int i = 0;; ++i) {
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
    {
        std::lock_guard<std::mutex> lk(g_mu);
        if (!g_transitions) g_transitions = nxrt::platform::rwlock_create();
        if (!g_transitions) return;
        nxrt::platform::page_fault_handler_install(on_fault_handler);
    }
    for (int i = 0; slot_ < 0; ++i) {
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
    const bool use_table = config_get_bool("memory", "freeze_table", true);
    const uint64_t epoch = use_table ? heap_epoch() : 0;
    const int table = use_table ? heap_rw_pages(lo, hi - lo) : -1;
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
            std::vector<uint64_t> ours((np + 63) / 64, 0);
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
                    if (nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(e - a), 0x1u)) runs_.emplace_back(a, e);
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
            nxrt::platform::protect(reinterpret_cast<void*>(a), static_cast<std::size_t>(end - a), 0x1u))
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
        nxrt::platform::protect(reinterpret_cast<void*>(r.first), static_cast<std::size_t>(r.second - r.first), 0x1u | 0x2u);
    }
    nxrt::platform::rwlock_unlock_exclusive(g_transitions);
    g_frz[slot_].hi.store(0, std::memory_order_seq_cst);
    g_frz[slot_].lo.store(0, std::memory_order_seq_cst);
    g_frz_n.fetch_sub(1, std::memory_order_seq_cst);
}

OsWriteScope::~OsWriteScope() { os_slot_release(slot_); }

OsReadScope::OsReadScope(const void* p, uint64_t n) {
    if (!n) return;
    slot_ = os_slot_acquire(p, n);
    before_os_read(p, n);
}

OsReadScope::~OsReadScope() { os_slot_release(slot_); }

void before_os_read(const void* p, uint64_t n) {
    if (!n || g_live.load(std::memory_order_acquire) == 0) return;
    const uint64_t lo = reinterpret_cast<uint64_t>(p) & ~(kPage - 1), hi = reinterpret_cast<uint64_t>(p) + n;
    for (uint64_t a = lo; a < hi; a += kPage)
        if (guarded_at(a)) {
            if (GuardHandler fn = g_guard_fn.load(std::memory_order_acquire)) fn(a);
            drop_guard(a, kPage);
        }
}

void before_os_write(const void* p, uint64_t n) {
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
            if (test(r->prot.get(), q)) {
                const uint64_t w = q >> 6;
                const uint64_t b = uint64_t{1} << (q & 63);
                const uint64_t old = r->prot[w].fetch_and(~b, std::memory_order_acq_rel);
                r->valid[w].fetch_and(~b, std::memory_order_acq_rel);
                r->written[w].fetch_or(b, std::memory_order_acq_rel);
                if (old & b) {
                    const uint64_t addr = r->base + q * kPage;
                    nxrt::platform::protect(reinterpret_cast<void*>(addr), kPage, 0x1u | 0x2u);
                    // Unregister from uffd
                    UffdRegion* ur = get_uffd_region(r);
                    if (ur) uffd_unregister_range(ur, addr, kPage);
                }
            }
    });
}

void forget(uint64_t addr, uint64_t size, bool ban) {
    for_regions(addr, size, [ban](Region* r, uint64_t first, uint64_t count) {
        if (ban) set_bits(r->ban.get(), first, count, true);
        if (!any_set(r->prot.get(), first, count) && !any_set(r->valid.get(), first, count)) return;
        for (uint64_t q = first; q < first + count; ++q)
            if (test(r->guard.get(), q)) {
                nxrt::platform::protect(reinterpret_cast<void*>(r->base + q * kPage), kPage, 0x1u);
                r->guard[q >> 6].fetch_and(~bit(q), std::memory_order_acq_rel);
            }
        for (uint64_t q = first; q < first + count; ++q)
            if (test(r->prot.get(), q)) {
                const uint64_t w = q >> 6;
                const uint64_t b = uint64_t{1} << (q & 63);
                const uint64_t old = r->prot[w].fetch_and(~b, std::memory_order_acq_rel);
                r->valid[w].fetch_and(~b, std::memory_order_acq_rel);
                r->written[w].fetch_or(b, std::memory_order_acq_rel);
                if (old & b) {
                    const uint64_t page_addr = r->base + q * kPage;
                    nxrt::platform::protect(reinterpret_cast<void*>(page_addr), kPage, 0x1u | 0x2u);
                    // Unregister from uffd
                    UffdRegion* ur = get_uffd_region(r);
                    if (ur) uffd_unregister_range(ur, page_addr, kPage);
                }
            }
        set_bits(r->valid.get(), first, count, false);
        set_bits(r->written.get(), first, count, true);
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