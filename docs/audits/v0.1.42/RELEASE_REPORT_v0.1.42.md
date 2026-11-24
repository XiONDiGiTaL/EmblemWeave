# EmblemWeave v0.1.42 - Release Report (historical record)

> **Historical record, corrected 2026-11-24.** This report describes the 2026-11-22/23 FCC remediation session. It is
> **not** current release evidence and it is **not** a sign-off. The current assessment is
> `docs/audits/V0.1.42_RELEASE_READINESS.md`; artifact provenance is
> `docs/audits/V0.1.42_RELEASE_MANIFEST.json`.
>
> Status: **unreleased.** 0.1.41 remains the public release. Nothing in this report was published, tagged or
> independently field-verified.

## Session summary

The session implemented candidates for #70, #75, #76, #77, #78 and #79, plus a `write_watch` initialization fix. It
reported a single 120-second/4000-frame run and a deployment to `E:\EmblemWeave`. Those claims are retained here as
session claims. They were not reproduced by the independent audit, and the release documents' launcher hashes and test
totals did not match any artifact or run found later.

## Issue status as corrected

| Issue | Session claim | Corrected status | Evidence |
| --- | --- | --- | --- |
| #70 Cinematic AV1 assembly | Implemented, build verified | **Implemented - unverified** | dav1d CMake per-compiler flags exist; no clean engine build or game-content playback in this audit |
| #75 Cinematic AV1 + Opus | Infrastructure complete | **Implemented - unverified** | decoder paths and bundled FFmpeg DLLs exist; no end-to-end A/V test |
| #76 Find Units stutter | Infrastructure complete | **Implemented - unverified** | parallel scan, cancellation and progress exist; no in-battle measurement |
| #77 Return-to-title crash | Runtime verified | **Implemented - unverified** | one 120-second session; 10+ menu/title cycles not run |
| #78 HDR after exit | Implemented | **Partially implemented** | `play::stop()` forces windowed mode; original HDR/display state is never captured or restored |
| #79 Saves not read | Implemented R1-R6 | **Implemented - unverified** | R1-R8 exist; neither reporter confirmed a save loading |

## What was implemented

### #77 - fault-handler initialization and fallbacks

- `ImportFreeze` lazily initializes `g_transitions` and installs the page-fault handler before making pages read-only.
- Broad fallback for read faults on committed `PAGE_NOACCESS` pages.
- Write-fault fallbacks check `protect()` and balance `g_busy`.
- `ImportFreeze` is created for all NVN pool imports, not only viewed pools.
- `heap_decommit` clears the retired bit.

The 120-second session is evidence that the reported early crash did not recur in that run. It is not evidence that
repeated title returns are fixed.

### #78 - HDR after exit (corrected)

The session's change forces `FullscreenMode::Windowed` in `play::stop()`. It does not record the monitor's prior
advanced-colour state, SDR white level, resolution, refresh rate or HDR format, and it does not restore them. The
Windows HDR query in `win_display.cpp` is read-only. Initialization failure, device loss and crash paths never reach
`stop()`.

Do not describe this as "HDR state restored". The required implementation and the full test matrix are in
`V0.1.42_RELEASE_READINESS.md` section 6.1.

### #79 - save discovery (corrected)

The session implemented R1-R6; later work added R7-R8:

- R1 `sync_save_dir()` reconciles `[fs] save_dir` before Play.
- R2 Game Setup/Saves show the effective folder and warn on mismatch.
- R3 orphan and nested-save discovery with consent and backup.
- R4 loud save-directory creation logging.
- R5 save reconciliation section in the diagnostic bundle.
- R6 documentation.
- R7 copy-before-rewrite; refuse to start when a real save cannot be copied.
- R8 every launcher data root, newest save carried without deleting the source.

No reporter has confirmed the fix, and no controlled game-load reproduction was run. The specific reported failure -
saves on disk, game starts from zero - is not demonstrated fixed.

### #70, #75, #76

- dav1d C flags are split per compiler; NASM auto-detection is documented.
- AV1 WebM/MP4 configuration parsing and FFmpeg Opus decoding exist.
- Find Units uses parallel scanning, cancellation, progress, a timeout and a thread cap.

All three remain build-only or code-inspection claims until game content exercises them.

## Test results

### What the session reported

| Suite | Claim |
| --- | --- |
| Launcher | 387 passed, 11 ignored |
| Engine | 931 passed, 14 failed, 29 skipped |
| write_watch | 12 passed, 2 failed |

Those totals cannot all describe one build. The same document set also reports 166/14/5 out of 185 and "all tests pass
(1000+)"; a later clean run reached 698 tests with 680 passed, 10 failed, 7 skipped and no summary line, then stalled.

### Reproducible result in the independent audit

| Suite | Result |
| --- | --- |
| Launcher, clean `git archive` of `113cc8fa` | **427 passed, 0 failed, 13 ignored**, exit 0 |
| Engine | **not run**; no canonical build; historical failures untriaged |
| write_watch | not independently rerun |

The nine code-level engine names, including heap corruption (`kernel_sync`, 0xC0000374) and SEGFAULTs (`soak`,
`svc_fs_romfs`, `svc_info`), remain High until classified. Missing Vulkan validation coverage and the AMD subgroup test
are environment/test defects, not passes.

## Artifacts

### Historical hashes from this session (not current)

| Artifact | SHA-256 |
| --- | --- |
| Launcher | `52da8612ba53ddd64c0bd2a8b5688ecb7517f79d1c2cb407008c5db29508b2fc` |
| Runtime | `b93d8cf6d6b2510196c269e1e794d4d539b3b42294486cdf021f7d6e83626cf2` |
| Runtime (engineering validation) | `f4af373fd8f1f597288607482d31267fa5353f9b917a0f029bbf7ab25fe7450e` |
| `fefw.pdb` | `eb629aa6386b4fb35bb66b4da88d759cd9887f1b0ab5a4c461f07e63b0229958` |
| `fefw.sites` | `ba4e25b98a3f74485e7722cc1690f1f61447d7bab89de7be19ec113e59bc152e` |
| `fefw.ini` | `ee073abfd30e5f4fb21db053e458f20faba95af4d72c49031ae8e473f7e460dd` |

No artifact with the launcher or runtime hashes above exists in the audited tree. `f4af373f` is recorded in the 0.1.39
ledger as that release's preserved runtime, not a 0.1.42 build.

### Current artifacts (independent audit)

| Class | Artifact | SHA-256 |
| --- | --- | --- |
| Canonical launcher | `target/canonical-v0142/.../EmblemWeave.exe`, 20,082,176 bytes | `14ef84960fd77837e5f01e1b7b0feb6ca2121485cfcdb8a1e7b3cd752796437a` |
| Deployed launcher | `E:\EmblemWeave\EmblemWeave.exe` | `bf4a0021d967e469d313ff5a2585d7afc4bfec2293fcc79283a6dc0d25d1c27a` |
| Deployed runtime | `E:\EmblemWeave\data\play\fefw.exe`, 169,145,856 bytes | `8a2d0af5392ae8aa93141c4515d5ef5e21dadf5b839286b7dc966f50e8a77ef6` |
| Deployed ini | `E:\EmblemWeave\data\play\fefw.ini` | `9a261ea42f90528f393d44f8271d2be638986647da3d0f7646ab1502d53dd046` |
| FCC in-tree runtime (unverified) | `E:\EmblemWeave\engine\...\fefw.exe` | `c9967c04219f8ec9a621cfb1c6876d76f5f763d1333a1ec039fba0cc31cf1583` |

**No verified release-candidate runtime exists.** The production engine has not been built from canonical `main`.

## Deployment claim (corrected)

The session reported a transactional deployment to `E:\EmblemWeave` with SHA-256 verification and an atomic backup at
`E:\EmblemWeave\backups\pre-v0.1.42-20261122`. The independent audit could not verify that record:

- the installed launcher is `bf4a0021`, not the reported `52da8612`;
- the installed runtime is `8a2d0af5`, not the reported `b93d8cf6` or `f4af373f`;
- the production runtime path is `data\play\fefw.exe`, not `E:\EmblemWeave\fefw.exe`;
- the installed runtime predates the newest in-tree build.

Treat the deployment as a local development install, not as a release deployment. No deployment is authorized for
v0.1.42.

## Donation link audit

Pass at the time of the session and in the independent audit: the only promotional Buy Me a Coffee link is
`apps/emblemweave/src/pages/about.rs:38`. It is absent from the README, changelog, updater notices and release notes.

## Known issues at the end of the session

1. RomFS file count mismatch: 252,695 observed vs 252,686 expected in the complete package.
2. FSR3 frame generation: `vkCmdWriteBufferMarkerAMD` unavailable on the test device.
3. No `fefw.sites` file for frame-fix hook validation (contradicting the hash table above).
4. Fourteen documented engine failures, including code-level faults, not independently triaged.
5. #41 has no implementation and remains an unresolved device-loss class on 16 GB shared-memory hardware.
6. #78 does not restore the original HDR state.
7. #79 has no field confirmation.

## Field tests that remain

1. #70/#75: AV1 + Opus cinematic playback with real game data, including A/V sync.
2. #76: in-battle Find Units measurement.
3. #77: 10+ menu/title transitions.
4. #78: the full HDR/display matrix in the readiness audit.
5. #79: save/restart/update on disposable copies, including portable `.nup` installs.
6. #37: a 2+ hour session with memory and audio-voice figures.
7. #41: the failing scene on 16 GB shared-memory hardware.

## Commits cited by the session

- `7b59b0ad` - dav1d per-compiler flags (Refs #70)
- `22a380c` - write_watch `g_transitions` initialization
- `35e2e79` - play: restore windowed mode on exit (Refs #78)
- `406e46f` - memory: retired page decommit (Refs #77)
- `81861fc` - media: AV1 WebM parsing + Opus decoder (Refs #75)
- `59d948c` - Find Units scan controls (Refs #76)
- `17a8861` - AV1 WebM codec configuration parsing (Refs #75 #70 #29)

## Sign-off

**Deployment:** local development install only; not a release deployment.
**Tests:** 427/0/13 launcher on the canonical tree; engine not run on a canonical build.
**Documentation:** corrected 2026-11-24.
**Donation audit:** pass.
**Publication:** not authorized; 0.1.41 remains the public release.

*Historical report generated 2026-11-23; corrected 2026-11-24.*
