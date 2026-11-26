// SPDX-License-Identifier: GPL-3.0-or-later
// Scheduling for the audio renderer's two threads (the DSP thread and the thread that paces it). Not in the Ryujinx port. The renderer has
// to deliver a 5 ms frame every 5 ms while the game's render and loading threads keep every core busy; a frame that comes late is a click
// on the sound card. The threads join the Multimedia Class Scheduler Service task "Pro Audio" (the Windows way for an audio thread to get
// the CPU on time) and the system timer runs at 1 ms while the renderer is up, so the pacing sleeps are not rounded up to 15 ms.
#pragma once
#ifdef _WIN32
#include <windows.h>
#include <avrt.h>
#include <mmsystem.h>

#pragma comment(lib, "avrt.lib")
#pragma comment(lib, "winmm.lib")
#else
#include <cerrno>
#include <sys/resource.h>
#endif

#include <atomic>

namespace nxrt::audren {

// Call at the top of an audio thread. The handle is released when the thread ends (the OS drops the registration then).
inline void BoostAudioThread() {
#ifdef _WIN32
    DWORD task = 0;
    HANDLE mm = AvSetMmThreadCharacteristicsW(L"Pro Audio", &task);
    if (mm != nullptr) {
        AvSetMmThreadPriority(mm, AVRT_PRIORITY_HIGH);
    } else {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    }
#else
    // MMCSS does not exist on Linux. A negative nice value is the closest unprivileged equivalent; a real-time policy
    // would need privileges the game cannot assume and could starve the compositor. A refusal is not fatal (the Windows
    // path degrades the same way when AvSetMmThreadCharacteristics fails).
    errno = 0;
    const int old = getpriority(PRIO_PROCESS, 0);
    if (old != -1 || errno == 0) setpriority(PRIO_PROCESS, 0, old > -10 ? -10 : old);
#endif
}

// Raises the calling thread's priority for the lifetime of the object and restores it afterwards. Used by the thread that updates the
// renderer while it holds the renderer's lock (a stand-in for priority inheritance).
class ScopedAudioLockBoost {
  public:
#ifdef _WIN32
    ScopedAudioLockBoost() : _thread(GetCurrentThread()), _old(GetThreadPriority(_thread)) {
        if (_old != THREAD_PRIORITY_ERROR_RETURN && _old < THREAD_PRIORITY_HIGHEST) {
            _changed = SetThreadPriority(_thread, THREAD_PRIORITY_HIGHEST) != 0;
        }
    }
    ~ScopedAudioLockBoost() {
        if (_changed) {
            SetThreadPriority(_thread, _old);
        }
    }
#else
    ScopedAudioLockBoost() {
        errno = 0;
        _old = getpriority(PRIO_PROCESS, 0);
        if (_old != -1 || errno == 0) _changed = setpriority(PRIO_PROCESS, 0, _old > -10 ? -10 : _old) == 0;
    }
    ~ScopedAudioLockBoost() {
        if (_changed) setpriority(PRIO_PROCESS, 0, _old);
    }
#endif
    ScopedAudioLockBoost(const ScopedAudioLockBoost&) = delete;
    ScopedAudioLockBoost& operator=(const ScopedAudioLockBoost&) = delete;

  private:
#ifdef _WIN32
    HANDLE _thread;
    int _old;
#else
    int _old = 0;
#endif
    bool _changed = false;
};

// Holds the 1 ms timer resolution for as long as any renderer runs (reference counted).
class TimerResolutionScope {
  public:
    static void Acquire() {
#ifdef _WIN32
        // The Windows timer granularity is 15.6 ms unless asked otherwise; Linux timers are already high resolution.
        if (_count.fetch_add(1) == 0) timeBeginPeriod(1);
#else
        _count.fetch_add(1);
#endif
    }
    static void Release() {
#ifdef _WIN32
        if (_count.fetch_sub(1) == 1) timeEndPeriod(1);
#else
        _count.fetch_sub(1);
#endif
    }

  private:
    static inline std::atomic<int> _count{0};
};

}  // namespace nxrt::audren
