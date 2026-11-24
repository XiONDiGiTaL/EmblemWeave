# Issue #79: Save lifecycle test matrix

**Purpose:** define the isolated, non-destructive tests that distinguish the causes in
`docs/ISSUE_79_SAVE_FAILURE_INVESTIGATION.md`.
**Status of every runtime scenario: UNTESTED.** No build, launch, gameplay run or save copy was performed for
this assignment. Where a row says "static", the expected result is derived from the code path named in the
evidence column, not from an executed test. Rows marked "observed" were read from a real installation
(read-only directory names; no file contents, no writes).

**Mandatory fixture protocol for every executed row**

1. Use a **copy** of a save tree in a scratch directory. Never use the reporter's or the personal save.
2. Record a SHA-256 manifest of the source tree before and after; the source must be byte-identical.
3. Give each run its own `--ini` file whose `[fs] save_dir` points at the fixture.
4. Assert on the **filesystem** and on the runtime's own log lines, never on a launcher badge:
   * `fsp-srv: … save data in '<path>'` (effective root),
   * `OpenSaveDataFileSystem account/<uid>: no such save data` vs `save data created: account/<uid>`,
   * the slot files the volume lists.
5. Do not run while the personal game is running; do not use the personal build folder or `E:\EmblemWeave`.

**Legend:** PASS = expected state observed; FAIL = expected state not observed; UNTESTED = not executed;
N/A = scenario not applicable to this build.

---

## A. Launcher-side scenarios (no game required)

| # | Scenario | Expected filesystem state | Expected runtime save path | Expected game behaviour | Actual | Status | Evidence / how to verify |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | Existing profile with a valid save | `save\profiles\<name>\000\<slots>`; current account untouched until Load | `save\account\DEFAULT_USER\000` | Continue offered after Load | not run | UNTESTED | `saves::profiles`, `profile_is_installed`; verify file hashes before/after Load |
| 2 | Newly created empty profile | `save\profiles\<name>\.emblemweave-empty` only | unchanged until Load | If loaded: new game (old save in `backups\<stamp>`) | not run | UNTESTED | `create_empty_profile` (`saves.rs:856`), `load_empty_profile` (`:986`) |
| 3 | Imported yuzu-format save | `account\DEFAULT_USER\000\<slots>`; source untouched; `DEFAULT_USER.meta` present | `save\account\DEFAULT_USER\000` | Continue offered | not run | UNTESTED | `import` + `resolve_source`; manifest source before/after |
| 4 | Imported Eden-format save (alternate save id) | same as #3 | same | Continue offered | not run | UNTESTED | `find` scans ids; `nand_named`; see `saves.rs:59-160` |
| 5 | Import from a folder containing `account/<uid>/000` | files land under `DEFAULT_USER\000`, not nested | `save\account\DEFAULT_USER\000` | Continue offered | not run | UNTESTED | `account_below` → `account_save`; test with a **foreign** uid to prove normalization |
| 6 | Import from a folder containing only the contents of `000` | `DEFAULT_USER\000\<slots>` | same | Continue offered | not run | UNTESTED | `flat_save` → `SaveSource::Flat` → `copy_flat` |
| 7 | Save with playthrough slots and `Global` | `Global` copied; not counted/validated as a slot | same | Continue offered; global data present | not run | UNTESTED | `is_global_save_file`, `check_profile`; assert `Global` exists in target |
| 8 | Multiple profiles with different playthroughs | each profile intact; only the loaded one changes the account dir | the loaded profile's tree | The loaded playthrough | not run | UNTESTED | `profile_is_installed` after load and after a restart (cache rebuild) |
| 9 | Multiple account directories | import prefers `DEFAULT_USER`, else the newest other id; installs into `DEFAULT_USER`; other dir left in place | `DEFAULT_USER` only | Continue offered | not run | UNTESTED | `account_save` (`saves.rs:261-300`); #10 regression guard |
| 10 | Existing portable installation | `<exe>\data\save\account\DEFAULT_USER\000`; `portable.txt` present | `<exe>\data\save` (if the ini says so) | Continue offered | Partially observed: `<data>\save` tree exists on the audit machine | PARTIAL | `platform::portable_root`; read-only listing |
| 11 | Existing `%APPDATA%` installation | `%APPDATA%\EmblemWeave\save\account\DEFAULT_USER\000` | that path | Continue offered | Observed on the audit machine (full slot tree) | PARTIAL | read-only listing; `Config::save_dir()` |
| 12 | Portable ↔ non-portable migration | new root gets `save`, `config.json`, `play\mods`; **`play\fefw.ini` is not copied** | old absolute path in any copied ini | See #16 | not run | UNTESTED | `set_portable` (`platform.rs:139-167`); `run_dir` = `<root>\play` |
| 13 | Installation upgraded from an earlier release | `data\` preserved; ini may be from the old location | old `[fs] save_dir` until a build rewrites it | Depends on #16 | not run | UNTESTED | `docs/UPGRADE_AND_RECOVERY.md`; `engine::play` |
| 14 | Fresh installation with a valid imported save | save installed at the canonical path before first Play | `Config::save_dir()` | Continue offered | not run | UNTESTED | as #3 plus a clean data root |
| 15 | Multiple profiles after restart | active badge still names the installed profile | unchanged | unchanged | not run | UNTESTED | `installed_profile` recomputed from files, not from the session |

---

## B. Root and layout scenarios (the #79 candidates)

| # | Scenario | Expected filesystem state | Expected runtime path | Expected game behaviour | Actual | Status | Evidence |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 16 | `data\` copied to a new folder; `play\fefw.ini` still names the old absolute `save_dir` | new `<B>\save` intact; ini in B says `A\save` | **A** (stale ini) | If A exists: the old save. If A is gone: an empty save is created at A; the game starts a new game | not run | UNTESTED | `engine::ini_text` writes absolute; `play()` passes `--ini`; no reconciliation (`engine.rs:4960-5030`) |
| 17 | Portable copy moved (marker + `data` move with the exe) | saves move with `data`; ini keeps the old absolute path | old path | Old save if present; otherwise a fresh empty save at the old path | not run | UNTESTED | same as #16, plus `portable_root` follows the exe |
| 18 | `fefw.exe` started directly without `--ini` | launcher root untouched | `%APPDATA%\FortunesWeaveRecompPC\save` | New game in the fallback root | **Observed stub**: that root holds `account\DEFAULT_USER\` with no `000`, plus `.meta` | PARTIAL | `common.cpp:376-382`; `fefw_data.cpp:33`; `nxrt.log.prev` "save data in '…FortunesWeaveRecompPC\save'" |
| 19 | Save copied under a foreign account id (`11110000…`) | real save at `account\1111…\000`; `account\DEFAULT_USER` absent or empty | `account\DEFAULT_USER` | **New game**; log: "no such save data" then "created" | not run | UNTESTED | `fs.cpp:108,163,662-670`; `saves.rs:203` |
| 20 | Save copied one level too deep (`DEFAULT_USER\000\000\…`) | files exist, not at the volume root | volume opens at `DEFAULT_USER\000` | **New game**; no "no such save data" line | not run | UNTESTED | `fs::Volume` root listing; `saves::slots` walks recursively (so the launcher may still show files) |
| 21 | Save copied to `save\account\account\DEFAULT_USER\000` | files one level above the account dir | `save\account\DEFAULT_USER` | New game | not run | UNTESTED | `account_dir` is exact; no search at runtime |
| 22 | `.meta` missing, `000` present | runtime writes `.meta`, opens existing tree | canonical path | Continue offered (self-heal) | not run | UNTESTED | `save_exists` (`fs.cpp:163`), `create_save` (`:172`) |
| 23 | `.meta` present but from another title/user | runtime opens the tree regardless; info reader reports the foreign attr | canonical path | Continue offered in the launcher; the game may ignore it if it enumerates by attr | not run | UNTESTED | `read_meta` ignores owner; `fill_info` reports `attr`; `savefile`/game behaviour unknown |
| 24 | Empty profile loaded | old tree at `save\backups\<stamp>`; new account dir has `.meta` only | canonical path | New game; Restore brings the old playthrough back | not run | UNTESTED | `load_empty_profile` (`saves.rs:986-1040`) |
| 25 | Multiple account dirs, one empty `DEFAULT_USER` and one full foreign dir | stage 2 sees both; `account_save` resolves the foreign one for import | runtime still opens `DEFAULT_USER` | Load/Import works only after the launcher writes `DEFAULT_USER` | not run | UNTESTED | `save_below`/`account_save`; proves why Import fixes C2 but a manual copy does not |

---

## C. Runtime-visible assertions to capture per run

For each executed row, record all of the following in the evidence column:

| Assertion | Where it comes from |
| --- | --- |
| Effective save root | `fsp-srv: … save data in '<path>'` in `fefw.log`/`nxrt.log` |
| Account key used | `OpenSaveDataFileSystem account/<hex>` or `save data created: account/<hex>` |
| Whether a save was auto-created | presence/absence of `creating the account save data of the starting user` |
| Whether the volume opened the intended tree | slot names returned by the game's directory read; or a names-only listing of `<root>\account\<uid>\000` |
| Source-tree integrity | SHA-256 manifest before/after |
| Launcher-reported path | Game Setup "Save folder" and the `[fs] save_dir` line of the ini actually passed with `--ini` |

A launcher badge, the carousel selection or the "installed" label is **not** evidence for any row.

---

## D. Result summary

| Status | Count | Rows |
| --- | --- | --- |
| UNTESTED | 21 | 1–9, 12–17, 19–25 |
| PARTIAL (read-only observation, not a run) | 3 | 10, 11, 18 |
| FAIL | 0 | none |
| PASS | 0 | none |

No row is reported as passing. The three partial rows establish only that both roots and the silent
auto-create exist on a real machine; they do not establish the reporter's cause.

**Order to execute when a build is available (cheapest first):** 19, 20, 16, 24, 22, then 1–9. Rows 19 and 16
separate the two leading hypotheses (foreign account folder vs stale root) with one launch each.

---

## E. v0.1.42 controlled verification plan (prepared 2026-11-24; not executed)

This plan verifies the R1-R8 reconciliation and adoption work on disposable copies. It does not use
`E:\EmblemWeave`, a reporter's only save, or a personal profile.

### E.1 Fixture protocol

1. Create a scratch root outside every installation, for example `target/validation/issue79/<run>/`.
2. Copy the save tree with its directory shape; record a SHA-256 manifest of the **source** before and after.
3. Write a dedicated ini for the run whose `[fs] save_dir` points at the fixture. Do not edit the installed ini.
4. Start the engine directly with that ini (or through a launcher whose data root points at the fixture) so the run
   cannot touch `E:`.
5. Capture the filesystem tree, the runtime log and the launcher's displayed effective folder after each step.
6. Restore nothing into the source; the source must remain byte-identical.

### E.2 Scenario mapping

| Required scenario | Matrix rows | Fixture | Pass evidence |
| --- | --- | --- | --- |
| Existing profile after shutdown and restart | 1, 8, 15 | profile tree with two playthroughs | game offers Continue and loads the expected slot after restart |
| Discovery after updating from 0.1.41 | 13, 16, 17 | 0.1.41 data root copied beside a 0.1.42 install; stale absolute `save_dir` | R1/R7 copies the save before rewriting; game loads it; source unchanged |
| Multiple profiles | 8, 9, 15 | two profiles plus a foreign account directory | only the loaded profile changes the account tree; the other profile stays intact |
| Portable installation | 10, 12, 17 | `portable.txt` + `data\save` copied to a new path | save found under the new `data\save`; no `%APPDATA%` fallback |
| Profile backup and restoration | 2, 24 | empty profile plus a populated current account | Load writes a backup; Restore returns the exact previous slot set |
| Autosave discovery | 1, 7, 22 | save with autosave/`Global` files and a missing `.meta` | autosave listed and loadable; `.meta` self-heals; `Global` preserved |
| Different installation paths | 16, 17, 19, 20 | path with spaces; save one level too deep; foreign account id | R8 finds the newest save without deleting the source; adoption normalizes into `DEFAULT_USER` |
| Launcher/runtime save-directory agreement | 2, 16 | configured folder deliberately different from the ini | confirmed effective folder equals `[fs] save_dir`; mismatch warning appears; R1 reconciles |
| Orphan/nested adoption | 20, 21, 25 | save under `account/<uid>/<uid>/000` and under `account/account/...` | Saves page lists it; **Use this save** backs up first and installs it |
| Copy failure refusal | 16 | read-only destination holding a real save | Play refuses to start and names the copy failure; no empty game opens |

### E.3 Execution order and evidence

1. Matrix 19, then 20 (one launch each): distinguishes foreign-account from nested-folder failure.
2. Matrix 16, then 22, then 24: stale root, missing `.meta`, empty-profile restore.
3. Matrix 1-9 and 15: profile, restart, autosave and multi-profile paths.
4. Matrix 10, 12, 17: portable and moved-install paths.
5. R3/R7/R8 adoption and copy-failure cases.

For every run retain: the source manifest before/after, the fixture tree, `fefw.log` lines (`save data in '<path>'`,
`OpenSaveDataFileSystem account/<uid>`, `save data created` and any copy refusal), the launcher's effective-folder line,
and a screenshot showing the loaded save. A launcher badge or carousel label is not evidence.

### E.4 Status rules

- **PASS** only when the game loads the expected save and the source tree is byte-identical.
- **FAIL** when the game starts a new game, the source changes, or the copy refusal does not happen.
- **BLOCKED** when no canonical runtime exists or the game data cannot be launched; record the blocker rather than
  substituting a launcher-only check.
- #79 stays open until at least one reporter (or an identical portable reproduction) confirms the save survives restart
  and update. R3 adoption is a recovery path, not proof of discovery.
