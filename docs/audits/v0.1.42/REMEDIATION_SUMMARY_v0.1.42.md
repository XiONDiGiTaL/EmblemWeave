# EmblemWeave v0.1.42 - Remediation Summary (historical record)

> **Historical record, corrected 2026-11-24.** This summarises the 2026-11-22/23 FCC session. It is not current release
> evidence and not a sign-off. See `docs/audits/V0.1.42_RELEASE_READINESS.md` for the current assessment and
> `docs/audits/V0.1.42_RELEASE_MANIFEST.json` for artifact provenance.

## Session summary

The session implemented candidates for #70, #75, #76, #77, #78 and #79 and fixed a `write_watch` initialization crash.
It reported a 120-second/4000-frame run and a deployment to `E:\EmblemWeave`. The independent audit reproduced the
launcher build and tests on a clean tree, but not the runtime claims, the deployment record or the reported test totals.

## Status as corrected

| Issue | Status | What exists | What is missing |
| --- | --- | --- | --- |
| #70 Cinematic AV1 assembly | Implemented - unverified | per-compiler dav1d flags, NASM documentation | clean engine build; real AV1 playback |
| #75 Cinematic AV1 + Opus | Implemented - unverified | WebM/MP4 AV1 parsing, FFmpeg Opus path, bundled DLLs | end-to-end A/V sync test |
| #76 Find Units | Implemented - unverified | parallel scan, cancellation, progress, timeout, thread cap | in-battle measurement |
| #77 Return-to-title | Implemented - unverified | fault-handler init, PAGE_NOACCESS fallback, pool coverage, retired-bit clear | 10+ menu/title cycles |
| #78 HDR after exit | Partially implemented | `play::stop()` forces windowed mode | capture and restore of original HDR/display state; edge-case paths |
| #79 Saves not read | Implemented - unverified | R1-R8 reconciliation/adoption/diagnostics | a demonstrated game load after restart/update |

## Key corrections

### #78 is not "HDR state restored"

`play::stop()` only calls `set_fullscreen(Windowed)`. The runtime never records the monitor's advanced-colour state,
SDR white level, resolution, refresh rate or HDR format, and never restores them. The Windows HDR query is read-only.
Initialization failure, device loss and crash paths bypass `stop()`.

Required implementation and the 11-case test matrix are in `V0.1.42_RELEASE_READINESS.md` section 6.1. Do not edit the
overlapping runtime files until FCC hands them over.

### #79 is implemented, not confirmed

Both reporters are on 0.1.41. kalhawke's portable install shows a profile moved to `backups/` before launch and a
successful first frame, but no rediscovery. msalgadob's portable install reports a next-day session starting from zero.
Neither has run the 0.1.42 reconciliation code. The backup/adoption path is recovery, not proof of discovery.

The controlled plan is in `docs/ISSUE_79_SAVE_TEST_MATRIX.md`; it uses disposable copies and asserts on the filesystem
and runtime log, not a launcher badge.

## Test results

### Session report (not reproducible)

- Launcher: 387 passed, 11 ignored.
- Engine: 931 passed, 14 failed, 29 skipped.
- write_watch: 12 passed, 2 failed.

The same document set also reports 166/14/5 out of 185, and "all engine tests pass (1000+)". These totals are
irreconcilable. A later clean run reached 698 tests, 680 passed, 10 failed, 7 skipped and no summary line before
stalling; that run is incomplete, not a pass.

### Independent audit

- Launcher on a clean `git archive` of `113cc8fa`: **427 passed, 0 failed, 13 ignored**, exit 0.
- Engine: **not run**; no canonical engine build. The nine code-level failures remain untriaged.

The "69 assertions" total is arithmetic on a mislabelled category: the table says five code-level tests but lists nine
names. The 44 validation-layer assertions and 19 controller assertions are environmental coverage gaps, not code
passes. The AMD subgroup expectation should query the device instead of hardcoding 32.

## Changes implemented in the session

1. `engine/fefw-recomp/runtime/third_party/dav1d/CMakeLists.txt` - per-compiler C flags (#70).
2. `engine/fefw-recomp/runtime/docs/CONFIG.md` - dav1d/NASM options (#70). This file is now also modified by FCC; do
   not overwrite FCC's version.
3. `engine/fefw-recomp/runtime/src/mem/write_watch.cpp` - fault-handler initialization and fallbacks (#77).
4. `engine/fefw-recomp/runtime/src/gfx/nvn/nvn_gpu_pool.cpp` - ImportFreeze for all pools (#77).
5. `engine/fefw-recomp/runtime/src/mem/memory.cpp` - `heap_decommit` clears the retired bit (#77).
6. `engine/fefw-recomp/runtime/src/gfx/play/play.cpp` - force windowed on stop (#78, partial).
7. `engine/fefw-recomp/runtime/src/services/misc.cpp` - hwopus FFmpeg Opus decoding (#75).
8. `apps/emblemweave/src/cheats.rs` - Find Units timeout, cancellation, progress (#76).
9. `engine/fefw-recomp/runtime/src/gfx/nvn/nvn_pipeline.h` - add `<array>`.

## Artifacts (historical)

| Artifact | SHA-256 |
| --- | --- |
| Launcher | `52da8612ba53ddd64c0bd2a8b5688ecb7517f79d1c2cb407008c5db29508b2fc` |
| Runtime | `b93d8cf6d6b2510196c269e1e794d4d539b3b42294486cdf021f7d6e83626cf2` |
| `fefw.pdb` | `eb629aa6386b4fb35bb66b4da88d759cd9887f1b0ab5a4c461f07e63b0229958` |
| `fefw.sites` | `ba4e25b98a3f74485e7722cc1690f1f61447d7bab89de7be19ec113e59bc152e` |
| `fefw.ini` | `ee073abfd30e5f4fb21db053e458f20faba95af4d72c49031ae8e473f7e460dd` |

No artifact with these hashes exists in the audited tree. They are retained as history only.

Current canonical and deployed hashes are in `V0.1.42_RELEASE_MANIFEST.json`. The canonical launcher is
`14ef8496...`, the deployed launcher is `bf4a0021...`, and the deployed runtime is `8a2d0af5...`.

## Remaining work

### Before publication

1. FCC handoff: commit/integrate its MAKO/Vulkan/Linux work.
2. Build the production engine from canonical `main`.
3. Run and classify the engine suite; install the validation layer.
4. Field-test #79 on disposable save copies.
5. Run #77 cycles, #78's HDR matrix, #37's 2-hour session and #41's failing scene.
6. Add #41 to known issues or fix it.
7. Cut release metadata, build and verify the zip and checksums.
8. Deploy only with explicit authorization.

### Documentation

The corrected documents are listed in `V0.1.42_RELEASE_READINESS.md` section 7. The session documents remain as
historical records with provenance banners; they must not be quoted as current release evidence.

## Risk assessment (corrected)

- The launcher change is reproducible and tested on a clean tree.
- The runtime changes are defensive but not field-confirmed.
- #78 is not complete.
- #79 is not confirmed.
- #41 and #37 remain open stability risks.
- Shipping while #41 affects 16 GB shared-memory hardware is a known-limitation decision that must be explicit.
