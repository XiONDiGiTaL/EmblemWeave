# Issue #79: Save Data Forensic Analysis (two independent reports)

Status: **both diagnostic bundles and msalgadob's `otherdata.zip` were examined on
2026-11-26. One concrete defect explains the observable failure in both reports
and is fixed; msalgadob's four-hour playthrough was rebuilt from the archive in a
separate copy and mounts in the runner. A scripted-pad run then listed it as
**Continue**, showed the Chapter 4 slot and loaded it into Dagson Base (0.7); a
stamped-only control offered no Continue.**

## 0. Update 2026-11-26: the archives, the cause, and the recovery

### 0.1 The archives

| Archive | SHA-256 | Contents |
| --- | --- | --- |
| `emblemweave-report-1795382099629.zip` (KalHawke) | `3a3ee40c…cabef7696` | `report.txt`, `environment.txt`, `config.json`, `game/{fefw.ini,fefw.log,controls.ini}`, the launcher logs; **no save tree** |
| `emblemweave-report-1795483280940.zip` (msalgadob) | `3500f822…67d6a0e5b` | the same shape; **no save tree** |
| `otherdata.zip` (msalgadob) | `17302107…852ef45` | 33 files, 1.2 MB: the `account` folder, its `.meta`, and the `backups`/`profiles` contents flattened into one folder |

`otherdata.zip` is original user data. Every examination below ran on a copy
(`tmp/issue79/…`, not committed); the extracted tree's SHA-256 manifest is
identical before and after the work.

### 0.2 KalHawke

* Build 0.1.41 (`a56d1e2`), portable data root `G:\Games\EmblemWeave-v0.1.40\data`.
* `game/fefw.ini` has `[fs] save_dir = G:/Games/EmblemWeave-v0.1.40/data/save`, and
  `game/fefw.log` records `save data in 'G:/Games/EmblemWeave-v0.1.40/data/save'`:
  the launcher and the runtime agreed on the root. No stale-root or wrong-user
  cause applies to this report.
* `game/fefw.log` (a 146-second session, ended from the F1 menu) contains **no**
  `no such save data` line and **no** `creating the account save data` line. On
  0.1.41 those were the messages of the empty-save path, so the runtime found an
  account save and mounted it. (0.1.41 logged mount failures, not successes, so
  the successful mount itself is not in the log.)
* The launcher activity shows the profile `First Run 0.1.41` moved to
  `backups/1795381792-profile-First Run 0.1.41` at 21:09:52, then the game started
  at 21:10:04 and presented its first frame at 21:10:11. A profile is a copy; moving
  it does not touch the account save.
* The bundle cannot show the contents of `000` at that launch: 0.1.41 had no save
  inventory, and the report was generated after the user had already moved Auto
  into and back out of the backups folder. It therefore cannot prove the original
  state, only rule causes out.

The operation the reporter describes — Auto moved into the backups folder and
back — is exactly the operation that produces a stamped file (0.4). The bundle is
consistent with that mechanism and rules out a root, UID or title mismatch, but
the tree that would prove it was never attached.

### 0.3 msalgadob

| Path in the archive | Size | What it is |
| --- | --- | --- |
| `0100000000000000706d635257464546.meta` | 191 | canonical metadata: `size=67108864`, `journal=67108864`, `owner=4007ef01f8ae000`, the UID at `attr[8..24]`, `type=1` — the layout `meta_text()` writes |
| `0100…/000/1795325701-Auto` | 33,520 | the **only** file in the live account save; SHA-256 `2a3011e1…b2d4ca8`; the valid slot header `ddbc efcd` |
| `1795393583/000/` | 8 files | the four-hour session: `Chapter_Ism_0_2`, `Chapter_Ism_0_3`, `Chapter_Led_1_1`…`1_4`, `Route_Ism`, `Global`; written 2026-11-22 00:05–02:33 local |
| 15 × `<stamp>/000/1795325701-Auto` | 33,520 each | repeated recovery attempts, each a whole-save backup of an account that held only the stamped autosave |
| `1795655727/000/{Auto, Global}` | 198,200 + 64 | a later playthrough, 2026-11-25 22:15 local, after the user deleted profiles/accounts/backups and started again |
| `1795655769-profile-test`, `1795655767-profile-test2` | | profile backups made while trying to recover |

Three facts settle the case:

1. The live account folder had **no `Auto`**. It had one file whose name is the
   backups stamp plus the slot name (`1795325701-Auto`), so the game could not open
   an autosave and offered no Continue.
2. The four-hour playthrough was not lost: all its slots were in
   `backups/1795393583/000`, made when the user restored or loaded something. The
   launcher's own backup-first behavior preserved it.
3. The stamp `1795325701` is 2026-11-22 02:35 local, three minutes after the
   autosave's own write time (02:32) — the moment the launcher renamed it.

Timestamps inside the archive are the user's local time (UTC−3); the epoch in a
folder name is UTC, which is why `1795393583` reads as 2026-11-23 00:26 UTC.

### 0.4 The cause

`saves::archive_slot` — the Saves page's **Move to backups** — renames
`account/<uid>/000/Auto` to `<save>/backups/<stamp>-Auto`. Moving that file back
into `000` by hand keeps the stamped name, and the game opens only `Auto`, so the
volume mounts, the game enumerates no autosave and starts as if the save were
empty. The launcher listed the stamped file but had no way to promote it, and
neither 0.1.41 nor 0.1.42 copied it to the name the game reads. That is the whole
failure: no save format, `.meta`, UID, title-id or journal defect is involved.

msalgadob's archive is that state, on disk, with the missing playthrough sitting
in `backups`. KalHawke's report describes the same move; his bundle cannot show
the resulting tree, but it rules out the other candidates.

### 0.5 The fix (in the same tree as this document)

* `saves::stamped_slot_name` recognises `<stamp>-<slot>`, and `kind_of` reports it
  as the slot it names, so the Saves page lists it instead of hiding it.
* Before Play, `saves::repair_backup_named_slots` copies every readable stamped
  slot (`Auto` and manual chapter saves only) to its own name. The stamped file
  is never renamed, moved or deleted; an existing slot always wins; a file that
  fails `Slot::verify` or the native zstd decode is never promoted.
* The Saves page shows **Recover** beside such a file and keeps a note of what it
  did. `EmblemWeave.exe --recover-saves` runs the same repair without the window.
* `--import-save`, Restore and Load keep their backup-first behavior, so the
  account that is replaced is still in `backups/`.

### 0.6 Recovery of msalgadob's playthrough (on a copy)

1. A fresh portable fixture root was filled from `otherdata.zip` under an override
   of `APPDATA`; the live installation was never touched.
2. `EmblemWeave.exe --import-save <archive>/1795393583` printed
   `Imported 8 save files …; the previous save is in …\backups\1795682327`
   (the stamped account was moved aside, not deleted).
3. The reporter's `1795325701-Auto` was put back into `000` and
   `EmblemWeave.exe --recover-saves` printed
   `Recovered the save the game could not open: 1795325701-Auto -> Auto`.
4. Result: `000` holds `Auto` (33,520 bytes, SHA-256 `2a3011e1…b2d4ca8`, byte-for-byte
   the archive's file), the six chapter saves, `Route_Ism` and `Global`; the `.meta`
   is present; `backups/1795393583` is byte-identical.
5. `fefw.exe --ini <fixture>` logged
   `save data opened: account/0100000000000000706d635257464546 … (10 files in 000, meta present)`
   and no `creating the account save data` line; the game reached its title
   sequence.

The Continue entry and a loaded playthrough are covered by the run in 0.7: the
title menu listed it, the slot showed Chapter 4, and choosing Yes loaded Dagson
Base.

### 0.7 The in-game run: Continue, Chapter 4 and Dagson Base

The recovered tree was copied to `tmp/issue79/ingame` (not committed) and driven
with the runtime's own `[boot] press` / `[boot] stick` keys. Those feed a scripted
player-1 pad inside the runtime (`ManualInputSource`), which the game reads like a
controller: OS-level keyboard injection never reached the game window. The run
used the clean runtime at `target/issue79/clean-runtime/fefw.exe`,
`max_seconds=110`, and a presented-frame screenshot every 2 s.

| Time | Frame |
| --- | --- |
| 15 s | “FIRE EMBLEM / FORTUNE'S WEAVE”, “PRESS ANY BUTTON” |
| 25 s | Title menu: **Continue** (selected), Descend Again, Extras, DLC; “Load previously saved data.” |
| 33 s | “Part I: Leda's Path”, “Chapter 4 — THE INDELIBLE PAST”, “3/1/1449 DAGSON BASE”, slot time 2:54, “Normal/Classic”, “Continue from where you left off? Yes/No” |
| ~55 s | Dagson Base gameplay: blue-haired character, minimap, “3/1 1449 Thu”, objective “Register for the Heroic Games.”, Menu/Dash/Map prompts |

A at 20 s left the logo and the class-reel movie for the menu (the movie ignores
input, which is why a 30 s press still showed only the reel); A at 30 s opened the
Continue slot; A at 40 s answered Yes. The runtime log at 41.4 s reads
`fs: save data opened: … (10 files in 000, meta present)` with no
`creating the account save data` line, so the slot is the recovered autosave.

Holding the pad down (stick Y 32767, 75–78 s) walked the character from beside
the table to the wall between the 73 s and 79 s frames: the loaded scene takes
live input and is not a frozen frame.

Control, same runtime, ini and timing: a `000` holding only `1795325701-Auto` (the
reporter's stamped file) logged `(1 files in 000, meta present)`, offered **no
Continue**, and ran the first-run flow with the “Autosave — Saving…” tutorial; no
`Auto` was written inside 40 s. The stamped name alone therefore produces no
Continue entry, which is the state the repair fixes.

Limits: a virtual pad, not a physical controller; the run stopped at the opening
of Chapter 4, so no battle, menu save or chapter transition was played; no
save-write and restart round-trip was attempted (the fixture stayed read-only and
its files kept their original mtimes); one build on one machine. The 48 presented
frames, the ini and both logs are under `tmp/issue79/ingame/` and are not
committed; timings are the runtime's clock and the log's `scripted press` lines.

### 0.8 Limits

* KalHawke's live tree was never in the bundle; his cause is established by the
  operation he describes plus the archive that proves the mechanism, not by his
  own files.
* 0.1.40/0.1.41 binaries are not installed here, so cross-version checks are
  format-level fixtures, not runs of those releases.
* Continue and the load are demonstrated, but only through the scripted pad; a
  physical controller, battle and menu saves, and a save-write/restart round-trip
  were not exercised (0.7).

The earlier text below is kept as the investigation record it was; its "open"
status is superseded by this section.

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
- `saves::empty_namespace_blocker()`: stops Play when the account save is
  **missing or empty** and recoverable saves exist. The launcher shows an
  explicit “Start with an empty save?” confirmation; the engine refuses the
  empty namespace unless that one-shot confirmation was given. The message names
  the account folder and recoverable saves, so a deliberate new game remains
  possible without silently overwriting the path to the old one.
- `saves::save_manifest()`: bounded save-tree listing for reports — filenames,
  sizes, timestamps, SHA-256 for files ≤ 1 MiB, metadata, profiles, backups,
  recoverable saves and the tail of the runtime's save log. Save contents are
  not included.
- Saves page: prominent “No current save” warning listing recoverable
  profiles/backups before the carousel.
- `engine::sync_save_dir()`: repairs missing metadata before reconciliation.
- `engine::play(config, allow_empty)`: applies the empty-namespace guard after
  reconciliation; `allow_empty` is set only by the launcher's confirmation.
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
| Missing/empty current save with recoverables | Confirmation guard and Saves warning added; library tests pass | `recoverable_inventory_and_empty_namespace_guard` |
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
- The launcher Saves page and recovery banner were opened in an isolated
  portable fixture and visually verified. Its Play button was disabled because
  the fixture has no complete game build, so the new “Start with an empty save?”
  dialog was not visually exercised; its blocking condition and one-shot
  confirmation are covered by the library guard test and compilation. A new
  gameplay save was not created and reloaded.
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
5. If the launcher says the current save is empty while recoverable saves
   exist, Play asks before starting a new save. Load/restore one, or confirm the
   new game deliberately.
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
| This change | `42d9a612`; follow-up confirmation `b33564ec` |
| Launcher tests | `cargo test -p emblemweave --locked`: 424 passed, 0 failed; `saves::tests`: 45 passed |
| Runtime build | `release2/fefw.exe` (working) and clean `target/issue79/clean-runtime/fefw.exe` (deployment), both built |
| Deployed launcher SHA-256 | `b9bb3fa17f6cfec462532c439e5917a020d48b02a9e8686f3851f0b1f050bc0b` |
| Deployed runtime SHA-256 | `8a2d0af5392ae8aa93141c4515d5ef5e21dadf5b839286b7dc966f50e8a77ef6` |
| X: backup | `X:\EmblemWeave-backup-20261123-2130` (8.707 GB; robocopy 104,598 files, 0 failed) |
| X: version after deploy | launcher `0.1.42 (b33564ec)`, Saves page opened |
| Prior launcher kept for rollback | `target/issue79/deploy/EmblemWeave-before-confirm.exe` (`b6162725…`) |

Issue #79 remains **open**. Do not close it from this work: the reporters have
not confirmed recovery, and the exact cause of their lost saves is still
unproven.
