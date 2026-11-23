# Session 2026-11-23: timed waits, log volume, and current-run errors

This session followed the kernel-sync deadlock fix (`c8bb70ea`). It addressed
four concrete defects found by inspecting the running engine and the launcher.

## 1. Timed kernel waits slept instead of waiting

`engine/fefw-recomp/runtime/src/kernel/kernel_sync.cpp`

`block_on_wake` handled a finite timeout by unlocking the kernel lock, calling
`sleep_ns`, and locking again. The thread was not on its condition variable, so
a `wake_thread` during the sleep was only observed after the whole timeout. A
stale `TODO: Add timed cond_wait to platform layer` marked the path.

The timeout path now uses the real `platform::cond_wait_ms(thread_cv(t), lock, ms)`.
The loop still re-checks the deadline and waits again after a spurious wake.
Infinite waits keep `cond_wait`.

Expected effect: a timed wait returns when it is signaled rather than at its
deadline. No frame-time improvement is claimed from the single sample below.

## 2. The same handler installed once per NVN pool was logged every time

`engine/fefw-recomp/runtime/src/mem/write_watch.cpp`

Every region creation logged "installing page fault handler" and "installed
successfully". The handler is process-wide and `page_fault_handler_install` is
already idempotent; the first run had **183 pairs in one 73 s log** (566 lines,
27% of the file). Both lines are now emitted once, on the first successful
installation. A failure still logs on every attempt.

Measured: 183 pairs before, **1 pair** in a 200 s run after.

## 3. POSIX disabled write tracking after the first region

`engine/fefw-recomp/runtime/src/platform/posix/platform_posix.cpp`

`page_fault_handler_install` returned `false` when the SIGSEGV handler was
already installed. `region_create` treats `false` as "no write tracking", so on
Linux only the first region was tracked. It now updates the handler and returns
`true`, matching the Windows implementation. Not exercised on Linux hardware in
this session; verified by code inspection and the Windows build.

## 4. The launcher quoted a previous run's crash

`apps/emblemweave/src/engine.rs`

`watch_first_frame` read the whole `fefw.log` on a non-zero exit, so a quick
exit could show `Last line: [6.1 ... tid 14484 ...] crash: ...` from an older
run. (That is exactly how the 06:19 pre-fix crash report reappeared after the
fixed build had already run.)

`last_error_since(log, from)` now reads only the bytes written since this
launch; a log that became shorter is read whole. `last_error_text` holds the
filter. Two regression tests cover the newest-failure selection and the
previous-run case.

## Verification

| Check | Result |
| --- | --- |
| `cmake --build . --target fefw --parallel 16` | exit 0 |
| Rebuilt engine, 48 s run, mod enabled | 60 fps, one region announcement, no watchdog/crash |
| Rebuilt engine, 200 s soak, mod enabled | 60 fps through the end, one region announcement, no watchdog/crash, no `fefw_crash.txt` |
| `cargo test -p emblemweave` | 411 passed, 0 failed, 12 ignored |
| Clean release build of the launcher (`git archive HEAD` + this change) | 3m12s, exit 0 |
| Deployed launcher `EmblemWeave.exe --verify-log <log>` | exit 0, correct first/last error output |

The 155 s pre-fix log was 2121 lines / 323,464 bytes; the 200 s post-fix soak
was 1821 lines / 321,441 bytes, with the region lines down from 566 to 2. The
line rate fell from 13.7/s to 9.1/s. Scenes differ, so this is a log-volume
measurement, not a frame-time benchmark.

The game-thread system-call wait median was 1.10-1.12 ms in both logs; the p99
sample moved from 2.10 ms to 2.91 ms in one non-controlled run. No claim of a
frame-time improvement is made; the change removes a polling sleep and makes the
wakeup immediate, and no 60 fps regression was observed.

## Deployment

- Engine source commit: `f1296fe4` (plus the prior deadlock fix `c8bb70ea`).
- `E:\EmblemWeave\data\play\fefw.exe` SHA-256
  `78a577034427c5c62427d51863be40f9af7c4b170f9f8056d46d863a0780b015`;
  backup `fefw.exe.pre-logsync-20261123-071036`.
- Launcher `E:\EmblemWeave\EmblemWeave.exe` SHA-256
  `81d7f07a6dc5d69d859df77646c04375d15f8216979c9bd2ceda6fa04da2c32b`;
  backup `E:\EmblemWeave\backups\pre-session-20261123-071714\EmblemWeave.exe`
  (SHA-256 `870c3ecded63f927759ea5056ef23c45139161d188ed1010d4e31a06d4dc4197`).

The installed launcher was built from `HEAD` plus `engine.rs`, deliberately not
from the dirty tree: FCC's uncommitted `controls.rs` (controller artwork) and
`updater.rs` (multiple previous versions) were left untouched. The engine's
earlier 06:19 crash report is preserved as
`game-build/fefw/build/release/fefw_crash.old-20261123-0619.txt`.

## Remaining

- POSIX write tracking and the 48 s / 200 s runs are Windows-only evidence; a
  Linux run is still needed for the POSIX fix.
- The timed-wait change should be A/B tested on one fixed scene before any
  performance claim is published.
- FCC's controller-artwork and multi-backup changes remain uncommitted in the
  working tree.
