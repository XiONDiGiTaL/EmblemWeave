# Next Release Readiness

**Date:** 2026-11-24
**Release in preparation:** 0.1.42
**Public release:** 0.1.41 (unchanged; the site, the updater and the release pointers still name it)
**Verdict: NOT RELEASE-READY.** No tag, no package, no deployment. The blockers are behavioural, not documentary.

This supersedes the earlier release-readiness summary for the items it covers and must be read together with
`V0.1.42_RELEASE_READINESS.md` (the per-issue audit) and `ENGINE_AND_RENDERER_VALIDATION.md` (what was actually
built and tested in this cycle).

---

## 1. What is true about the tree today

| Check | State |
|---|---|
| Production engine builds from a clean configure | **Yes** — 1,026 targets, Windows x64, exit 0 |
| Launcher tests | **Yes** — 442 tests, 429 passed, 0 failed, 13 ignored |
| Engine tests | **169/182**, 13 classified failures (6 environment, 4 pre-existing crashes, 1 hardware, 1 gamepad, 1 load-sensitive) |
| One canonical launcher/engine build for packaging | **No** — the build exists, the package does not |
| Gameplay verified in this cycle | **No** |
| Save loading verified | **No** (#79 is the critical path) |
| Cinematics verified | **No** (#70/#75) |
| Return-to-title cycles ≥ 10 | **No** (#77) |
| HDR restore | **No** — not implemented (#78) |
| Long-session memory bound | **No** (#37/#41) |
| Linux / Steam Deck | **No** — does not build for Linux yet |

---

## 2. Version metadata

- Workspace `Cargo.toml` version: `0.1.42`; the application crate inherits it (`version.workspace = true`).
- Release channel stays `development`; the published release is still 0.1.41.
- No release pointers were changed in this cycle: `4646…`-era "keep 0.1.41 as the published release" commits
  (`ea4a7adc`) and the site/updater configuration are untouched.
- **Action before tagging:** bump the site/updater/release pointers in the same change that publishes, not before.

## 3. Changelog

`docs/changelog/0.1.42.md` exists, is marked **unreleased**, and describes 0.1.42 as draft notes rather than a
published build. It now carries the diagnostics/renderer-instrumentation work from this cycle with explicit
"unverified" wording. It must not gain a "fixed" claim for #70, #75, #76, #77 or #79 before a run demonstrates it.

## 4. Release manifest

`docs/audits/V0.1.42_RELEASE_MANIFEST.json` is honest about its own limits:

- `"status": "release-candidate-not-published"`, `"published": false`, `"tag": null`, `"packaged": false`;
- it distinguishes the audit base (`6646b5f7`) from the audited code (`113cc8fa`) and says the audited code is older
  than the documentation;
- after this cycle, the audited code is the FCC integration commit; the manifest's `source.commit` and
  `auditedSource` must be updated to it **and** the hashes below must be regenerated from that build.

## 5. Runtime and launcher hashes

Recorded for this cycle's build (`build_int`, see the validation report):

| Artifact | SHA-256 |
|---|---|
| `mpr_launcher.exe` | `c9b0304c9f21980690a1dbeed723c01e7d206dd9e5556c38dbc627b634cf211b` |
| `nxrt_tests.exe` | `8a06cd04abf488c951f465984cd3680a47b1c26344ae8560a9a3854eaf4d76b6` |
| `nxrt_gfx_tests.exe` | `943cc446311c918a9e38bab4b3ef71cbf882da8ed45028eb4bb68f33139071e0` |
| `nxrt_gfx_demo.exe` | `6a9ff4027e4d8e1d28809a924d27b6d8b49bb75040cd395bbcb397ff4f5669c4` |

These are **not** release hashes: they come from a dirty-tree build, and the Rust launcher binary was not packaged.
The hashes in `engine/fefw-recomp/runtime/DEPLOYMENT_ISSUE37.md` are a different, unreproducible local build and must
not be reused.

## 6. Dependency licensing

- Engine and launcher are GPL-3.0-or-later; every new source file added in this cycle carries the SPDX header.
- `THIRD_PARTY_NOTICES.md` lists the adapted MIT sources (SuperMarioWonderRecomp and others) with their licences.
- Bundled third parties: dav1d (BSD-2-Clause), zstd (BSD/GPLv2 dual), SDL3 (zlib), ImGui (MIT), FFmpeg
  (LGPL/GPL depending on build), FSR1/NIS/DLSS/XeSS/Streamline, DejaVu and Sora fonts (permissive), Kenney input
  prompts (CC0).
- **No MAKO frame-generation weights or other restricted assets were imported.** MAKO references in the preserved
  stash are upscaler plumbing; the stash is not merged, so nothing restricted enters the release.
- No Rekarina source was copied; the Rekarina documents are analysis of a GPL project.
- **Action before packaging:** re-run the licence inventory over the packaged file set, including the FFmpeg DLLs
  actually shipped.

## 7. Bundled FFmpeg

`upscalers/ffmpeg/` contains `avcodec-62.dll`, `avutil-60.dll`, `swresample-6.dll`, `LICENSE.txt`, `COPYING.GPLv3`,
`README.md` and `SHA256SUMS`. The launcher installs them through `tools/install_local.ps1` (`113cc8fa`). The DLLs are
optional: the engine's default decoder is built-in dav1d and only `movie_decoder = ffmpeg` loads them.

**Action before packaging:** decide whether the release ships GPLv3 FFmpeg DLLs or LGPL ones; the current set is
`COPYING.GPLv3`, which imposes obligations on distribution.

## 8. Installation and upgrade behaviour

- Portable mode and the install layout are documented in `docs/PACKAGE.md`, `docs/PLATFORMS.md` and `docs/USAGE.md`.
- Upgrade preserves saves, configuration and the FFmpeg runtime; the E: and X: installations were **not** touched in
  this cycle, and no installer was run.
- **Not verified in this cycle:** a clean install, an upgrade over 0.1.41, an uninstall, and portable-mode relocation.

## 9. Save migration and recovery

- The save editor reads names/classes/items from the game's messages, off the UI thread, and writes the stored
  character name; background name loading is on `main`.
- Empty-save namespaces are guarded and the user is asked to confirm before starting one.
- `docs/ISSUE_79_SAVE_TEST_MATRIX.md` defines the controlled fixture run; `tools/issue79_fixture.py` implements the
  read-only preparation for rows 16, 17, 19, 20 and 22, refusing the user's real installation, `E:`, `X:`,
  `%APPDATA%` saves and the repository's `data`.
- **Still missing:** the actual run. The fixture harness prints the game commands; nobody has executed them and
  loaded a save.

## 10. Known issues and unsupported configurations

| Item | State |
|---|---|
| #79 saves not read | Implemented, unverified — **release blocker** |
| #78 HDR left on after exit | Partially implemented; display state is not captured or restored |
| #41 crashes past Chapter 12 | Outstanding; no reproduction, no fix; 16 GB shared-memory devices unsupported for any claim |
| #37 two-hour audio/RAM growth | Instrumented, not measured |
| #70/#75 cinematics | Implemented, unverified |
| #76 Find Units stutter | Implemented, unverified |
| #77 return to title | Implemented, unverified (one 120 s session only) |
| Linux / Steam Deck / Proton / Gamescope | Unsupported; the runtime does not build for Linux |
| Vulkan validation-layer tests | Cannot run without the SDK; 6 test binaries depend on it |
| Hardware-dependent tests | AMD subgroup size, physical/virtual gamepad, full GPU context |
| Kernel test crashes (`soak`, `svc_info`, `svc_memory`) | Pre-existing; reproduce on the pre-integration binary; need triage |

---

## 11. Blockers, in order

1. **Issue #79.** Run the fixture matrix and load a save in the game. Without it, the release's headline save fix is
   unproven.
2. **Cinematics (#70/#75).** One cutscene with audio, plus recorded frame times, is the difference between "the flag
   fix compiles" and "playback works".
3. **A soak (#37/#41).** Two hours with the soak and renderer CSVs on, ten transitions. It is also the only way to
   know whether the new instrumentation itself is affordable.
4. **#78.** Implement capture/restore of HDR, SDR white level and display mode, or state the limitation in the
   release notes and keep the issue open.
5. **Packaging.** Only after 1–4: a clean build from the tagged commit, package hashes, licence inventory, FFmpeg
   licence decision, install/upgrade test.

## 12. What can be published now, and what cannot

- **Can be published as documentation:** the audit set, including this file.
- **Cannot be published as a release:** anything that describes #79, #70, #75, #76, #77 or the renderer instrumentation
  as fixed, faster or verified. No speedup was measured, no gameplay was run, and the one canonical build in this
  cycle was not packaged.
- **Not authorized in this cycle:** a tag, a GitHub release, a deployment to `E:` or `X:`. Those remain separate,
  explicit decisions.
