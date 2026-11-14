// SPDX-License-Identifier: GPL-3.0-or-later
#include "test_framework.h"

#include "nxrt/config.h"
#include "nxrt/memory.h"

#include <windows.h>

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <random>
#include <string>
#include <utility>
#include <vector>

NXTEST_GROUP(memory)

namespace {

namespace mem = nxrt::mem;

constexpr uint64_t kPg = mem::kPage;
constexpr uint64_t kGr = mem::kAllocGranularity;
constexpr uint64_t kHeapSmall = 256ull << 20;
constexpr uint32_t kCode = static_cast<uint32_t>(mem::kMemCode);
constexpr uint32_t kHeap = static_cast<uint32_t>(mem::kMemHeap);
constexpr uint32_t kStatic = static_cast<uint32_t>(mem::kMemStatic);
constexpr uint32_t kFree = static_cast<uint32_t>(mem::kMemFree);

uint64_t align_up(uint64_t v, uint64_t a) { return (v + a - 1) & ~(a - 1); }

// The access itself lives in a non-inlined callee: clang-cl's SEH only models faults at call sites (without
// -fasync-exceptions), so a plain store inside the __try block would not reliably reach the handler.
__declspec(noinline) void access_byte(uint64_t addr, bool write) {
    if (write) {
        *reinterpret_cast<volatile uint8_t*>(addr) = 0x5A;
    } else {
        volatile uint8_t v = *reinterpret_cast<volatile uint8_t*>(addr);
        (void)v;
    }
}

// 1 when one byte at addr can be read (write = false) or written (write = true), 0 when it raises an access violation.
// Kept free of C++ objects so that the __try block is legal.
__declspec(noinline) int touch(uint64_t addr, bool write) {
    int ok = 1;
    __try {
        access_byte(addr, write);
    } __except (EXCEPTION_EXECUTE_HANDLER) {
        ok = 0;
    }
    return ok;
}

uint32_t* word_at(uint64_t addr) { return reinterpret_cast<uint32_t*>(addr); }

// The host's answer for [addr, addr + size): VirtualQuery over the same page span that mem::is_mapped uses, with no page table.
bool host_mapped(uint64_t addr, uint64_t size) {
    const uint64_t start = addr & ~(kPg - 1);
    const uint64_t end = (addr + size + kPg - 1) & ~(kPg - 1);
    for (uint64_t cur = start; cur < end;) {
        MEMORY_BASIC_INFORMATION mbi{};
        if (VirtualQuery(reinterpret_cast<void*>(cur), &mbi, sizeof mbi) != sizeof mbi || mbi.RegionSize == 0) return false;
        if (mbi.State != MEM_COMMIT) return false;
        if ((mbi.Protect & 0xFF) == PAGE_NOACCESS || (mbi.Protect & PAGE_GUARD)) return false;
        cur = std::min<uint64_t>(end, reinterpret_cast<uint64_t>(mbi.BaseAddress) + mbi.RegionSize);
    }
    return true;
}

}  // namespace

NXTEST(init_is_idempotent_and_reinitialises) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::initialised());
    CHECK(mem::init());  // second call: no-op
    CHECK(mem::initialised());
    mem::shutdown();
    CHECK(!mem::initialised());
    CHECK(mem::init());  // after shutdown the module starts again
    CHECK(mem::initialised());
    mem::shutdown();
}

NXTEST(reserve_commit_write_read_roundtrip) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t a = mem::kMainBase;
    CHECK(mem::reserve_fixed(a, 0x10000, "test"));
    CHECK(mem::commit(a, 3 * kPg, mem::Prot::RW));
    for (uint32_t i = 0; i < 3; i++) word_at(a + i * kPg)[0] = 0x1000u + i;
    for (uint32_t i = 0; i < 3; i++) CHECK_EQ(word_at(a + i * kPg)[0], 0x1000u + i);
    CHECK(mem::is_mapped(a, 3 * kPg));
    CHECK(!mem::is_mapped(a, 0x10000));  // the rest of the reservation is reserved, not committed
    CHECK(mem::release_fixed(a));
    mem::shutdown();
}

NXTEST(reserve_fixed_refuses_occupied_range) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t a = mem::kMainBase;
    CHECK(mem::reserve_fixed(a, 0x10000, "first"));
    CHECK(!mem::reserve_fixed(a, 0x10000, "same range"));
    CHECK(!mem::reserve_fixed(a, 0x20000, "overlapping"));
    CHECK(mem::commit(a, kPg, mem::Prot::RW));
    CHECK(touch(a, true));  // the first reservation is untouched by the refused ones
    CHECK(mem::release_fixed(a));
    mem::shutdown();
}

NXTEST(decommit_then_access_faults) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t a = mem::kMainBase;
    CHECK(mem::reserve_fixed(a, 0x10000, "test"));
    CHECK(mem::commit(a, 2 * kPg, mem::Prot::RW));
    CHECK(touch(a, true));
    CHECK(touch(a + kPg, true));
    CHECK(mem::decommit(a, kPg));
    CHECK(!touch(a, false));  // decommitted page: access violation
    CHECK(!touch(a, true));
    CHECK(touch(a + kPg, true));  // neighbour page still works
    CHECK(!mem::is_mapped(a, kPg));
    CHECK(mem::release_fixed(a));
    mem::shutdown();
}

NXTEST(protect_read_only_then_back_to_rw) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t a = mem::kMainBase;
    CHECK(mem::reserve_fixed(a, 0x10000, "test"));
    CHECK(mem::commit(a, kPg, mem::Prot::RW));
    CHECK(touch(a, true));
    CHECK(mem::protect(a, kPg, mem::Prot::R));
    CHECK(touch(a, false));   // readable
    CHECK(!touch(a, true));   // a write faults
    CHECK(mem::protect(a, kPg, mem::Prot::RW));
    CHECK(touch(a, true));    // writable again
    CHECK(mem::release_fixed(a));
    mem::shutdown();
}

NXTEST(commit_prot_none_read_faults) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t a = mem::kMainBase;
    CHECK(mem::reserve_fixed(a, 0x10000, "test"));
    CHECK(mem::commit(a, kPg, mem::Prot::None));
    CHECK(!touch(a, false));  // committed but PAGE_NOACCESS: read faults
    CHECK(!mem::is_mapped(a, kPg));
    CHECK(mem::protect(a, kPg, mem::Prot::RW));
    CHECK(touch(a, true));
    CHECK(mem::is_mapped(a, kPg));
    CHECK(mem::release_fixed(a));
    mem::shutdown();
}

NXTEST(protect_refuses_uncommitted_range) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t a = mem::kMainBase;
    CHECK(mem::reserve_fixed(a, 0x10000, "test"));
    CHECK(mem::commit(a, kPg, mem::Prot::RW));
    CHECK(!mem::protect(a, 2 * kPg, mem::Prot::R));  // page 1 is only reserved
    CHECK(touch(a, true));                            // page 0 kept its protection (no half change)
    CHECK(mem::release_fixed(a));
    mem::shutdown();
}

NXTEST(release_fixed_makes_range_reservable_again) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t a = mem::kMainBase;
    CHECK(mem::reserve_fixed(a, 0x10000, "test"));
    CHECK(mem::release_fixed(a));
    CHECK(!mem::release_fixed(a));  // second release: nothing there any more
    CHECK(mem::reserve_fixed(a, 0x10000, "again"));
    CHECK(mem::release_fixed(a));
    mem::shutdown();
}

NXTEST(alloc_kernel_aligned_committed_and_disjoint) {
    mem::shutdown();
    CHECK(mem::init());
    std::vector<std::pair<uint64_t, uint64_t>> got;
    for (uint32_t i = 0; i < 100; i++) {
        const uint64_t size = (i % 7 + 1) * 1000ull;      // 1..7000 bytes, one page each
        const uint64_t align = (i % 2) ? kGr : kPg;
        const uint64_t p = mem::alloc_kernel(size, align);
        CHECK(p != 0);
        if (!p) break;
        CHECK_EQ(p % align, 0ull);
        CHECK(p >= mem::kKernelBase && p + size <= mem::kKernelBase + mem::kKernelSize);
        CHECK(mem::is_mapped(p, size));
        CHECK(touch(p, true));
        got.emplace_back(p, align_up(size, kPg));
    }
    std::sort(got.begin(), got.end());
    for (size_t i = 1; i < got.size(); i++) CHECK(got[i].first >= got[i - 1].first + got[i - 1].second);
    mem::shutdown();
}

NXTEST(heap_commit_counts_each_page_once) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_exists());
    CHECK_EQ(mem::heap_reserved(), kHeapSmall);
    CHECK_EQ(mem::heap_committed(), 0ull);
    CHECK(mem::heap_commit(0, kPg));
    CHECK_EQ(mem::heap_committed(), kPg);
    CHECK(mem::heap_commit(0, kPg));  // same page again: still counted once
    CHECK_EQ(mem::heap_committed(), kPg);
    CHECK(mem::heap_commit(kPg, 2 * kPg));
    CHECK_EQ(mem::heap_committed(), 3 * kPg);
    CHECK(mem::heap_commit(100, 2 * kPg));  // spans pages 0..2, all already committed
    CHECK_EQ(mem::heap_committed(), 3 * kPg);
    CHECK(mem::heap_decommit(0, kPg));
    CHECK_EQ(mem::heap_committed(), 2 * kPg);
    CHECK(!touch(mem::kHeapBase, false));          // decommitted page faults
    CHECK(touch(mem::kHeapBase + kPg, true));      // its neighbour does not
    CHECK(mem::heap_commit(0, kPg));               // recommit brings it back, usable
    CHECK(touch(mem::kHeapBase, true));
    CHECK_EQ(mem::heap_committed(), 3 * kPg);
    CHECK(!mem::heap_commit(kHeapSmall - kPg, 2 * kPg));  // runs past the section
    CHECK(mem::heap_commit(kHeapSmall - kPg, kPg));       // last page is fine
    CHECK_EQ(mem::heap_committed(), 4 * kPg);
    mem::shutdown();
    CHECK(!mem::heap_exists());
}

NXTEST(alias_commit_through_alias_view_is_seen_by_primary) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    const uint64_t a = mem::heap_alias(0, kGr, 0, mem::Prot::RW);
    CHECK(a != 0);
    // Commit the range through the alias view itself, bypassing heap_commit.
    CHECK(VirtualAlloc(reinterpret_cast<void*>(a), static_cast<SIZE_T>(kGr), MEM_COMMIT, PAGE_READWRITE) != nullptr);
    word_at(a)[7] = 0xCAFE0001u;
    CHECK_EQ(word_at(mem::kHeapBase)[7], 0xCAFE0001u);  // visible through the primary view
    CHECK_EQ(mem::heap_committed(), 0ull);              // the module counter only counts heap_commit
    CHECK(mem::heap_alias_unmap(a));
    mem::shutdown();
}

NXTEST(heap_create_rounding_second_call_and_limit) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(!mem::heap_create(mem::kHeapMax + kGr));  // above the 8 GiB heap region
    CHECK(!mem::heap_exists());
    CHECK(mem::heap_create(kGr + 1));                 // rounded up to two granules
    CHECK_EQ(mem::heap_reserved(), 2 * kGr);
    CHECK(mem::heap_create(kHeapSmall));              // a second call is ignored
    CHECK_EQ(mem::heap_reserved(), 2 * kGr);
    mem::shutdown();
}

NXTEST(alias_sees_primary_writes_and_back) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_commit(0, kGr));
    uint32_t* prim = word_at(mem::kHeapBase);
    for (uint32_t i = 0; i < 4096; i++) prim[i] = 0xA5A50000u + i;

    const uint64_t a = mem::heap_alias(0, kGr, 0, mem::Prot::RW);
    CHECK(a != 0);
    CHECK(a >= mem::kAliasBase && a + kGr <= mem::kAliasBase + mem::kAliasSize);
    uint32_t* av = word_at(a);
    CHECK_EQ(av[0], 0xA5A50000u);
    CHECK_EQ(av[1000], 0xA5A50000u + 1000);
    av[5] = 0x12345678u;
    CHECK_EQ(prim[5], 0x12345678u);  // write through the alias, read through the primary view

    const uint64_t b = mem::heap_alias(0, kGr, 0, mem::Prot::RW);
    CHECK(b != 0 && b != a);
    CHECK_EQ(word_at(b)[5], 0x12345678u);  // a second alias of the same pages sees both writes
    CHECK_EQ(word_at(b)[0], 0xA5A50000u);
    CHECK_EQ(mem::heap_alias_count(), 2u);
    CHECK(mem::heap_alias_unmap(a));
    CHECK(mem::heap_alias_unmap(b));
    CHECK_EQ(mem::heap_alias_count(), 0u);
    mem::shutdown();
}

NXTEST(alias_of_nonzero_offset_maps_right_pages) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_commit(kGr, kGr));  // second granule of the section
    word_at(mem::kHeapBase + kGr)[0] = 0xBEEF0001u;
    word_at(mem::kHeapBase + kGr + kPg)[0] = 0xBEEF0002u;
    const uint64_t a = mem::heap_alias(kGr, kGr, 0, mem::Prot::RW);
    CHECK(a != 0);
    CHECK_EQ(word_at(a)[0], 0xBEEF0001u);
    CHECK_EQ(word_at(a + kPg)[0], 0xBEEF0002u);
    word_at(a + 2 * kPg)[0] = 0xBEEF0003u;
    CHECK_EQ(word_at(mem::kHeapBase + kGr + 2 * kPg)[0], 0xBEEF0003u);
    CHECK(mem::heap_alias_unmap(a));
    mem::shutdown();
}

NXTEST(unmapping_one_alias_keeps_the_others) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_commit(0, kGr));
    word_at(mem::kHeapBase)[0] = 0x77u;
    const uint64_t a1 = mem::heap_alias(0, kGr, 0, mem::Prot::RW);
    const uint64_t a2 = mem::heap_alias(0, kGr, 0, mem::Prot::RW);
    CHECK(a1 != 0 && a2 != 0 && a1 != a2);
    CHECK(touch(a1, false));
    CHECK(mem::heap_alias_unmap(a1));
    CHECK(!touch(a1, false));              // the unmapped view faults
    CHECK_EQ(word_at(a2)[0], 0x77u);        // the other alias is still valid
    CHECK(touch(mem::kHeapBase, true));    // and so is the primary view
    CHECK(!mem::heap_alias_unmap(a1));     // already gone
    CHECK_EQ(mem::heap_alias_count(), 1u);
    CHECK(mem::heap_alias_unmap(a2));
    mem::shutdown();
}

NXTEST(heap_offset_of_primary_alias_and_unrelated) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    const uint64_t a = mem::heap_alias(kGr, kGr, 0, mem::Prot::RW);
    CHECK(a != 0);
    uint64_t off = 0xFFFFFFFFull;
    CHECK(mem::heap_offset_of(mem::kHeapBase + 100, &off));
    CHECK_EQ(off, 100ull);
    CHECK(mem::heap_offset_of(a + 8, &off));
    CHECK_EQ(off, kGr + 8);
    CHECK(!mem::heap_offset_of(a + kGr, &off));                 // one past the alias
    CHECK(!mem::heap_offset_of(mem::kHeapBase + kHeapSmall, &off));  // past the section
    CHECK(!mem::heap_offset_of(mem::kMainBase, &off));
    CHECK(mem::heap_alias_unmap(a));
    mem::shutdown();
}

// [memory] heap_section = 0 (what auto picks under Wine): private memory at kHeapBase, commit/decommit as with the section, no views.
NXTEST(heap_private_memory_without_section) {
    mem::shutdown();
    nxrt::config_set("memory", "heap_section", "0");
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_exists());
    CHECK(!mem::heap_has_section());
    CHECK(mem::heap_commit(0, kGr));
    word_at(mem::kHeapBase)[3] = 0xCAFE0002u;
    CHECK_EQ(word_at(mem::kHeapBase)[3], 0xCAFE0002u);
    CHECK_EQ(touch(mem::kHeapBase + kGr, true), 0);              // not committed
    CHECK(mem::heap_decommit(0, kPg));
    CHECK_EQ(touch(mem::kHeapBase, false), 0);                   // decommitted: no access
    CHECK_EQ(mem::heap_alias(0, kGr, 0, mem::Prot::RW), 0ull);
    CHECK(mem::heap_private_view(0, kGr) == nullptr);
    mem::shutdown();
    CHECK(!mem::heap_exists());
    nxrt::config_set("memory", "heap_section", "auto");
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));                         // the region was released: a section fits again
    CHECK(mem::heap_has_section());
    mem::shutdown();
}

NXTEST(heap_alias_rejects_misaligned_arguments) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK_EQ(mem::heap_alias(kPg, kGr, 0, mem::Prot::RW), 0ull);    // misaligned offset
    CHECK_EQ(mem::heap_alias(0, kPg, 0, mem::Prot::RW), 0ull);      // misaligned size
    CHECK_EQ(mem::heap_alias(0, 0, 0, mem::Prot::RW), 0ull);        // empty
    CHECK_EQ(mem::heap_alias(0, kGr, kPg, mem::Prot::RW), 0ull);    // misaligned target
    CHECK_EQ(mem::heap_alias_count(), 0u);
    mem::shutdown();
}

NXTEST(vm_set_and_query_blocks_and_gaps) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t A = 0x100000000ull, B = 0x100020000ull, C = 0x100100000ull;
    mem::vm_set(A, 0x10000, kCode, 5);
    mem::vm_set(B, 0x20000, kHeap, 3);
    mem::vm_set(C, 0x10000, kStatic, 1);

    mem::VmInfo q = mem::vm_query(A + 0x100);
    CHECK_EQ(q.addr, A);
    CHECK_EQ(q.size, 0x10000ull);
    CHECK_EQ(q.state, kCode);
    CHECK_EQ(q.perm, 5u);

    q = mem::vm_query(B + 0x1FFFF);
    CHECK_EQ(q.addr, B);
    CHECK_EQ(q.size, 0x20000ull);
    CHECK_EQ(q.state, kHeap);
    CHECK_EQ(q.perm, 3u);

    q = mem::vm_query(C);
    CHECK_EQ(q.addr, C);
    CHECK_EQ(q.state, kStatic);

    q = mem::vm_query(A + 0x10000);  // gap between A and B
    CHECK_EQ(q.addr, A + 0x10000);
    CHECK_EQ(q.size, 0x10000ull);
    CHECK_EQ(q.state, kFree);

    q = mem::vm_query(A - 0x1000);  // gap before the first block starts at 0
    CHECK_EQ(q.addr, 0ull);
    CHECK_EQ(q.size, A);
    CHECK_EQ(q.state, kFree);

    q = mem::vm_query(C + 0x10000);  // gap after the last block runs to the end of the address space
    CHECK_EQ(q.addr, C + 0x10000);
    CHECK_EQ(q.size, 0x8000000000ull - (C + 0x10000));
    CHECK_EQ(q.state, kFree);
    mem::shutdown();
}

NXTEST(vm_overlap_cuts_and_splits_blocks) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t A = 0x100000000ull;
    mem::vm_set(A, 0x40000, kCode, 5);
    mem::vm_set(A + 0x10000, 0x10000, kHeap, 3);  // cut into the middle of the first block

    mem::VmInfo q = mem::vm_query(A + 0x8000);
    CHECK_EQ(q.addr, A);
    CHECK_EQ(q.size, 0x10000ull);
    CHECK_EQ(q.state, kCode);
    CHECK_EQ(q.perm, 5u);

    q = mem::vm_query(A + 0x10000);
    CHECK_EQ(q.addr, A + 0x10000);
    CHECK_EQ(q.size, 0x10000ull);
    CHECK_EQ(q.state, kHeap);
    CHECK_EQ(q.perm, 3u);

    q = mem::vm_query(A + 0x20000);
    CHECK_EQ(q.addr, A + 0x20000);
    CHECK_EQ(q.size, 0x20000ull);
    CHECK_EQ(q.state, kCode);
    CHECK_EQ(q.perm, 5u);
    mem::shutdown();
}

NXTEST(vm_free_set_punches_a_hole) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t A = 0x100000000ull;
    mem::vm_set(A, 0x30000, kStatic, 1);
    mem::vm_set(A + 0x10000, 0x10000, kFree, 0);

    mem::VmInfo q = mem::vm_query(A);
    CHECK_EQ(q.addr, A);
    CHECK_EQ(q.size, 0x10000ull);
    CHECK_EQ(q.state, kStatic);

    q = mem::vm_query(A + 0x10000);
    CHECK_EQ(q.addr, A + 0x10000);
    CHECK_EQ(q.size, 0x10000ull);
    CHECK_EQ(q.state, kFree);

    q = mem::vm_query(A + 0x20000);
    CHECK_EQ(q.addr, A + 0x20000);
    CHECK_EQ(q.size, 0x10000ull);
    CHECK_EQ(q.state, kStatic);
    mem::shutdown();
}

NXTEST(is_mapped_reports_committed_accessible_pages) {
    mem::shutdown();
    CHECK(mem::init());
    const uint64_t a = mem::kMainBase;
    CHECK(mem::reserve_fixed(a, 0x10000, "test"));
    CHECK(mem::commit(a, 2 * kPg, mem::Prot::RW));
    CHECK(mem::is_mapped(a, 2 * kPg));
    CHECK(mem::is_mapped(a + 16, kPg));
    CHECK(!mem::is_mapped(a, 3 * kPg));                     // third page only reserved
    CHECK(!mem::is_mapped(a + 0x8000, 0x8000));             // reserved, uncommitted
    CHECK(!mem::is_mapped(mem::kMainBase + 0x100000, kPg));  // not reserved at all
    CHECK(mem::decommit(a, kPg));
    CHECK(!mem::is_mapped(a, kPg));
    CHECK(mem::protect(a + kPg, kPg, mem::Prot::None));
    CHECK(!mem::is_mapped(a + kPg, kPg));                   // PAGE_NOACCESS counts as not mapped
    CHECK(mem::release_fixed(a));
    mem::shutdown();
}

NXTEST(heap_is_mapped_tracks_commit_state) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_commit(0, 4ull << 20));
    CHECK(mem::is_mapped(mem::kHeapBase, 4ull << 20));          // the whole committed part
    CHECK(mem::is_mapped(mem::kHeapBase + (1ull << 20), 64ull << 10));
    CHECK(mem::is_mapped(mem::kHeapBase + 100, 8));              // unaligned start inside a committed page
    CHECK(!mem::is_mapped(mem::kHeapBase + (4ull << 20) - kPg, 2 * kPg));  // last committed page plus one never committed
    CHECK(!mem::is_mapped(mem::kHeapBase + (4ull << 20), kPg));  // first never-committed page
    CHECK(!mem::is_mapped(mem::kHeapBase + (8ull << 20), kPg));  // far inside the section, never committed
    CHECK(!mem::is_mapped(mem::kHeapBase, kHeapSmall));          // the range runs past the committed part
    CHECK(mem::heap_decommit(0, kPg));
    CHECK(!mem::is_mapped(mem::kHeapBase, kPg));                 // decommitted page
    CHECK(mem::is_mapped(mem::kHeapBase + kPg, kPg));            // its neighbour still committed
    mem::shutdown();
}

NXTEST(heap_protect_none_and_back_keeps_neighbours) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_commit(0, 4ull << 20));
    const uint64_t r = mem::kHeapBase + (1ull << 20);
    CHECK(mem::protect(r, 64ull << 10, mem::Prot::None));
    CHECK(!mem::is_mapped(r, 64ull << 10));
    CHECK(!touch(r, false));                                     // the host agrees: reads fault
    CHECK(mem::is_mapped(r - (64ull << 10), 64ull << 10));       // left neighbour stays mapped
    CHECK(mem::is_mapped(r + (64ull << 10), 64ull << 10));       // right neighbour stays mapped
    CHECK(!mem::is_mapped(r - kPg, 2 * kPg));                    // a range that touches the protected pages
    CHECK(mem::protect(r, 64ull << 10, mem::Prot::RW));
    CHECK(mem::is_mapped(r, 64ull << 10));
    CHECK(touch(r, true));                                       // writable again
    // A range that includes never-committed pages is refused and changes nothing.
    CHECK(!mem::protect(mem::kHeapBase + (3ull << 20), 2ull << 20, mem::Prot::None));
    CHECK(mem::is_mapped(mem::kHeapBase + (3ull << 20), 1ull << 20));
    CHECK(touch(mem::kHeapBase + (3ull << 20), true));
    CHECK(!mem::protect(mem::kHeapBase + (4ull << 20), kPg, mem::Prot::RW));  // wholly uncommitted
    CHECK(!mem::is_mapped(mem::kHeapBase + (4ull << 20), kPg));
    mem::shutdown();
}

NXTEST(heap_decommit_then_protect_restores_access) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_commit(0, 4ull << 20));
    const uint64_t r = mem::kHeapBase + (1ull << 20);
    CHECK(mem::heap_decommit(1ull << 20, 128ull << 10));
    CHECK_EQ(mem::heap_committed(), (4ull << 20) - (128ull << 10));
    CHECK(!mem::is_mapped(r, 128ull << 10));
    CHECK(!mem::is_mapped(r, kPg));
    CHECK(mem::is_mapped(mem::kHeapBase, 1ull << 20));           // neighbour below stays mapped
    CHECK(mem::protect(r, 128ull << 10, mem::Prot::RW));         // host pages are still committed, so this succeeds
    CHECK(mem::is_mapped(r, 128ull << 10));
    CHECK(touch(r, true));
    CHECK(host_mapped(r, 128ull << 10));                         // the host agrees
    mem::shutdown();
}

NXTEST(heap_is_mapped_agrees_with_virtualquery_on_random_ranges) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_commit(0, 4ull << 20));
    // Holes in the table: a protected run, a decommitted run, and a decommitted run made accessible again.
    CHECK(mem::protect(mem::kHeapBase + (512ull << 10), 256ull << 10, mem::Prot::None));
    CHECK(mem::heap_decommit(2ull << 20, 512ull << 10));
    CHECK(mem::protect(mem::kHeapBase + (2ull << 20) + (128ull << 10), 128ull << 10, mem::Prot::RW));
    const uint64_t lo = mem::kHeapBase - 64 * kPg;               // 64 pages before the section
    const uint64_t span = kHeapSmall + 128 * kPg;                // through 64 pages after the section
    const uint64_t hot_span = (6ull << 20) + 128 * kPg;          // the committed part and a little beyond it
    std::mt19937_64 rng(0x5EED0003ull);
    size_t yes = 0, no = 0;
    for (uint32_t i = 0; i < 200; i++) {
        // Every other range starts around the committed part, so both answers occur often; the rest cover the whole span.
        const uint64_t addr = (i % 2 == 0) ? lo + rng() % hot_span : lo + rng() % span;
        const uint64_t size = (i % 4 == 0) ? 1 + rng() % (16ull << 10) : 1 + rng() % (2ull << 20);
        const bool want = host_mapped(addr, size);
        const bool got = mem::is_mapped(addr, size);
        (want ? yes : no)++;
        char buf[96];
        std::snprintf(buf, sizeof buf, "addr %llx size %llx", static_cast<unsigned long long>(addr),
                      static_cast<unsigned long long>(size));
        CHECK_MSG(got == want, buf);
    }
    CHECK(yes > 20 && no > 20);  // both answers occur often enough for the comparison to mean something
    mem::shutdown();
}

NXTEST(heap_protect_cycle_is_fast_with_full_heap) {
    mem::shutdown();
    CHECK(mem::init());
    CHECK(mem::heap_create(kHeapSmall));
    CHECK(mem::heap_commit(0, kHeapSmall));                      // the whole 256 MiB section committed
    const uint64_t src = mem::kHeapBase + (1ull << 20);
    const auto t0 = std::chrono::steady_clock::now();
    for (uint32_t i = 0; i < 50; i++) {
        CHECK(mem::protect(src, 1ull << 20, mem::Prot::None));
        CHECK(!mem::is_mapped(src, 1ull << 20));
        CHECK(mem::protect(src, 1ull << 20, mem::Prot::RW));
        CHECK(mem::is_mapped(src, 1ull << 20));
    }
    const auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now() - t0).count();
    CHECK_MSG(ms < 2000, "50 protect cycles on a 256 MiB committed heap took " + std::to_string(ms) + " ms");
    mem::shutdown();
}
