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

## 4. What remains

- Rows 16, 17, 20 (stale absolute `save_dir`, portable copy, nested `000`) were prepared and their manifests recorded, but
  not run in this pass.
- Row 24 (launcher-side empty profile + backup/restore) was not run; it needs a scratch portable install.
- **No reporter save was loaded.** The fixture rows test the engine's discovery path; they do not prove that a user's
  real save from the affected build now loads. That still needs a reporter or a real save copy, and the issue must stay
  open until then.
- The 240 s run is a boot/stability observation, not the two-hour soak.
