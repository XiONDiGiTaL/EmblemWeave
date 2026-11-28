# EmblemWeave 0.1.43 issue map

This table answers two questions: **what in 0.1.43 relates to this issue?** and **is it actually confirmed?** It is traceability, not a closure notice. Only #115 is treated as a confirmed code/build fix in this material.

## Status key

| Status | Meaning |
| --- | --- |
| **Confirmed** | Reproduced/fixed in the candidate and the stated build or test passed. Reporter confirmation may still be pending. |
| **Candidate** | The change is implemented and tested in isolation, but the reporter's scenario or a full end-to-end run is still missing. |
| **Partial** | Some related work landed, but the issue's main failure or a required platform/field test remains open. |
| **Not fixed** | No change in 0.1.43 establishes the requested fix. |
| **Unsupported** | The requested platform is not supported by this Windows release. |

## Open issues

| Issue | What 0.1.43 contains | Commit references | Evidence | Status |
| --- | --- | --- | --- | --- |
| **#115** `0.1.42 failed to rebuild` | The Windows 11 24H2 display requests are declared locally, with `static_assert`s on the header/field sizes; the 24H2 enum cast is avoided. Builds work with SDK 10.0.19041.0 and 10.0.28000.0. | `3acf62ff`, `235d3643` | Changelog: reproduced pre-fix errors, 634/634 targets with both SDKs, full `fefw.exe` build; [0.1.43 notes](../../changelog/0.1.43.md#build-and-toolchain). | **Confirmed** (reporter confirmation not recorded) |
| **#117** `Obelisk Chamber locks up and freezes` | The wait queues return to the single kernel lock; the sharding that caused the mutex-to-sync inversion is reverted. | Upstream engine commits **`74e7defc`** and **`cf09114c`**, cited by the changelog; **these SHAs are not objects in this repository's history**. The integrated source is in `engine/fefw-recomp`. | Isolated live retest: chamber loaded, Wolf Obelisk panel opened/backed out, Replay from Any Chapter opened, no 30-second freeze or watchdog report; character switch not visually confirmed. | **Candidate** (fix integrated; reporter confirmation pending) |
| **#79** `Saves files not being read` | Orphan save discovery and adoption (`6e3ae1b0`), stamped-slot recovery and the cross-process save-folder lock/atomic roll-forward (`3512b836`; GitHub serves the equivalent `98a0deae`), plus the `D`/`M` marker fix (`ac7ccd81`) and the pending-marker guard (`b9eaf1a5`). | `6e3ae1b0`, `bdc9e9d5`, `e2587a1e`, `3512b836` / GitHub `98a0deae`, `ac7ccd81`, `b9eaf1a5` | Runtime 1019/0, volume 17/0; archived route save imports byte-identically and its stamped `Route_Ism`/`Auto` recover. **Report C remains unresolved:** no route-completed save → restart → Continue cycle. | **Partial** |
| **#76** `Find Units Fails and Stutters Audio/Visual` | The scan runs in a supervised helper in a kill-on-close Job Object; Cancel, deadline, target loss or game exit terminates it even during a blocked read. The in-process fallback remains. | `7652aaf2`, `f42b27b6` | Field session: Cancel 1.05 s, 0.5 s deadline 1.44 s, game termination 1.29 s, no helper left behind; reporter's map/stutter not confirmed. | **Candidate** |
| **#77** `After 10 seconds the game crashes` (return to title) | A damaged autosave is quarantined before Play; frame-length and zstd validation reject frames before the game opens. | `e3b2e661`, `ceb17591`, `0eb9f1ff` | Launcher tests and save guards; the reporter's save and return-to-title reproduction were not available. | **Candidate** (no reporter reproduction) |
| **#116** `Bitdefender nuked EmblemWeave after update` | Carried-toolchain files are verified against an install manifest, a partly quarantined toolchain is detected and repaired; the build refuses a damaged tool. | `9fa85c02`, `31b9d60e` | Fault matrix and `--verify-tools`; **this is toolchain-integrity hardening, not a Bitdefender false-positive fix**. No protected endpoint was tested. | **Partial** (AV detection not fixed/confirmed) |
| **#59** `Launcher too slow` | Mods-page filesystem scans move off the UI thread; the Graphics probes cache with a 30-second TTL and background rescans; Overview quick-action scans cache. | `e956867e`, `59821c33`, `d0ec8c5c`, `620ec465` | Code and launcher tests; no startup/UI benchmark was recorded for this pass. | **Partial** |
| **#70/#75** cinematics / cutscenes | AV1 WebM configuration, dav1d assembly detection, Opus decoder plumbing, movie cadence and clean movie teardown. | `361a0334`, `68ed73aa`, `4ca0080b`, `438f4741`, `ca383be7` | AV1 verified through dav1d 1.5.1 + x86-64 assembly and FFmpeg/D3D11VA; Opus decode/seek/pause and measured A/V drift remain unverified. | **Partial** |
| **#37/#41** long-session stability / device loss | Renderer profiling, bounded prewarm, write-watch split, partial-init teardown and worker lifecycle fixes. | `ac785067`, `2fbb44a7`, `65e76d20`, `543de45e`, `77c6d991` | No multi-hour session or 16 GB shared-memory device-loss run in this pass. The related commits do not reproduce or close the reports. | **Not fixed** |
| **#29** framerate/lighting/audio-video sync/upscalers | AV1 WebM configuration and a shader/eye fix are related; the wider lighting, framerate and upscaler report has no new end-to-end test. | `4ca0080b`, `135e64b3` | Not re-tested. | **Not fixed** |
| **#7** native Linux/Steam Deck | No Linux release is produced; the Windows build does not run natively on Linux. | None | [Linux plan](../../LINUX.md); Windows-only release. | **Unsupported** in this release |
| **#78** HDR state restoration | Desktop display state capture/restore on exit. | `ee3a8c3b` | Not exercised live in this pass; HDR changes alter the display. | **Partial / not live-tested** |

## Commit references: what resolves where

- Local SHAs (`3acf62ff`, `235d3643`, `6e3ae1b0`, `3512b836`, `ac7ccd81`, `b9eaf1a5`, `7652aaf2`, `f42b27b6`, `e3b2e661`, `ceb17591`, `0eb9f1ff`, `9fa85c02`, `31b9d60e`, `e956867e`, `59821c33`, `d0ec8c5c`, `620ec465`, `361a0334`, `68ed73aa`, `4ca0080b`, `438f4741`, `ca383be7`, `ac785067`, `2fbb44a7`, `65e76d20`, `543de45e`, `77c6d991`, `135e64b3`, `ee3a8c3b`) resolve with `git show <sha>` in this repository.
- `98a0deae` is the GitHub-side save-lock/Route/atomic commit. It is **not a local object**; local `3512b836` has the same stable patch ID (`0f7e6ac7a02a623487ceafb0f142b096b2d4c07e`) and the same change content. The tree differs because the local commit was rebased.
- `74e7defc` and `cf09114c` are **upstream engine commit IDs cited by the changelog; they are not in this repository's history**. Do not present them as resolvable launcher commits. The integrated engine source is the evidence in this tree.
- New documentation commits for this material reference the issue numbers in their messages, but a documentation commit is not a product fix and must not be used as proof that an issue is closed.

## What would change a status

- **#115:** reporter confirms the build on the affected SDK; then it can be called fixed for that report.
- **#117:** reporter retests the chamber and, if the character switch is part of the report, someone visually confirms it.
- **#79:** a route-completed save reproduces/fixes the Continue failure (Report C) and a restart cycle keeps it.
- **#76/#77:** the reporter's map/save and a full field run.
- **#37/#41:** a multi-hour session and a device-loss run on the reported hardware.
- **#70/#75:** Opus decode/seek/pause and a measured A/V drift run.
- **#29/#59:** a repeatable frame-time/startup measurement.
- **#7:** an actual Linux build and play session.

No issue status, comment or label was changed while preparing this map.
