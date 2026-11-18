# 0.1.39 implementation ledger

This ledger records what was actually changed and checked for 0.1.39, the
release cut from this tree.

## Commits

| Commit | Contents |
| --- | --- |
| `242ac32` | Baseline for the in-progress 0.1.39 launcher/engine work already in the working tree: DLC discovery, release updates, save import/profile handling, carried build tools, updater tests, and the existing engine changes. |
| `375e658` | Standard/Advanced interface modes, Standard graphics presets, Windows short-path build handling, Requirements build-preservation guidance, and the matching documentation. |
| `654d271` | Verification ledger. |
| `12563cb` | Standard-mode navigation correction: Requirements stays visible; explicit build-detail, diagnostics and log links reveal Advanced; Quick Access hides advanced-only pages. |

## Implemented and verified

| Task | Issue | Files | Implementation | Tests |
| --- | --- | --- | --- | --- |
| Standard and Advanced modes | Launcher usability assignment | `config.rs`, `pages/mod.rs`, `shell.rs`, `pages/appearance.rs`, `main.rs`, `pages/overview.rs`, docs | New installs default to Standard; Appearance switches modes; the mode is persisted; Recompilation, Diagnostics and Game Log are hidden from the rail and Quick Access; explicit links reveal Advanced; Requirements stays visible because it is part of setup; an old config stays Advanced; explicit `--page` and `--tour` handle the mode correctly. | Config round-trip and migration tests; page availability tests; navigation-group coverage; rendered Standard and Advanced checks. |
| Standard graphics page | Launcher usability assignment | `pages/media.rs`, docs | Performance/Balanced/Quality presets, display controls, render scale, FSR 1.0/Off, sharpening, shader precompilation; all existing advanced controls remain behind Advanced. | `cargo check`; launcher tests; focused preset tests (each preset's values, resolution and display mode kept, Advanced-only fields untouched); focused mode filtering tests; focused mode-switch tests (Advanced values survive Standard and back, a hidden page falls back to Overview, choosing the current mode is a no-op); rendered check of the Standard page; rendered check that Advanced restores the full page. |
| Special-character build paths | Build-path report (unnumbered in this ledger; same failure as issue #38's path) | `engine.rs`, `Cargo.toml`, docs | On Windows, a project path containing `& \| < > ^ ( ) %` is built through a temporary directory junction whose spelling has none of those characters, and the hook-site path is spelled beneath it. A short 8.3 name is not used because CMake turns it back into the long spelling for a custom command's `WORKING_DIRECTORY`. The junction is removed after the build; if it cannot be made, the error names the character, path, and remedy. | `cmd_breaker` coverage; a real CMake/Ninja build from a temporary `&` folder that configures, compiles and links through the mapping, runs a lift-shaped command with an absolute working directory, and asserts the generated build file holds no long spelling; the refusal message test; full launcher test suite (224 passed, 10 ignored). |
| Build-preservation guidance | #56 | `pages/requirements.rs`, README, INSTALL, USAGE, changelog | Requirements names the settings/saves/mods folder, engine/build folder and run folder, and states that updates keep them and Build reuses a valid build unless its inputs changed. | Compile and test; text reviewed against the existing updater test `an_update_replaces_the_release_and_keeps_the_build_and_data`. |
| Built-in update in a new folder | #56 | `updater.rs`, `tools/package_release.ps1` | The release overlay updates the launcher files and engine sources in place, skips `data`, `portable.txt`, the built game and generated tools, and packages the fetched toolchain downloads out of the player zip. | `D:\EmblemWeave` was created from the 0.1.38 package with the real `game-build/fefw/build/release/fefw.exe`; overlaying the 0.1.39 package kept its SHA-256 (`f4af373f…`), `data/sentinel.txt`, `portable.txt` and `data/play/fefw.ini` byte-identical, while installing the 0.1.39 README and changelog. The 0.1.39 launcher then started there and Overview showed **GAME Ready**, **BUILD Ready** and **PLAY**, with the optional “built by 0.1.38” rebuild notice. |

## Existing work recorded, not newly re-verified

- The 0.1.39 release updater, DLC discovery, save import/profile handling and
  carried-toolchain changes are in `242ac32`.
- CMake discovery was already extended in the baseline to include
  `C:\Program Files\CMake\bin`, Visual Studio's bundled CMake, the x86 program
  files path, and the build PATH. Issue #35's reporter environment has not been
  reproduced on this machine.
- The engine build cache and artifact checks existed before this change; the
  new path handling feeds them the safe build root rather than replacing them.

## Verification actually run

- `cargo check -p emblemweave`: passed.
- `cargo test -p emblemweave`: **215 passed, 0 failed, 10 ignored**.
- `git diff --check`: passed before each commit.
- Native screenshots rendered from a temporary profile:
  - Standard Graphics: presets and everyday controls visible; advanced pages absent.
  - Advanced Graphics: upscaler files, ReShade, memory profile and advanced controls visible; advanced navigation restored.
  - Appearance: both modes switch correctly; Advanced is shown as current after loading an Advanced config.
- `D:\EmblemWeave` overlay check: passed; the built `fefw.exe` hash was unchanged by the overlay, and the updated launcher rendered Overview with **GAME Ready**, **BUILD Ready** and **PLAY**.
- `tools/package_release.ps1 -Version 0.1.39 -SkipBuild -SkipConfigureCheck`: produced a 75.2 MB zip and verified 9,958 files; the earlier 1.9 GB package exposed the unexcluded `engine/fefw-recomp/tools/download` archives, which are now excluded.

## Not verified / known limitations

- No benchmark or frame-time measurement was possible without a runtime capture.
- The full engine build and game launch under a path containing `&`; the
  short-name resolution and error path are tested, not the complete game build.
- Audio/RAM leak (#37), lighting/frame-rate reports (#29), stability (#41) and
  native Linux/Steam Deck support (#7) still need engine runtime work and
  hardware validation. No fix or performance claim is made for them here.
- The built-in updater was exercised through its overlay against a real new
  folder; the live GitHub download path is covered by the release workflow and
  the existing download tests, not by a signed-in click-through here.
