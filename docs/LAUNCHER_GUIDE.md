# Launcher guide

This is the short, task-first path through EmblemWeave. For every page and control, see [Using EmblemWeave](USAGE.md); for the download and build-tool details, see [Install and play](INSTALL.md).

## Before you start

- Windows x64.
- Your own legally obtained copy of *Fire Emblem: Fortune's Weave*. EmblemWeave does not include game code, assets, firmware or keys.
- The Windows C++ build tools, the Windows SDK, LLVM, CMake, Ninja and Python for the first build. The **Requirements** page checks them and shows the official install command. A fresh release does not carry the compiler toolchain; the launcher can fetch its pinned pieces with `EmblemWeave.exe --fetch-tools`.

A release zip is portable by default: `portable.txt` keeps settings, saves, builds, mods and logs in `data` beside `EmblemWeave.exe`. **About > Storage** shows which folder the running copy uses, including a profile install under `%APPDATA%\EmblemWeave`.

## First run in five steps

1. **Unzip the release** into a folder you can write to and run `EmblemWeave.exe`.
2. Open **Game Setup** and choose your dump: a `.nup`, `.nx`, plaintext `.nsp`, plaintext `.xci` or an extracted folder. The launcher validates the container and the build identity; it only reads your dump.
3. If you chose a format that needs it, use **Decode main module** once. Follow the page's prompt; the launcher explains the Python `zbic` helper it needs.
4. Open **Overview** and press **Build the game**. The first build takes the longest; later builds reuse what is still valid.
5. Press **Play**. The game runs from the launcher's `play` folder, not from your dump.

If the build stops, read the failed stage and the command output on **Recompilation**, then open **Troubleshooter**. The [Troubleshooting](TROUBLESHOOTING.md) page maps the common failures to the page that fixes them.

## The rail

A new installation starts in **Standard** mode. **Appearance > Interface > Advanced** adds the engineering pages without moving files or changing settings.

| Group | Pages | What they are for |
| --- | --- | --- |
| Home | Overview | Play, build, readiness, recent activity and next steps. |
| Game | Game Setup, Requirements | Choose and validate the dump; check the build tools. |
| Library | Saves, Mods, Cheats | Profiles and the slot editor; RomFS/code mods; quick and table cheats. |
| Runtime | Graphics, Audio, Controls | Presets, resolution, upscaling, volume/equalizer and controller bindings. |
| System | Troubleshooter | Real checks, safe repairs and a diagnostic ZIP. |
| Settings | Appearance, About | Interface mode, colour scheme, storage, version and updates. |
| Advanced | Recompilation, Diagnostics, Game Log | Pipeline detail, stop diagnostics and the live engine log. |

## Build and play

1. Choose the dump on **Game Setup** and wait for the validation line to finish.
2. On **Overview**, press **Build the game**. The status column names the stage that is running.
3. When the build reports ready, press **Play**. **Stop** returns to the launcher; closing the game window does the same.
4. After changing mods, code patches, the engine revision or the toolchain, the launcher marks the build stale and asks for a rebuild.

The built game is under `engine\fefw-recomp\game-build\fefw\build\release\fefw.exe` in a source checkout; in a normal install the launcher owns that `engine` folder. Do not edit the built executable; change settings in the launcher or F1 instead.

## Saves and profiles

- **Saves** lists the slots in the current save and opens the slot editor (gold, renown, level, experience, stats, items and spells). Names come from your own game data.
- **Create New Profile** keeps an independent copy without changing the current save. **Load Profile** copies a profile into the account folder and moves the replaced save to `backups\<stamp>` first.
- A folder that is on disk but not at the account path is listed as an orphan; **Use this save** copies it into place after backing up the current one. The old folder is only read.
- A file moved back under a backup name (`<stamp>-Auto`, `<stamp>-Route_*`, `<stamp>-Chapter_*`) can be copied to the name the game opens with **Recover** or `EmblemWeave.exe --recover-saves`. The stamped file is left untouched.
- Import and profile loads have a rollback path on an error, but they are not crash-atomic between the backup rename and the copy. Keep `save\backups` until the game has loaded the save and a restart has kept it.

The complete walkthrough, including the account-folder layout, is in [Save recovery](SAVE_RECOVERY_GUIDE.md).

## Mods

1. Open **Mods** and choose **Install**; pick a `.zip`, `.7z` or `.rar`, a folder, a GameBanana link, or browse the game's GameBanana list.
2. When a page offers several files and versions, the launcher asks which to install and marks the build that fits this PC. Install one variant of a mutually exclusive set; optional parts can be installed together.
3. Switch a mod on or off with its toggle. A mod with settings files gets **Settings**; the launcher opens the file in the app Windows associates with it and does not edit it itself.
4. ExeFS code patches live under `mods/<name>/exefs` and are applied to the launcher's copy of `main` before the lift. Rebuild after changing the active set.

Mods are made and hosted by their authors. EmblemWeave does not review or host them; install only what you trust.

## Cheats

- **Quick cheats** are per-unit switches for the units found in memory: infinite HP, quick level-up and max stats. Search again after a map loads.
- **Atmosphere codes** can be pasted or imported as `.txt`; each is switched on or off and applied while the game runs. Data writes, pointer chains, conditionals, loops and button tests work; instruction patches do not.
- **Cheat Engine tables** (`.CT`) support `aobscan`/`aobscanregion`; their values are live, editable and freezable.

The same quick switches appear in EmblemWeave's in-game menu on **F10**; the engine's own **F1** menu is separate and does not replace F10. Use cheats in single-player and back up your save first.

## Graphics and audio

1. On **Graphics**, choose a preset (Performance, Balanced or Quality) or set resolution, display mode, VSync, frame cap, render scale, upscaling and sharpening yourself.
2. Vendor upscalers (FSR 4.1.2/3.1, DLSS/DLAA, XeSS) are not shipped. Put their DLLs in the folders the page names, or use **Look for the files**; the page lists exactly what is found and what is missing.
3. On **Audio**, set the master volume (up to 200%) and the ten-band equalizer. The full F1 Audio panel applies the same master gain, presets and band sliders live.
4. Both pages read the files back. **What the game will start with** names the values that will be in force on the next start and those still waiting for it.

The engine's **F1** overlay holds the in-game versions of these controls. See the [F1 menu guide](F1_MENU_GUIDE.md) and [Graphics](GRAPHICS.md) for the file layouts.

## Diagnostics and reporting

1. **Troubleshooter** re-runs the real game-data and toolchain checks and offers the safe repairs it knows (rebuild, reset graphics, open logs).
2. **Create report bundle** writes a timestamped ZIP under the launcher's `reports` folder: settings, logs, the game log and any freeze/crash report. It does not upload anything.
3. **Report An Issue** opens a prefilled GitHub issue after you describe what happened. Attach the ZIP yourself.
4. **Diagnostics** and **Game Log** (Advanced) give the stop reason, guest threads, host events and the engine log for a deeper report.

## Updates

- When a newer release is published, **Overview** shows **Update available** and **Update now**; **About > Updates** has the same action.
- The updater replaces the launcher, docs and engine sources, keeps `data\`, the built game and fetched tools, and restarts once. It never adds or removes `portable.txt`.
- To update by hand, unzip the new release into the same folder (keeping `data\` and `engine\`), or point a fresh copy at the old engine folder on **Game Setup > Engine folder**. The exact layouts are in [Portable installation](PORTABLE_INSTALLATION.md) and [Upgrade and recovery](UPGRADE_AND_RECOVERY.md).

## If something goes wrong

- **The build stops:** open **Recompilation** for the failed stage and command output, then **Troubleshooter**; the log bundle is under `logs`.
- **The game does not see a save:** close the game, open **Saves**, compare **Save folder** with **Effective (fefw.ini)**, and follow [Save recovery](SAVE_RECOVERY_GUIDE.md).
- **A setting does not apply:** check **What the game will start with**; some values need the next start. The F1 overlay and the launcher write the same keys.
- **The launcher will not start or build:** run `EmblemWeave.exe --verify-tools`, check **Requirements**, and read the newest log under `logs`.

## Related

- [Documentation hub](GUIDES.md)
- [F1 menu guide](F1_MENU_GUIDE.md)
- [Using EmblemWeave](USAGE.md)
- [Save recovery](SAVE_RECOVERY_GUIDE.md)
- [Troubleshooting](TROUBLESHOOTING.md)
- [0.1.43 release notes](changelog/0.1.43.md)
