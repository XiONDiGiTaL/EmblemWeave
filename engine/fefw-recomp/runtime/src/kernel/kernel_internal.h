// SPDX-License-Identifier: GPL-3.0-or-later
// Internal to src/kernel: state shared by thread.cpp, kernel_sync.cpp and svc_thread.cpp.
#pragma once

#include "nxrt/kernel.h"
#include "nxrt/platform.h"

namespace nxrt::kernel::detail {

void sync_reset();                       // forget every wait queue (kernel_sync.cpp)
void thread_mark_exited(KThread& t);     // marks exited under the kernel lock and wakes WaitSynchronization waiters (kernel_sync.cpp)

// Sharded kernel locks: one per wait queue type to reduce contention.
// Lock order (must be consistent to avoid deadlocks): sync → mutex → cv → addr
struct KernelLocks {
    nxrt::platform::Mutex* sync = nullptr;   // g_sync_waiters, g_woken_at, g_stats
    nxrt::platform::Mutex* mutex = nullptr;  // g_mutex_waiters, g_broadcast_mutexes
    nxrt::platform::Mutex* cv = nullptr;     // g_cv_waiters
    nxrt::platform::Mutex* addr = nullptr;   // g_addr_waiters

    KernelLocks() {
        sync = nxrt::platform::mutex_create();
        mutex = nxrt::platform::mutex_create();
        cv = nxrt::platform::mutex_create();
        addr = nxrt::platform::mutex_create();
    }
    ~KernelLocks() {
        if (sync) nxrt::platform::mutex_destroy(sync);
        if (mutex) nxrt::platform::mutex_destroy(mutex);
        if (cv) nxrt::platform::mutex_destroy(cv);
        if (addr) nxrt::platform::mutex_destroy(addr);
    }
};
extern KernelLocks g_klocks;

// Initialize the kernel locks (called once at startup).
void init_kernel_locks();

// Lock guard for sync queue operations
struct GuardSync {
    GuardSync() { nxrt::platform::mutex_lock(g_klocks.sync); }
    ~GuardSync() { nxrt::platform::mutex_unlock(g_klocks.sync); }
};

// Lock guard for mutex queue operations
struct GuardMutex {
    GuardMutex() { nxrt::platform::mutex_lock(g_klocks.mutex); }
    ~GuardMutex() { nxrt::platform::mutex_unlock(g_klocks.mutex); }
};

// Lock guard for cv queue operations
struct GuardCV {
    GuardCV() { nxrt::platform::mutex_lock(g_klocks.cv); }
    ~GuardCV() { nxrt::platform::mutex_unlock(g_klocks.cv); }
};

// Lock guard for addr queue operations
struct GuardAddr {
    GuardAddr() { nxrt::platform::mutex_lock(g_klocks.addr); }
    ~GuardAddr() { nxrt::platform::mutex_unlock(g_klocks.addr); }
};

// Lock guard for operations needing sync+mutex (e.g., sync_wait_report)
struct GuardSyncMutex {
    GuardSyncMutex() {
        nxrt::platform::mutex_lock(g_klocks.sync);
        nxrt::platform::mutex_lock(g_klocks.mutex);
    }
    ~GuardSyncMutex() {
        nxrt::platform::mutex_unlock(g_klocks.mutex);
        nxrt::platform::mutex_unlock(g_klocks.sync);
    }
};

// Lock guard for operations needing cv+mutex (e.g., signal_process_wide_key)
// Lock order: mutex → cv (consistent with Guard below; the old cv → mutex order could
// deadlock against a thread in Guard that already held the mutex).
struct GuardCVMutex {
    GuardCVMutex() {
        nxrt::platform::mutex_lock(g_klocks.mutex);
        nxrt::platform::mutex_lock(g_klocks.cv);
    }
    ~GuardCVMutex() {
        nxrt::platform::mutex_unlock(g_klocks.cv);
        nxrt::platform::mutex_unlock(g_klocks.mutex);
    }
};

// Lock guard for operations needing sync+cv+mutex (e.g., wait_process_wide_key cleanup)
// Lock order: sync → mutex → cv.
struct GuardSyncCVMutex {
    GuardSyncCVMutex() {
        nxrt::platform::mutex_lock(g_klocks.sync);
        nxrt::platform::mutex_lock(g_klocks.mutex);
        nxrt::platform::mutex_lock(g_klocks.cv);
    }
    ~GuardSyncCVMutex() {
        nxrt::platform::mutex_unlock(g_klocks.cv);
        nxrt::platform::mutex_unlock(g_klocks.mutex);
        nxrt::platform::mutex_unlock(g_klocks.sync);
    }
};

// Legacy Guard for compatibility (stats, sync_reset, etc.) - takes all locks in order
struct Guard {
    Guard() {
        nxrt::platform::mutex_lock(g_klocks.sync);
        nxrt::platform::mutex_lock(g_klocks.mutex);
        nxrt::platform::mutex_lock(g_klocks.cv);
        nxrt::platform::mutex_lock(g_klocks.addr);
    }
    ~Guard() {
        nxrt::platform::mutex_unlock(g_klocks.addr);
        nxrt::platform::mutex_unlock(g_klocks.cv);
        nxrt::platform::mutex_unlock(g_klocks.mutex);
        nxrt::platform::mutex_unlock(g_klocks.sync);
    }
};

// Initialize the thread list lock (called once at startup).
void init_thread_list_lock();

// Blocks the calling thread on its own condition variable with the given lock held until t.wake or the deadline (ns, < 0 = forever).
// kind (for [kernel] wait_stats): 0 WaitForAddress, 1 ArbitrateLock, 2 WaitProcessWideKeyAtomic.
bool block_on_wake(KThread& t, int64_t timeout_ns, int kind, platform::Mutex* lock);
void wake_thread(KThread& t, uint32_t result = 0);   // appropriate kernel lock held (sync for WaitSynchronization, mutex for ArbitrateLock, etc.)
// The set_blocked_hook function when the calling thread is the one it names, else null (kernel_sync.cpp).
void (*blocked_hook_here())();

// Thread condition variable accessor. Creates the cv lazily for host stand-in threads.
inline nxrt::platform::Cond* thread_cv(KThread& t) {
    if (!t.cv) t.cv = nxrt::platform::cond_create();
    return t.cv;
}

}  // namespace nxrt::kernel::detail

namespace nxrt::kernel {
void install_thread_diag();                       // [kernel] thread_log: nn::os thread creation / core queries logged (thread_diag.cpp)
void thread_diag_note_created(uint32_t handle);   // svcCreateThread: the handle it made, for the nn::os::CreateThread log line
void install_host_placement();                    // [sched] render_place / place_log (host_place.cpp)
// [kernel] occupancy (occupancy.cpp): which of the game's pools run while main is blocked. Hooks are no-ops unless it is on.
extern bool g_occ_on;
void occ_block(const KThread& t, int kind);      // appropriate kernel lock held (sync for WaitSynchronization, mutex for ArbitrateLock, etc.)
void occ_unblock(const KThread& t);              // appropriate kernel lock held
void occ_name(KThread& t, const char* game_name);   // nn::os::SetThreadName: the pool the thread belongs to
bool occupancy_wanted();
void install_occupancy();
void occ_tag_main(KThread& t);                    // main_thread_adopt
void occ_main_woken();                            // sync lock held: the calling thread wakes main (who main waits for)
void occ_lock_contended();                        // sync lock was busy: takes it, counting the wait
// The game named thread t (nn::os::SetThreadName; main: "main" when adopted): keeps the name for the wait statistics and applies
// [kernel] spin_threads / lock_spin_threads (kernel_sync.cpp). Takes appropriate kernel lock.
void kernel_thread_named(KThread& t, const char* game_name);
bool kernel_names_wanted();                       // wait_stats or a thread list is set: thread_diag.cpp wraps SetThreadName
}
