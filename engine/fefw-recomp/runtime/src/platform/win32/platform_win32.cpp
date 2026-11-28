// SPDX-License-Identifier: GPL-3.0-or-later
// The Windows side of nxrt/platform.h. Every call here is the one the runtime made directly before the platform split,
// so the win32 build keeps its exact behaviour: QPC for the clock, a high-resolution waitable timer for long sleeps,
// VirtualAlloc for the guest arenas, SRW locks for the runtime's short critical sections and CreateFileW for files.
#include <windows.h>
#include <bcrypt.h>
#include <tlhelp32.h>

#include <process.h>

#include <algorithm>
#include <cctype>
#include <string>
#include <vector>

#pragma comment(lib, "bcrypt.lib")

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

bool cond_wait_ms(Cond* c, Mutex* m, uint32_t ms) {
    return SleepConditionVariableSRW(&c->cv, &m->lock, ms, 0) != 0;
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
    // Read, write and delete sharing: the save volumes keep a file open for reading while another handle opens it for
    // writing (the same file can be materialized and reopened), as the volume code did with CreateFileW before the split.
    HANDLE h = CreateFileW(w.c_str(), write ? GENERIC_WRITE : GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
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

bool named_mutex_lock(void* handle) {
    if (!handle) return false;
    // WAIT_ABANDONED means the previous owner died while holding it: this call now owns the mutex, which is exactly what a
    // crash-interrupted save commit needs. CreateMutexA with bInitialOwner FALSE does not own it, so this is the wait the
    // save folder relies on.
    const DWORD r = WaitForSingleObject(static_cast<HANDLE>(handle), INFINITE);
    return r == WAIT_OBJECT_0 || r == WAIT_ABANDONED;
}

void named_mutex_unlock(void* handle) {
    if (handle) ReleaseMutex(static_cast<HANDLE>(handle));
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
    if (state) {
        // MEM_RESERVE and MEM_COMMIT have the same values as PageState, but MEM_FREE is 0x10000 while PageState::Free is 0:
        // casting it left every free page looking occupied, and svcMapPhysicalMemory (via granule_free) refused every range.
        switch (mbi.State) {
            case MEM_COMMIT: *state = PageState::Commit; break;
            case MEM_RESERVE: *state = PageState::Reserve; break;
            default: *state = PageState::Free; break;   // MEM_FREE and MEM_FREE|MEM_TOP_DOWN
        }
    }
    if (protect) *protect = static_cast<PageProtect>(mbi.Protect);
    if (region_base) *region_base = reinterpret_cast<uint64_t>(mbi.BaseAddress);
    if (region_size) *region_size = mbi.RegionSize;
    return true;
}

uint64_t tick_count_ms() {
    return GetTickCount64();
}

uintptr_t module_base() {
    return reinterpret_cast<uintptr_t>(GetModuleHandleW(nullptr));
}

[[noreturn]] void process_exit(int code) {
    ExitProcess(static_cast<UINT>(code));
}

// --- positioned files, names, libraries and process identity ---------------------------------------------------------------------------

bool file_read_at(File* f, int64_t offset, void* buf, std::size_t bytes, std::size_t* done) {
    if (done) *done = 0;
    if (!f || f->handle == INVALID_HANDLE_VALUE || offset < 0) return false;
    auto* p = static_cast<char*>(buf);
    std::size_t at = 0;
    while (at < bytes) {
        const DWORD chunk = static_cast<DWORD>(bytes - at > 0x40000000 ? 0x40000000 : bytes - at);
        OVERLAPPED ov{};
        const uint64_t ofs = static_cast<uint64_t>(offset) + at;
        ov.Offset = static_cast<DWORD>(ofs & 0xFFFFFFFFu);
        ov.OffsetHigh = static_cast<DWORD>(ofs >> 32);
        DWORD got = 0;
        if (!ReadFile(f->handle, p + at, chunk, &got, &ov)) {
            if (GetLastError() == ERROR_HANDLE_EOF) break;
            return false;
        }
        at += got;
        if (got < chunk) break;
    }
    if (done) *done = at;
    return true;
}

bool file_write_at(File* f, int64_t offset, const void* buf, std::size_t bytes) {
    if (!f || f->handle == INVALID_HANDLE_VALUE || offset < 0) return false;
    const auto* p = static_cast<const char*>(buf);
    std::size_t at = 0;
    while (at < bytes) {
        const DWORD chunk = static_cast<DWORD>(bytes - at > 0x40000000 ? 0x40000000 : bytes - at);
        OVERLAPPED ov{};
        const uint64_t ofs = static_cast<uint64_t>(offset) + at;
        ov.Offset = static_cast<DWORD>(ofs & 0xFFFFFFFFu);
        ov.OffsetHigh = static_cast<DWORD>(ofs >> 32);
        DWORD wrote = 0;
        if (!WriteFile(f->handle, p + at, chunk, &wrote, &ov) || wrote == 0) return false;
        at += wrote;
    }
    return true;
}

bool file_truncate(File* f, int64_t size) {
    if (!f || f->handle == INVALID_HANDLE_VALUE || size < 0) return false;
    LARGE_INTEGER to;
    to.QuadPart = size;
    if (!SetFilePointerEx(f->handle, to, nullptr, FILE_BEGIN)) return false;
    return SetEndOfFile(f->handle) != 0;
}

bool file_flush(File* f) {
    if (!f || f->handle == INVALID_HANDLE_VALUE) return false;
    return FlushFileBuffers(f->handle) != 0;
}

File* file_open_update(const char* path_utf8) {
    const std::wstring w = wide(path_utf8);
    if (w.empty()) return nullptr;
    HANDLE h = CreateFileW(w.c_str(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                           OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) return nullptr;
    auto* f = new File;
    f->handle = h;
    return f;
}

File* file_create_new(const char* path_utf8) {
    const std::wstring w = wide(path_utf8);
    if (w.empty()) return nullptr;
    HANDLE h = CreateFileW(w.c_str(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr,
                           CREATE_NEW, FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) return nullptr;
    auto* f = new File;
    f->handle = h;
    return f;
}

bool file_sync(const char* path_utf8) {
    const std::wstring w = wide(path_utf8);
    if (w.empty()) return false;
    HANDLE h = CreateFileW(w.c_str(), GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_EXISTING,
                           FILE_ATTRIBUTE_NORMAL, nullptr);
    if (h == INVALID_HANDLE_VALUE) return false;
    const bool ok = FlushFileBuffers(h) != 0;
    CloseHandle(h);
    return ok;
}

bool file_rename(const char* from_utf8, const char* to_utf8, bool replace) {
    const std::wstring a = wide(from_utf8), b = wide(to_utf8);
    if (a.empty() || b.empty()) return false;
    return MoveFileExW(a.c_str(), b.c_str(), MOVEFILE_WRITE_THROUGH | (replace ? MOVEFILE_REPLACE_EXISTING : 0)) != 0;
}

bool file_times(const char* path_utf8, FileTimes* out) {
    if (!out) return false;
    *out = FileTimes{};
    const std::wstring w = wide(path_utf8);
    if (w.empty()) return false;
    WIN32_FILE_ATTRIBUTE_DATA d{};
    if (!GetFileAttributesExW(w.c_str(), GetFileExInfoStandard, &d)) return false;
    const auto secs = [](const FILETIME& ft) {
        const uint64_t t = (static_cast<uint64_t>(ft.dwHighDateTime) << 32) | ft.dwLowDateTime;
        return static_cast<int64_t>(t / 10000000ull) - 11644473600ll;
    };
    out->created = secs(d.ftCreationTime);
    out->modified = secs(d.ftLastWriteTime);
    out->accessed = secs(d.ftLastAccessTime);
    return true;
}

std::string host_path(const char* path_utf8) {
    std::string p = path_utf8 ? path_utf8 : "";
    if (p.size() < 240) return p;
    for (char& c : p)
        if (c == '/') c = '\\';
    if (p.compare(0, 4, "\\\\?\\") == 0) return p;
    if (p.compare(0, 2, "\\\\") == 0) return "\\\\?\\UNC\\" + p.substr(2);
    return "\\\\?\\" + p;
}

uint32_t process_id() {
    return static_cast<uint32_t>(GetCurrentProcessId());
}

bool process_alive(uint32_t pid) {
    HANDLE p = OpenProcess(SYNCHRONIZE, FALSE, static_cast<DWORD>(pid));
    if (!p) return GetLastError() == ERROR_ACCESS_DENIED;
    const bool alive = WaitForSingleObject(p, 0) == WAIT_TIMEOUT;
    CloseHandle(p);
    return alive;
}

void* library_open(const char* path_utf8) {
    const std::wstring w = wide(path_utf8);
    if (w.empty()) return nullptr;
    return reinterpret_cast<void*>(LoadLibraryExW(w.c_str(), nullptr, LOAD_WITH_ALTERED_SEARCH_PATH));
}

void* library_symbol(void* library, const char* name) {
    if (!library || !name) return nullptr;
    return reinterpret_cast<void*>(GetProcAddress(reinterpret_cast<HMODULE>(library), name));
}

void library_close(void* library) {
    if (library) FreeLibrary(reinterpret_cast<HMODULE>(library));
}

bool library_file_name(const char* stem, const char* file_name, int* version) {
    if (version) *version = 0;
    if (!stem || !file_name) return false;
    std::string name;
    for (const char* p = file_name; *p; ++p) name.push_back(static_cast<char>(std::tolower(static_cast<unsigned char>(*p))));
    const std::string prefix = std::string(stem) + "-", suffix = ".dll";
    if (name.size() <= prefix.size() + suffix.size()) return false;
    if (name.compare(0, prefix.size(), prefix) != 0) return false;
    if (name.compare(name.size() - suffix.size(), suffix.size(), suffix) != 0) return false;
    const std::string mid = name.substr(prefix.size(), name.size() - prefix.size() - suffix.size());
    if (mid.empty() || mid.size() > 4) return false;
    int v = 0;
    for (char c : mid) {
        if (c < '0' || c > '9') return false;
        v = v * 10 + (c - '0');
    }
    if (version) *version = v;
    return true;
}

std::string module_path() {
    std::wstring buf(260, L'\0');
    for (;;) {
        const DWORD n = GetModuleFileNameW(nullptr, buf.data(), static_cast<DWORD>(buf.size()));
        if (n == 0) return {};
        if (n < buf.size()) {
            buf.resize(n);
            break;
        }
        buf.resize(buf.size() * 2);
    }
    const int need = WideCharToMultiByte(CP_UTF8, 0, buf.data(), static_cast<int>(buf.size()), nullptr, 0, nullptr, nullptr);
    if (need <= 0) return {};
    std::string out(static_cast<std::size_t>(need), '\0');
    WideCharToMultiByte(CP_UTF8, 0, buf.data(), static_cast<int>(buf.size()), out.data(), need, nullptr, nullptr);
    return out;
}

bool random_bytes(void* buf, std::size_t bytes) {
    if (!buf && bytes) return false;
    auto* p = static_cast<unsigned char*>(buf);
    std::size_t done = 0;
    while (done < bytes) {
        const ULONG n = static_cast<ULONG>(bytes - done > 0x10000000 ? 0x10000000 : bytes - done);
        if (!BCRYPT_SUCCESS(BCryptGenRandom(nullptr, p + done, n, BCRYPT_USE_SYSTEM_PREFERRED_RNG))) return false;
        done += n;
    }
    return true;
}

// --- host threads and topology ------------------------------------------------------------------------------------------------------

namespace {

HANDLE open_thread(uint32_t id) {
    return OpenThread(THREAD_SET_INFORMATION | THREAD_QUERY_INFORMATION, FALSE, static_cast<DWORD>(id));
}

// The parts of the NtQuerySystemInformation(SystemProcessInformation) records this needs (Windows Vista and later, x64).
struct NtThreadInfo {
    LARGE_INTEGER kernel_time, user_time, create_time;
    ULONG wait_time;
    PVOID start_address;
    PVOID unique_process, unique_thread;
    LONG priority, base_priority;
    ULONG context_switches, thread_state, wait_reason;
};
static_assert(sizeof(NtThreadInfo) == 0x50, "SYSTEM_THREAD_INFORMATION");
struct NtProcInfo {
    ULONG next_entry_offset, number_of_threads;
    uint8_t body[0x100 - 8];
};
static_assert(sizeof(NtProcInfo) == 0x100, "SYSTEM_PROCESS_INFORMATION");

using NtQuerySystemInformationFn = LONG(NTAPI*)(ULONG, PVOID, ULONG, PULONG);

}  // namespace

void process_thread_ids(std::vector<uint32_t>* out) {
    if (!out) return;
    out->clear();
    HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
    if (snap == INVALID_HANDLE_VALUE) return;
    THREADENTRY32 te{};
    te.dwSize = sizeof te;
    const DWORD pid = GetCurrentProcessId();
    if (Thread32First(snap, &te)) do {
        if (te.th32OwnerProcessID == pid) out->push_back(te.th32ThreadID);
    } while (Thread32Next(snap, &te));
    CloseHandle(snap);
}

int host_thread_ideal_processor(uint32_t id) {
    HANDLE h = open_thread(id);
    if (!h) return -1;
    PROCESSOR_NUMBER pn{};
    const int v = GetThreadIdealProcessorEx(h, &pn) ? static_cast<int>(pn.Number) : -1;
    CloseHandle(h);
    return v;
}

int host_thread_priority(uint32_t id) {
    HANDLE h = open_thread(id);
    if (!h) return 0;
    const int v = GetThreadPriority(h);
    CloseHandle(h);
    return v == THREAD_PRIORITY_ERROR_RETURN ? 0 : v;
}

bool host_thread_set_affinity(uint32_t id, uint64_t mask) {
    HANDLE h = open_thread(id);
    if (!h) return false;
    const bool ok = SetThreadAffinityMask(h, static_cast<DWORD_PTR>(mask)) != 0;
    CloseHandle(h);
    return ok;
}

bool host_thread_set_ideal_processor(uint32_t id, int processor) {
    if (processor < 0 || processor > 63) return false;
    HANDLE h = open_thread(id);
    if (!h) return false;
    PROCESSOR_NUMBER pn{};
    pn.Number = static_cast<BYTE>(processor);
    const bool ok = SetThreadIdealProcessorEx(h, &pn, nullptr) != 0;
    CloseHandle(h);
    return ok;
}

bool host_thread_set_priority(uint32_t id, int priority) {
    HANDLE h = open_thread(id);
    if (!h) return false;
    const bool ok = SetThreadPriority(h, priority) != 0;
    CloseHandle(h);
    return ok;
}

uint64_t physical_core_mask(int processor) {
    const uint64_t bit = processor >= 0 && processor < 64 ? 1ull << processor : 0;
    DWORD len = 0;
    GetLogicalProcessorInformationEx(RelationProcessorCore, nullptr, &len);
    if (len) {
        std::vector<unsigned char> buf(len);
        auto* base = reinterpret_cast<PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>(buf.data());
        if (GetLogicalProcessorInformationEx(RelationProcessorCore, base, &len)) {
            for (DWORD off = 0; off < len;) {
                auto* e = reinterpret_cast<PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>(buf.data() + off);
                if (e->Relationship == RelationProcessorCore && e->Processor.GroupMask[0].Group == 0 &&
                    (e->Processor.GroupMask[0].Mask & bit))
                    return static_cast<uint64_t>(e->Processor.GroupMask[0].Mask);
                off += e->Size;
            }
        }
    }
    return bit;
}

bool host_snapshot(HostSnapshot* out) {
    if (!out) return false;
    out->threads.clear();
    out->processes.clear();
    static NtQuerySystemInformationFn query = reinterpret_cast<NtQuerySystemInformationFn>(
        reinterpret_cast<void*>(GetProcAddress(GetModuleHandleA("ntdll.dll"), "NtQuerySystemInformation")));
    if (!query) return false;
    std::vector<uint8_t> buf(1 << 20);
    const uint32_t pid = GetCurrentProcessId();
    for (int attempt = 0; attempt < 4; ++attempt) {
        ULONG need = 0;
        const LONG st = query(5, buf.data(), static_cast<ULONG>(buf.size()), &need);
        if (st < 0) {
            if (need <= buf.size()) return false;
            buf.resize(need + (need >> 3));
            continue;
        }
        for (std::size_t off = 0;;) {
            const uint8_t* rec = buf.data() + off;
            const auto* p = reinterpret_cast<const NtProcInfo*>(rec);
            const uint64_t ppid = *reinterpret_cast<const uint64_t*>(rec + 0x50);
            if (ppid == pid) {
                const auto* t = reinterpret_cast<const NtThreadInfo*>(rec + sizeof(NtProcInfo));
                for (ULONG i = 0; i < p->number_of_threads; ++i) {
                    HostSnapshot::Thread e;
                    e.id = static_cast<uint32_t>(reinterpret_cast<uintptr_t>(t[i].unique_thread));
                    e.ready = t[i].thread_state == 1 || t[i].thread_state == 3;   // ready, standby
                    e.switches = t[i].context_switches;
                    e.cpu_100ns = static_cast<uint64_t>(t[i].kernel_time.QuadPart) + static_cast<uint64_t>(t[i].user_time.QuadPart);
                    out->threads.push_back(e);
                }
            } else {
                HostSnapshot::Proc proc;
                proc.id = static_cast<uint32_t>(ppid);
                proc.cpu_100ns = *reinterpret_cast<const uint64_t*>(rec + 0x28) + *reinterpret_cast<const uint64_t*>(rec + 0x30);
                out->processes.push_back(proc);
            }
            if (!p->next_entry_offset) break;
            off += p->next_entry_offset;
        }
        return true;
    }
    return false;
}

bool process_name(uint32_t pid, std::string* name) {
    if (name) name->clear();
    HANDLE p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, static_cast<DWORD>(pid));
    if (!p) return false;
    wchar_t buf[MAX_PATH];
    DWORD n = static_cast<DWORD>(sizeof buf / sizeof buf[0]);
    const bool ok = QueryFullProcessImageNameW(p, 0, buf, &n) != 0;
    CloseHandle(p);
    if (!ok) return false;
    if (!name) return true;
    const wchar_t* base = buf;
    for (DWORD i = 0; i < n; ++i)
        if (buf[i] == L'\\' || buf[i] == L'/') base = buf + i + 1;
    const int bytes = WideCharToMultiByte(CP_UTF8, 0, base, static_cast<int>(n - (base - buf)), nullptr, 0, nullptr, nullptr);
    if (bytes <= 0) return false;
    name->resize(static_cast<std::size_t>(bytes));
    WideCharToMultiByte(CP_UTF8, 0, base, static_cast<int>(n - (base - buf)), name->data(), bytes, nullptr, nullptr);
    return true;
}

}  // namespace nxrt::platform
