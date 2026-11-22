// SPDX-License-Identifier: GPL-3.0-or-later
// The POSIX side of nxrt/platform.h, for the Linux port (docs/LINUX.md, phase 2). It answers the same contract as the
// win32 file: mmap for the guest arenas, pthreads for threads, pthread mutexes and conditions for the runtime's short
// critical sections, CLOCK_MONOTONIC for the tick and plain file descriptors for the file calls.
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <signal.h>
#include <sys/file.h>
#include <sys/mman.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/syscall.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <string>
#include <unordered_map>
#include <vector>

#if defined(__linux__)
#include <ucontext.h>
#endif

#include "nxrt/platform.h"

namespace nxrt::platform {

// --- memory -----------------------------------------------------------------------------------------------------------

void* reserve(std::size_t bytes) {
    // MAP_NORESERVE keeps a 4 GiB guest arena from accounting against the host's commit charge the way VirtualAlloc's
    // MEM_RESERVE does not.
    void* p = mmap(nullptr, bytes, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_NORESERVE, -1, 0);
    return p == MAP_FAILED ? nullptr : p;
}

bool commit(void* p, std::size_t bytes) {
    return p && mprotect(p, bytes, PROT_READ | PROT_WRITE) == 0;
}

bool decommit(void* p, std::size_t bytes) {
    if (!p) return false;
    // Tell the kernel the pages are no longer needed, then take the access away again; the address range stays reserved.
    madvise(p, bytes, MADV_DONTNEED);
    return mprotect(p, bytes, PROT_NONE) == 0;
}

void release(void* p, std::size_t bytes) {
    if (p) munmap(p, bytes);
}

bool protect(void* p, std::size_t bytes, uint32_t protect) {
    int posix_prot = 0;
    if (protect & PROT_READ) posix_prot |= PROT_READ;
    if (protect & PROT_WRITE) posix_prot |= PROT_WRITE;
    if (protect & PROT_EXEC) posix_prot |= PROT_EXEC;
    return p && mprotect(p, bytes, posix_prot) == 0;
}

std::size_t page_size() {
    const long p = sysconf(_SC_PAGESIZE);
    return p > 0 ? static_cast<std::size_t>(p) : 4096;
}

// --- threads ----------------------------------------------------------------------------------------------------------

struct Thread {
    pthread_t thread{};
    const char* name = nullptr;
    ThreadFn fn = nullptr;
    void* arg = nullptr;
};

namespace {

void* thread_main(void* p) {
    auto* t = static_cast<Thread*>(p);
#if defined(__linux__)
    if (t->name) pthread_setname_np(pthread_self(), t->name);
#endif
    t->fn(t->arg);
    return nullptr;
}

}  // namespace

Thread* thread_start(const char* name, ThreadFn fn, void* arg) {
    auto* t = new Thread;
    t->name = name;
    t->fn = fn;
    t->arg = arg;
    if (pthread_create(&t->thread, nullptr, &thread_main, t) != 0) {
        delete t;
        return nullptr;
    }
    return t;
}

void thread_join(Thread* t) {
    if (!t) return;
    pthread_join(t->thread, nullptr);
    delete t;
}

void thread_detach(Thread* t) {
    if (!t) return;
    pthread_detach(t->thread);
    delete t;
}

uint32_t thread_id() {
#if defined(__linux__)
    return static_cast<uint32_t>(syscall(SYS_gettid));
#else
    return 0;
#endif
}

Thread* thread_self() {
    static thread_local Thread self;
    self.thread = pthread_self();
    self.name = nullptr;
    self.fn = nullptr;
    self.arg = nullptr;
    return &self;
}

uint64_t thread_cpu_time(Thread* t) {
    if (!t) return 0;
#if defined(__linux__)
    // Use clock_gettime with CLOCK_THREAD_CPUTIME_ID for the calling thread
    // For other threads, we'd need to use a different approach (not implemented)
    if (pthread_equal(t->thread, pthread_self())) {
        timespec ts{};
        if (clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts) == 0) {
            return static_cast<uint64_t>(ts.tv_sec) * 10000000ull + static_cast<uint64_t>(ts.tv_nsec) / 100ull;
        }
    }
    // Fallback: use getrusage for the whole process (not per-thread)
    struct rusage ru;
    if (getrusage(RUSAGE_THREAD, &ru) == 0) {
        return static_cast<uint64_t>(ru.ru_utime.tv_sec + ru.ru_stime.tv_sec) * 10000000ull +
               static_cast<uint64_t>(ru.ru_utime.tv_usec + ru.ru_stime.tv_usec) * 10ull;
    }
#endif
    return 0;
}

Thread* thread_current_handle() {
    // On POSIX, thread_self() already returns the current thread's handle
    return thread_self();
}

void* thread_native_handle(Thread* t) {
    return t ? reinterpret_cast<void*>(t->thread) : nullptr;
}

void yield_cpu() {
    sched_yield();
}

// --- synchronisation --------------------------------------------------------------------------------------------------

struct Mutex {
    pthread_mutex_t m = PTHREAD_MUTEX_INITIALIZER;
};

struct Cond {
    pthread_cond_t c = PTHREAD_COND_INITIALIZER;
};

Mutex* mutex_create() {
    auto* m = new Mutex;
    pthread_mutex_init(&m->m, nullptr);
    return m;
}

void mutex_destroy(Mutex* m) {
    if (!m) return;
    pthread_mutex_destroy(&m->m);
    delete m;
}

void mutex_lock(Mutex* m) {
    pthread_mutex_lock(&m->m);
}

void mutex_unlock(Mutex* m) {
    pthread_mutex_unlock(&m->m);
}

bool mutex_trylock(Mutex* m) {
    return pthread_mutex_trylock(&m->m) == 0;
}

Cond* cond_create() {
    auto* c = new Cond;
    pthread_cond_init(&c->c, nullptr);
    return c;
}

void cond_destroy(Cond* c) {
    if (!c) return;
    pthread_cond_destroy(&c->c);
    delete c;
}

void cond_wait(Cond* c, Mutex* m) {
    pthread_cond_wait(&c->c, &m->m);
}

void cond_signal(Cond* c) {
    pthread_cond_signal(&c->c);
}

void cond_broadcast(Cond* c) {
    pthread_cond_broadcast(&c->c);
}

// --- time -------------------------------------------------------------------------------------------------------------

uint64_t clock_hz() {
    return 1000000000ull;   // CLOCK_MONOTONIC counts nanoseconds
}

uint64_t clock_ticks() {
    timespec ts{};
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return static_cast<uint64_t>(ts.tv_sec) * 1000000000ull + static_cast<uint64_t>(ts.tv_nsec);
}

int64_t now_ns() {
    return static_cast<int64_t>(clock_ticks());
}

void sleep_ns(int64_t ns) {
    if (ns <= 0) return;
    timespec ts;
    ts.tv_sec = static_cast<time_t>(ns / 1000000000ll);
    ts.tv_nsec = static_cast<long>(ns % 1000000000ll);
    while (nanosleep(&ts, &ts) != 0 && errno == EINTR) {
    }
}

// --- files ------------------------------------------------------------------------------------------------------------

struct File {
    int fd = -1;
};

File* file_open(const char* path_utf8, bool write) {
    if (!path_utf8) return nullptr;
    const int fd = write ? ::open(path_utf8, O_WRONLY | O_CREAT | O_TRUNC, 0644) : ::open(path_utf8, O_RDONLY);
    if (fd < 0) return nullptr;
    auto* f = new File;
    f->fd = fd;
    return f;
}

void file_close(File* f) {
    if (!f) return;
    if (f->fd >= 0) ::close(f->fd);
    delete f;
}

std::size_t file_read(File* f, void* buf, std::size_t bytes) {
    if (!f || f->fd < 0) return 0;
    std::size_t done = 0;
    auto* p = static_cast<char*>(buf);
    while (done < bytes) {
        const ssize_t got = ::read(f->fd, p + done, bytes - done);
        if (got <= 0) break;
        done += static_cast<std::size_t>(got);
    }
    return done;
}

std::size_t file_write(File* f, const void* buf, std::size_t bytes) {
    if (!f || f->fd < 0) return 0;
    std::size_t done = 0;
    auto* p = static_cast<const char*>(buf);
    while (done < bytes) {
        const ssize_t wrote = ::write(f->fd, p + done, bytes - done);
        if (wrote <= 0) break;
        done += static_cast<std::size_t>(wrote);
    }
    return done;
}

bool file_seek(File* f, int64_t offset) {
    if (!f || f->fd < 0) return false;
    return ::lseek(f->fd, static_cast<off_t>(offset), SEEK_SET) >= 0;
}

int64_t file_size(File* f) {
    if (!f || f->fd < 0) return -1;
    struct stat st{};
    if (fstat(f->fd, &st) != 0) return -1;
    return static_cast<int64_t>(st.st_size);
}

bool file_exists(const char* path_utf8) {
    if (!path_utf8) return false;
    struct stat st{};
    return stat(path_utf8, &st) == 0 && S_ISREG(st.st_mode);
}

bool file_mkdirs(const char* path_utf8) {
    if (!path_utf8) return false;
    std::string part;
    for (std::size_t i = 0; path_utf8[i]; ++i) {
        part.push_back(path_utf8[i]);
        if (path_utf8[i] == '/' || !path_utf8[i + 1]) {
            if (part != "/") mkdir(part.c_str(), 0755);
        }
    }
    struct stat st{};
    return stat(path_utf8, &st) == 0 && S_ISDIR(st.st_mode);
}

int physical_core_count() {
#if defined(__linux__)
    // Parse /proc/cpuinfo to count unique physical cores (by core id + physical id)
    FILE* f = fopen("/proc/cpuinfo", "r");
    if (!f) return 0;
    std::unordered_map<std::string, bool> seen;
    char line[256];
    int physical_id = -1, core_id = -1;
    int count = 0;
    while (fgets(line, sizeof(line), f)) {
        if (strncmp(line, "physical id", 11) == 0) {
            physical_id = atoi(strchr(line, ':') + 1);
        } else if (strncmp(line, "core id", 7) == 0) {
            core_id = atoi(strchr(line, ':') + 1);
        } else if (line[0] == '\n') {
            if (physical_id >= 0 && core_id >= 0) {
                std::string key = std::to_string(physical_id) + ":" + std::to_string(core_id);
                if (!seen[key]) {
                    seen[key] = true;
                    ++count;
                }
            }
            physical_id = -1;
            core_id = -1;
        }
    }
    // Handle last entry without trailing newline
    if (physical_id >= 0 && core_id >= 0) {
        std::string key = std::to_string(physical_id) + ":" + std::to_string(core_id);
        if (!seen[key]) ++count;
    }
    fclose(f);
    return count > 0 ? count : 1;
#else
    // Fallback: use sysconf for online processors (may count SMT threads)
    long n = sysconf(_SC_NPROCESSORS_ONLN);
    return n > 0 ? static_cast<int>(n) : 1;
#endif
}

bool thread_set_affinity(Thread* t, uint64_t mask) {
#if defined(__linux__)
    cpu_set_t cpuset;
    CPU_ZERO(&cpuset);
    for (int i = 0; i < 64; i++) {
        if (mask & (1ull << i)) CPU_SET(i, &cpuset);
    }
    return pthread_setaffinity_np(t->thread, sizeof(cpu_set_t), &cpuset) == 0;
#else
    (void)t; (void)mask;
    return false;
#endif
}

bool thread_set_ideal_processor(Thread* t, int processor) {
    // On POSIX, ideal processor is a hint; we set affinity to just that processor
    // as a best-effort approximation.
    return thread_set_affinity(t, 1ull << processor);
}

bool thread_set_priority(Thread* t, ThreadPriority prio) {
    // Linux carries a nice value per thread, so the priority can be set on the thread the caller named instead of the
    // whole process (setpriority(PRIO_PROCESS, 0, ...) would move every thread, the launcher included). SCHED_OTHER only:
    // raising above nice 0 needs CAP_SYS_NICE and is not required for the console core model.
    if (!t) return false;
    int nice_val = 0;
    switch (prio) {
        case ThreadPriority::Lowest: nice_val = 19; break;
        case ThreadPriority::BelowNormal: nice_val = 10; break;
        case ThreadPriority::Normal: nice_val = 0; break;
        case ThreadPriority::AboveNormal: nice_val = -5; break;
        case ThreadPriority::Highest: nice_val = -10; break;
        case ThreadPriority::TimeCritical: nice_val = -20; break;
    }
#if defined(__linux__) && defined(__GLIBC__) && __GLIBC_PREREQ(2, 30)
    return setpriority(PRIO_PROCESS, static_cast<id_t>(pthread_gettid_np(t->thread)), nice_val) == 0;
#else
    (void)t;
    return setpriority(PRIO_PROCESS, 0, nice_val) == 0;
#endif
}

void thread_disable_boost(Thread* t) {
    // No-op on POSIX; Windows has a dynamic priority boost feature
    (void)t;
}

int thread_current_processor() {
#if defined(__linux__)
    // sched_getcpu() returns the CPU number; syscall(SYS_getcpu, ...) returns 0 on success and would report CPU 0 for
    // every thread.
    const int cpu = sched_getcpu();
    return cpu >= 0 ? cpu : -1;
#else
    return 0;
#endif
}

bool process_get_affinity_mask(uint64_t* proc_mask, uint64_t* sys_mask) {
#if defined(__linux__)
    cpu_set_t cpuset;
    if (sched_getaffinity(0, sizeof(cpu_set_t), &cpuset) != 0) return false;
    uint64_t mask = 0;
    for (int i = 0; i < 64; i++) {
        if (CPU_ISSET(i, &cpuset)) mask |= (1ull << i);
    }
    *proc_mask = mask;
    *sys_mask = mask;  // On Linux, system mask is typically the same as process mask
    return true;
#else
    *proc_mask = 1;
    *sys_mask = 1;
    return true;
#endif
}

bool process_set_priority_class(int priority_class) {
    // Map Windows priority classes to nice values
    int nice_val = 0;
    if (priority_class == 0x00000080) nice_val = -10;      // HIGH_PRIORITY_CLASS
    else if (priority_class == 0x00008000) nice_val = -5;  // ABOVE_NORMAL_PRIORITY_CLASS
    else if (priority_class == 0x00000020) nice_val = 0;   // NORMAL_PRIORITY_CLASS
    return setpriority(PRIO_PROCESS, 0, nice_val) == 0;
}

namespace {

// The named mutex is an advisory lock on a file rather than a named semaphore: closing a semaphore leaves its name in
// /dev/shm, while an flock is released when the handle is closed or the process dies, which is what a "is another copy
// running" check needs. The name is reduced to a file name so a caller cannot escape the directory.
std::string named_mutex_path(const char* name) {
    std::string leaf = "nxrt-";
    for (const char* p = name; p && *p; ++p) {
        const char c = *p;
        leaf.push_back((c == '/' || c == '\\' || c == ':') ? '_' : c);
    }
    const char* runtime = std::getenv("XDG_RUNTIME_DIR");
    return std::string(runtime && *runtime ? runtime : "/tmp") + "/" + leaf + ".lock";
}

struct NamedMutex {
    int fd = -1;
};

}  // namespace

void* named_mutex_create(const char* name, bool* already_exists) {
#if defined(__linux__)
    if (!name) return nullptr;
    const std::string path = named_mutex_path(name);
    const int fd = ::open(path.c_str(), O_RDWR | O_CREAT | O_CLOEXEC, 0600);
    if (fd < 0) return nullptr;
    bool existed = false;
    if (flock(fd, LOCK_EX | LOCK_NB) != 0) {
        if (errno != EWOULDBLOCK) {
            ::close(fd);
            return nullptr;
        }
        existed = true;
        // Another live holder: wait for it, as CreateMutexA does when the name already exists.
        while (flock(fd, LOCK_EX) != 0) {
            if (errno != EINTR) {
                ::close(fd);
                return nullptr;
            }
        }
    }
    if (already_exists) *already_exists = existed;
    auto* m = new NamedMutex;
    m->fd = fd;
    return m;
#else
    (void)name; (void)already_exists;
    return nullptr;
#endif
}

void named_mutex_destroy(void* handle) {
#if defined(__linux__)
    auto* m = static_cast<NamedMutex*>(handle);
    if (!m) return;
    if (m->fd >= 0) {
        flock(m->fd, LOCK_UN);
        ::close(m->fd);
    }
    delete m;
#else
    (void)handle;
#endif
}

struct Section {
    int fd = -1;
    uint64_t size = 0;
};

Section* section_create(uint64_t size) {
#if defined(__linux__)
    // Use memfd_create for anonymous shared memory
    int fd = memfd_create("nxrt_heap", MFD_CLOEXEC | MFD_ALLOW_SEALING);
    if (fd < 0) return nullptr;
    if (ftruncate(fd, static_cast<off_t>(size)) != 0) {
        close(fd);
        return nullptr;
    }
    // Seal the file to prevent further resizing
    fcntl(fd, F_ADD_SEALS, F_SEAL_SHRINK | F_SEAL_GROW);
    auto* s = new Section;
    s->fd = fd;
    s->size = size;
    return s;
#else
    (void)size;
    return nullptr;
#endif
}

uint64_t section_map_view(Section* s, uint64_t addr, uint64_t offset, uint64_t size, bool write) {
#if defined(__linux__)
    int prot = write ? (PROT_READ | PROT_WRITE) : PROT_READ;
    int flags = MAP_SHARED;
    // NOREPLACE, not MAP_FIXED: MapViewOfFileEx fails when the requested range is taken, and silently replacing an
    // existing guest mapping to satisfy a heap view would corrupt it.
    if (addr != 0) flags |= MAP_FIXED_NOREPLACE;
    void* p = mmap(reinterpret_cast<void*>(addr), static_cast<size_t>(size), prot, flags, s->fd, static_cast<off_t>(offset));
    if (p == MAP_FAILED) return 0;
    return reinterpret_cast<uint64_t>(p);
#else
    (void)s; (void)addr; (void)offset; (void)size; (void)write;
    return 0;
#endif
}

bool section_unmap_view(uint64_t addr, uint64_t size) {
#if defined(__linux__)
    return munmap(reinterpret_cast<void*>(addr), static_cast<size_t>(size)) == 0;
#else
    (void)addr; (void)size;
    return false;
#endif
}

void section_destroy(Section* s) {
    if (!s) return;
#if defined(__linux__)
    if (s->fd >= 0) close(s->fd);
#endif
    delete s;
}

bool section_protect(uint64_t addr, uint64_t size, bool writable) {
#if defined(__linux__)
    int prot = writable ? (PROT_READ | PROT_WRITE) : PROT_READ;
    return mprotect(reinterpret_cast<void*>(addr), static_cast<size_t>(size), prot) == 0;
#else
    (void)addr; (void)size; (void)writable;
    return false;
#endif
}

void* reserve_fixed(void* addr, std::size_t bytes) {
#if defined(__linux__)
    void* p = mmap(addr, bytes, PROT_NONE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED_NOREPLACE, -1, 0);
    return p == addr ? p : nullptr;
#else
    (void)addr; (void)bytes;
    return nullptr;
#endif
}

bool release_fixed(void* addr, std::size_t bytes) {
#if defined(__linux__)
    return munmap(addr, bytes) == 0;
#else
    (void)addr; (void)bytes;
    return false;
#endif
}

// --- write-watch (page fault tracking) ------------------------------------------------------

struct RWLock {
    pthread_rwlock_t lock{};
};

RWLock* rwlock_create() {
    auto* lk = new RWLock;
    pthread_rwlock_init(&lk->lock, nullptr);
    return lk;
}

void rwlock_destroy(RWLock* lk) {
    if (!lk) return;
    pthread_rwlock_destroy(&lk->lock);
    delete lk;
}

void rwlock_lock_shared(RWLock* lk) {
    pthread_rwlock_rdlock(&lk->lock);
}

void rwlock_unlock_shared(RWLock* lk) {
    pthread_rwlock_unlock(&lk->lock);
}

void rwlock_lock_exclusive(RWLock* lk) {
    pthread_rwlock_wrlock(&lk->lock);
}

void rwlock_unlock_exclusive(RWLock* lk) {
    pthread_rwlock_unlock(&lk->lock);
}

namespace {

// Parse /proc/self/maps to find page protection for an address
uint32_t parse_maps_protect(uint64_t addr) {
#if defined(__linux__)
    std::ifstream maps("/proc/self/maps");
    if (!maps) return 0;
    std::string line;
    while (std::getline(maps, line)) {
        uint64_t start = 0, end = 0;
        char perms[5] = {0};
        char path[256] = {0};
        // Format: start-end perms offset dev inode path
        if (sscanf(line.c_str(), "%llx-%llx %4s %*s %*s %*s %255s",
                   reinterpret_cast<unsigned long long*>(&start),
                   reinterpret_cast<unsigned long long*>(&end), perms, path) < 3) {
            continue;
        }
        if (addr >= start && addr < end) {
            uint32_t prot = 0;
            if (perms[0] == 'r') prot |= PROT_READ;
            if (perms[1] == 'w') prot |= PROT_WRITE;
            if (perms[2] == 'x') prot |= PROT_EXEC;
            if (perms[3] == 'p') prot |= 0x1000;  // private (COW)
            else if (perms[3] == 's') prot |= 0x2000;  // shared
            return prot;
        }
    }
#endif
    return 0;
}

bool parse_maps_query(uint64_t addr, PageState* state, PageProtect* protect, uint64_t* region_base, uint64_t* region_size) {
#if defined(__linux__)
    std::ifstream maps("/proc/self/maps");
    if (!maps) return false;
    std::string line;
    while (std::getline(maps, line)) {
        uint64_t start = 0, end = 0;
        char perms[5] = {0};
        char path[256] = {0};
        if (sscanf(line.c_str(), "%llx-%llx %4s %*s %*s %*s %255s",
                   reinterpret_cast<unsigned long long*>(&start),
                   reinterpret_cast<unsigned long long*>(&end), perms, path) < 3) {
            continue;
        }
        if (addr >= start && addr < end) {
            if (state) *state = PageState::Commit;
            if (protect) {
                uint32_t p = 0;
                if (perms[0] == 'r') p |= static_cast<uint32_t>(PageProtect::ReadOnly);
                if (perms[1] == 'w') p |= static_cast<uint32_t>(PageProtect::ReadWrite);
                if (perms[2] == 'x') p |= static_cast<uint32_t>(PageProtect::Execute);
                if (perms[3] == 's') p |= static_cast<uint32_t>(PageProtect::WriteCopy);
                if (p == 0) p = static_cast<uint32_t>(PageProtect::NoAccess);
                *protect = static_cast<PageProtect>(p);
            }
            if (region_base) *region_base = start;
            if (region_size) *region_size = end - start;
            return true;
        }
    }
#endif
    return false;
}

PageFaultHandler g_page_fault_handler = nullptr;
struct sigaction g_old_sigsegv{};
bool g_sigsegv_installed = false;

void sigsegv_handler(int sig, siginfo_t* info, void* uc) {
    (void)sig;
    if (!info) return;
    const uint64_t addr = reinterpret_cast<uint64_t>(info->si_addr);
    // The runtime's write-watch needs read and write faults told apart. si_code only says whether the address was
    // mapped or protected; on x86-64 the hardware error code is in the ucontext and bit 1 is set for a write.
    int kind = 1;
#if defined(__linux__) && defined(__x86_64__) && defined(REG_ERR)
    if (auto* u = static_cast<ucontext_t*>(uc)) kind = (u->uc_mcontext.gregs[REG_ERR] & 0x2) ? 1 : 0;
#else
    (void)uc;
#endif
    if (g_page_fault_handler && g_page_fault_handler(addr, kind)) return;   // the handler made the page accessible
    // Nothing made the page accessible. Returning would re-execute the faulting instruction forever, so restore the
    // default action and take the fault again: the process dies with a real SIGSEGV and a crash report.
    struct sigaction dfl{};
    dfl.sa_handler = SIG_DFL;
    sigemptyset(&dfl.sa_mask);
    sigaction(SIGSEGV, &dfl, nullptr);
    raise(SIGSEGV);
}

}  // namespace

bool page_fault_handler_install(PageFaultHandler fn) {
    if (g_sigsegv_installed) return false;
    g_page_fault_handler = fn;
    struct sigaction sa{};
    sa.sa_sigaction = sigsegv_handler;
    sa.sa_flags = SA_SIGINFO | SA_ONSTACK;
    sigemptyset(&sa.sa_mask);
    if (sigaction(SIGSEGV, &sa, &g_old_sigsegv) != 0) {
        g_page_fault_handler = nullptr;
        return false;
    }
    g_sigsegv_installed = true;
    return true;
}

void page_fault_handler_remove() {
    if (g_sigsegv_installed) {
        sigaction(SIGSEGV, &g_old_sigsegv, nullptr);
        g_sigsegv_installed = false;
        g_page_fault_handler = nullptr;
    }
}

uint32_t page_protect(uint64_t addr) {
    return parse_maps_protect(addr);
}

bool page_query(uint64_t addr, PageState* state, PageProtect* protect, uint64_t* region_base, uint64_t* region_size) {
    return parse_maps_query(addr, state, protect, region_base, region_size);
}

uint64_t tick_count_ms() {
    timespec ts{};
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return static_cast<uint64_t>(ts.tv_sec) * 1000ull + static_cast<uint64_t>(ts.tv_nsec) / 1000000ull;
}

[[noreturn]] void process_exit(int code) {
    _exit(code);
}

}  // namespace nxrt::platform
