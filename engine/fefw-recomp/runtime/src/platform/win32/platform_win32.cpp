// SPDX-License-Identifier: GPL-3.0-or-later
// The Windows side of nxrt/platform.h. Every call here is the one the runtime made directly before the platform split,
// so the win32 build keeps its exact behaviour: QPC for the clock, a high-resolution waitable timer for long sleeps,
// VirtualAlloc for the guest arenas, SRW locks for the runtime's short critical sections and CreateFileW for files.
#include <windows.h>

#include <process.h>

#include <string>

#include "nxrt/platform.h"

namespace nxrt::platform {
namespace {

std::wstring wide(const char* utf8) {
    if (!utf8) return {};
    const int n = MultiByteToWideChar(CP_UTF8, 0, utf8, -1, nullptr, 0);
    if (n <= 0) return {};
    std::wstring w(static_cast<std::size_t>(n), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, utf8, -1, w.data(), n);
    w.resize(static_cast<std::size_t>(n - 1));   // the terminator MultiByteToWideChar wrote
    return w;
}

}  // namespace

// --- memory -----------------------------------------------------------------------------------------------------------

void* reserve(std::size_t bytes) {
    return VirtualAlloc(nullptr, bytes, MEM_RESERVE, PAGE_NOACCESS);
}

bool commit(void* p, std::size_t bytes) {
    return p && VirtualAlloc(p, bytes, MEM_COMMIT, PAGE_READWRITE) != nullptr;
}

bool decommit(void* p, std::size_t bytes) {
    return p && VirtualFree(p, bytes, MEM_DECOMMIT) != 0;
}

void release(void* p, std::size_t) {
    if (p) VirtualFree(p, 0, MEM_RELEASE);
}

bool protect(void* p, std::size_t bytes, bool writable) {
    DWORD old = 0;
    return p && VirtualProtect(p, bytes, writable ? PAGE_READWRITE : PAGE_READONLY, &old) != 0;
}

std::size_t page_size() {
    SYSTEM_INFO si{};
    GetSystemInfo(&si);
    return si.dwPageSize ? si.dwPageSize : 4096;
}

// --- threads ----------------------------------------------------------------------------------------------------------

struct Thread {
    HANDLE handle = nullptr;
    unsigned id = 0;
    const char* name = nullptr;
    ThreadFn fn = nullptr;
    void* arg = nullptr;
};

namespace {

unsigned __stdcall thread_main(void* p) {
    auto* t = static_cast<Thread*>(p);
    t->fn(t->arg);
    return 0;
}

}  // namespace

Thread* thread_start(const char* name, ThreadFn fn, void* arg) {
    auto* t = new Thread;
    t->name = name;
    t->fn = fn;
    t->arg = arg;
    // _beginthreadex, not CreateThread: the CRT's per-thread state (errno, locale, floating point flags) is set up for
    // the new thread, and the runtime's threads allocate.
    const uintptr_t h = _beginthreadex(nullptr, 0, &thread_main, t, 0, &t->id);
    if (!h) {
        delete t;
        return nullptr;
    }
    t->handle = reinterpret_cast<HANDLE>(h);
    return t;
}

void thread_join(Thread* t) {
    if (!t) return;
    if (t->handle) WaitForSingleObject(t->handle, INFINITE);
    if (t->handle) CloseHandle(t->handle);
    delete t;
}

void thread_detach(Thread* t) {
    if (!t) return;
    if (t->handle) CloseHandle(t->handle);
    delete t;
}

uint32_t thread_id() {
    return static_cast<uint32_t>(GetCurrentThreadId());
}

void yield_cpu() {
    SwitchToThread();
}

// --- synchronisation --------------------------------------------------------------------------------------------------

struct Mutex {
    SRWLOCK lock{};
};

struct Cond {
    CONDITION_VARIABLE cv{};
};

Mutex* mutex_create() {
    auto* m = new Mutex;
    InitializeSRWLock(&m->lock);
    return m;
}

void mutex_destroy(Mutex* m) {
    delete m;
}

void mutex_lock(Mutex* m) {
    AcquireSRWLockExclusive(&m->lock);
}

void mutex_unlock(Mutex* m) {
    ReleaseSRWLockExclusive(&m->lock);
}

Cond* cond_create() {
    auto* c = new Cond;
    InitializeConditionVariable(&c->cv);
    return c;
}

void cond_destroy(Cond* c) {
    delete c;
}

void cond_wait(Cond* c, Mutex* m) {
    SleepConditionVariableSRW(&c->cv, &m->lock, INFINITE, 0);
}

void cond_signal(Cond* c) {
    WakeConditionVariable(&c->cv);
}

void cond_broadcast(Cond* c) {
    WakeAllConditionVariable(&c->cv);
}

// --- time -------------------------------------------------------------------------------------------------------------

uint64_t clock_hz() {
    static const uint64_t hz = [] {
        LARGE_INTEGER f;
        QueryPerformanceFrequency(&f);
        return static_cast<uint64_t>(f.QuadPart);
    }();
    return hz;
}

uint64_t clock_ticks() {
    LARGE_INTEGER c;
    QueryPerformanceCounter(&c);
    return static_cast<uint64_t>(c.QuadPart);
}

int64_t now_ns() {
    const uint64_t f = clock_hz();
    const uint64_t c = clock_ticks();
    return static_cast<int64_t>(c / f * 1000000000ull + c % f * 1000000000ull / f);
}

// The console timer is good to a few microseconds; Sleep() counts whole milliseconds on a 1 ms or 15.6 ms grid, so the
// wait runs on a high-resolution waitable timer (100 ns units) and Sleep() is only the fallback.
void sleep_ns(int64_t ns) {
    if (ns <= 0) return;
    static thread_local HANDLE timer = nullptr;
    static thread_local bool no_timer = false;
    if (!timer && !no_timer) {
        timer = CreateWaitableTimerExW(nullptr, nullptr, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS);
        if (!timer) no_timer = true;
    }
    LARGE_INTEGER due;
    due.QuadPart = -((ns + 99) / 100);
    if (!timer || !SetWaitableTimer(timer, &due, 0, nullptr, nullptr, FALSE))
        Sleep(static_cast<DWORD>((ns + 999999) / 1000000));
    else
        WaitForSingleObject(timer, INFINITE);
}

// --- files ------------------------------------------------------------------------------------------------------------

struct File {
    HANDLE handle = INVALID_HANDLE_VALUE;
};

File* file_open(const char* path_utf8, bool write) {
    const std::wstring w = wide(path_utf8);
    if (w.empty()) return nullptr;
    HANDLE h = CreateFileW(w.c_str(), write ? GENERIC_WRITE : GENERIC_READ, FILE_SHARE_READ, nullptr,
                           write ? CREATE_ALWAYS : OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) return nullptr;
    auto* f = new File;
    f->handle = h;
    return f;
}

void file_close(File* f) {
    if (!f) return;
    if (f->handle != INVALID_HANDLE_VALUE) CloseHandle(f->handle);
    delete f;
}

std::size_t file_read(File* f, void* buf, std::size_t bytes) {
    if (!f || f->handle == INVALID_HANDLE_VALUE) return 0;
    std::size_t done = 0;
    auto* p = static_cast<char*>(buf);
    while (done < bytes) {
        const DWORD chunk = static_cast<DWORD>(bytes - done > 0x40000000 ? 0x40000000 : bytes - done);
        DWORD got = 0;
        if (!ReadFile(f->handle, p + done, chunk, &got, nullptr) || got == 0) break;
        done += got;
    }
    return done;
}

std::size_t file_write(File* f, const void* buf, std::size_t bytes) {
    if (!f || f->handle == INVALID_HANDLE_VALUE) return 0;
    std::size_t done = 0;
    auto* p = static_cast<const char*>(buf);
    while (done < bytes) {
        const DWORD chunk = static_cast<DWORD>(bytes - done > 0x40000000 ? 0x40000000 : bytes - done);
        DWORD wrote = 0;
        if (!WriteFile(f->handle, p + done, chunk, &wrote, nullptr) || wrote == 0) break;
        done += wrote;
    }
    return done;
}

bool file_seek(File* f, int64_t offset) {
    if (!f || f->handle == INVALID_HANDLE_VALUE) return false;
    LARGE_INTEGER to;
    to.QuadPart = offset;
    return SetFilePointerEx(f->handle, to, nullptr, FILE_BEGIN) != 0;
}

int64_t file_size(File* f) {
    if (!f || f->handle == INVALID_HANDLE_VALUE) return -1;
    LARGE_INTEGER n;
    if (!GetFileSizeEx(f->handle, &n)) return -1;
    return static_cast<int64_t>(n.QuadPart);
}

bool file_exists(const char* path_utf8) {
    const std::wstring w = wide(path_utf8);
    if (w.empty()) return false;
    const DWORD a = GetFileAttributesW(w.c_str());
    return a != INVALID_FILE_ATTRIBUTES && !(a & FILE_ATTRIBUTE_DIRECTORY);
}

bool file_mkdirs(const char* path_utf8) {
    const std::wstring w = wide(path_utf8);
    if (w.empty()) return false;
    // CreateDirectoryW fails on a missing parent, so build the path one component at a time. A component that already
    // exists is not an error.
    std::wstring part;
    for (std::size_t i = 0; i < w.size(); ++i) {
        part.push_back(w[i]);
        const bool last = i + 1 == w.size();
        if (w[i] == L'/' || w[i] == L'\\' || last) {
            if (part.size() > 3 || (part.size() > 1 && part[1] != L':')) {
                std::wstring dir = part;
                while (!dir.empty() && (dir.back() == L'/' || dir.back() == L'\\')) dir.pop_back();
                if (!dir.empty()) CreateDirectoryW(dir.c_str(), nullptr);
            }
        }
    }
    const DWORD a = GetFileAttributesW(w.c_str());
    return a != INVALID_FILE_ATTRIBUTES && (a & FILE_ATTRIBUTE_DIRECTORY);
}

}  // namespace nxrt::platform
