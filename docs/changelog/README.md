# Changelog

One file per release, newest first. Each file is what the release notes said, word for word, so the README can stay a
description of the current state instead of a wall of history.

**0.1.43** is prepared as the next release and has not been published; **0.1.42** is the current public release.

| Version | Date | In one line |
| --- | --- | --- |
| [0.1.43](0.1.43.md) | release candidate | Windows SDK 10.0.19041 builds again, the Obelisk Chamber stall is fixed, and the engine picks up the 0.2.12-0.2.13 performance and stability work. |
| [0.1.42](0.1.42.md) | 2026-11-25 | Cinematic AV1+Opus infrastructure, Find Units optimization, title-return crash mitigation, HDR exit handling (original-state restoration outstanding), and save-directory reconciliation. Field verification remains listed in the release notes. |
| [0.1.41](0.1.41.md) | 2026-11-20 | A measured performance controller, a Performance page, a themed nine-tab F1 menu, 200% master volume, real Vulkan GPU selection, save portraits and party cheats, and no drag glide. |
| [0.1.40](0.1.40.md) | 2026-11-18 | Update and Play without a rebuild, cleaner add-on failures, and a bounded packed-audio cache. |
| [0.1.39](0.1.39.md) | 2026-11-18 | Standard mode, DLC discovery, special-character build paths, and an updater that keeps the built game. |
| [0.1.38](0.1.38.md) | 2026-11-17 | Save-profile corrections, a build fix when another Python's paths are inherited, and a rebuilt launcher. |
| [0.1.37](0.1.37.md) | 2026-11-17 | A guided first run, RomFS mods that stay above package data, faster lifted code, HDR UI brightness, and a broad engine update. |
| [0.1.36](0.1.36.md) | 2026-11-16 | The GT tone map is recognised by every HDR hook, build-cache resets only fire on real CMake errors, an existing mod folder uses its absolute path, and advanced settings fold into drawers. |
| [0.1.35](0.1.35.md) | 2026-11-16 | Plaintext `.xci` game cards build with `.nup`, `.nsp`, `.nx` and dumps; the `.nx` and CMake fixes get named regressions. |
| [0.1.34](0.1.34.md) | 2026-11-16 | Saves load again, shader compilation stops pinning every core, vibration can be tested, and the launcher has a modern dashboard. |
| [0.1.33](0.1.33.md) | 2026-11-16 | The launcher builds the lifted code for the processor it is running on, and Diagnostics says which build that is. |
| [0.1.32](0.1.32.md) | 2026-11-15 | Code mods: a DLL in a mod's `exefs` now runs inside the game, and a mod can bring the hook sites it needs |
| [0.1.31](0.1.31.md) | 2026-11-15 | A `.nx` plays from the archive again, and a build folder that lost its compiler starts over instead of failing |
| [0.1.30](0.1.30.md) | 2026-11-15 | NASM joined the optional list; the engine builds without it |
| [0.1.29](0.1.29.md) | 2026-11-15 | Three installs left the requirements: Git, PowerShell 7 and uv |
| [0.1.28](0.1.28.md) | 2026-11-15 | Portable mode takes the settings and installed mods with it, and can be switched from the command line |
| [0.1.27](0.1.27.md) | 2026-11-15 | CMake is found the way the build finds it, `.nx` archives play again, and an SDK unzipped whole is found one folder down |
| [0.1.26](0.1.26.md) | 2026-11-15 | The upscaler hints and the README now name the folder the launcher and the engine actually read |
| [0.1.25](0.1.25.md) | 2026-11-15 | The Graphics page now shows which upscaler files are on disk, where they go, and can find them for you |
| [0.1.24](0.1.24.md) | 2026-11-15 | A failed build now says so with the button that runs it again |
| [0.1.23](0.1.23.md) | 2026-11-15 | A build folder left behind by an older release no longer breaks the build with clang++'s "unknown argument" |
| [0.1.22](0.1.22.md) | 2026-11-15 | Play prepares a packed package and starts the game itself, so nobody has to know about the check first |
| [0.1.21](0.1.21.md) | 2026-11-15 | Game Setup has a one-click check that prepares a packed package in a minute, without compiling anything |
| [0.1.20](0.1.20.md) | 2026-11-15 | A package that packs part of its RomFS with zstd now plays without being repacked, and its compressed files cost 2 GB instead of a second copy of the game |
| [0.1.19](0.1.19.md) | 2026-11-15 | New areas compile about four fifths fewer shader parts, the compilers step aside when the picture is late, and a fresh copy keeps everything in its own folder |
| [0.1.18](0.1.18.md) | 2026-11-15 | The build survives spaced and apostrophe folders, the pipeline stops opening hundreds of console windows, and the launcher finally has a portable mode |
| [0.1.17](0.1.17.md) | 2026-11-15 | The engine is Obersteinn's preserved 0.2.0 tree, it builds from an empty folder on this PC, and the game was played to the menu and back to check it |
| [0.1.16](0.1.16.md) | 2026-11-15 | The release that finds its own toolchain, ships a zip that builds, and stops calling a half-built shader cache translated |
| [0.1.15](0.1.15.md) | 2026-11-14 | Releases are published from the changelog, and this one was published that way |
| [0.1.14](0.1.14.md) | 2026-11-14 | History moved here, the repair paths proven, and every write-up shortened |
| [0.1.13](0.1.13.md) | 2026-11-14 | `--verify-log` reads a build log without a rebuild |
| [0.1.12](0.1.12.md) | 2026-11-14 | The build reports its real error, and repairs two failures itself |
| [0.1.11](0.1.11.md) | 2026-11-14 | The shader translator ships in the zip, with its own .NET runtime |
| [0.1.10](0.1.10.md) | 2026-11-14 | The build-failure release: execution policy, preflight, logs, launch watchdog |
| [0.1.9](0.1.9.md) | 2026-11-14 | Save editor, cheat tables, and the launcher's own recompiler page |
| [0.1.8](0.1.8.md) | 2026-11-14 | Mods from GameBanana and archives, per-mod switches |
| [0.1.7](0.1.7.md) | 2026-11-14 | Save profiles and imports from the emulators |
| [0.1.6](0.1.6.md) | 2026-11-14 | In-game menu, screenshots, restart and stop |
| [0.1.5](0.1.5.md) | 2026-11-13 | First playable build of the baseline engine |

## Adding a release

One file is the source of everything:

1. Write `docs/changelog/<version>.md` with the title, a `> one line` summary under it, and the sections the release needs.
2. Run `pwsh -File tools/release.ps1 -Version <version>`. It builds and packages, writes the release notes from that file
   plus the install steps and the hashes it measured, updates the table above, commits that file and the table, pushes,
   and publishes the GitHub release with both assets, its tag on the commit it just pushed. Files you had not committed
   are named in a warning, because the tag cannot carry them.
3. Bump the version in `Cargo.toml` and the status line in the README. While a release is in development,
   `Cargo.toml` may name the next version and its row stays marked `unreleased`; `tools/check_release.py` accepts
   that only while `docs/release.json` and the site still name the published release, and it refuses to package
   until every pointer names the new version.

`-Local` stops after the notes and the index, without touching GitHub. `-SkipBuild` reuses the launcher already in
`target\release`. The generated notes are left in `target\pkg\release-notes-<version>.md`, and nothing has to be typed
twice.
