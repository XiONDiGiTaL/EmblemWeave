// SPDX-License-Identifier: GPL-3.0-or-later
// Platform layer (S4b): SDL3 window, fullscreen, relative (raw) mouse, gamepad hot-plug, Vulkan surface creation.
// Everything here is called from one thread (the one that created the window, normally the main thread). Events are not
// returned: pump_events() turns them into nxrt::input events (input.h).
// Configuration (config.h): [gfx] hidden / NXRT_GFX_HIDDEN=1 creates the window hidden (automated tests).
#pragma once
#include <cstddef>
#include <cstdint>

namespace nxrt::platform {

struct InitOptions {
    bool gamepads = true;       // init the joystick/gamepad subsystems and open pads as they are plugged in
};
bool init(const InitOptions& opt = {});   // idempotent; sets the SDL hints (raw mouse), SDL_Init(VIDEO|EVENTS|JOYSTICK|GAMEPAD)
void shutdown();                           // closes the window and every opened gamepad, SDL_Quit
bool initialised();

struct WindowDesc {
    const char* title = "nxrt";
    int width = 1280, height = 720;     // client size in screen coordinates at creation
    bool resizable = true;
    bool hidden = false;                // also forced by [gfx] hidden / NXRT_GFX_HIDDEN
};
bool create_window(const WindowDesc& desc);   // Vulkan-capable window (SDL_WINDOW_VULKAN, high pixel density)
void destroy_window();
bool has_window();
void* native_window();                        // SDL_Window*
void* native_hinstance();                     // Win32 HINSTANCE from SDL window properties
void* native_hwnd();                          // the Win32 HWND of the window (null when there is none)
void set_window_title(const char* title);
void pixel_size(int* w, int* h);              // client size in pixels (0, 0 while minimised)
bool set_window_size(int w, int h);           // windowed size in screen coordinates
bool minimized();
bool focused();
bool visible();

enum class FullscreenMode { Windowed, Borderless, Exclusive };
// Borderless: desktop-resolution borderless window. Exclusive: SDL fullscreen with a real display mode closest to w x h at hz
// (zeros: the desktop mode). Returns false when the change failed (the mode is then unchanged). Synchronous: waits until SDL
// reports the new state or ~2 s.
bool set_fullscreen(FullscreenMode mode, int w = 0, int h = 0, float hz = 0.f);
FullscreenMode fullscreen_mode();
float display_refresh_hz();                   // refresh rate of the display the window is on (desktop mode), 0 if unknown

// Displays (monitors), numbered 1.. in the system's order. display_name is "Name (1920x1080 @ 144 Hz)"-style text for menus, empty for a bad number.
int display_count();
bool display_name(int number, char* out, int cap);
int window_display();                         // the number of the display the window is on, 0 if unknown
// Moves the window to the centre of that display, keeping its mode (a fullscreen window leaves fullscreen, moves and goes back).
bool move_to_display(int number);

// Relative mouse mode: raw, unaccelerated deltas (SDL_SetWindowRelativeMouseMode with SDL_HINT_MOUSE_RELATIVE_SYSTEM_SCALE
// forced to "0" and SDL_HINT_MOUSE_RELATIVE_SPEED_SCALE to "1"). The cursor is hidden and confined while it is on.
bool set_relative_mouse(bool on);
bool relative_mouse();
void set_cursor_visible(bool on);             // the OS cursor over the window (not in relative mode, which hides it); no-op without a window
void warp_mouse(float x, float y);            // puts the OS cursor at window pixel (x, y) (client area); no-op without a window
// SDL text input on the window (input::EventType::Text events, the IME): off unless something takes typing (the overlay's on-screen
// keyboard). Main thread; cheap when nothing changes.
void set_text_input(bool on);
bool relative_mouse_is_raw();                 // true when the system-scale hint is off (the unaccelerated setup)

// Polled gamepad state (play mode): what SDL's gamepad layer reports for one open pad. SDL normalises every controller to the Xbox
// layout by position (SOUTH is the bottom face button), whatever is printed on it.
struct GamepadRaw {
    uint32_t buttons = 0;               // bit n set = SDL_GamepadButton n is down (SOUTH 0, EAST 1, WEST 2, NORTH 3, BACK 4, GUIDE 5, START 6,
                                        // LEFT_STICK 7, RIGHT_STICK 8, LEFT_SHOULDER 9, RIGHT_SHOULDER 10, DPAD_UP 11, DOWN 12, LEFT 13, RIGHT 14, MISC1 15)
    int16_t axes[6] = {};               // SDL_GamepadAxis order: LEFTX LEFTY RIGHTX RIGHTY (-32768..32767, y down = +), LEFT_TRIGGER RIGHT_TRIGGER (0..32767)
    uint8_t face_label[4] = {0, 1, 2, 3};   // what is printed on SOUTH, EAST, WEST, NORTH: 0 = A, 1 = B, 2 = X, 3 = Y
                                        // (PlayStation cross, circle, square, triangle count as A, B, X, Y; a Nintendo pad prints B, A, Y, X)
};
bool gamepad_poll(uint32_t id, GamepadRaw* out);                       // false when no such pad is open
bool gamepad_sensors(uint32_t id, float accel[3], float gyro[3]);      // latest accelerometer (m/s^2) and gyro (rad/s) in SDL's axes; false without both sensors
bool gamepad_rumble(uint32_t id, uint16_t low, uint16_t high, uint32_t duration_ms);   // 0..65535 per motor; (0, 0) stops it; false when unsupported

// Pumps the SDL event queue and translates it into input events and window state. wait: block for up to timeout_ms (-1 =
// forever) for the first event (used while minimised). Returns the number of SDL events handled.
int pump_events(bool wait = false, int timeout_ms = -1);
bool quit_requested();
void clear_quit();
void request_quit();

uint64_t ticks_ns();                          // SDL_GetTicksNS

// Vulkan: the instance extensions SDL needs (VK_KHR_surface + the platform one) and surface creation/destruction. The Vulkan
// handles are passed as integers so that this header needs no Vulkan include.
const char* const* vulkan_instance_extensions(uint32_t* count);
bool create_vulkan_surface(void* vk_instance, uint64_t* surface_out);
void destroy_vulkan_surface(void* vk_instance, uint64_t surface);

// --- host services ----------------------------------------------------------------------------------------------------
// What the runtime above the window layer needs from the operating system, behind one interface (docs/LINUX.md, phase 2):
// memory, threads, synchronisation, time and files. src/platform/win32 answers these with the calls the runtime used
// before (VirtualAlloc, _beginthreadex, SRW locks, QueryPerformanceCounter, a high-resolution waitable timer, CreateFileW)
// and src/platform/posix with mmap, pthreads, clock_gettime, nanosleep and file descriptors. Nothing here includes SDL or
// windows.h, so the kernel can use it without pulling the graphics shell in.

// Reserve address space without committing it, then commit pages as the guest asks for them. `release` frees the whole
// range; `decommit` hands the pages back to the system but keeps the reservation.
void* reserve(std::size_t bytes);
bool commit(void* p, std::size_t bytes);
bool decommit(void* p, std::size_t bytes);
void release(void* p, std::size_t bytes);
// Protect with POSIX-style flags (PROT_NONE, PROT_READ, PROT_WRITE, PROT_EXEC, or combinations).
// On Windows, maps to PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE, PAGE_EXECUTE_*.
bool protect(void* p, std::size_t bytes, uint32_t protect);
std::size_t page_size();
// Reserve address space at a specific fixed address (for stack, kernel region). Returns the address or nullptr on failure.
void* reserve_fixed(void* addr, std::size_t bytes);
// Release a fixed reservation.
bool release_fixed(void* addr, std::size_t bytes);

using ThreadFn = void (*)(void*);

struct Thread;
// Starts a thread that runs `fn(arg)` and exits. The handle is freed by `thread_join` or `thread_detach`.
Thread* thread_start(const char* name, ThreadFn fn, void* arg);
void thread_join(Thread* t);
void thread_detach(Thread* t);
Thread* thread_self();    // returns the handle for the calling thread
uint32_t thread_id();     // the host's id for the calling thread, for log tags
void yield_cpu();         // offers the processor to another ready thread

// Thread CPU time in 100ns units (kernel + user time). Returns 0 on failure or if not supported.
uint64_t thread_cpu_time(Thread* t);
// Returns a pseudo-handle for the current thread (like GetCurrentThread on Windows, pthread_self on POSIX).
// This is only used for thread_cpu_time() when the calling thread wants its own CPU time.
Thread* thread_current_handle();
// Returns the native thread handle (HANDLE on Windows, pthread_t on POSIX) for use with host APIs.
// Returns nullptr on failure or if not supported.
void* thread_native_handle(Thread* t);

// A lock the runtime holds for a few instructions at a time, never across a guest wait.
struct Mutex;
struct Cond;
Mutex* mutex_create();
void mutex_destroy(Mutex* m);
void mutex_lock(Mutex* m);
bool mutex_trylock(Mutex* m);
void mutex_unlock(Mutex* m);
Cond* cond_create();
void cond_destroy(Cond* c);
void cond_wait(Cond* c, Mutex* m);   // releases `m` while waiting and takes it again before returning
void cond_signal(Cond* c);
void cond_broadcast(Cond* c);

// A monotonic counter and its frequency. Windows keeps QueryPerformanceCounter and its frequency, so the console tick
// derived here is the number the runtime produced before the split.
uint64_t clock_ticks();
uint64_t clock_hz();
int64_t now_ns();         // clock_ticks() in nanoseconds, without dropping the sub-tick remainder
// Waits at least `ns`; zero or negative returns at once. This primitive never spins: kernel::sleep_ns decides whether a
// short span is better spent yielding.
void sleep_ns(int64_t ns);

struct File;
// Opens a UTF-8 path. `write` creates or truncates the file; a read opens it only if it exists.
File* file_open(const char* path_utf8, bool write);
void file_close(File* f);
std::size_t file_read(File* f, void* buf, std::size_t bytes);
std::size_t file_write(File* f, const void* buf, std::size_t bytes);
bool file_seek(File* f, int64_t offset);   // absolute, from the start
int64_t file_size(File* f);                // -1 when it cannot be asked
bool file_exists(const char* path_utf8);
bool file_mkdirs(const char* path_utf8);   // every missing component, like mkdir -p

// Number of physical CPU cores (SMT siblings counted once). Used for auto-tuning spin waits.
int physical_core_count();

// Thread scheduling and affinity (for the console core model)
enum class ThreadPriority { Lowest, BelowNormal, Normal, AboveNormal, Highest, TimeCritical };
// Set the affinity mask of a thread (bit n = processor n). Returns false on failure.
bool thread_set_affinity(Thread* t, uint64_t mask);
// Set the ideal processor for a thread (soft affinity hint). Returns false on failure.
bool thread_set_ideal_processor(Thread* t, int processor);
// Set the priority of a thread. Returns false on failure.
bool thread_set_priority(Thread* t, ThreadPriority prio);
// Disable Windows' dynamic priority boost for a thread (no-op on POSIX).
void thread_disable_boost(Thread* t);
// Get the current processor number for the calling thread. Returns -1 on failure.
int thread_current_processor();
// Get the process affinity mask. Returns false on failure.
bool process_get_affinity_mask(uint64_t* proc_mask, uint64_t* sys_mask);
// Set the process priority class. Returns false on failure.
bool process_set_priority_class(int priority_class);
// Named mutex for cross-process coordination (used for console core slot allocation).
// Returns a handle (opaque pointer) or nullptr on failure/already exists.
void* named_mutex_create(const char* name, bool* already_exists);
void named_mutex_destroy(void* handle);

// Shared memory section for the guest heap (like Windows SEC_RESERVE section with multiple view mappings).
// Creates a section of the given size (bytes). Returns an opaque handle or nullptr on failure.
struct Section;
Section* section_create(uint64_t size);
// Maps a view of the section at the specified address (or lets the OS choose if addr=0).
// offset = section offset, size = view size. Returns the mapped address or 0 on failure.
uint64_t section_map_view(Section* s, uint64_t addr, uint64_t offset, uint64_t size, bool write);
// Unmaps a view at the given address. Returns true on success.
bool section_unmap_view(uint64_t addr, uint64_t size);
// Destroys the section.
void section_destroy(Section* s);
// Changes the protection of a mapped view range. Returns true on success.
bool section_protect(uint64_t addr, uint64_t size, bool writable);

// --- write-watch (page fault tracking) ------------------------------------------------------
// Read-write lock for the write-watch transition lock (g_transitions equivalent).
// Multiple readers (fault handlers, arm's shared path) or one writer (region_destroy, arm exclusive).
struct RWLock;
RWLock* rwlock_create();
void rwlock_destroy(RWLock* lk);
void rwlock_lock_shared(RWLock* lk);
void rwlock_unlock_shared(RWLock* lk);
void rwlock_lock_exclusive(RWLock* lk);
void rwlock_unlock_exclusive(RWLock* lk);

// Page protection query (equivalent to VirtualQuery for a single page).
// Returns 0 on failure. On Windows: PAGE_READONLY, PAGE_READWRITE, PAGE_NOACCESS, etc.
// On POSIX: PROT_READ, PROT_WRITE, PROT_NONE, or 0 if not committed.
uint32_t page_protect(uint64_t addr);

// Cross-platform page protection constants (match POSIX sys/mman.h values).
// On Windows, these map to PAGE_* constants; on POSIX they are the native values.
constexpr uint32_t PROT_NONE  = 0x0;
constexpr uint32_t PROT_READ  = 0x1;
constexpr uint32_t PROT_WRITE = 0x2;
constexpr uint32_t PROT_EXEC  = 0x4;

// Installs a handler for page faults (SIGSEGV on POSIX, VEH on Windows).
// The handler receives the fault address and the access kind (0=read, 1=write, 8=execute).
// Returns true if the fault was handled (page made accessible), false if it should continue
// to the default handler (crash). Only one handler can be installed (the write-watch subsystem).
using PageFaultHandler = bool (*)(uint64_t addr, int kind);
bool page_fault_handler_install(PageFaultHandler fn);
void page_fault_handler_remove();

// Memory state constants (subset of Windows MEMORY_BASIC_INFORMATION.State / Protect)
enum class PageState : uint32_t {
    Free = 0,
    Reserve = 0x2000,   // MEM_RESERVE
    Commit = 0x1000,    // MEM_COMMIT
};
enum class PageProtect : uint32_t {
    NoAccess = 0x01,
    ReadOnly = 0x02,
    ReadWrite = 0x04,
    WriteCopy = 0x08,
    Execute = 0x10,
    ExecuteRead = 0x20,
    ExecuteReadWrite = 0x40,
    ExecuteWriteCopy = 0x80,
    Guard = 0x100,
    NoCache = 0x200,
    WriteCombine = 0x400,
};

// Queries memory info for a page. Returns false on failure.
// If successful, fills state, protect, region_base, region_size.
bool page_query(uint64_t addr, PageState* state, PageProtect* protect, uint64_t* region_base, uint64_t* region_size);

// High-resolution tick count (milliseconds since boot, like GetTickCount64)
uint64_t tick_count_ms();

// Exit the current process with the given exit code. Does not return.
[[noreturn]] void process_exit(int code);

}  // namespace nxrt::platform
