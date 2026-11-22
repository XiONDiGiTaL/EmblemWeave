# Upgrade and recovery

This page describes the updater that exists in `apps/emblemweave/src/updater.rs` and what to do when an update, a manual
overlay or an interrupted download leaves a mixed install. It is deliberately conservative: diagnose first, preserve
user data, and use the release manifest to prove what is missing or changed.

## What the updater does

1. **Check and download.** The About/Overview update action reads the latest GitHub release, picks the
   `*-windows-x64.zip` asset, downloads it with `curl.exe`, and unpacks it with `tar.exe` into a temporary cache
   (`%TEMP%\emblemweave-update`). The download must be HTTPS.
2. **Stage the launcher.** The new `EmblemWeave.exe` is copied beside the running launcher as `EmblemWeave.exe.new`.
   The sources are written only after the staged executable exists; if that copy fails, the staged file is removed and
   the install is not changed.
3. **Overlay the release.** The release's launcher-side files (README, docs, licenses, `upscalers`) are copied over the
   install. `data`, `portable.txt`, `EmblemWeave.exe` and the `engine` folder are skipped in this pass. The engine
   sources are copied separately into the engine folder the running launcher actually reads. The updater skips these
   generated paths:

   ```text
   engine/fefw-recomp/game-build/fefw/build
   engine/fefw-recomp/work
   engine/fefw-recomp/tools/.venv
   engine/fefw-recomp/tools/bin
   engine/fefw-recomp/tools/textures/third_party
   engine/fefw-recomp/tools/shaders/third_party
   engine/fefw-recomp/tools/shaders/build
   ```

   Files are replaced, never deleted. A stale file that the new release no longer ships remains; it is harmless but is
   reported by `verify_install.py` unless it is in a documented user-data/generated path.
4. **Record the carried build.** If a play folder and an existing build are present, the updater writes `carried_by` to
   the build's `fefw.ini` so the restarted launcher does not demand a rebuild just because the launcher version changed.
5. **Back up and swap.** The current executable is copied to `backups\EmblemWeave.exe.old` (one previous build is kept),
   then a short script waits for the process to exit, retries `move /y` for up to about a minute, starts the new launcher,
   and deletes itself. Failure details go to the launcher log folder's `update.log`; a failure marker is written where the
   next launcher start can show it.

The overlay is not a filesystem transaction. If the process loses power, the disk fills or a destination is locked partway
through step 3, some release files can be new and some old. The old launcher still starts (the executable swap happens
last), and the built game and `data` are not touched, but the source tree can be mixed. The recovery procedure below is
for that case.

## What is preserved

| Path | On update |
| --- | --- |
| `data\` (portable mode) | Never copied from the release; preserved. |
| `%APPDATA%\EmblemWeave\` (profile mode) | Not touched; the launcher keeps using it. |
| `portable.txt` | Never added, removed or rewritten by the updater. |
| Game dump / `engine/fefw-recomp/game/` | Never copied or deleted. |
| `engine/fefw-recomp/work/` | Skipped. |
| `engine/fefw-recomp/game-build/fefw/build/` | Skipped; the compiled game survives. |
| `tools/.venv`, `tools/bin`, `tools/download`, fetched `third_party` | Skipped/preserved. |
| `backups\` | Kept; the previous launcher is replaced only by the next successful update. |
| User-supplied `upscalers\` DLLs | Preserved unless a file of the same name is in the release. |

## Diagnose a suspect install

Use the release manifest and the install verifier:

```powershell
python tools\verify_install.py --root D:\EmblemWeave --manifest release-manifest.json
```

- **Missing or changed release file**: the overlay did not finish, the wrong release was unpacked, or something edited a
  release file. Overlay the matching release again, or restore the launcher backup for a launcher-only problem.
- **Unknown extra file**: often a stale source or doc from the previous release, or a file a user added. Check the path;
  use `--strict` when you want extras to fail.
- **User data/build output**: ignored by design; do not delete it to make the check pass.
- **No manifest**: get the `release-manifest.json` published beside the zip. If it is unavailable, use the zip's SHA-256
  from the release notes and compare it with `Get-FileHash`; then overlay the same zip and inspect the logs.

## Recover a failed launcher swap

Symptom: the launcher does not restart, or the next start shows an update failure.

1. Close any running `EmblemWeave.exe`.
2. Read `logs\update.log` (in the portable `data\` or the profile config folder) and `update-failed.txt` if present.
3. The previous launcher is `backups\EmblemWeave.exe.old`. Restore it with **About > Restore** if the launcher still
   starts, or manually:
   ```powershell
   Copy-Item -LiteralPath .\backups\EmblemWeave.exe.old -Destination .\EmblemWeave.exe -Force
   ```
4. Start the old launcher. Do not delete `data`, `engine`, `backups` or the built game.
5. If the update had already overlaid docs/engine sources, the old launcher can still run; use
   `verify_install.py` with the **new** release manifest to see which release files are present, then overlay the new
   release again when convenient. Never mix a manifest check with deleted user data.

## Recover an interrupted download or overlay

- The download cache is `%TEMP%\emblemweave-update`. The launcher's `clean_cache()` removes it on a later start; the
  cache is disposable. Do not move `data` or `engine` there.
- If an older launcher moved the new executable out of the cache but left the rest, the new launcher's
  `finish_interrupted_update()` finds the matching release folder and writes the missing docs/engine sources on its first
  start. If that does not happen, overlay the same release zip manually.
- If the overlay was interrupted, run `verify_install.py`; it reports missing/changed files. Overlay the same release
  again. Files are replaced, not deleted; the operation is idempotent for release files.

## Recover a mixed install

1. Keep `data/`, the game dump, `work/`, the build folder, `tools/download`, fetched `third_party` and `backups/`.
2. Obtain the exact release zip and `release-manifest.json` for the launcher version you want.
3. Verify the zip with `verify_archive.py` and the manifest with `release_manifest.py`.
4. Extract to a **new** staging folder. Do not extract over the live install first.
5. Compare the staged release with the live install using `verify_install.py`; use the report to see what is stale.
6. Overlay the release files over the live install, or use the launcher's updater. Do not delete the live `data/` or
   build trees.
7. Start the launcher, verify **About > Storage** points at the expected data folder, and run the game once.

## Roll back to the previous release

The updater keeps one previous **executable**, not a full previous source tree. That is enough to recover a failed
launcher swap, but not to downgrade the engine sources by itself.

- **Launcher-only rollback**: close the app, restore `backups\EmblemWeave.exe.old` as above. The docs/engine sources
  remain at the newer release; this is a mixed install, so run `verify_install.py` and expect a report. Use it only to
  regain a working launcher, then overlay the matching release or update again.
- **Full rollback**: keep the old release zip and its manifest; extract it to a new folder, preserve `data/` and the
  game folder, and point the old launcher at the existing build. Do not delete the newer install until the old one is
  verified.
- **Build preservation**: the compiled game is intentionally carried across an in-place update. If the engine ABI or
  sources changed, the launcher may offer a rebuild; do not delete the build folder unless a rebuild is required and the
  user has a backup of any data they care about.

## Transactional deployment: current state and recommendation

The updater stages the executable and backups the previous launcher, but the docs/engine overlay is a file-by-file copy
with no all-file journal. A fully transactional implementation would:

1. extract to a staging folder and verify every file against `release-manifest.json`;
2. snapshot the release-owned files that will be replaced (not `data/` or generated build output);
3. apply the overlay;
4. verify the installed files against the manifest;
5. restore the snapshot if verification fails.

`tools/verify_install.py` and the release manifest provide steps 1 and 4. The snapshot/rollback journal is not in this
change because it belongs in the launcher updater; the highest-value follow-up is to add manifest preflight and an
all-file rollback there. Until then, this page's recovery procedure is the supported path.

## Safety rules

- Never delete `data/`, `backups/`, `game/`, `work/`, the build folder or `tools/download` to fix an update.
- Never run a destructive test against a live personal installation. Use a temporary copy with a synthetic `data/`
  sentinel and a fake build folder; verify the sentinel is byte-identical after the test.
- Check for a running `EmblemWeave.exe` and active builds before replacing files. A running executable cannot be
  replaced on Windows, and a running build can corrupt its output.
- Use the release manifest and SHA-256 values; do not trust a re-zipped or renamed download.
