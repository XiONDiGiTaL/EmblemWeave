# Find Units (#76): the stall, the blocked read, and the supervised helper

**Date:** 2026-11-28
**Code:** `apps/emblemweave/src/cheats.rs` (scan), `apps/emblemweave/src/scan_helper.rs` (supervision), `apps/emblemweave/src/pages/cheats.rs` (Find Units), `apps/emblemweave/src/main.rs` (helper entry point).

## 1. Reproduction

A disposable session was started from a copy of the recovered Chapter 4 save (`tmp/issue76/session`, ini points at the
copy; the source tree was never used). The game reached Dagson Base and stayed at 60 fps. An instrumented probe
(`cheats::live_probe::live_find_units_probe`, an `#[ignore]` diagnostic test) ran the **production** `scan_units`
against the live `fefw.exe` and recorded, for every read:

- the address, length and duration of the `ReadProcessMemory` call;
- a 2-second progress beat;
- when a read stayed in flight past 1.5 s, the calling thread's instruction pointer (the thread was suspended for the
  duration of `GetThreadContext`, then resumed);
- the game's `fps` lines from `fefw.log` over the same window.

The probe does not alter the game: it opens it read-only, and the scan is the same code the launcher runs.

## 2. What the measurements show

| Run | Configuration | Result |
| --- | --- | --- |
| Normal (8 consecutive runs) | product defaults: 2 GB fallback cap, 8 threads, 30 s deadline | **0.80 s each**, 182–184 reads, 2,147,480,992 bytes, **slowest read 10–15 ms**, read time 0.26–0.32 s (36–44 % of the scan), 60 fps with two frames at 55.8 |
| Uncapped (0.1.42 behaviour) | `max_fallback_bytes` raised to 64 GB, 8 threads | **31.4 s, timed out at 31.3 s**, 4,762 reads, **30.8 GB** read, **slowest read 20.247 s at `0x1bd46d000`**, 0 units, fps **6.9 and 6.4** |
| Cancel | 8 threads, cancel after 0.15 s | returned in 0.20 s, `cancelled=true`, zero reads, `scan_outcome` = “Scan cancelled by user” |
| Deadline | 1 thread, 0.2 s timeout | returned in 0.40 s, `timed_out=true` after 15 reads / 80 MB |
| Device loss | 1 thread, game terminated at 1.2 s | returned in 0.20 s, `device_lost=true`, `scan_outcome` = “The game closed during the scan; no partial results were used.” |

The uncapped run is the stall from the report, reproduced on this machine: the whole process is scanned (30.8 GB at
this point, because the launcher-side cap is what bounds it), the game drops to 6 fps for that period, and the scan
ends by timing out with no units — “never completes successfully, audio and visual stutter”.

### Where the worker waits

The monitor suspended the worker while a read was outstanding and read its instruction pointer:

```
BLOCKED addr=0x1bd46d000 len=50266112 in flight 20.038s;
  thread 36768: rip=0x7ffacc082534 ntdll.dll+0x162534 region_base=0x7ffacbf20000
BLOCKED addr=0x7600960000 len=35848192 in flight 8.183s;
  thread 10732: rip=0x7ffacc082534 ntdll.dll+0x162534 region_base=0x7ffacbf20000
```

- `ntdll.dll+0x162534` is the syscall stub (`NtReadVirtualMemory`); `region_base` is ntdll's image base.
- The first address is below the guest range (fallback pass), the second is inside it (`0x76…`, the mapped region).
  A read can block for seconds in either pass.
- `ReadProcessMemory` has no timeout or cancellation parameter, so the 30 s deadline and the Cancel button are only
  observed **between** reads. In this run a worker spent 20 of the scan's 31 seconds inside one call.

## 3. The supervised helper

`ReadProcessMemory` cannot be interrupted, so the scan now runs in a process the launcher owns and can terminate.

- **Single-use child:** `EmblemWeave.exe --scan-units-helper <pid> <timeout_secs> <cap_bytes> <threads>`. `main`
  dispatches it before any GUI or configuration work.
- **Job Object:** the child is assigned to a job with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The launcher reads JSON
  progress and the final address list from the child's stdout; it never calls `ReadProcessMemory` itself.
- **Hard stop:** Cancel, the deadline (plus one second of grace), a dropped scan or a panic terminate the job. The OS
  releases the child's process handle and the partially copied pages, and the launcher returns regardless of what the
  child was doing — including a `ReadProcessMemory` that never returns.
- **Read-only:** the child opens the game with `PROCESS_VM_READ | PROCESS_QUERY_INFORMATION` rights through the same
  `Process` type; it never writes. Killing it cannot corrupt the game, the save or the save volume.
- **Fallback:** if the helper cannot be started (no executable, restricted machine), the launcher runs the in-process
  scan exactly as before, with the uninterruptible-read behaviour. The UI and result handling are unchanged.

## 4. Tests against the real game

All tests are `#[ignore]` and require a running `fefw.exe` plus `EW_HELPER_EXE` pointing at the built launcher:

| Test | Result |
| --- | --- |
| `supervised_scan_matches_the_in_process_scan_and_reaps_the_helper` | **PASS** — direct and supervised scans returned the same address list (empty in the field at that time), supervised took 7.25 s, a second scan started immediately, no helper left running |
| `cancel_mid_scan_terminates_the_helper_promptly` | **PASS** — cancel at 1.0 s returned in 1.05 s with `cancelled=true` and no helper left |
| `the_deadline_kills_a_helper_that_is_stuck_in_a_read` | **PASS** — 0.5 s deadline returned in 1.44 s (deadline + grace) with `timed_out=true`, 146/956 pieces |
| `the_game_exiting_mid_scan_is_device_lost_and_reaps_the_helper` | **PASS** — the game was terminated at 1.0 s; the scan returned in 1.29 s with `device_lost=true`, 115/957 pieces, no helper left |

After every run, `tasklist` showed no `EmblemWeave.exe` child: the job is reaped inside the call.

## 5. Limits of this evidence

- The session was Dagson Base, not a skirmish battle; the reporter's battle state was not available. The blocked read
  is reproducible in the field with the cap removed, so it is not battle-specific, but the in-battle unit list was not
  observed. The field returned zero unit records in the final session (one earlier), so the helper's result is
  validated by equality with the in-process scan, not by a non-empty unit list.
- The 20 s read was observed in one uncapped run; other uncapped runs finished in ~3 s. Whether a read blocks depends
  on what the game is doing with that page, which is exactly why the supervisor must not rely on the read returning.
- Windows only, x86-64 (the page is Windows-only). The POSIX path returns a disconnected scan as before.
- The in-process fallback keeps the old behaviour when the helper cannot start; that path is what the earlier audit
  described.

## 6. What changed in the product

- Find Units now calls `scan_helper::scan_units_supervised`; Cancel and the 30 s deadline now **return**, even from a
  read that does not.
- The launcher-side scan code, result filtering (`scan_outcome`) and unit list are unchanged.
- `Cargo.toml` gains the `Win32_System_JobObjects`, `Win32_System_Kernel` and `Win32_Security` windows-sys features.
