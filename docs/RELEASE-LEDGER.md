# 0.1.39 implementation ledger

This ledger records what was actually changed and checked for 0.1.39. It is not a
release announcement; 0.1.39 has not been published.

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
| Standard graphics page | Launcher usability assignment | `pages/media.rs`, docs | Performance/Balanced/Quality presets, display controls, render scale, FSR 1.0/Off, sharpening, shader precompilation; all existing advanced controls remain behind Advanced. | `cargo check`; launcher tests; rendered check of the Standard page; rendered check that Advanced restores the full page. |
| Special-character build paths | Build-path report (unnumbered in this ledger; same failure as issue #38's path) | `engine.rs`, `Cargo.toml`, docs | On Windows, a project path containing `& \| < > ^ ( ) %` is mapped to its short 8.3 spelling for the build working directory and for `FEFW_SITES`; if short names are unavailable, the error names the character, path, and both remedies. | `cmd_breaker` coverage; a real temporary path test that uses the short name when available and checks the fallback message otherwise; full launcher test suite. |
| Build-preservation guidance | #56 | `pages/requirements.rs`, README, INSTALL, USAGE, changelog | Requirements names the settings/saves/mods folder, engine/build folder and run folder, and states that updates keep them and Build reuses a valid build unless its inputs changed. | Compile and test; text reviewed against the existing updater test `an_update_replaces_the_release_and_keeps_the_build_and_data`. |

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

## Not verified / remaining release blockers

- A full engine build and game launch under a path containing `&` was not run;
  the short-name resolution and error path are unit-tested, not proven against
  the complete game build.
- Audio/RAM leak (#37), lighting/frame-rate reports (#29), stability (#41) and
  native Linux/Steam Deck support (#7) need engine runtime work and hardware
  validation. No fix or performance claim is made for them here.
- No benchmark or frame-time measurement was possible without a runtime capture.
- No release artifact was built, no commit was pushed, no GitHub issue was
  closed or replied to, and no mirror or website was updated.
- 0.1.39 should not be published until the remaining launcher and engine checks,
  packaging, and platform validation are complete.
