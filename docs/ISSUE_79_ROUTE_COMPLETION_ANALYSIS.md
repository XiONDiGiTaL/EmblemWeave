# Issue #79: route completion and the save lifecycle

**Baseline:** `fbcfe2c6cb8b13b84c53f422a3e46ed19b16ed9f` (`main` = `origin/main`, `v0.1.42-161-gfbcfe2c6`).
**Investigated:** 2026-11-28.
**Scope:** the third report (`sp-r-Samson`: "stopped playing after completing a route"), the runtime save
lifecycle at route completion, and whether the stamped-slot recovery confirmed for Reports A/B covers it.

This document separates what was measured from what is inferred. Nothing here was run against a reporter's
personal save; every fixture is a disposable copy under `tmp/issue79/`.

---

## 0. Short version

1. **Reports A/B are explained and fixed.** A slot moved back from `backups/` keeps the launcher's
   `<stamp>-` prefix (`1795325701-Auto`); the game opens only `Auto`. Recovery copies a verified stamped
   file to its game name, never overwriting a valid slot and never removing the original. A real
   `otherdata.zip` recovered this way loads Chapter 4 in-game on the current runtime.
2. **No route-specific save path exists in the runtime.** The game opens one account save at boot and
   writes it through the journalled volume; `Route_*` is an ordinary container file in the same folder.
   Nothing switches namespace, account or save id at route completion.
3. **Two defects were found in the surrounding code and fixed:** the cross-process save-folder lock that
   the 0.1.43 platform port silently dropped (`9013b800`), and stamped-slot promotion that was not atomic
   and skipped `Route_*`.
4. **Report C is not resolved.** It has no bundle and no sequence. The one route-shaped gap that could be
   reproduced from the archive was `Route_*` recovery, and it is now fixed. Whether Report C is that gap,
   an uncommitted write at exit, or something else cannot be decided without the runtime log and save
   manifest requested in §7.

---

## 1. What the runtime does with saves

Source: `engine/fefw-recomp/runtime/src/services/fs_volume.{h,cpp}`, `services/fs.cpp`.

- The account save (`save/account/<titleid><userid>/000`) is opened once at boot and mounted as a
  journalled volume. The committed folder is never written outside `commit()`.
- A write creates a copy-on-write file in `<root>.work.<pid>/`; a delete records a tombstone. Readers see
  the work folder over the committed folder minus tombstones.
- `commit()` is two-phase: flush the changed files and write `.journal.<pid>`, rename it to `.commit`
  (the decision), then apply deletions/folders/file moves and remove the marker. A process killed before
  the rename leaves the committed state untouched; one killed after it is completed by the next open
  (roll forward).
- A successful commit is logged at INFO: `save commit <root>: N changes applied: <names>`. Each failure
  stage logs `NXLOG_E` (`writing the journal failed`, `making the decided marker failed`, `applying the
  decided marker failed`, `commit: moving … failed`). The runtime does not fail silently.
- Uncommitted work is dropped when the volume is destroyed unless a decided marker exists — the same as
  unmounting a save on the console without `Commit`. The runtime never commits on the game's behalf.
- `IMultiCommitManager::Commit` forwards `Commit` to each added file system, so a game that batches saves
  still ends in the same two-phase commit.

Consequence for route completion: the route-completion save is durable when, and only when, the game calls
`Commit`. If the game is closed between writing and its commit call, the previous committed state is what
remains — the runtime cannot distinguish that from "the save was not read". Only the log
(`save commit …`) and the pending `.work.`/`.journal.` files show which happened, which is exactly what
the diagnostic manifest now reports.

## 2. Route completion does not change the save target

Measured on the Report B playthrough (`tmp/issue79/route-test`, a copy of the recovered Chapter 4 save):

- One account folder: `0100000000000000706d635257464546/000`, `.meta` present.
- Ten files in `000`: `Auto`, `Chapter_Ism_0_2`, `Chapter_Ism_0_3`, `Chapter_Led_1_1` … `Chapter_Led_1_4`,
  `Route_Ism`, `Global`, plus the recovered `1795325701-Auto`.
- `Route_Ism` (29,546 bytes) has the same container header as the slots (`dd bc ef cd e0 00 00 00 …`).
  It is a save container the game writes itself, not a launcher file.
- No second account id, title id or user id appears at any point; the runtime log shows one
  `save data opened: account/0100…` per open, with `(10 files in 000, meta present)`.
- The save tree was byte-identical after the in-game run (23 files hashed before and after).

So route completion does not create a namespace or a new folder: it writes the same account save. The
save the game offers as **Continue** after a route is the same `Auto`/chapter set.

## 3. Defect found: the save-folder lock was dropped in the platform port

Commit `9013b800` (0.1.43 platform port) rewrote `Volume`'s `RootLock` to `named_mutex_create` only. On
Win32 that call *creates* the mutex (or opens it) and does not wait; the pre-port code held the folder
with `WaitForSingleObject`. Two runtimes pointed at one save folder could therefore apply the same
decided `.commit` marker at once — the exact window in which a save is being applied, and the window a
launcher preflight or a second instance can overlap.

Fix: the platform layer exposes explicit `named_mutex_lock`/`named_mutex_unlock`
(`platform.h`, `platform_win32.cpp`, `platform_posix.cpp`); `SaveFolderLock` takes the lock for its scope
and logs when it cannot. `named_mutex_create` keeps its "already exists" semantics because
`sched.cpp` relies on them.

Regression test: `svc_fs_volume.save_folder_lock_waits_for_another_process` holds the folder lock in one
process and starts a second copy of the test binary that tries to take it. The child must not write its
marker until the parent releases the lock. Verified both ways:

| Variant | Result |
| --- | --- |
| `SaveFolderLock` with the wait (current code) | PASS — child blocked for 600 ms, ran only after release |
| `named_mutex_lock` call skipped (pre-port behaviour) | FAIL — `!stdfs::exists(marker)` at 600 ms |

## 4. Defect found: stamped `Route_*` was never recovered

`kind_of()` classifies names by prefix: `Auto`, `Chapter*/Manual*/Save*`, everything else `Other`.
`Route_<id>` is `Other`, so `repair_backup_named_slots`/`recover_stamped_slot` skipped it. A player who
moved a whole `backups/<stamp>/000` back by hand (the Reports A/B mechanism) therefore got `Auto` and the
chapters back but left `1795325701-Route_Ism` unread — the autosave loaded, the route file did not.

Fix: `is_recoverable_slot()` admits `Route_*` alongside `Auto` and the manual saves, still gated by
`is_slot_candidate` (no extension, not `Global`, not metadata) and by the same container parse, checksum
and native zstd verification. `Global` remains excluded (different format).

## 5. Defect found: promotion was not atomic

`recover_stamped_slot` copied straight to the final name. An interruption mid-copy left a truncated
`Auto`, which the next Play would move to `backups/` before the stamped original could be tried again.
Recovery now copies to `<slot>.emblemweave-recovering` and renames it into place; a failed copy or rename
removes the temporary and returns the error, and Play refuses to launch rather than opening an empty game.

## 6. Validation performed

| Check | Result | Evidence |
| --- | --- | --- |
| Save and recovery unit tests | **PASS** — 75 passed, 0 failed | `cargo test -p emblemweave --locked saves::tests` |
| Save volume tests | **PASS** — 14 passed, 0 failed | `nxrt_tests svc_fs_volume` (includes four crash/recovery cases) |
| Platform self-test (new lock API) | **PASS** — 0 failures | `nxrt_platform_selftest.exe` |
| Cross-process lock exclusion | **PASS**, and fails without the wait | §3 table |
| Recovered Chapter 4 save loads in-game, final build | **PASS** | Continue → “Part I: Leda's Path Chapter 4 / The Indelible Past / 3/1/1449 Dagson Base” → Yes → Dagson Base field, objective “Register for the Heroic Games”; log `save data opened: account/0100… (10 files in 000, meta present)`; no create line |
| Fixture unchanged by the load | **PASS** — 23 files byte-identical | `final/before.sha` = `final/after.sha` |
| Route completion played through | **NOT TESTED** — requires finishing a route from a save with the route's remaining battles; not feasible in the available environment | — |
| Save written after route completion, then restart | **NOT TESTED** on the current runtime | the in-game save menu could not be opened by the scripted pad (`PLUS` opens the Base Map; `A`/`X`/`MINUS` did nothing); only the pre-port runtime has a committed in-game save (`save commit …: 3 changes applied`, Nov 26) |

Build used for the in-game check: `engine/fefw-recomp/game-build/fefw/build/release/fefw.exe`,
170,863,616 bytes, SHA-256 `174202632b0bc76727f8229855b321ac8b1cd297f5636eab04295e8540899b66`.
Runtime tests built in `engine/fefw-recomp/runtime/build/release3` (`NXRT_BUILD_TESTS=ON`).

## 7. What Report C needs

Report C supplied no bundle and no sequence, so none of the table's outcomes can be attributed to it. The
next report should include the Saves page's **Save report** (`--save-manifest`), which now prints:

- the effective save root and account namespace;
- `.meta` presence, and each slot's name, size and `parse=`/`verify=`/`native=` verdicts;
- the `commit state:` section (`.commit`, `.work.`, `.journal.` files), which says whether a save was
  left decided or uncommitted;
- the journal/marker state and the recovery note Play prints when it promotes a stamped file.

Plus the Game Log's last `save data opened` and `save commit` lines, and the sequence: was the save made
by a manual save, the autosave, or at a chapter/route end; how the game was closed (menu, window close,
Alt+F4, crash); whether Continue was offered and what the slot card said.

## 8. Determination

**Partially resolved.** The confirmed stamped-slot defect is fixed, covered by tests, and demonstrated
in-game. Two further defects on the same path (lock regression, non-atomic promotion) and one
route-shaped gap (`Route_*` recovery) are fixed and tested. Report C is **unresolved**: the runtime has no
route-specific save path, so the report is most likely one of (a) the `Route_*`/stamped recovery gap now
fixed, (b) an exit before the game's `Commit`, or (c) a cause that needs the manifest and log to identify.
Closing #79 on Reports A/B alone would be premature.
