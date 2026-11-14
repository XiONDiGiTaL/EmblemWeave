// SPDX-License-Identifier: GPL-3.0-or-later
// Scheduling for the audio renderer's two threads (the DSP thread and the thread that paces it). Not in the Ryujinx port. The renderer has
// to deliver a 5 ms frame every 5 ms while the game's render and loading threads keep every core busy; a frame that comes late is a click
// on the sound card. The threads join the Multimedia Class Scheduler Service task "Pro Audio" (the Windows way for an audio thread to get
// the CPU on time) and the system timer runs at 1 ms while the renderer is up, so the pacing sleeps are not rounded up to 15 ms.
#pragma once
#include <windows.h>
#include <avrt.h>
#include <mmsystem.h>

#include <atomic>

#pragma comment(lib, "avrt.lib")
#pragma comment(lib, "winmm.lib")

namespace nxrt::audren {

// Call at the top of an audio thread. The handle is released when the thread ends (the OS drops the registration then).
inline void BoostAudioThread() {
    DWORD task = 0;
    HANDLE mm = AvSetMmThreadCharacteristicsW(L"Pro Audio", &task);
    if (mm != nullptr) {
        AvSetMmThreadPriority(mm, AVRT_PRIORITY_HIGH);
    } else {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    }
}

// Raises the calling thread's priority for the lifetime of the object and restores it afterwards. Used by the thread that updates the
// renderer while it holds the renderer's lock (a stand-in for priority inheritance).
class ScopedAudioLockBoost {
  public:
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
    ScopedAudioLockBoost(const ScopedAudioLockBoost&) = delete;
    ScopedAudioLockBoost& operator=(const ScopedAudioLockBoost&) = delete;

  private:
    HANDLE _thread;
    int _old;
    bool _changed = false;
};

// Holds the 1 ms timer resolution for as long as any renderer runs (reference counted).
class TimerResolutionScope {
  public:
    static void Acquire() {
        if (_count.fetch_add(1) == 0) {
            timeBeginPeriod(1);
        }
    }
    static void Release() {
        if (_count.fetch_sub(1) == 1) {
            timeEndPeriod(1);
        }
    }

  private:
    static inline std::atomic<int> _count{0};
};

}  // namespace nxrt::audren
