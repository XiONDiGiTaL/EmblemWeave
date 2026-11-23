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

bool protect(void* p, std::size_t bytes, uint32_t protect) {
    DWORD win_protect = PAGE_NOACCESS;
    if (protect & PROT_READ) {
        win_protect = (protect & PROT_WRITE) ? PAGE_READWRITE : PAGE_READONLY;
    } else if (protect & PROT_WRITE) {
        win_protect = PAGE_READWRITE;
    }
    if (protect & PROT_EXEC) {
        if (win_protect == PAGE_NOACCESS) win_protect = PAGE_EXECUTE;
        else if (win_protect == PAGE_READONLY) win_protect = PAGE_EXECUTE_READ;
        else if (win_protect == PAGE_READWRITE) win_protect = PAGE_EXECUTE_READWRITE;
    }
    DWORD old = 0;
    return p && VirtualProtect(p, bytes, win_protect, &old) != 0;
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

Thread* thread_self() {
    // Create a temporary thread handle for the current thread
    static thread_local Thread self;
    self.handle = GetCurrentThread();
    self.id = GetCurrentThreadId();
    self.name = nullptr;
    self.fn = nullptr;
    self.arg = nullptr;
    return &self;
}

uint64_t thread_cpu_time(Thread* t) {
    if (!t || !t->handle) return 0;
    FILETIME created{}, exited{}, kernel_time{}, user_time{};
    if (!GetThreadTimes(t->handle, &created, &exited, &kernel_time, &user_time)) return 0;
    auto to_u64 = [](const FILETIME& f) { return (static_cast<uint64_t>(f.dwHighDateTime) << 32) | f.dwLowDateTime; };
    return to_u64(kernel_time) + to_u64(user_time);
}

Thread* thread_current_handle() {
    // GetCurrentThread returns a pseudo-handle that can be used with GetThreadTimes
    static thread_local Thread self;
    self.handle = GetCurrentThread();
    self.id = GetCurrentThreadId();
    self.name = nullptr;
    self.fn = nullptr;
    self.arg = nullptr;
    return &self;
}

void* thread_native_handle(Thread* t) {
    return t ? t->handle : nullptr;
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

bool mutex_trylock(Mutex* m) {
    return TryAcquireSRWLockExclusive(&m->lock) != 0;
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

int physical_core_count() {
    DWORD len = 0;
    GetLogicalProcessorInformationEx(RelationProcessorCore, nullptr, &len);
    if (!len) return 0;
    std::vector<uint8_t> buf(len);
    if (!GetLogicalProcessorInformationEx(RelationProcessorCore, reinterpret_cast<PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>(buf.data()), &len)) return 0;
    int n = 0;
    for (DWORD off = 0; off < len;) {
        auto* e = reinterpret_cast<PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>(buf.data() + off);
        if (e->Size == 0) break;
        if (e->Relationship == RelationProcessorCore && e->Processor.GroupCount >= 1 && e->Processor.GroupMask[0].Group == 0) ++n;
        off += e->Size;
    }
    return n;
}

bool thread_set_affinity(Thread* t, uint64_t mask) {
    return SetThreadAffinityMask(t->handle, static_cast<DWORD_PTR>(mask)) != 0;
}

bool thread_set_ideal_processor(Thread* t, int processor) {
    PROCESSOR_NUMBER pn{};
    pn.Group = 0;
    pn.Number = static_cast<BYTE>(processor);
    return SetThreadIdealProcessorEx(t->handle, &pn, nullptr) != 0;
}

bool thread_set_priority(Thread* t, ThreadPriority prio) {
    int win_prio = THREAD_PRIORITY_NORMAL;
    switch (prio) {
        case ThreadPriority::Lowest: win_prio = THREAD_PRIORITY_LOWEST; break;
        case ThreadPriority::BelowNormal: win_prio = THREAD_PRIORITY_BELOW_NORMAL; break;
        case ThreadPriority::Normal: win_prio = THREAD_PRIORITY_NORMAL; break;
        case ThreadPriority::AboveNormal: win_prio = THREAD_PRIORITY_ABOVE_NORMAL; break;
        case ThreadPriority::Highest: win_prio = THREAD_PRIORITY_HIGHEST; break;
        case ThreadPriority::TimeCritical: win_prio = THREAD_PRIORITY_TIME_CRITICAL; break;
    }
    return SetThreadPriority(t->handle, win_prio) != 0;
}

void thread_disable_boost(Thread* t) {
    SetThreadPriorityBoost(t->handle, TRUE);
}

int thread_current_processor() {
    PROCESSOR_NUMBER pn{};
    GetCurrentProcessorNumberEx(&pn);
    if (pn.Group == 0) return static_cast<int>(pn.Number);
    return -1;
}

bool process_get_affinity_mask(uint64_t* proc_mask, uint64_t* sys_mask) {
    DWORD_PTR proc = 0, sys = 0;
    if (!GetProcessAffinityMask(GetCurrentProcess(), &proc, &sys)) return false;
    *proc_mask = static_cast<uint64_t>(proc);
    *sys_mask = static_cast<uint64_t>(sys);
    return true;
}

bool process_set_priority_class(int priority_class) {
    return SetPriorityClass(GetCurrentProcess(), static_cast<DWORD>(priority_class)) != 0;
}

void* named_mutex_create(const char* name, bool* already_exists) {
    HANDLE m = CreateMutexA(nullptr, FALSE, name);
    if (!m) return nullptr;
    if (already_exists) *already_exists = (GetLastError() == ERROR_ALREADY_EXISTS);
    return m;
}

void named_mutex_destroy(void* handle) {
    if (handle) CloseHandle(static_cast<HANDLE>(handle));
}

struct Section {
    HANDLE handle = nullptr;
    uint64_t size = 0;
};

Section* section_create(uint64_t size) {
    // Create a pagefile-backed section (like SEC_RESERVE)
    HANDLE h = CreateFileMappingW(INVALID_HANDLE_VALUE, nullptr, PAGE_READWRITE | SEC_RESERVE,
                                  static_cast<DWORD>(size >> 32), static_cast<DWORD>(size & 0xFFFFFFFF), nullptr);
    if (!h) return nullptr;
    auto* s = new Section;
    s->handle = h;
    s->size = size;
    return s;
}

uint64_t section_map_view(Section* s, uint64_t addr, uint64_t offset, uint64_t size, bool write) {
    DWORD protect = write ? FILE_MAP_WRITE : FILE_MAP_READ;
    void* p = MapViewOfFileEx(s->handle, protect, static_cast<DWORD>(offset >> 32), static_cast<DWORD>(offset & 0xFFFFFFFF),
                              static_cast<SIZE_T>(size), reinterpret_cast<void*>(addr));
    return reinterpret_cast<uint64_t>(p);
}

bool section_unmap_view(uint64_t addr, uint64_t size) {
    return UnmapViewOfFile(reinterpret_cast<void*>(addr)) != 0;
}

void section_destroy(Section* s) {
    if (!s) return;
    if (s->handle) CloseHandle(s->handle);
    delete s;
}

bool section_protect(uint64_t addr, uint64_t size, bool writable) {
    DWORD old = 0;
    return VirtualProtect(reinterpret_cast<void*>(addr), static_cast<SIZE_T>(size), writable ? PAGE_READWRITE : PAGE_READONLY, &old) != 0;
}

void* reserve_fixed(void* addr, std::size_t bytes) {
    void* p = VirtualAlloc(addr, bytes, MEM_RESERVE, PAGE_NOACCESS);
    return p == addr ? p : nullptr;
}

bool release_fixed(void* addr, std::size_t bytes) {
    (void)bytes;
    return VirtualFree(addr, 0, MEM_RELEASE) != 0;
}

// --- write-watch (page fault tracking) ------------------------------------------------------

struct RWLock {
    SRWLOCK lock{};
};

RWLock* rwlock_create() {
    auto* lk = new RWLock;
    InitializeSRWLock(&lk->lock);
    return lk;
}

void rwlock_destroy(RWLock* lk) {
    delete lk;
}

void rwlock_lock_shared(RWLock* lk) {
    AcquireSRWLockShared(&lk->lock);
}

void rwlock_unlock_shared(RWLock* lk) {
    ReleaseSRWLockShared(&lk->lock);
}

void rwlock_lock_exclusive(RWLock* lk) {
    AcquireSRWLockExclusive(&lk->lock);
}

void rwlock_unlock_exclusive(RWLock* lk) {
    ReleaseSRWLockExclusive(&lk->lock);
}

uint32_t page_protect(uint64_t addr) {
    MEMORY_BASIC_INFORMATION mbi{};
    if (VirtualQuery(reinterpret_cast<void*>(addr), &mbi, sizeof mbi) != sizeof mbi) return 0;
    if (mbi.State != MEM_COMMIT) return 0;
    return mbi.Protect;
}

namespace {
PageFaultHandler g_page_fault_handler = nullptr;
PVOID g_veh_handle = nullptr;

LONG CALLBACK veh_handler(EXCEPTION_POINTERS* ep) {
    if (!ep || !ep->ExceptionRecord) return EXCEPTION_CONTINUE_SEARCH;
    const EXCEPTION_RECORD* rec = ep->ExceptionRecord;
    if (rec->ExceptionCode != EXCEPTION_ACCESS_VIOLATION || rec->NumberParameters < 2) return EXCEPTION_CONTINUE_SEARCH;
    const ULONG_PTR kind = rec->ExceptionInformation[0];  // 0=read, 1=write, 8=execute
    if (kind != 0 && kind != 1 && kind != 8) return EXCEPTION_CONTINUE_SEARCH;
    const uint64_t addr = static_cast<uint64_t>(rec->ExceptionInformation[1]);
    if (g_page_fault_handler) {
        bool handled = g_page_fault_handler(addr, static_cast<int>(kind));
        if (handled) return EXCEPTION_CONTINUE_EXECUTION;  // retry the faulting instruction
    }
    // Not handled - let the system handle it (crash or next handler)
    return EXCEPTION_CONTINUE_SEARCH;
}

}  // namespace

bool page_fault_handler_install(PageFaultHandler fn) {
    if (g_veh_handle) {
        // Already installed - just update the handler function
        g_page_fault_handler = fn;
        return true;
    }
    g_page_fault_handler = fn;
    g_veh_handle = AddVectoredExceptionHandler(1, veh_handler);
    return g_veh_handle != nullptr;
}

void page_fault_handler_remove() {
    // Keep the VEH handler installed permanently to avoid add/remove races.
    // Just clear the handler function so faults pass through.
    g_page_fault_handler = nullptr;
}

bool page_query(uint64_t addr, PageState* state, PageProtect* protect, uint64_t* region_base, uint64_t* region_size) {
    MEMORY_BASIC_INFORMATION mbi{};
    if (VirtualQuery(reinterpret_cast<void*>(addr), &mbi, sizeof mbi) != sizeof mbi) return false;
    if (state) *state = static_cast<PageState>(mbi.State);
    if (protect) *protect = static_cast<PageProtect>(mbi.Protect);
    if (region_base) *region_base = reinterpret_cast<uint64_t>(mbi.BaseAddress);
    if (region_size) *region_size = mbi.RegionSize;
    return true;
}

uint64_t tick_count_ms() {
    return GetTickCount64();
}

[[noreturn]] void process_exit(int code) {
    ExitProcess(static_cast<UINT>(code));
}

}  // namespace nxrt::platform
