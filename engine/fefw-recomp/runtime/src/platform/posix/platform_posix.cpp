// SPDX-License-Identifier: GPL-3.0-or-later
// The POSIX side of nxrt/platform.h, for the Linux port (docs/LINUX.md, phase 2). It answers the same contract as the
// win32 file: mmap for the guest arenas, pthreads for threads, pthread mutexes and conditions for the runtime's short
// critical sections, CLOCK_MONOTONIC for the tick and plain file descriptors for the file calls.
#include <fcntl.h>
#include <pthread.h>
#include <sched.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <time.h>
#include <unistd.h>

#include <cerrno>
#include <cstdlib>
#include <cstring>
#include <string>

#include "nxrt/platform.h"

#if defined(__linux__)
#include <sys/syscall.h>
#endif

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

bool protect(void* p, std::size_t bytes, bool writable) {
    return p && mprotect(p, bytes, writable ? PROT_READ | PROT_WRITE : PROT_READ) == 0;
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

}  // namespace nxrt::platform
