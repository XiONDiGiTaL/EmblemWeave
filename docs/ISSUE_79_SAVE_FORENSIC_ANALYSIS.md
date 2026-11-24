# Issue #79: Save Data Forensic Analysis (two independent reports)

Status: **open — one dangerous failure path is now guarded, but neither reporter's
exact lost save tree is available, so a single root cause is not proven.**

This document replaces the earlier single-report conclusion that the original
reporter had accidentally deleted a profile. That event is in the bundle, but it
does not establish where the missing save was, and the second reporter never
mentions a profile deletion.

## 1. Reports examined

| | Original report (`KalHawke`) | Second report (`msalgadob`) |
|---|---|---|
| Attachment | `emblemweave-report-1795382099629.zip` | `emblemweave-report-1795483280940.zip` |
| SHA-256 | `3a3ee40cc428d842a1ed56760bf47fe06a7b2db646b2f5776daa4c2cabef7696` | `3500f822ce8f0a5f0c67e273e0dc3de9021c8f73bcef1cef81ec22867d6a0e5b` |
| Generated | day 20779, 21:14:59 UTC | day 20781, 01:21:20 UTC |
| Version | `0.1.41 (a56d1e2)` | `0.1.41 (a56d1e2)` |
| Platform | Windows x86_64 | Windows x86_64 |
| Data root | `G:\Games\EmblemWeave-v0.1.40\data` (portable) | `D:\switch\EmblemWeave-v0.1.41\data` (portable) |
| `fefw.ini` `save_dir` | `G:/Games/EmblemWeave-v0.1.40/data/save` | `D:/switch/EmblemWeave-v0.1.41/data/save` |
| Launcher save root | same folder | same folder |
| Runtime log save root | same folder | same folder |
| `[account] uid` in runtime | `(default)` → fixed `0100000000000000706d635257464546` | same |
| Title / save owner | `04007EF01F8AE000` | `04007EF01F8AE000` |
| Game data | extracted `131072` update; build matches | `.nup` `131072`; build matches |
| `profiles` in launcher config | `[]` | `[]` |
| `profile` | `""` | `""` |
| `slot_names` | `{}` | `{"test": {}}` |
| `profile_portraits` | `{}` | `{"test": "theodora"}` |
| Notable launcher activity | profile `First Run 0.1.41` moved to backups at 21:09:52 | theme changes, report generation; no save/profile action |
| Save tree in ZIP | **no** | **no** |
| Runtime mount/commit lines | **none** | **none** |

Both ZIPs contain only `report.txt`, `environment.txt` and `config.json`, plus
the launcher's logs and run-folder files. Neither contains the save tree, so the
contents of `account/0100000000000000706d635257464546/000` at the time of the
report are unknown.

### What the evidence rules out for these two reports

- **A launcher/runtime save-root mismatch.** The configured root, the
  `fefw.ini` root and the runtime's own `fs: fsp-srv ... save data in ...` line
  agree in both reports.
- **A wrong user or title id.** Both runtimes resolve the built-in user
  (`uid` empty, nickname `Player`) and title `04007EF01F8AE000`; the runtime's
  fixed account path is the one the launcher lists as expected.
- **A 0.1.40 → 0.1.41 save-path regression.** The 0.1.40 → 0.1.41 runtime
  `fs.cpp` change is a one-line RomFS condition; save-path and account handling
  are unchanged. Both reports are on 0.1.41 and both logs show the expected
  root.
- **An interrupted journal commit visible in the log.** The old runtime logged
  commit only at debug level and logged successful mounts not at all, so the
  absence of those lines is not evidence that no commit happened. It is also not
  evidence that one did.

### What the evidence does not establish

- Whether each reporter's `000` folder was populated, empty, missing, or only
  present under a profile/backup.
- Whether the save was written but never committed, or committed and later
  moved/removed.
- That the original reporter's profile move caused the loss. The move is real
  (`backups/1795381792-profile-First Run 0.1.41`), but the account save is a
  separate folder; the bundle does not show whether the playthrough was in that
  profile, in `account`, or in both.
- That the two reports share one root cause. They share the symptom and build,
  not a demonstrated cause.

The earlier document's claim of a confirmed accidental profile deletion is
therefore **downgraded to a possible event in one report**, not a proven cause.

## 2. Save lifecycle and the confirmed code gap

The intended path is:

```
game save -> runtime commit -> <save_dir>/account/<uid>/000/<slot>
          -> .meta beside <uid>
          -> launcher reads the same account folder
          -> Play reconciles fefw.ini
          -> runtime mounts <save_dir>/account/<uid>
          -> game enumerates slots
```

Two behaviors made a missing/renamed account folder look like data loss:

1. **The runtime silently created an empty account save.** When `account/<uid>`
   was absent, `open_save()` called `ensure_account_save()` and the game started
   from zero. The old log line existed, but no mounted root, file count or
   namespace state was recorded in a normal report.
2. **The launcher offered no preflight recovery.** `sync_save_dir()` carried a
   save from another launcher data root, but if the current account folder was
   missing while local profiles/backups held the only copies, Play still
   proceeded and the game created a new save. The Saves page could show those
   folders, but nothing blocked the empty launch.

A related safe case was tested: a populated `000` with its `.meta` missing. The
runtime's old code treated it as absent and logged “creating”, while preserving
the `000` files. The new runtime repairs only the missing metadata, logs a
warning, and mounts the same 12 files. This is not destructive, but the old
wording was misleading and is now explicit.

## 3. Changes implemented

### Launcher

- `saves::account_meta_path()` and `saves::ensure_account_meta()`:
  write the missing `.meta` for a populated account save before Play. Existing
  metadata is never replaced; no slot file is touched.
- `saves::recoverable_saves()`: inventory of valid profiles, profile backups,
  whole-save backups and orphan saves with at least one slot file.
- `saves::empty_namespace_blocker()`: refuses Play when the account folder is
  **completely missing** and recoverable saves exist. The message names the
  missing folder and the recoverable saves and sends the player to Saves. An
  existing account folder (including a meta-only one) is left to the game, so a
  deliberate new game is still possible.
- `saves::save_manifest()`: bounded save-tree listing for reports — filenames,
  sizes, timestamps, SHA-256 for files ≤ 1 MiB, metadata, profiles, backups,
  recoverable saves and the tail of the runtime's save log. Save contents are
  not included.
- Saves page: prominent “No current save” warning listing recoverable
  profiles/backups before the carousel.
- `engine::sync_save_dir()`: repairs missing metadata before reconciliation.
- `engine::play()`: applies the empty-namespace guard after reconciliation.
- `report::bundle()`: writes `saves.txt` with the manifest next to
  `report.txt`.

### Runtime

- `fs.cpp`: when an account `000` has files but `.meta` is missing, writes the
  metadata and logs
  `... has N files in 000 but no .meta; wrote the missing metadata and kept the files`.
- `fs.cpp`: every successful open logs
  `save data opened: <key> at '<absolute root>' (N files in 000, meta present)`;
  a missing/unmounted save logs the absolute root and empty-namespace warning;
  account creation logs the absolute root and that the game starts with no
  progress.
- `fs_volume.cpp`: successful commits log at normal level
  (`save commit <root>: N changes applied`); journal, marker and apply failures
  log errors instead of returning silently.

No save format, profile layout, `DEFAULT_USER`, title id, `.meta` schema or
backup/restore behavior was changed.

## 4. Reproduction and verification matrix

| Scenario | Result | Evidence |
|---|---|---|
| Fresh ZIP/portable install, no saves | Play is not blocked; runtime may create the first empty account | unit test for no recoverable saves; code path inspected |
| Portable install with a valid save | **Pass** — game showed Continue after a fresh process start | disposable fixture, 12 files, `release2` runtime |
| ZIP install with a valid save | Not separately reproduced; same portable data-root logic | fixture used a portable layout |
| Update from an older version | Not reproduced | 0.1.40→0.1.41 runtime save code diff reviewed; existing reconciliation tests |
| Restart after a save | **Read path pass** — save mounted again after killing/relaunching the process | fixture logs and Continue screen |
| Restart after days | Not simulated | no clock-dependent save logic found |
| Multiple profiles | Pass in library tests | `profiles_store_and_load`, profile backup/restore tests |
| Autosave moved to backup and restored | Pass in library tests | slot archive/restore tests |
| Missing `.meta`, populated `000` | **Pass** — repaired, 12 files preserved, game mounted the save | runtime log + unit test |
| Missing save directory / empty namespace with recoverables | Guard and warning added; library test passes | `recoverable_inventory_and_empty_namespace_guard` |
| Launcher/runtime mismatch | Existing reconciliation tests pass; not reproduced from a reporter | `sync_save_dir`/`carry_save_across` tests |
| Interrupted startup/shutdown | Existing journal recovery tests pass; no new corruption path | fs volume tests/build |
| Actual new save created in gameplay, then relaunched | **Not tested** | requires driving a new playthrough; fixture was used read-only for this check |

### Commands actually run

- `cargo check -p emblemweave --locked` — passed.
- `cargo test -p emblemweave --locked saves::tests` — **45 passed, 0 failed**.
- `cmake --build game-build/fefw/build/release2 --target fefw` — passed after
  fixing a `to_u8` compile error in the new log line.
- GUI fixture: launched the rebuilt `release2/fefw.exe` with `--ini` pointing at
  a disposable copy of the real save. The title screen showed **Continue**;
  the log recorded the absolute account path and `12 files in 000, meta present`.
- Removed the fixture `.meta`, relaunched: log recorded the 12-file repair and
  the save files remained byte-identical.

### Limits

- The reporters' exact environments (their original `G:`/`D:` folders) are not
  on this PC.
- The 0.1.41 diagnostic bundles have no save tree and no successful-mount line,
  so the loss cannot be reconstructed from them.
- The launcher GUI banner and Play guard were verified by library tests and
  compilation; the Saves page itself was not clicked through with a fabricated
  missing-account profile. A new gameplay save was not created and reloaded.
- These are reported as limitations, not as complete end-to-end verification.

## 5. Recovery instructions

Use the installation that owns the save folder. A different portable copy has a
different `data\save` and cannot see the first copy's files.

1. Open **Saves**. The new banner says when the current account save is empty and
   names recoverable profiles/backups.
2. If a profile is listed, select it and **Load Profile**. The current save is
   moved to `backups` first; the profile is copied in; the installed tree is
   verified.
3. If a profile backup is listed, use **Restore Profile**. It refuses to
   overwrite a profile with the same name.
4. If a whole-save backup is listed, use **Restore**. The save it replaces is
   moved to `backups` first.
5. If the launcher says the save folder is missing while recoverable saves
   exist, Play is refused deliberately. Restore/load one, then Play.
6. If `.meta` is the only missing file, Play repairs it and keeps `000`.
7. Never delete the backup folder until the game has loaded the restored
   progression and a restart has preserved it.

The new `saves.txt` report manifests show the account path, `000` file list and
hashes, profiles/backups and the runtime's last mount/commit lines. Ask a
reporter for a new bundle after the failing launch; that is the evidence needed
to distinguish a missing account folder from an uncommitted write or a rejected
slot.

## 6. Git and deployment record

| Item | Value |
|---|---|
| Branch | `main` |
| Base HEAD | `ea4a7adc` |
| This change | `42d9a612` |
| Launcher tests | `cargo test -p emblemweave --locked`: 424 passed, 0 failed; `saves::tests`: 45 passed |
| Runtime build | `release2/fefw.exe` (working) and clean `target/issue79/clean-runtime/fefw.exe` (deployment), both built |
| Deployed launcher SHA-256 | `b6162725a5450a874a046f4da790e73fe3e27eca979693bac628f034533a43f7` |
| Deployed runtime SHA-256 | `8a2d0af5392ae8aa93141c4515d5ef5e21dadf5b839286b7dc966f50e8a77ef6` |
| X: backup | `X:\EmblemWeave-backup-20261123-2130` (8.707 GB; robocopy 104,598 files, 0 failed) |
| X: version after deploy | launcher `0.1.42 (42d9a612)`, Saves page opened |

Issue #79 remains **open**. Do not close it from this work: the reporters have
not confirmed recovery, and the exact cause of their lost saves is still
unproven.
