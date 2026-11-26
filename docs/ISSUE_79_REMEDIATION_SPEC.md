# Issue #79: Remediation specification

> **Status 2026-11-26:** R1 (`engine::sync_save_dir` + the empty-namespace
> guard), R2 (both pages show the effective folder from `fefw.ini` next to the
> configured one), R4 (the runtime logs the full root, repairs a missing `.meta`
> and no longer says "created" for a tree that had files), R5 (`saves.txt` and the
> save inventory in every bundle) and R6 are implemented. R3 is implemented as the
> Saves page's orphan list + **Use this save**, the stamped-slot **Recover** action,
> and `--recover-saves`. The verified behavior, including the archive examination
> and the recovery of msalgadob's playthrough, is in
> `ISSUE_79_SAVE_FORENSIC_ANALYSIS.md` §0; this document stays as the design
> record.

**Audience:** FCC (owner of the active runtime remediation and the personal gameplay installation).
**Status:** proposal for review. This document changes no code. Nothing here authorizes a data migration on a
live installation.
**Related:** `docs/ISSUE_79_SAVE_FAILURE_INVESTIGATION.md`, `docs/ISSUE_79_SAVE_TEST_MATRIX.md`.

**One-line goal:** make the save root and the account folder a **single verified value at launch**, and make
the runtime's creation of an empty save **impossible to miss**.

**Non-goals (do not do these):**

* Do not change `DEFAULT_USER` (`saves.rs:193`) or the runtime UID defaults (`fefw_core.cpp:32-33`).
* Do not change the `.meta` layout, `SAVE_SIZE`, the title id, or the on-disk slot names.
* Do not move, merge, delete or rewrite a save without an explicit user action and a backup.
* Do not "fix" a user's installation automatically on startup.
* Do not touch `overview.rs`, `nvn_replay.cpp`, `svc_memory.cpp` or any file FCC is actively editing without
  coordinating first.

---

## 1. Defects this addresses

| ID | Defect | Current code | User-visible result |
| --- | --- | --- | --- |
| D1 | `[fs] save_dir` is an absolute path written only at build; Play passes it without checking it against `Config::save_dir()` | `engine::ini_text` (`engine.rs:2072-2102`), `engine::play` (`:4960-5030`), `play_blocker_in` (`:2250`) | Launcher and game can read different folders; the game starts fresh |
| D2 | The runtime silently creates an empty account save when the computed path has none | `fs.cpp:662-670`, `ensure_account_save` (`:914`) | A normal-looking launch with no saves and no error |
| D3 | The Saves page shows the launcher's intended root, not the engine's effective root | `pages/saves.rs`, `pages/setup.rs:495-520` | A profile can look installed while the game reads elsewhere |
| D4 | A save under a non-canonical account folder (or one level too deep) is invisible, and nothing offers to adopt it | `saves::account_dir` (`:203`), `fs::save_key` (`:108`) | "I moved my saves and the game still ignores them" |
| D5 | The diagnostic bundle cannot show the save tree or the effective root | `report.rs:190-240` | Issue reports like #79 arrive without the one file that settles them |

---

## 2. Required behavior

### R1: Reconcile the ini before launch (fixes D1)

**Where:** `apps/emblemweave/src/engine.rs`, at the start of `play()` after the `fefw.ini` existence check
(around `:4969`), plus a small helper next to `set_ini_key` (`:3797`).

**Behavior:**

1. Read `<run_dir>\fefw.ini`. Parse `[fs] save_dir`.
2. Compute `Config::save_dir()`.
3. If `save_dir` is missing or resolves (after normalizing separators/case on Windows) to a different path:
   * write the resolved value with `set_ini_key` so comments, key order and every other key are preserved;
   * write atomically (temp file + rename), as `Config::save` does;
   * log one line to the launcher log: `save root: <old> -> <new> (the game reads the ini)`.
4. If the write fails, **do not block Play**. Show a non-modal warning naming both paths and continue with the
   existing ini, because refusing to launch would turn a path problem into a cannot-play problem.
5. Re-read the file once and confirm the key now holds the resolved value; if not, use the re-read value when
   composing the warning.

**Why not rewrite unconditionally on every launch?** The user may deliberately point the engine at another
root by editing the ini. Reconciliation is only justified when the two *authoritative* sources disagree and the
launcher is about to start the game; that is exactly this case. Log the change so a deliberate override is
visible.

**Backward compatibility:** existing inis are edited in place, one key; `[engine] built_by`, `[fs] romfs`,
`mods`, `romfs_overlay`, `[nvl]` and all unknown keys are untouched. A build from an older release keeps
working.

### R2: Surface the effective path in the UI (fixes D3)

**Where:** `apps/emblemweave/src/pages/setup.rs::saves_section` (`:495-520`) and the Saves page header.

**Behavior:**

* Show two labelled values:
  * `Launcher saves:` `Config::save_dir()`
  * `Game reads:` the `[fs] save_dir` of the ini in the run folder, or "not built yet".
* When they differ, show a **Warn** row with one action: **Use the launcher folder** (runs R1 and refreshes) and
  one **Open both folders**. Do not silently "fix" on page open.
* When they match, keep the current single line to avoid clutter.
* The profile carousel's "installed" badge must remain file-comparison based (`installed_profile`), never
  selection based. No change needed there; only the path display is added.

### R3: Adopt an orphaned root or account folder, with consent and a backup (fixes D4)

**Where:** new function in `saves.rs`, called from the Saves page and from the "Save data" section; reuse
`account_save()` (`:260`) and `import()` (`:417`).

**Detection (read-only, bounded):**

1. **Foreign account folder:** `<Config::save_dir()>\account\*` where a directory contains `000` with files and
   its name is not `DEFAULT_USER`. Do not recurse deeper than one level; do not follow symlinks.
2. **Legacy runtime root:** `%APPDATA%\FortunesWeaveRecompPC\save\account\DEFAULT_USER` containing `000` with
   files. (This is the runtime's built-in default; see `common.cpp:376-382`.)
3. **Old install root:** only if the user points at it (a folder picker), never by scanning drives.

**Offer, never perform:**

> A save for this game was found in `<path>` (N files). The game reads `<canonical path>`. Install it there?
> The current save is kept in `save\backups\<stamp>` first.

On accept: call `import(found, &Config::save_dir(), title, &stamp())`. It already backs up, copies, writes the
`.meta` when absent, verifies that files landed in `000`, and restores the backup on failure. On decline:
remember the path in `config.json` so the offer does not repeat (follow the `reuse_dismissed` pattern), and
change nothing.

**Never:** rename the source, delete the source, or copy on startup without the answer.

### R4: Make the runtime's empty-save creation loud (fixes D2)

**Where:** `engine/fefw-recomp/runtime/src/services/fs.cpp`, `open_save()` (`:644-680`) and
`ensure_account_save()` (`:914`). **FCC-owned active area: coordinate before editing.**

**Behavior:**

1. When `save_exists(key)` is false and the auto-create branch runs, log at **WARN**:
   `no account save at '<full path>'; creating an empty one (the game will start a new game)`. Keep the
   existing INFO line for compatibility with log greps.
2. In the `no such save data` line, include the resolved root, not only the key:
   `OpenSaveDataFileSystem account/<uid>: no such save data under '<save_dir>'`.
3. When `root_of(key)` exists, contains `000` with at least one file, but `meta_of(key)` is missing, **repair**
   instead of reporting "created": write the meta with the configured size/journal/owner/attr and log
   `WARN … repaired missing .meta; existing save files were kept`. This is the C8 case; the files are the
   user's and must not be touched.
4. Do not refuse to start. A first run with no save must still work.

**Acceptance:** a run pointed at an empty root prints the WARN and the full path; a run with a valid tree
prints neither; a run with `000` but no `.meta` prints the repair line and the game sees the existing slots.

### R5: Bundle evidence (fixes D5)

**Where:** `apps/emblemweave/src/report.rs` (`:190-240`) and `report.txt`.

**Add, names and sizes only; never file contents:**

* `save root (launcher): <path>`
* `save root (game, from fefw.ini): <path>`
* `account folders:` each name under `<root>\account`, with `has_000`, file count, and whether the sibling
  `.meta` exists
* `save tree:` relative path, size and SHA-256 for files under the effective `<root>\account\DEFAULT_USER\000`
  and the immediate `backups\*` names
* `fefw.ini [fs]` section verbatim (already copied as a file at `:223`; also put the two path lines in
  `report.txt` so they survive a reader who never opens the ini)

The manifest must be produced with bounded depth and a file cap, and must not follow links.

### R6: Documentation

Update `docs/TROUBLESHOOTING.md` and `docs/UPGRADE_AND_RECOVERY.md` with:

* the exact path the game reads (`<save folder>\account\0100000000000000706d635257464546\000`);
* that copying a save folder by hand can leave the console's account id in the path, and that **Game Setup →
  Save data → Import from folder** is the supported way (it normalizes the layout and backs up the old save);
* that moving `data\` or a portable copy keeps the absolute `[fs] save_dir` in `play\fefw.ini` until the next
  build (until R1 lands), and how to check the value.

---

## 3. Files and functions

| File | Function / area | Change |
| --- | --- | --- |
| `apps/emblemweave/src/engine.rs` | `play` (`:4960`), new `sync_save_dir` helper near `set_ini_key` (`:3797`) | R1 |
| `apps/emblemweave/src/pages/setup.rs` | `saves_section` (`:495`) | R2 |
| `apps/emblemweave/src/pages/saves.rs` | page header / current-save section | R2, R3 entry point |
| `apps/emblemweave/src/saves.rs` | new `orphan_saves()` + `adopt_orphan()`; reuse `account_save` (`:260`), `import` (`:417`), `summary` (`:114`) | R3 |
| `apps/emblemweave/src/report.rs` | bundle writer (`:190-240`) | R5 |
| `engine/fefw-recomp/runtime/src/services/fs.cpp` | `open_save` (`:644`), `ensure_account_save` (`:914`), `save_exists` (`:165`) | R4 (coordinate) |
| `docs/TROUBLESHOOTING.md`, `docs/UPGRADE_AND_RECOVERY.md` | save section | R6 |

`config.rs` and `platform.rs` need **no** change: `Config::save_dir()` is already the single source the
launcher uses, and `data_root()` already implements portable vs profile correctly.

---

## 4. Failure handling

| Situation | Required behavior |
| --- | --- |
| `fefw.ini` unreadable | Play's existing "not built yet" refusal is enough; do not create an ini |
| `[fs] save_dir` absent | R1 inserts it; if the insert fails, warn with both paths and continue |
| Ini write fails (read-only, locked) | Warn, continue with the existing ini, log the error once |
| Both roots contain saves | Do **not** merge. Show both, let the user choose; default to the launcher root only on an explicit click |
| Adoption copy fails halfway | `import()` already removes the partial target and restores the backup; surface its error verbatim |
| `.meta` repair fails | Log the error; still open the volume if the directory exists? **No**: keep current behavior (`kTargetNotFound`), but the WARN must say the repair failed and name the path |
| Runtime cannot create the save (permissions, disk) | Existing `kGenericFailure` path; add the full path to the log |
| Two launcher copies share one engine folder | Each has its own `play\fefw.ini` and passes `--ini`, so R1 is per copy; do not write into the engine folder |

---

## 5. Backward compatibility

* **Save format:** unchanged. No file is rewritten by R1/R2/R5. R3 copies with the existing `import()`.
* **Ini format:** one key updated in place; comments and unknown keys preserved; older engines ignore nothing
  they did not already read.
* **Portable installs:** `Config::save_dir()` is already `<exe>\data\save`; R1 writes that absolute path, as the
  build did. No behavior change for a stable install.
* **Profile-mode installs:** unchanged.
* **Hand-edited ini overrides:** a deliberate override is overwritten by R1 at Play. This is intentional and
  must be logged; if the maintainers prefer to honor overrides, add a `[fs] save_dir_pinned = 1` escape hatch
  and skip R1 when it is set.
* **Old releases:** an old launcher with a new ini still reads `[fs] save_dir`; the added keys are unknown to it
  and are ignored.

---

## 6. Data-preservation requirements

1. No code path in this spec deletes a save, a profile, a backup or a slot.
2. Every write that can replace a save goes through `import()`'s backup-first sequence, or the existing
   `keep_slot_backup`/`load_empty_profile` path.
3. Source trees are opened read-only. Adoption copies; it never moves.
4. The `.meta` repair writes only the sibling `.meta` when it is missing; it never touches `000`.
5. All new file operations are confined to `Config::save_dir()` and the diagnostic work folder.
6. Tests use copies and SHA-256 manifests; the reporter's tree is never used as a fixture.

---

## 7. Regression tests

**Launcher unit tests (`apps/emblemweave/src/saves.rs` / `engine.rs` test modules):**

1. `set_ini_key` round-trip: replaces only `[fs] save_dir`, preserves comments, order and unknown keys.
2. `sync_save_dir`: missing key → inserted; same path → file byte-identical; different path → only that line
   changes; write failure → error returned but no partial file.
3. Orphan detection: `account/<foreign>/000` found; `account/DEFAULT_USER/000` not reported; an empty foreign
   folder not reported; the legacy `FortunesWeaveRecompPC` root found when populated.
4. Adoption: source unchanged (hash manifest), target canonical, current save in `backups/<stamp>`, `.meta`
   written, failure restores the previous tree.
5. Foreign-account import: `import` from `account/1111…/000` lands in `DEFAULT_USER\000` and leaves `1111…`
   intact (extends the existing #10 test).
6. Nested import: `account/DEFAULT_USER/000/000` is either resolved or refused with a message; it must never
   install a doubly nested tree.
7. `installed_profile` returns the same answer before and after a simulated restart (cache rebuild).

**Runtime tests (`runtime/tests/`, FCC-owned):**

8. Valid tree → no create line, no WARN.
9. Empty root → WARN contains the root and the account key; save created.
10. `000` present without `.meta` → repair line, meta written, slots readable, `000` untouched.
11. `fefw.ini` without `[fs] save_dir` → the built-in default is used **and logged as a fallback**, not silently.

**Manual acceptance (isolated copy):**

12. Copy `data\` to a new folder with the old absolute `save_dir`; launch; the log shows the reconciliation and
    the game reads the new root.
13. Copy a yuzu title folder under `account/<console uid>`; the Saves page offers adoption; accept; the game
    continues the playthrough; the source still exists.
14. Decline the offer; nothing changes and the offer does not repeat.

---

## 8. Acceptance criteria

* A save placed at `<Config::save_dir()>\account\DEFAULT_USER\000` is read by the game after Play, with no
  manual ini edit, on a stable install, a moved `data\`, and a moved portable copy.
* The Saves page always displays the root the engine will use, and flags a disagreement before Play.
* `fefw.log` names the effective root and never prints "created" for a tree that already had files.
* A save under a foreign account folder produces a one-click, backup-first adoption offer; declining changes
  nothing and is remembered.
* `report.zip` contains the effective root, the account-folder inventory and a hash manifest of the save tree,
  with no file contents.
* No existing save, profile or backup is modified by any of the above without an explicit user action.

---

## 9. Suggested sequencing for FCC

1. **R4 logging** (smallest, runtime-only, immediately improves the next report); coordinate, since
   `services/fs.cpp` is in the active remediation area.
2. **R1 + R2** (launcher-only; no runtime rebuild needed; fixes the class of failure that survives updates).
3. **R5** (report bundle; makes the next #79 answerable in one round trip).
4. **R3** (adoption UX; larger, needs the R2 path display to explain itself).
5. **R6** documentation last, so it describes the shipped behavior.

None of these require changing the save format, the account UID or the title id, and none requires a migration
of any existing installation.

---

## 10. Open questions for the maintainer

1. Should a deliberate `[fs] save_dir` override win over `Config::save_dir()`? If yes, add
   `[fs] save_dir_pinned = 1` and skip R1 when present.
2. Should the legacy `%APPDATA%\FortunesWeaveRecompPC\save` root be offered for adoption unconditionally, or
   only when it contains files under `000`? (Recommendation: only with files, and only when the canonical root
   has none.)
3. Is the runtime's fallback root (`FortunesWeaveRecompPC`) still wanted at all? Keeping it is safer for direct
   `fefw.exe` runs; the defect is its silence, not its existence.
