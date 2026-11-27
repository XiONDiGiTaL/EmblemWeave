// SPDX-License-Identifier: GPL-3.0-or-later
// Guest address space on the host (layout and contract: include/nxrt/memory.h).
//  - fixed reservations at exact addresses (images, stacks, the kernel region), committed on demand;
//  - the heap: one shared memory section with a primary view at kHeapBase and alias views of the same
//    section pages, so one physical page can be seen at two guest addresses;
//  - a Horizon-style vm map kept beside the real mappings for svcQueryMemory.
#include "nxrt/game.h"
#include "nxrt/memory.h"

#include "nxrt/config.h"
#include "nxrt/diag.h"
#include "nxrt/gauge.h"
#include "nxrt/log.h"
#include "nxrt/write_watch.h"
#include "nxrt/platform.h"

#include <algorithm>
#include <atomic>
#include <cctype>
#include <functional>
#include <cstdio>
#include <map>
#include <mutex>
#include <string>
#include <tuple>
#include <utility>
#include <vector>
#if defined(_WIN32)
#include <windows.h>
#endif

namespace nxrt::mem {
namespace {

constexpr const char* kTag = "mem";
constexpr uint64_t kAddrLimit = 0x800000000000ull;  // user-mode address limit of x64 (47 bits)
constexpr uint64_t kVmEnd = 0x8000000000ull;         // end of the address space reported by vm_query
constexpr int kAliasTries = 16;

uint64_t align_up(uint64_t v, uint64_t a) { return (v + a - 1) & ~(a - 1); }

// Whole pages covering [addr, addr+size). False when the range leaves the user address space.
bool page_span(uint64_t addr, uint64_t size, uint64_t* start, uint64_t* end) {
    if (size > kAddrLimit || addr > kAddrLimit - size) return false;
    *start = addr & ~(kPage - 1);
    *end = align_up(addr + size, kPage);
    return true;
}

// Platform-independent protection flags
int protect_of(Prot p) {
    switch (p) {
        case Prot::R: return 1;      // read-only
        case Prot::RW: return 3;     // read-write
        case Prot::RX: return 5;     // read-execute
        case Prot::RWX: return 7;    // read-write-execute
        case Prot::None: break;
    }
    return 0;  // no access
}

// ---- fixed reservations (guarded by g_fixed_mu) ----
struct Fixed {
    uint64_t size;
    std::string tag;
};
std::mutex g_fixed_mu;
std::map<uint64_t, Fixed> g_fixed;

// ---- module state ----
std::mutex g_state_mu;                    // init / shutdown
std::atomic<bool> g_inited{false};
std::atomic<uint64_t> g_default_heap{0};  // bytes, from Config or [memory] heap_gb

std::mutex g_kernel_mu;
uint64_t g_kernel_next = kKernelBase;     // guarded by g_kernel_mu

std::mutex g_vm_mu;
std::map<uint64_t, VmInfo> g_vm;          // start -> block, guarded by g_vm_mu

// ---- heap (guarded by g_heap_mu) ----
std::mutex g_heap_mu;
nxrt::platform::Section* g_section = nullptr;
uint64_t g_heap_size = 0;                 // 0 = no heap; otherwise the section size = size of the primary view
std::vector<uint64_t> g_commit_bits;      // one bit per 4 KiB page of the section, set by heap_commit
uint64_t g_committed = 0;                 // bytes of pages whose bit is set
// Per 4 KiB page of the section, what the host reports for it: 0 never committed, 1 committed but no-access (a decommitted page or one
// the guest made inaccessible), 2 committed and accessible but not writable, 3 committed read-write.
std::vector<uint8_t> g_heap_cls;
std::atomic<uint64_t> g_heap_epoch{0};    // bumped by every change of the table, inside the change's OsWriteScope
struct Alias {
    uint64_t size;
    uint64_t offset;                      // section offset of the first byte of the view
};
std::map<uint64_t, Alias> g_aliases;      // view address -> record
// Alias region in use: start -> size for every live view and for every candidate address the kernel refused.
std::map<uint64_t, uint64_t> g_alias_used;

// ---- gauges (include/nxrt/gauge.h); each callback takes the lock of the state it reads ----
// Removes its gauge when this file's statics are torn down (declared after them, so it goes first).
struct GaugeHook {
    const char* name;
    GaugeHook(const char* n, std::function<int64_t()> f) : name(n) { nxrt::gauge_register(n, std::move(f)); }
    ~GaugeHook() { nxrt::gauge_unregister(name); }
};
GaugeHook gauge_heap_reserved("mem.heap_reserved", [] { return static_cast<int64_t>(heap_reserved()); });   // section size
GaugeHook gauge_heap_committed("mem.heap_committed", [] { return static_cast<int64_t>(heap_committed()); });   // committed through heap_commit
GaugeHook gauge_aliases("mem.aliases", [] {   // heap alias views live (g_heap_mu)
    std::lock_guard<std::mutex> lk(g_heap_mu);
    return static_cast<int64_t>(g_aliases.size());
});
GaugeHook gauge_alias_used("mem.alias_used", [] {   // alias-region ranges in use, plus addresses the kernel refused (never freed: only grows)
    std::lock_guard<std::mutex> lk(g_heap_mu);
    return static_cast<int64_t>(g_alias_used.size());
});
GaugeHook gauge_fixed("mem.fixed_regions", [] {   // fixed reservations of this module (images, stacks, kernel region)
    std::lock_guard<std::mutex> lk(g_fixed_mu);
    return static_cast<int64_t>(g_fixed.size());
});
GaugeHook gauge_fixed_bytes("mem.fixed_bytes", [] {
    std::lock_guard<std::mutex> lk(g_fixed_mu);
    uint64_t n = 0;
    for (const auto& kv : g_fixed) n += kv.second.size;
    return static_cast<int64_t>(n);
});
GaugeHook gauge_vm_blocks("mem.vm_blocks", [] {   // blocks of the Horizon-style vm map kept for QueryMemory
    std::lock_guard<std::mutex> lk(g_vm_mu);
    return static_cast<int64_t>(g_vm.size());
});

// [memory] fast_query = false: ask the host (VirtualQuery) instead of the page table above; for before/after measurements.
bool fast_query() {
    static const bool v = config_get_bool("memory", "fast_query", true);
    return v;
}

// True when [start, end) lies inside the heap primary view (g_heap_mu held).
bool in_heap_view(uint64_t start, uint64_t end) {
    return fast_query() && g_heap_size && !g_heap_cls.empty() && start >= kHeapBase && end <= kHeapBase + g_heap_size && start < end;
}

bool bit_test(uint64_t page) { return (g_commit_bits[page / 64] >> (page % 64)) & 1; }
void bit_set(uint64_t page) { g_commit_bits[page / 64] |= uint64_t{1} << (page % 64); }
void bit_clear(uint64_t page) { g_commit_bits[page / 64] &= ~(uint64_t{1} << (page % 64)); }

// First address >= from at which `size` bytes are free in the alias region; 0 when there is none.
uint64_t find_free_alias(uint64_t from, uint64_t size) {
    const uint64_t limit = kAliasBase + kAliasSize;
    uint64_t cand = std::max(from, kAliasBase);
    for (;;) {
        if (cand >= limit || size > limit - cand) return 0;
        bool moved = false;
        auto it = g_alias_used.upper_bound(cand);
        if (it != g_alias_used.begin()) {
            auto p = std::prev(it);
            if (p->first + p->second > cand) {
                cand = align_up(p->first + p->second, kAllocGranularity);
                moved = true;
            }
        }
        if (!moved && it != g_alias_used.end() && it->first < cand + size) {
            cand = align_up(it->first + it->second, kAllocGranularity);
            moved = true;
        }
        if (!moved) return cand;
    }
}

}  // namespace

// ---------------------------------------------------------------------------------------------------------------------
// init / shutdown

bool init(const Config& cfg) {
    std::lock_guard lk(g_state_mu);
    if (g_inited.load()) return true;

    uint64_t heap = cfg.heap_reserve;
    if (heap == 0) {
        int64_t gb = nxrt::config_get_int("memory", "heap_gb", nxrt::game::info().platform == nxrt::game::Platform::Switch2 ? 12 : 6);
        if (gb < 1 || gb > 1024) {
            NXLOG_W(kTag, "[memory] heap_gb=%lld is out of range: using 6", static_cast<long long>(gb));
            gb = 6;
        }
        heap = static_cast<uint64_t>(gb) << 30;
    }
    g_default_heap.store(align_up(heap, kAllocGranularity));

    if (cfg.reserve_regions) {
        if (!reserve_fixed(kStackBase, kStackSize, "stack")) {
            NXLOG_E(kTag, "init failed: the stack region could not be reserved");
            return false;
        }
        if (!reserve_fixed(kKernelBase, kKernelSize, "kernel")) {
            NXLOG_E(kTag, "init failed: the kernel region could not be reserved");
            release_fixed(kStackBase);
            return false;
        }
    }
    {
        std::lock_guard kl(g_kernel_mu);
        g_kernel_next = kKernelBase;
    }
    g_inited.store(true);
    NXLOG_I(kTag, "init: stack %llx (+%llx), kernel %llx (+%llx), default heap %llu MiB, regions %s",
            static_cast<unsigned long long>(kStackBase), static_cast<unsigned long long>(kStackSize),
            static_cast<unsigned long long>(kKernelBase), static_cast<unsigned long long>(kKernelSize),
            static_cast<unsigned long long>(g_default_heap.load() >> 20), cfg.reserve_regions ? "reserved" : "skipped");
    return true;
}

void shutdown() {
    std::lock_guard lk(g_state_mu);
    {
        std::lock_guard hl(g_heap_mu);
        for (auto& [view, rec] : g_aliases) {
            (void)rec;
            if (!nxrt::platform::section_unmap_view(view, rec.size)) {
                NXLOG_E(kTag, "shutdown: section_unmap_view(alias %llx) failed", static_cast<unsigned long long>(view));
            }
        }
        g_aliases.clear();
        g_alias_used.clear();
        if (g_heap_size && !g_section) {
            nxrt::platform::release(reinterpret_cast<void*>(kHeapBase), 0);
        } else if (g_heap_size) {
            if (!nxrt::platform::section_unmap_view(kHeapBase, g_heap_size)) {
                NXLOG_E(kTag, "shutdown: section_unmap_view(heap) failed");
            }
        }
        nxrt::platform::section_destroy(g_section);
        g_section = nullptr;
        g_heap_size = 0;
        g_commit_bits.clear();
        g_heap_cls.clear();
        g_committed = 0;
    }
    {
        std::lock_guard fl(g_fixed_mu);
        for (auto& [addr, f] : g_fixed) {
            nxrt::platform::release(reinterpret_cast<void*>(addr), f.size);
        }
        g_fixed.clear();
    }
    {
        std::lock_guard vl(g_vm_mu);
        g_vm.clear();
    }
    {
        std::lock_guard kl(g_kernel_mu);
        g_kernel_next = kKernelBase;
    }
    g_default_heap.store(0);
    g_inited.store(false);
    NXLOG_I(kTag, "shutdown: all mappings of this module released");
}

bool initialised() { return g_inited.load(); }

// ---------------------------------------------------------------------------------------------------------------------
// fixed mappings

bool reserve_fixed(uint64_t addr, uint64_t size, const char* tag) {
    if (size == 0 || addr % kAllocGranularity != 0 || addr >= kAddrLimit || size > kAddrLimit - addr) {
        NXLOG_E(kTag, "reserve %llx (+%llx) %s: address must be 64 KiB aligned and inside the user address space",
                static_cast<unsigned long long>(addr), static_cast<unsigned long long>(size), tag ? tag : "");
        return false;
    }
    size = align_up(size, kPage);
    std::lock_guard lk(g_fixed_mu);
    void* p = nxrt::platform::reserve_fixed(reinterpret_cast<void*>(addr), static_cast<std::size_t>(size));
    if (!p) {
        NXLOG_E(kTag, "reserve %llx (+%llx) %s: platform::reserve_fixed failed",
                static_cast<unsigned long long>(addr), static_cast<unsigned long long>(size), tag ? tag : "");
        return false;
    }
    if (reinterpret_cast<uint64_t>(p) != addr) {
        nxrt::platform::release_fixed(p, size);
        NXLOG_E(kTag, "reserve %llx %s: got %p instead", static_cast<unsigned long long>(addr), tag ? tag : "", p);
        return false;
    }
    g_fixed[addr] = Fixed{size, tag ? tag : ""};
    NXLOG_D(kTag, "reserved %llx (+%llx) %s", static_cast<unsigned long long>(addr),
            static_cast<unsigned long long>(size), tag ? tag : "");
    return true;
}

bool release_fixed(uint64_t addr) {
    std::lock_guard lk(g_fixed_mu);
    auto it = g_fixed.find(addr);
    if (it == g_fixed.end()) {
        NXLOG_W(kTag, "release %llx: no reservation of this module starts there", static_cast<unsigned long long>(addr));
        return false;
    }
    if (!nxrt::platform::release_fixed(reinterpret_cast<void*>(addr), it->second.size)) {
        NXLOG_E(kTag, "release %llx: platform::release_fixed failed", static_cast<unsigned long long>(addr));
        return false;
    }
    g_fixed.erase(it);
    return true;
}

bool commit(uint64_t addr, uint64_t size, Prot p) {
    uint64_t start = 0, end = 0;
    if (!page_span(addr, size, &start, &end)) return false;
    if (start == end) return true;
    const ww::OsWriteScope wws(reinterpret_cast<void*>(start), end - start);
    if (!nxrt::platform::commit(reinterpret_cast<void*>(start), static_cast<std::size_t>(end - start))) {
        NXLOG_E(kTag, "commit %llx (+%llx): platform::commit failed", static_cast<unsigned long long>(start),
                static_cast<unsigned long long>(end - start));
        return false;
    }
    uint32_t prot = 0;
    switch (p) {
        case Prot::None:  prot = 0; break;
        case Prot::R:     prot = nxrt::platform::PROT_READ; break;
        case Prot::RW:    prot = nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE; break;
        case Prot::RX:    prot = nxrt::platform::PROT_READ | nxrt::platform::PROT_EXEC; break;
        case Prot::RWX:   prot = nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE | nxrt::platform::PROT_EXEC; break;
    }
    if (!nxrt::platform::protect(reinterpret_cast<void*>(start), static_cast<std::size_t>(end - start), prot)) {
        NXLOG_E(kTag, "commit %llx (+%llx): platform::protect failed", static_cast<unsigned long long>(start),
                static_cast<unsigned long long>(end - start));
        return false;
    }
    return true;
}

bool decommit(uint64_t addr, uint64_t size) {
    uint64_t start = 0, end = 0;
    if (!page_span(addr, size, &start, &end)) return false;
    if (start == end) return true;
    const ww::OsWriteScope wws(reinterpret_cast<void*>(start), end - start);
    if (!nxrt::platform::decommit(reinterpret_cast<void*>(start), static_cast<std::size_t>(end - start))) {
        NXLOG_E(kTag, "decommit %llx (+%llx): platform::decommit failed", static_cast<unsigned long long>(start),
                static_cast<unsigned long long>(end - start));
        return false;
    }
    return true;
}

bool protect(uint64_t addr, uint64_t size, Prot p) {
    uint64_t start = 0, end = 0;
    if (!page_span(addr, size, &start, &end)) return false;
    if (start == end) return true;
    uint32_t prot = 0;
    switch (p) {
        case Prot::None:  prot = 0; break;
        case Prot::R:     prot = nxrt::platform::PROT_READ; break;
        case Prot::RW:    prot = nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE; break;
        case Prot::RX:    prot = nxrt::platform::PROT_READ | nxrt::platform::PROT_EXEC; break;
        case Prot::RWX:   prot = nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE | nxrt::platform::PROT_EXEC; break;
    }
    const ww::OsWriteScope wws(reinterpret_cast<void*>(start), end - start);
    {   // heap: the page table says whether every page is committed, so one protect does it
        std::lock_guard hk(g_heap_mu);
        if (in_heap_view(start, end)) {
            const uint64_t p0 = (start - kHeapBase) / kPage, p1 = (end - kHeapBase) / kPage;
            bool all = true;
            for (uint64_t q = p0; q < p1 && all; ++q) all = g_heap_cls[q] != 0;
            if (all && nxrt::platform::protect(reinterpret_cast<void*>(start), static_cast<std::size_t>(end - start), prot)) {
                uint8_t cls = 0;
                if (p == Prot::None) cls = 1;  // committed but no access
                else if (p == Prot::R || p == Prot::RX) cls = 2;  // readable
                else cls = 3;  // writable
                std::fill(g_heap_cls.begin() + static_cast<std::ptrdiff_t>(p0), g_heap_cls.begin() + static_cast<std::ptrdiff_t>(p1), cls);
                g_heap_epoch.fetch_add(1, std::memory_order_seq_cst);
                return true;
            }
        }
    }
    // For non-heap memory, use platform::protect
    if (!nxrt::platform::protect(reinterpret_cast<void*>(start), static_cast<std::size_t>(end - start), prot)) {
        NXLOG_E(kTag, "protect %llx (+%llx): platform::protect failed", static_cast<unsigned long long>(start),
                static_cast<unsigned long long>(end - start));
        return false;
    }
    return true;
}

uint64_t alloc_kernel(uint64_t size, uint64_t align) {
    if (!initialised() && !init()) return 0;
    if (align == 0) align = kPage;
    if (align & (align - 1)) {
        NXLOG_E(kTag, "alloc_kernel: alignment %llx is not a power of two", static_cast<unsigned long long>(align));
        return 0;
    }
    align = std::max(align, kPage);
    if (size == 0) size = 1;
    if (size > kKernelSize) return 0;
    size = align_up(size, kPage);

    std::lock_guard lk(g_kernel_mu);
    const uint64_t a = align_up(g_kernel_next, align);
    if (a < g_kernel_next || a > kKernelBase + kKernelSize || size > kKernelBase + kKernelSize - a) {
        NXLOG_E(kTag, "alloc_kernel: kernel region exhausted (%llx used of %llx)",
                static_cast<unsigned long long>(g_kernel_next - kKernelBase),
                static_cast<unsigned long long>(kKernelSize));
        return 0;
    }
    if (!commit(a, size, Prot::RW)) return 0;
    g_kernel_next = a + size;
    return a;
}

bool is_mapped(uint64_t addr, uint64_t size) {
    uint64_t start = 0, end = 0;
    if (!page_span(addr, size, &start, &end)) return false;
    {
        std::lock_guard hk(g_heap_mu);
        if (in_heap_view(start, end)) {
            const uint64_t p0 = (start - kHeapBase) / kPage, p1 = (end - kHeapBase) / kPage;
            for (uint64_t q = p0; q < p1; ++q)
                if (g_heap_cls[q] < 2) return false;
            return true;
        }
    }
    // For non-heap memory, use platform page_query to check commit state
    for (uint64_t cur = start; cur < end; ) {
        nxrt::platform::PageState state;
        nxrt::platform::PageProtect protect;
        uint64_t region_base = 0, region_size = 0;
        if (!nxrt::platform::page_query(cur, &state, &protect, &region_base, &region_size))
            return false;
        if (state != nxrt::platform::PageState::Commit) return false;
        // Check if page has any access (not NOACCESS)
        if (protect == nxrt::platform::PageProtect::NoAccess) return false;
        cur = std::min(end, region_base + region_size);
    }
    return true;
}

// ---------------------------------------------------------------------------------------------------------------------
// heap section

int heap_section_parse(const std::string& text, bool* recognised) {
    std::string v = text;
    for (char& ch : v) ch = static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
    if (recognised) *recognised = true;
    if (v.empty() || v == "auto") return -1;
    if (v == "1" || v == "true" || v == "on" || v == "yes") return 1;
    if (v == "0" || v == "false" || v == "off" || v == "no") return 0;
    if (recognised) *recognised = false;
    return -1;
}

bool heap_create(uint64_t reserve_bytes) {
    if (!initialised() && !init()) return false;
    std::lock_guard lk(g_heap_mu);
    if (g_heap_size) return true;  // a heap already exists: nothing to do

    const uint64_t want = reserve_bytes ? reserve_bytes : g_default_heap.load();
    if (want == 0) {
        NXLOG_E(kTag, "heap_create: no size given and no configured default");
        return false;
    }
    const uint64_t size = align_up(want, kAllocGranularity);
    if (size > kHeapMax) {
        NXLOG_E(kTag, "heap_create: %llu bytes exceed the heap region limit of %llu bytes",
                static_cast<unsigned long long>(size), static_cast<unsigned long long>(kHeapMax));
        return false;
    }
    // [memory] heap_section: auto (default) = a section unless running under Wine (memory.h), 1 = always, 0 = never.
    // true / on / yes and false / off / no work too (heap_section_parse).
    const std::string mode_text = nxrt::config_get_str("memory", "heap_section", "auto");
    bool mode_known = true;
    const int mode = heap_section_parse(mode_text, &mode_known);
    if (!mode_known)
        NXLOG_W(kTag, "[memory] heap_section = %s is not auto, 1/true/on/yes or 0/false/off/no: auto is used", mode_text.c_str());
    // Check for Wine on Linux
    bool wine = false;
#if defined(__linux__)
    // Wine detection would go here if needed
#else
    const HMODULE ntdll = GetModuleHandleW(L"ntdll.dll");
    wine = ntdll && GetProcAddress(ntdll, "wine_get_version");
#endif
    if (mode == 0 || (mode != 1 && wine)) {
        void* p = nxrt::platform::reserve_fixed(reinterpret_cast<void*>(kHeapBase), static_cast<std::size_t>(size));
        if (!p || reinterpret_cast<uint64_t>(p) != kHeapBase) {
            NXLOG_E(kTag, "heap_create: platform::reserve_fixed(%llu MiB) at %llx failed", static_cast<unsigned long long>(size >> 20),
                    static_cast<unsigned long long>(kHeapBase));
            if (p) nxrt::platform::release_fixed(p, size);
            return false;
        }
        g_heap_size = size;
        g_commit_bits.assign((size / kPage + 63) / 64, 0);
        g_heap_cls.assign(static_cast<size_t>(size / kPage), 0);
        g_committed = 0;
        NXLOG_I(kTag, "heap: private memory of %llu MiB at %llx, no section (%s): no alias views, pools import at their own address",
                static_cast<unsigned long long>(size >> 20), static_cast<unsigned long long>(kHeapBase),
                wine ? "Wine" : (mode_text.empty() ? "[memory] heap_section" : mode_text.c_str()));
        return true;
    }
    nxrt::platform::Section* sec = nxrt::platform::section_create(size);
    if (!sec) {
        NXLOG_E(kTag, "heap_create: platform::section_create(%llu MiB) failed", static_cast<unsigned long long>(size >> 20));
        return false;
    }
    uint64_t v = nxrt::platform::section_map_view(sec, kHeapBase, 0, size, true);
    if (!v) {
        NXLOG_E(kTag, "heap_create: section_map_view at %llx failed", static_cast<unsigned long long>(kHeapBase));
        nxrt::platform::section_destroy(sec);
        return false;
    }
    if (v != kHeapBase) {
        NXLOG_E(kTag, "heap_create: primary view landed at %llx, not %llx", v, static_cast<unsigned long long>(kHeapBase));
        nxrt::platform::section_unmap_view(v, size);
        nxrt::platform::section_destroy(sec);
        return false;
    }
    g_section = sec;
    g_heap_size = size;
    g_commit_bits.assign((size / kPage + 63) / 64, 0);
    g_heap_cls.assign(static_cast<size_t>(size / kPage), 0);
    g_committed = 0;
    NXLOG_I(kTag, "heap: section of %llu MiB, primary view at %llx", static_cast<unsigned long long>(size >> 20),
            static_cast<unsigned long long>(kHeapBase));
    return true;
}

bool heap_exists() {
    std::lock_guard lk(g_heap_mu);
    return g_heap_size != 0;
}

bool heap_has_section() {
    std::lock_guard lk(g_heap_mu);
    return g_section != nullptr;
}

uint64_t heap_reserved() {
    std::lock_guard lk(g_heap_mu);
    return g_heap_size;
}

namespace {
// The cause the crash report starts with when the system cannot commit the game's heap: the game answers with svcBreak, which on its own
// says nothing. The launcher shows this sentence after "The game stopped: " (launcher/src-tauri/src/memory.rs has the same one).
constexpr const char* kOutOfMemoryHint =
    "it ran out of memory. Close other programs and try again; on Windows, keep the page file on (system managed).";
bool g_oom_hinted = false;   // under g_heap_mu: the hint is ours to clear
}  // namespace

// Lock order: the write tracker's scope is taken before g_heap_mu, as protect() does. The scope waits for an ImportFreeze over the range,
// and an ImportFreeze holds its slot while it reads the heap's page table (heap_rw_pages, under g_heap_mu): waiting with the lock held,
// neither would end. The commit-limit retries sleep without the lock too, so the page table stays readable for those seconds.
bool heap_commit(uint64_t offset, uint64_t size) {
    uint64_t start = 0, end = 0;
    {
        std::lock_guard lk(g_heap_mu);
        if (!g_heap_size) {
            NXLOG_W(kTag, "heap_commit: no heap");
            return false;
        }
        if (size == 0) return true;
        if (offset > g_heap_size || size > g_heap_size - offset) {
            NXLOG_E(kTag, "heap_commit: [%llx, +%llx) is outside the %llu MiB section",
                    static_cast<unsigned long long>(offset), static_cast<unsigned long long>(size),
                    static_cast<unsigned long long>(g_heap_size >> 20));
            return false;
        }
        start = offset & ~(kPage - 1);
        end = align_up(offset + size, kPage);
    }
    const ww::OsWriteScope wws(reinterpret_cast<void*>(kHeapBase + start), end - start);   // MEM_COMMIT makes committed pages read-write again
    std::unique_lock lk(g_heap_mu);
    if (!g_heap_size || end > g_heap_size) return false;   // the heap went away meanwhile
    // The game sets a heap of about 9 GB at boot and all of it is committed here. When the system's commit charge is near its limit (just after
    // the launcher's setup, or with a small page file) this fails, the game turns the result into nn::os OutOfMemory (0x1003) and calls svcBreak.
    // The system grows a page file after such a failure and other processes give memory back, so wait a little and try again.
    // Windows: only for the out-of-commit errors (others will not change by waiting). The lock is let go while sleeping.
    const auto try_commit = [&] {
        bool done = nxrt::platform::commit(reinterpret_cast<void*>(kHeapBase + start), static_cast<std::size_t>(end - start));
#ifdef _WIN32
        const DWORD err = done ? 0 : GetLastError();   // platform::commit is VirtualAlloc here: its error is still the last one
#endif
        if (done) done = nxrt::platform::protect(reinterpret_cast<void*>(kHeapBase + start), static_cast<std::size_t>(end - start), nxrt::platform::PROT_READ | nxrt::platform::PROT_WRITE);
#ifdef _WIN32
        return std::make_pair(done, err);
#else
        return std::make_pair(done, 0ul);
#endif
    };
    auto [ok, e] = try_commit();
    for (int attempt = 1; !ok && attempt <= 40; ++attempt) {
#ifdef _WIN32
        if (e != ERROR_COMMITMENT_LIMIT && e != ERROR_NOT_ENOUGH_MEMORY && e != ERROR_COMMITMENT_MINIMUM) break;
#endif
        if (attempt == 1)
            NXLOG_W(kTag, "heap_commit [%llx, +%llx): the system is out of commit (error %lu); waiting up to 10 s for the page file to grow",
                    static_cast<unsigned long long>(start), static_cast<unsigned long long>(end - start), static_cast<unsigned long>(e));
        lk.unlock();
        nxrt::platform::sleep_ns(250000000);  // 250 ms
        lk.lock();
        if (!g_heap_size || end > g_heap_size) return false;
        std::tie(ok, e) = try_commit();
        if (ok) NXLOG_I(kTag, "heap_commit: committed after %d retries", attempt);
    }
    if (!ok) {
#ifdef _WIN32
        NXLOG_E(kTag, "heap_commit [%llx, +%llx): platform::commit failed, error %lu",
                static_cast<unsigned long long>(start), static_cast<unsigned long long>(end - start), static_cast<unsigned long>(e));
        if (e == ERROR_COMMITMENT_LIMIT || e == ERROR_NOT_ENOUGH_MEMORY || e == ERROR_COMMITMENT_MINIMUM) {
            crash_set_memory_hint(kOutOfMemoryHint);
            g_oom_hinted = true;
        }
#else
        NXLOG_E(kTag, "heap_commit [%llx, +%llx): platform::commit failed",
                static_cast<unsigned long long>(start), static_cast<unsigned long long>(end - start));
#endif
        return false;
    }
    if (g_oom_hinted) {         // the game asked again and got it: whatever ends the run now has another cause
        crash_set_memory_hint("");
        g_oom_hinted = false;
    }
    for (uint64_t p = start / kPage; p < end / kPage; p++) {
        if (!bit_test(p)) {
            bit_set(p);
            g_committed += kPage;
        }
        g_heap_cls[p] = 3;       // committed read-write
    }
    g_heap_epoch.fetch_add(1, std::memory_order_seq_cst);
    return true;
}

bool heap_decommit(uint64_t offset, uint64_t size) {
    uint64_t start = 0, end = 0;
    {
        std::lock_guard lk(g_heap_mu);
        if (!g_heap_size) return false;
        if (size == 0) return true;
        if (offset > g_heap_size || size > g_heap_size - offset) {
            NXLOG_E(kTag, "heap_decommit: [%llx, +%llx) is outside the section", static_cast<unsigned long long>(offset),
                    static_cast<unsigned long long>(size));
            return false;
        }
        start = offset & ~(kPage - 1);
        end = align_up(offset + size, kPage);
    }
    const ww::OsWriteScope wws(reinterpret_cast<void*>(kHeapBase + start), end - start);   // before g_heap_mu, as in heap_commit
    std::lock_guard lk(g_heap_mu);
    if (!g_heap_size || end > g_heap_size) return false;   // the heap went away meanwhile
    // For section views, decommit is not supported (ERROR_INVALID_PARAMETER), so the pages are made
    // PAGE_NOACCESS through the primary view instead: guest accesses fault, the host pages stay charged to the section
    // (as in smo-recomp's SetMemoryHeapSize), and the alias views still show the section's contents.
    for (uint64_t p = start / kPage; p < end / kPage;) {
        if (!bit_test(p)) {
            p++;
            continue;
        }
        uint64_t run = p;
        while (run < end / kPage && bit_test(run)) {
            const uint64_t addr = kHeapBase + run * kPage;
            if (nxrt::mem::ww::is_retired_page(addr)) {
                // Retired page: the write-watch region was destroyed but guest may still write.
                // Decommit it anyway (make NOACCESS) but clear the retired bit so future faults
                // are not misclassified. This handles the race where guest writes occur between
                // region_destroy and heap_decommit (Issue #77).
                break;
            }
            run++;
        }
        if (run == p) {
            // Single retired page: decommit it and clear the retired bit.
            const uint64_t addr = kHeapBase + p * kPage;
            if (!nxrt::platform::protect(reinterpret_cast<void*>(addr), kPage, nxrt::platform::PROT_NONE)) {
                NXLOG_E(kTag, "heap_decommit [%llx, +%llx): platform::protect failed",
                        static_cast<unsigned long long>(p * kPage), static_cast<unsigned long long>(kPage));
                return false;
            }
            bit_clear(p);
            g_committed -= kPage;
            g_heap_cls[p] = 1;  // no-access
            nxrt::mem::ww::clear_retired_page(addr);
            g_heap_epoch.fetch_add(1, std::memory_order_seq_cst);
            p++;
            continue;
        }
        if (!nxrt::platform::protect(reinterpret_cast<void*>(kHeapBase + p * kPage), static_cast<std::size_t>((run - p) * kPage), nxrt::platform::PROT_NONE)) {
            NXLOG_E(kTag, "heap_decommit [%llx, +%llx): platform::protect failed",
                    static_cast<unsigned long long>(p * kPage), static_cast<unsigned long long>((run - p) * kPage));
            return false;
        }
        for (uint64_t q = p; q < run; q++) {
            bit_clear(q);
            g_committed -= kPage;
            g_heap_cls[q] = 1;
        }
        g_heap_epoch.fetch_add(1, std::memory_order_seq_cst);
        p = run;
    }
    return true;
}

int heap_rw_pages(uint64_t addr, uint64_t size) {
    uint64_t start = 0, end = 0;
    if (!page_span(addr, size, &start, &end)) return 0;
    std::lock_guard lk(g_heap_mu);
    if (!in_heap_view(start, end)) return -1;
    const uint64_t p0 = (start - kHeapBase) / kPage, p1 = (end - kHeapBase) / kPage;
    for (uint64_t q = p0; q < p1; ++q)
        if (g_heap_cls[q] != 3) return 0;
    return 1;
}

uint64_t heap_epoch() { return g_heap_epoch.load(std::memory_order_seq_cst); }

uint64_t heap_committed() {
    std::lock_guard lk(g_heap_mu);
    return g_committed;
}

uint64_t heap_alias(uint64_t offset, uint64_t size, uint64_t at, Prot p) {
    std::lock_guard lk(g_heap_mu);
    if (!g_heap_size || !g_section) {
        NXLOG_W(kTag, "heap_alias: %s", g_heap_size ? "the heap is private memory (no section): no alias views" : "no heap");
        return 0;
    }
    if (size == 0 || offset % kAllocGranularity || size % kAllocGranularity || offset > g_heap_size ||
        size > g_heap_size - offset) {
        NXLOG_W(kTag, "heap_alias: [%llx, +%llx) must be 64 KiB multiples inside the section",
                static_cast<unsigned long long>(offset), static_cast<unsigned long long>(size));
        return 0;
    }
    if (at % kAllocGranularity) {
        NXLOG_W(kTag, "heap_alias: target %llx is not 64 KiB aligned", static_cast<unsigned long long>(at));
        return 0;
    }
    const bool write = (p == Prot::RW || p == Prot::RWX);

    ww::ban_section(offset, size);   // before the view exists: writes through it would not fault in the primary view
    if (at != 0) {
        uint64_t v = nxrt::platform::section_map_view(g_section, at, offset, size, write);
        if (!v) {
            NXLOG_E(kTag, "heap_alias at %llx (section %llx, +%llx): section_map_view failed",
                    static_cast<unsigned long long>(at), static_cast<unsigned long long>(offset),
                    static_cast<unsigned long long>(size));
            return 0;
        }
        if (v != at) {
            nxrt::platform::section_unmap_view(v, size);
            NXLOG_E(kTag, "heap_alias: view landed at %llx, not %llx", v, static_cast<unsigned long long>(at));
            return 0;
        }
        g_aliases.emplace(at, Alias{size, offset});
        g_alias_used[at] = size;
        NXLOG_I(kTag, "heap alias: section %llx (+%llx) at %llx", static_cast<unsigned long long>(offset),
                static_cast<unsigned long long>(size), static_cast<unsigned long long>(at));
        return at;
    }

    uint64_t cand = kAliasBase;
    for (int tries = 0; tries < kAliasTries; tries++) {
        cand = find_free_alias(cand, size);
        if (!cand) break;
        uint64_t v = nxrt::platform::section_map_view(g_section, cand, offset, size, write);
        if (v == cand) {
            g_aliases.emplace(cand, Alias{size, offset});
            g_alias_used[cand] = size;
            NXLOG_I(kTag, "heap alias: section %llx (+%llx) at %llx", static_cast<unsigned long long>(offset),
                    static_cast<unsigned long long>(size), static_cast<unsigned long long>(cand));
            return cand;
        }
        if (v) nxrt::platform::section_unmap_view(v, size);
        NXLOG_W(kTag, "heap_alias: %llx is taken: marking it unusable",
                static_cast<unsigned long long>(cand));
        g_alias_used[cand] = size;  // refused by the kernel: never offer it again
    }
    NXLOG_E(kTag, "heap_alias: no free range for %llx bytes in the alias region", static_cast<unsigned long long>(size));
    return 0;
}

void* heap_private_view(uint64_t offset, uint64_t size) {
    std::lock_guard lk(g_heap_mu);
    if (!g_section || size == 0 || offset % kAllocGranularity || offset > g_heap_size || size > g_heap_size - offset) return nullptr;
    uint64_t v = nxrt::platform::section_map_view(g_section, 0, offset, size, true);
    return v ? reinterpret_cast<void*>(v) : nullptr;
}

bool heap_private_view_unmap(void* view) {
    return view && nxrt::platform::section_unmap_view(reinterpret_cast<uint64_t>(view), 0);
}

bool heap_alias_unmap(uint64_t view_addr) {
    std::lock_guard lk(g_heap_mu);
    auto it = g_aliases.find(view_addr);
    if (it == g_aliases.end()) {
        NXLOG_W(kTag, "heap_alias_unmap: %llx is not an alias view", static_cast<unsigned long long>(view_addr));
        return false;
    }
    if (!nxrt::platform::section_unmap_view(view_addr, it->second.size)) {
        NXLOG_E(kTag, "heap_alias_unmap %llx: section_unmap_view failed",
                static_cast<unsigned long long>(view_addr));
        return false;
    }
    g_alias_used.erase(view_addr);
    g_aliases.erase(it);
    return true;
}

void heap_alias_ranges(std::vector<std::pair<uint64_t, uint64_t>>& out) {
    std::lock_guard lk(g_heap_mu);
    for (const auto& a : g_aliases) out.emplace_back(a.second.offset, a.second.size);
}

size_t heap_alias_count() {
    std::lock_guard lk(g_heap_mu);
    return g_aliases.size();
}

bool heap_offset_of(uint64_t addr, uint64_t* offset) {
    if (!offset) return false;
    std::lock_guard lk(g_heap_mu);
    if (g_heap_size && addr >= kHeapBase && addr - kHeapBase < g_heap_size) {
        *offset = addr - kHeapBase;
        return true;
    }
    auto it = g_aliases.upper_bound(addr);
    if (it == g_aliases.begin()) return false;
    --it;
    if (addr - it->first < it->second.size) {
        *offset = it->second.offset + (addr - it->first);
        return true;
    }
    return false;
}

// ---------------------------------------------------------------------------------------------------------------------
// vm map (svcQueryMemory)

std::string describe_page(uint64_t addr) {
    std::string out;
    char line[256];
    {
        std::unique_lock lk(g_heap_mu, std::try_to_lock);
        if (!lk.owns_lock()) out += "  heap metadata: busy\n";
        else if (addr >= kHeapBase && addr - kHeapBase < g_heap_size) {
            const uint64_t pg = (addr - kHeapBase) / kPage;
            std::snprintf(line, sizeof line, "  heap metadata: page=%llu committed=%d class=%u epoch=%llu (class: 0=reserved 1=noaccess 2=readonly 3=readwrite)\n",
                (unsigned long long)pg, bit_test(pg), (unsigned)g_heap_cls[pg], (unsigned long long)g_heap_epoch.load());
            out += line;
        } else out += "  heap metadata: outside primary heap\n";
    }
    {
        std::unique_lock lk(g_vm_mu, std::try_to_lock);
        if (!lk.owns_lock()) out += "  guest VM metadata: busy\n";
        else {
            auto it = g_vm.upper_bound(addr);
            if (it != g_vm.begin() && addr - std::prev(it)->second.addr < std::prev(it)->second.size) {
                const auto& v = std::prev(it)->second;
                std::snprintf(line, sizeof line, "  guest VM: base=0x%llx size=0x%llx state=0x%x perm=0x%x\n",
                    (unsigned long long)v.addr, (unsigned long long)v.size, v.state, v.perm);
                out += line;
            } else out += "  guest VM: free\n";
        }
    }
    return out;
}

void vm_set(uint64_t addr, uint64_t size, uint32_t state, uint32_t perm) {
    if (size == 0 || addr + size < addr) return;
    const uint64_t end = addr + size;
    std::lock_guard lk(g_vm_mu);
    // Cut every block that overlaps [addr, end); the parts outside the new range stay.
    auto it = g_vm.lower_bound(addr);
    if (it != g_vm.begin()) {
        auto p = std::prev(it);
        if (p->second.addr + p->second.size > addr) it = p;
    }
    while (it != g_vm.end() && it->second.addr < end) {
        VmInfo b = it->second;
        it = g_vm.erase(it);
        if (b.addr < addr) g_vm.emplace(b.addr, VmInfo{b.addr, addr - b.addr, b.state, b.perm});
        if (b.addr + b.size > end) g_vm.emplace(end, VmInfo{end, b.addr + b.size - end, b.state, b.perm});
    }
    if (state != kMemFree) g_vm.emplace(addr, VmInfo{addr, size, state, perm});
}

VmInfo vm_query(uint64_t addr) {
    std::lock_guard lk(g_vm_mu);
    auto it = g_vm.upper_bound(addr);  // first block that starts after addr
    if (it != g_vm.begin()) {
        auto p = std::prev(it);
        if (addr < p->second.addr + p->second.size) return p->second;
        const uint64_t gap_start = p->second.addr + p->second.size;
        const uint64_t gap_end = it == g_vm.end() ? kVmEnd : it->first;
        return VmInfo{gap_start, gap_end - gap_start, kMemFree, 0};
    }
    const uint64_t gap_end = it == g_vm.end() ? kVmEnd : it->first;
    return VmInfo{0, gap_end, kMemFree, 0};
}

}  // namespace nxrt::mem
