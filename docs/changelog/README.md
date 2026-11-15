# Changelog

One file per release, newest first. Each file is what the release notes said, word for word, so the README can stay a
description of the current state instead of a wall of history.

| Version | Date | In one line |
| --- | --- | --- |
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
3. Bump the version in `Cargo.toml` and the status line in the README.

`-Local` stops after the notes and the index, without touching GitHub. `-SkipBuild` reuses the launcher already in
`target\release`. The generated notes are left in `target\pkg\release-notes-<version>.md`, and nothing has to be typed
twice.
