#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Measure what stands between the runtime and a Linux build.

The port is not a mystery: it is a list of Windows calls, per subsystem, per
file. This reads the runtime's own sources and prints that list as Markdown, so
docs/LINUX.md can be regenerated instead of drifting.

    python tools/port/win32_audit.py            # the table in docs/LINUX.md
    python tools/port/win32_audit.py --top 40   # more files per subsystem

Only calls are counted, not includes: a file that includes <windows.h> for a
type and makes no call costs nothing to port.
"""

from __future__ import annotations

import argparse
import re
from collections import Counter, defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RUNTIME = ROOT / "engine" / "fefw-recomp" / "runtime"

# One entry per thing that has to be replaced by something on the other side.
# The order is the order the port should go in: nothing runs until threads,
# memory and files work.
SUBSYSTEMS: list[tuple[str, str, tuple[str, ...]]] = [
    ("Threads", "pthreads", (
        "CreateThread", "CreateRemoteThread", "ResumeThread", "SuspendThread", "GetExitCodeThread",
        "SetThreadPriority", "GetThreadPriority", "SetThreadAffinityMask", "GetCurrentThreadId",
        "SetThreadDescription", "GetThreadDescription", "SetThreadInformation", "OpenThread",
        "TlsAlloc", "TlsFree", "TlsGetValue", "TlsSetValue", "GetCurrentThread", "DuplicateHandle",
        "SwitchToThread", "YieldProcessor", "SetThreadStackGuarantee",
    )),
    ("Memory", "mmap/mprotect", (
        "VirtualAlloc", "VirtualAlloc2", "VirtualFree", "VirtualProtect", "VirtualQuery", "VirtualLock",
        "VirtualUnlock", "GetSystemInfo", "GlobalMemoryStatusEx", "FlushInstructionCache",
        "CreateFileMappingW", "MapViewOfFile", "UnmapViewOfFile", "HeapAlloc", "HeapFree",
        "GetProcessHeap", "VirtualAllocEx", "WriteProcessMemory", "ReadProcessMemory",
    )),
    ("Sync", "pthreads/futex", (
        "CreateEventW", "CreateEventExW", "SetEvent", "ResetEvent", "CreateMutexW", "ReleaseMutex",
        "CreateSemaphoreW", "ReleaseSemaphore", "WaitForSingleObject", "WaitForMultipleObjects",
        "WaitForSingleObjectEx", "InitializeCriticalSection", "EnterCriticalSection",
        "LeaveCriticalSection", "DeleteCriticalSection", "TryEnterCriticalSection",
        "InitializeSRWLock", "AcquireSRWLockExclusive", "ReleaseSRWLockExclusive",
        "AcquireSRWLockShared", "ReleaseSRWLockShared", "SleepConditionVariableSRW",
        "WakeConditionVariable", "WakeAllConditionVariable", "InitOnceExecuteOnce",
    )),
    ("Time", "clock_gettime/timerfd", (
        "QueryPerformanceCounter", "QueryPerformanceFrequency", "GetTickCount", "GetTickCount64",
        "timeGetTime", "Sleep", "SleepEx", "CreateWaitableTimerExW", "SetWaitableTimer",
        "CancelWaitableTimer", "timeBeginPeriod", "timeEndPeriod", "GetSystemTimeAsFileTime",
        "GetSystemTimePreciseAsFileTime", "SetThreadExecutionState",
    )),
    ("Files", "POSIX file API", (
        "CreateFileW", "CreateFile2", "ReadFile", "WriteFile", "SetFilePointerEx", "GetFileSizeEx",
        "FlushFileBuffers", "CloseHandle", "DeleteFileW", "MoveFileExW", "CopyFileW",
        "CreateDirectoryW", "RemoveDirectoryW", "FindFirstFileW", "FindNextFileW", "FindClose",
        "GetFileAttributesW", "SetFileAttributesW", "GetFullPathNameW", "GetModuleFileNameW",
        "GetModuleHandleW", "LoadLibraryW", "GetProcAddress", "FreeLibrary", "GetTempPathW",
        "GetDiskFreeSpaceExW", "LockFileEx", "UnlockFileEx",
    )),
    ("Registry", "config files instead", (
        "RegOpenKeyExW", "RegQueryValueExW", "RegSetValueExW", "RegCreateKeyExW", "RegCloseKey",
        "RegDeleteValueW", "RegEnumValueW",
    )),
    ("Window", "SDL3 window", (
        "CreateWindowExW", "RegisterClassExW", "DestroyWindow", "ShowWindow", "SetWindowPos",
        "GetClientRect", "GetWindowRect", "AdjustWindowRectEx", "SetWindowLongPtrW",
        "GetWindowLongPtrW", "DefWindowProcW", "PeekMessageW", "GetMessageW", "DispatchMessageW",
        "TranslateMessage", "PostQuitMessage", "SetCursor", "LoadCursorW", "GetMonitorInfoW",
        "MonitorFromWindow", "SetWindowDisplayAffinity", "SetWindowTextW", "GetDC", "ReleaseDC",
        "ChangeDisplaySettingsExW", "EnumDisplaySettingsW", "SetProcessDpiAwarenessContext",
        "SetProcessInformation",
    )),
    ("Input", "SDL3 gamepad", (
        "XInputGetState", "XInputSetState", "XInputGetCapabilities", "DirectInput8Create",
        "GetRawInputData", "RegisterRawInputDevices", "GetKeyboardState", "GetAsyncKeyState",
        "GetKeyState", "MapVirtualKeyW", "GetKeyboardLayout",
    )),
    ("Audio", "PipeWire/ALSA", (
        "CoInitializeEx", "CoCreateInstance", "CoUninitialize", "IMMDeviceEnumerator",
        "ActivateAudioInterfaceAsync", "CreateThreadpoolWait", "SetThreadpoolWait",
        "CloseThreadpoolWait", "CreateThreadpoolTimer", "SetThreadpoolTimer", "WaitForThreadpoolTimerCallbacks",
    )),
    ("Media", "FFmpeg/libvpx", (
        "MFCreateSourceReaderFromURL", "MFCreateMediaType", "IMFSourceReader", "MFStartup", "MFShutdown",
        "MFCreateAttributes", "MFCreateSample", "MFCreateMemoryBuffer", "MFGetStrideForBitmapInfoHeader",
        "MFSetAttributeSize", "MFCreateMediaTypeFromRepresentation",
    )),
    ("Console/diag", "POSIX equivalent", (
        "OutputDebugStringW", "OutputDebugStringA", "IsDebuggerPresent", "SetUnhandledExceptionFilter",
        "AddVectoredExceptionHandler", "RemoveVectoredExceptionHandler", "MiniDumpWriteDump",
        "GetLastError", "SetLastError", "FormatMessageW", "RaiseException", "GetCurrentProcess",
        "GetCurrentProcessId", "TerminateProcess", "GetEnvironmentVariableW", "SetEnvironmentVariableW",
        "GetCommandLineW", "CommandLineToArgvW", "WideCharToMultiByte", "MultiByteToWideChar",
    )),
]

CALL = re.compile(r"\b([A-Z][A-Za-z0-9_]{2,})\s*\(")
SOURCES = ("*.cpp", "*.h", "*.hpp", "*.c")


def runtime_files() -> list[Path]:
    out: list[Path] = []
    for folder in ("src", "include"):
        for pattern in SOURCES:
            out.extend(sorted((RUNTIME / folder).rglob(pattern)))
    return out


def scan() -> tuple[dict[str, Counter], dict[str, set[str]], Counter]:
    """Calls per subsystem, files per subsystem, and every call seen."""
    calls: dict[str, Counter] = {name: Counter() for name, _, _ in SUBSYSTEMS}
    files: dict[str, set[str]] = {name: set() for name, _, _ in SUBSYSTEMS}
    everything: Counter = Counter()
    api_to_subsystem = {api: name for name, _, apis in SUBSYSTEMS for api in apis}

    for path in runtime_files():
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        rel = path.relative_to(RUNTIME).as_posix()
        for name in CALL.findall(text):
            everything[name] += 1
            subsystem = api_to_subsystem.get(name)
            if subsystem:
                calls[subsystem][name] += 1
                files[subsystem].add(rel)
    return calls, files, everything


def table(calls: dict[str, Counter], files: dict[str, set[str]], top: int) -> str:
    lines = ["| Subsystem | Files | Calls | Replace with | Most used |", "|---|---:|---:|---|---|"]
    for name, replacement, _ in SUBSYSTEMS:
        c, f = calls[name], files[name]
        used = ", ".join(f"`{api}`" for api, _ in c.most_common(4)) or "—"
        lines.append(f"| {name} | {len(f)} | {sum(c.values())} | {replacement} | {used} |")
    total_files = len(set().union(*files.values())) if files else 0
    lines.append(f"| **Total** | **{total_files}** | **{sum(sum(c.values()) for c in calls.values())}** | | |")
    lines.append("")
    lines.append("### Files with the most Windows calls")
    lines.append("")
    weight = Counter()
    for name in calls:
        for path in files[name]:
            weight[path] += 1
    for path, count in weight.most_common(top):
        lines.append(f"- `{path}` ({count} subsystems)")
    return "\n".join(lines)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--top", type=int, default=15, help="files to list (default 15)")
    args = ap.parse_args()

    calls, files, everything = scan()
    print(f"<!-- generated by tools/port/win32_audit.py: {len(runtime_files())} runtime sources -->")
    print(table(calls, files, args.top))
    known = {api for _, _, apis in SUBSYSTEMS for api in apis}
    unknown = [(n, c) for n, c in everything.most_common() if n not in known and c >= 3]
    print()
    print(f"Other calls seen three or more times ({len(unknown)}): " + ", ".join(f"`{n}`" for n, _ in unknown[:25]))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
