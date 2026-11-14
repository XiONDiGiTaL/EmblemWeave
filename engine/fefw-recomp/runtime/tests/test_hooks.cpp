// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for the site-hook registry (include/nxrt/hooks.h, src/core/hooks.cpp): registration, chain order, removal, counters and
// concurrent calls. The lifted-code entry points nx_site_inline / nx_site_midhook / nx_hook_entry are called directly.
// Rule used throughout: CHECK is per host thread, so the worker threads in the concurrency test never CHECK. They store results in
// variables and the test thread checks them after join().
#include "test_framework.h"

#include "nxrt/hooks.h"

#include <atomic>
#include <cstdint>
#include <set>
#include <thread>
#include <vector>

NXTEST_GROUP(hooks)

using nxrt::HookKind;

namespace {

// Hook addresses are never dereferenced; each test uses its own addresses where it matters.
constexpr uint64_t kAddrA = 0x7100001000ull;
constexpr uint64_t kAddrB = 0x7100001100ull;
constexpr uint64_t kAddrC = 0x7100001200ull;

// Records the order in which tagged hooks ran.
struct Log {
    std::vector<int> order;
};

// The user pointer of a tagged hook: which tag to append, what to return, and whether to write guest register x5 = 77.
struct Tagged {
    Log* log = nullptr;
    int tag = 0;
    int ret = 0;
    bool set_x5 = false;
};

int entry_tagged(::Ctx*, uint64_t, void* user) {
    auto* t = static_cast<Tagged*>(user);
    t->log->order.push_back(t->tag);
    return t->ret;
}

void inline_tagged(::Ctx* c, uint64_t, void* user) {
    auto* t = static_cast<Tagged*>(user);
    t->log->order.push_back(t->tag);
    if (t->set_x5) c->x[5] = 77;
}

int noop_entry(::Ctx*, uint64_t, void*) { return 0; }
void noop_inline(::Ctx*, uint64_t, void*) {}

// Entry hook used by the concurrency test: counts calls in the atomic given as user, then handles the call.
int count_entry(::Ctx*, uint64_t, void* user) {
    static_cast<std::atomic<uint64_t>*>(user)->fetch_add(1);
    return 1;
}

// Entry hook that records its address and user pointer.
struct Probe {
    int calls = 0;
    uint64_t addr = 0;
    void* user = nullptr;
};
int probe_entry(::Ctx*, uint64_t addr, void* user) {
    auto* p = static_cast<Probe*>(user);
    p->calls++;
    p->addr = addr;
    p->user = user;
    return 1;
}

// Entry hook that removes itself through hook_remove on its own id.
struct SelfRemove {
    uint32_t id = 0;
    int calls = 0;
    bool removed = false;
};
int self_remove_entry(::Ctx*, uint64_t, void* user) {
    auto* s = static_cast<SelfRemove*>(user);
    s->calls++;
    s->removed = nxrt::hook_remove(s->id);
    return 1;
}

// Entry hook that writes the guest return register, as a function replacement would.
int replace_entry(::Ctx* c, uint64_t, void*) {
    c->x[0] = 42;
    return 1;
}

}  // namespace

NXTEST(entry_hook_runs_with_addr_and_user) {
    nxrt::hooks_clear();
    ::Ctx c{};
    Probe p;
    nxrt::hook_add_entry(kAddrA, probe_entry, &p);

    int handled = nx_hook_entry(&c, kAddrA);
    CHECK(handled != 0);
    CHECK_EQ(p.calls, 1);
    CHECK_EQ(p.addr, kAddrA);
    CHECK(p.user == &p);

    // An address with no hook registered: nothing runs and the lifted function continues (returns 0).
    CHECK_EQ(nx_hook_entry(&c, kAddrA + 8), 0);
    CHECK_EQ(p.calls, 1);
    nxrt::hooks_clear();
}

NXTEST(entry_chain_stops_at_first_nonzero) {
    nxrt::hooks_clear();
    ::Ctx c{};
    Log log;
    Tagged t1{&log, 1, 0, false};
    Tagged t2{&log, 2, 1, false};
    Tagged t3{&log, 3, 1, false};
    uint32_t id1 = nxrt::hook_add_entry(kAddrA, entry_tagged, &t1);
    uint32_t id2 = nxrt::hook_add_entry(kAddrA, entry_tagged, &t2);
    uint32_t id3 = nxrt::hook_add_entry(kAddrA, entry_tagged, &t3);

    CHECK_EQ(nx_hook_entry(&c, kAddrA), 1);
    CHECK_EQ(log.order.size(), 2u);
    CHECK(log.order == (std::vector<int>{1, 2}));
    CHECK_EQ(nxrt::hook_calls(id1), 1u);
    CHECK_EQ(nxrt::hook_calls(id2), 1u);
    CHECK_EQ(nxrt::hook_calls(id3), 0u);
    nxrt::hooks_clear();
}

NXTEST(entry_chain_all_zero_returns_zero) {
    nxrt::hooks_clear();
    ::Ctx c{};
    Log log;
    Tagged t1{&log, 1, 0, false};
    Tagged t2{&log, 2, 0, false};
    Tagged t3{&log, 3, 0, false};
    nxrt::hook_add_entry(kAddrA, entry_tagged, &t1);
    nxrt::hook_add_entry(kAddrA, entry_tagged, &t2);
    nxrt::hook_add_entry(kAddrA, entry_tagged, &t3);

    CHECK_EQ(nx_hook_entry(&c, kAddrA), 0);
    CHECK(log.order == (std::vector<int>{1, 2, 3}));
    nxrt::hooks_clear();
}

NXTEST(mid_hooks_are_independent_of_entry_hooks) {
    nxrt::hooks_clear();
    ::Ctx c{};
    Log log;
    Tagged mid{&log, 10, 1, false};
    Tagged entry{&log, 20, 1, false};
    uint32_t mid_id = nxrt::hook_add_mid(kAddrA, entry_tagged, &mid);
    uint32_t entry_id = nxrt::hook_add_entry(kAddrA, entry_tagged, &entry);

    CHECK_EQ(nxrt::hook_count_at(kAddrA, HookKind::Mid), 1u);
    CHECK_EQ(nxrt::hook_count_at(kAddrA, HookKind::Entry), 1u);

    CHECK_EQ(nx_site_midhook(&c, kAddrA), 1);
    CHECK(log.order == (std::vector<int>{10}));
    CHECK_EQ(nxrt::hook_calls(mid_id), 1u);
    CHECK_EQ(nxrt::hook_calls(entry_id), 0u);

    log.order.clear();
    CHECK_EQ(nx_hook_entry(&c, kAddrA), 1);
    CHECK(log.order == (std::vector<int>{20}));
    CHECK_EQ(nxrt::hook_calls(mid_id), 1u);
    CHECK_EQ(nxrt::hook_calls(entry_id), 1u);
    nxrt::hooks_clear();
}

NXTEST(inline_hooks_all_run_in_order) {
    nxrt::hooks_clear();
    ::Ctx c{};
    Log log;
    Tagged t1{&log, 1, 0, false};
    Tagged t2{&log, 2, 0, true};  // writes c.x[5] = 77
    Tagged t3{&log, 3, 0, false};
    nxrt::hook_add_inline(kAddrA, inline_tagged, &t1);
    nxrt::hook_add_inline(kAddrA, inline_tagged, &t2);
    nxrt::hook_add_inline(kAddrA, inline_tagged, &t3);

    nx_site_inline(&c, kAddrA);
    CHECK(log.order == (std::vector<int>{1, 2, 3}));
    CHECK_EQ(c.x[5], 77u);
    nxrt::hooks_clear();
}

NXTEST(hook_remove) {
    nxrt::hooks_clear();
    ::Ctx c{};
    Log log;
    Tagged t1{&log, 1, 0, false};
    Tagged t2{&log, 2, 0, false};
    Tagged t3{&log, 3, 0, false};
    uint32_t id1 = nxrt::hook_add_inline(kAddrA, inline_tagged, &t1);
    uint32_t id2 = nxrt::hook_add_inline(kAddrA, inline_tagged, &t2);
    uint32_t id3 = nxrt::hook_add_inline(kAddrA, inline_tagged, &t3);
    CHECK_EQ(nxrt::hook_count(), 3u);
    CHECK_EQ(nxrt::hook_count_at(kAddrA, HookKind::Inline), 3u);

    CHECK(nxrt::hook_remove(id2));
    CHECK_EQ(nxrt::hook_count(), 2u);
    CHECK_EQ(nxrt::hook_count_at(kAddrA, HookKind::Inline), 2u);
    CHECK(!nxrt::hook_remove(id2));      // already removed
    CHECK(!nxrt::hook_remove(999999));   // never registered

    nx_site_inline(&c, kAddrA);
    CHECK(log.order == (std::vector<int>{1, 3}));
    CHECK_EQ(nxrt::hook_calls(id1), 1u);
    CHECK_EQ(nxrt::hook_calls(id2), 0u);  // removed: unknown id reports 0
    CHECK_EQ(nxrt::hook_calls(id3), 1u);

    CHECK(nxrt::hook_remove(id1));
    CHECK(nxrt::hook_remove(id3));
    CHECK_EQ(nxrt::hook_count(), 0u);
    CHECK_EQ(nxrt::hook_count_at(kAddrA, HookKind::Inline), 0u);
    nxrt::hooks_clear();
}

NXTEST(null_function_returns_zero_id) {
    nxrt::hooks_clear();
    CHECK_EQ(nxrt::hook_add_entry(kAddrA, nullptr), 0u);
    CHECK_EQ(nxrt::hook_add_inline(kAddrA, nullptr), 0u);
    CHECK_EQ(nxrt::hook_add_mid(kAddrA, nullptr), 0u);
    CHECK_EQ(nxrt::hook_count(), 0u);
    CHECK_EQ(nxrt::hook_count_at(kAddrA, HookKind::Entry), 0u);
    CHECK_EQ(nxrt::hook_count_at(kAddrA, HookKind::Inline), 0u);
    CHECK_EQ(nxrt::hook_count_at(kAddrA, HookKind::Mid), 0u);
    nxrt::hooks_clear();
}

NXTEST(ids_are_unique_and_nonzero) {
    nxrt::hooks_clear();
    std::set<uint32_t> ids;
    bool all_nonzero = true;
    for (int i = 0; i < 50; i++) {
        uint64_t addr = kAddrA + static_cast<uint64_t>(i % 5) * 0x100;
        uint32_t id = 0;
        switch (i % 3) {
            case 0: id = nxrt::hook_add_inline(addr, noop_inline); break;
            case 1: id = nxrt::hook_add_mid(addr, noop_entry); break;
            default: id = nxrt::hook_add_entry(addr, noop_entry); break;
        }
        if (id == 0) all_nonzero = false;
        ids.insert(id);
    }
    CHECK(all_nonzero);
    CHECK_EQ(ids.size(), 50u);
    CHECK_EQ(nxrt::hook_count(), 50u);

    // Ids are not reused after removal.
    uint32_t first = *ids.begin();
    CHECK(nxrt::hook_remove(first));
    uint32_t fresh = nxrt::hook_add_entry(kAddrB, noop_entry);
    CHECK(fresh != 0);
    CHECK(ids.count(fresh) == 0);
    nxrt::hooks_clear();
}

NXTEST(counters) {
    nxrt::hooks_clear();
    ::Ctx c{};
    Log log;
    Tagged t{&log, 1, 0, false};
    uint32_t id = nxrt::hook_add_entry(kAddrA, entry_tagged, &t);

    // Three calls at the hooked address and one at an address with no hook: the site counter counts all four.
    nx_hook_entry(&c, kAddrA);
    nx_hook_entry(&c, kAddrA);
    nx_hook_entry(&c, kAddrA);
    nx_hook_entry(&c, kAddrB);
    CHECK_EQ(nxrt::hook_calls(id), 3u);
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Entry), 4u);
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Inline), 0u);
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Mid), 0u);

    nx_site_inline(&c, kAddrA);
    nx_site_inline(&c, kAddrC);
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Inline), 2u);
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Entry), 4u);

    nx_site_midhook(&c, kAddrB);
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Mid), 1u);

    nxrt::hooks_clear();
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Entry), 0u);
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Inline), 0u);
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Mid), 0u);
    CHECK_EQ(nxrt::hook_count(), 0u);
    CHECK_EQ(nxrt::hook_calls(id), 0u);
}

NXTEST(hook_modifies_registers_like_a_function_replacement) {
    nxrt::hooks_clear();
    ::Ctx c{};
    nxrt::hook_add_entry(kAddrA, replace_entry);
    int handled = nx_hook_entry(&c, kAddrA);
    CHECK(handled != 0);
    CHECK_EQ(c.x[0], 42u);
    nxrt::hooks_clear();
}

NXTEST(concurrent_entry_calls_with_registration_churn) {
    nxrt::hooks_clear();
    constexpr uint64_t kHotAddr = 0x7100002000ull;
    constexpr int kThreads = 4;
    constexpr int kCallsPerThread = 20000;
    constexpr int kChurn = 200;

    std::atomic<uint64_t> hits{0};
    uint32_t perm_id = nxrt::hook_add_entry(kHotAddr, count_entry, &hits);
    CHECK(perm_id != 0);

    std::atomic<bool> go{false};
    std::atomic<int> unhandled{0};  // calls that returned 0 although the permanent hook always handles
    std::vector<std::thread> workers;
    for (int t = 0; t < kThreads; t++) {
        workers.emplace_back([&go, &unhandled] {
            while (!go.load()) std::this_thread::yield();
            ::Ctx own{};
            int bad = 0;
            for (int i = 0; i < kCallsPerThread; i++)
                if (nx_hook_entry(&own, kHotAddr) == 0) bad++;
            unhandled.fetch_add(bad);
        });
    }

    // Main thread: register and remove other hooks at other addresses while the workers call the hot address.
    go.store(true);
    std::vector<uint32_t> churn_ids;
    bool churn_ok = true;
    for (int i = 0; i < kChurn; i++) {
        uint64_t addr = 0x7100010000ull + static_cast<uint64_t>(i) * 0x100;
        uint32_t id = nxrt::hook_add_entry(addr, noop_entry);
        if (id == 0) churn_ok = false;
        churn_ids.push_back(id);
    }
    for (uint32_t id : churn_ids)
        if (!nxrt::hook_remove(id)) churn_ok = false;

    for (auto& w : workers) w.join();

    CHECK(churn_ok);
    CHECK_EQ(unhandled.load(), 0);
    CHECK_EQ(hits.load(), static_cast<uint64_t>(kThreads) * kCallsPerThread);
    CHECK_EQ(nxrt::hook_calls(perm_id), static_cast<uint64_t>(kThreads) * kCallsPerThread);
    CHECK_EQ(nxrt::hook_site_calls(HookKind::Entry), static_cast<uint64_t>(kThreads) * kCallsPerThread);
    CHECK_EQ(nxrt::hook_count(), 1u);
    CHECK_EQ(nxrt::hook_count_at(kHotAddr, HookKind::Entry), 1u);
    nxrt::hooks_clear();
}

NXTEST(hook_can_remove_itself) {
    nxrt::hooks_clear();
    ::Ctx c{};
    constexpr uint64_t kAddr = 0x7100003000ull;
    SelfRemove s;
    s.id = nxrt::hook_add_entry(kAddr, self_remove_entry, &s);
    CHECK(s.id != 0);

    CHECK(nx_hook_entry(&c, kAddr) != 0);
    CHECK_EQ(s.calls, 1);
    CHECK(s.removed);
    CHECK_EQ(nxrt::hook_count(), 0u);

    // The site has no hooks any more: the second call is not handled and the hook does not run again.
    CHECK_EQ(nx_hook_entry(&c, kAddr), 0);
    CHECK_EQ(s.calls, 1);
    nxrt::hooks_clear();
}
