// SPDX-License-Identifier: GPL-3.0-or-later
// Internal to src/kernel: state shared by thread.cpp, kernel_sync.cpp and svc_thread.cpp.
#pragma once

#include "nxrt/kernel.h"
#include "nxrt/platform.h"

namespace nxrt::kernel::detail {

void sync_reset();                       // forget every wait queue (kernel_sync.cpp)
void thread_mark_exited(KThread& t);     // marks exited under the kernel lock and wakes WaitSynchronization waiters (kernel_sync.cpp)
// The kernel lock: protects wait queues and wait state of every thread.
// On Windows this is an SRWLOCK; on POSIX it's a platform::Mutex.
struct KernelLock {
    nxrt::platform::Mutex* m = nullptr;
    KernelLock() { m = nxrt::platform::mutex_create(); }
    ~KernelLock() { if (m) nxrt::platform::mutex_destroy(m); }
    void lock() { nxrt::platform::mutex_lock(m); }
    void unlock() { nxrt::platform::mutex_unlock(m); }
};
extern KernelLock g_k;

// Initialize the sharded kernel locks (called once at startup).
void init_kernel_locks();

// Initialize the thread list lock (called once at startup).
void init_thread_list_lock();

// Blocks the calling thread on its own condition variable with the given lock held until t.wake or the deadline (ns, < 0 = forever).
// kind (for [kernel] wait_stats): 0 WaitForAddress, 1 ArbitrateLock, 2 WaitProcessWideKeyAtomic.
bool block_on_wake(KThread& t, int64_t timeout_ns, int kind, platform::Mutex* lock);
void wake_thread(KThread& t, uint32_t result = 0);   // g_k held
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
void occ_block(const KThread& t, int kind);      // g_k held, the thread is about to block (kind as block_on_wake)
void occ_unblock(const KThread& t);              // g_k held, it is running again
void occ_name(KThread& t, const char* game_name);   // nn::os::SetThreadName: the pool the thread belongs to
bool occupancy_wanted();
void install_occupancy();
void occ_tag_main(KThread& t);                    // main_thread_adopt
void occ_main_woken();                            // g_k held: the calling thread wakes main (who main waits for)
void occ_lock_contended();                        // g_k was busy: takes it, counting the wait
// The game named thread t (nn::os::SetThreadName; main: "main" when adopted): keeps the name for the wait statistics and applies
// [kernel] spin_threads / lock_spin_threads (kernel_sync.cpp). Takes g_k.
void kernel_thread_named(KThread& t, const char* game_name);
bool kernel_names_wanted();                       // wait_stats or a thread list is set: thread_diag.cpp wraps SetThreadName
}
