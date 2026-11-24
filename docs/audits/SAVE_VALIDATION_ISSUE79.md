# Issue #79 Save Validation — Fixture Runs

**Date:** 2026-11-24
**Harness:** `tools/issue79_fixture.py` (commit `9cf69180`, tests in this pass)
**Engine run:** `engine/fefw-recomp/game-build/fefw/build/release/fefw.exe`, ExeFS from `engine/fefw-recomp/game/dump/exefs`,
RomFS from the user's `H:` image, **read-only**, disposable save trees under `_recovery/issue79_run/`.

The user's saves (`%APPDATA%\EmblemWeave\save`), `E:\EmblemWeave` and `X:\EmblemWeave` were not read, copied, moved or
written. The source fixture was a synthetic tree created for this run, not a copy of a live save.

## 1. Harness checks

| Check | Result |
|---|---|
| `--source E:\EmblemWeave` | refused before reading |
| `--source %APPDATA%\EmblemWeave` | refused before reading |
| repo `data/` as source, scratch inside source, existing scratch | refused |
| symlink inside a source | refused (`tree_manifest` raises) |
| one save copy and one ini per direct row | yes (rows 16, 17, 19, 20, 22) |
| source before/after manifests | byte-identical |
| `--verify` after the game runs | `source unchanged: yes`; only the fixture shows the game's writes |

Regression tests: `tools/test_issue79_fixture.py` (8 tests, all pass) — refusals, per-row layout, second-prepare refusal,
source-change detection, and fixture-write reporting.

Synthetic source (SHA-256):

```
dd73ae67adbe0b3b2609ad879631fc3d96a4d7256e6f251694e84ace94caa082  account/0100000000000000706d635257464546/000/000.bin
4e93866b14452d1466efcbd465e8eee337adb6d5391cba3ed50feee53227849a  account/0100000000000000706d635257464546/000/header.bin
849d57c00d2340cf754cff05b5459422d74a1a8079da1106a326eb89c4143f9a  account/0100000000000000706d635257464546.meta
```

## 2. Row 22 — account folder without `.meta`

Run: 240 s headless, then stopped by the harness timeout; the game ran continuously, no crash.

```
fs: OpenSaveDataFileSystem account/0100000000000000706d635257464546: '...row-22/save\account\0100...' has 2 files
    in 000 but no .meta; wrote the missing metadata and kept the files
fs: save data opened: account/0100000000000000706d635257464546 (2 files in 000, meta present)
```

`--verify`:

```
row 22 active-save: fixture changes (expected game writes):
  added: account/0100000000000000706d635257464546.meta
```

This is the reconciliation/adoption path working: the existing tree was adopted rather than discarded, the metadata was
regenerated, and the source copy stayed unchanged.

## 3. Row 19 — foreign account id

Run: 90 s headless, stopped by the timeout; no crash.

```
fs: OpenSaveDataFileSystem account/0100000000000000706d635257464546: creating the account save data of the starting
    user at '...row-19/save\account\0100000000000000706d635257464546' (000 has 0 files); the game starts with no progress
fs: save data created: account/0100000000000000706d635257464546 (67108864 bytes, journal 67108864)
```

The foreign tree `account/1111…` was left alone, a fresh default-user save was created beside it, and the run started with
no progress — the expected "do not adopt another account's save" behaviour.

## 4. Rows 16, 17 and 20 — run after the first report

All three ran headless for 75 s each (stopped by the harness timeout, no crash), with the workspace ExeFS dump and the
read-only `H:` RomFS. The source tree stayed byte-identical and none of the three fixtures changed:

```
row 16 active-save: no fixture changes
row 16 new-root: no fixture changes
row 17 active-save: no fixture changes
row 17 new-root: no fixture changes
row 20 active-save: no fixture changes
```

| Row | Log evidence | What it proves |
|---|---|---|
| 16 (stale absolute `save_dir`) | `save data in '.../row-16/stale-target'`; `save data opened ... stale-targetccount0... (2 files in 000, meta present)` | An existing stale absolute `save_dir` is honoured: the old tree loads, and the new root (`row-16/save`) is not adopted or written while the old path exists. |
| 17 (portable copy with stale absolute `save_dir`) | same lines under `row-17/stale-target`, with `portable.txt` beside the fixture | The portable marker beside the installation does not change the engine's ini behaviour; the old absolute path still wins and the new root stays untouched. |
| 20 (save nested one level too deep) | `save data opened ... row-20/saveccount0... (2 files in 000, meta present)`; **no** "no such save data" line | The account tree is opened, the nested `000/000/*.bin` is not flattened into a slot, and the game starts with no progress. The "2 files" count is `file_count_below`, which is recursive by design, so it counts the nested files; no file was renamed, moved or adopted. |

Fixture hashes after the runs (all equal to the source):

```
849d57c00d2340cf754cff05b5459422d74a1a8079da1106a326eb89c4143f9a  account/0100000000000000706d635257464546.meta
dd73ae67adbe0b3b2609ad879631fc3d96a4d7256e6f251694e84ace94caa082  account/0100000000000000706d635257464546/000/000.bin
4e93866b14452d1466efcbd465e8eee337adb6d5391cba3ed50feee53227849a  account/0100000000000000706d635257464546/000/header.bin
```

## 5. Row 24 — launcher-side, not runnable through the harness or the engine

The harness states it does not prepare this row: a direct engine run does not exercise profile loading, and the row needs
the launcher's Saves/Setup pages. There is no headless launcher command for the empty-profile/restore flow. What was run
instead is the closest launcher-side path that does exist, in a throwaway **portable** copy so no real profile is read:

- scratch install `_recovery/row24-portable/` with `EmblemWeave.exe`, `portable.txt` and its own `data/save` holding
  tree A; `Config::save_dir()` resolves to that folder, not `%APPDATA%`;
- tree B in `_recovery/row24-import/`.

| Step | Command | Result |
|---|---|---|
| Import B over A | `EmblemWeave.exe --import-save ../row24-import` | "Imported 2 save files …; the previous save is in …/data/save/backups/1795507754"; target hashes became B's |
| Restore A from the backup | `EmblemWeave.exe --import-save data/save/backups/1795507754` | "the previous save is in …/backups/1795507762"; target hashes returned to A's, byte-identical to the pre-import tree |

Hashes:

```
A  (before)  000.bin dd73ae67…a082   header.bin 4e93866b…849a   .meta 849d57c0…3f9a
B  (import)  000.bin 5f37ee59…226f   header.bin 671cede3…4a92a
backup A     000.bin dd73ae67…a082   header.bin 4e93866b…849a
restored A   000.bin dd73ae67…a082   header.bin 4e93866b…849a
backup B     000.bin 5f37ee59…226f   header.bin 671cede3…4a92a
```

What this proves: the launcher's import backs up the current tree before replacing it, and importing that backup returns
the old tree bit for bit. The account `.meta` is local container metadata: it stays beside the account folder and is not
part of a backup (the engine regenerates it when missing — row 22), which is why the backup folders contain only `000`.

**Not proven:** the empty-profile load and the Saves page "Restore" button were not driven. `restore_backup_verified` is
the page's function and it wraps the same `import` path, and the `saves::` unit tests (45) pass, but the UI flow itself
was not executed. Row 24 should stay open until it is.

## 6. What remains

- Row 24's empty-profile/UI flow (above).
- A reporter's own save, loaded in the game. The fixture rows exercise discovery, reconciliation, adoption, stale paths
  and nesting; they do not prove that a save from the affected build now loads.
- The two-hour soak. The runs here are 75–240 s boot/stability observations.
