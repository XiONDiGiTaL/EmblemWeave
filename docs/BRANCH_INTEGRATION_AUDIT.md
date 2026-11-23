# Branch integration audit

Audit of the three branches left unmerged after `release-engineering` landed: `pr-40`, `pr36`, `stack-test`.
Base: `main` at `35e2e79` (`20396fc` plus `81861fc`, `406e46f`, `35e2e79`).

Method: `git log main..<branch>` for commits, `git cherry -v main <branch>` for patch equivalence, `git diff
<merge-base>..<branch>` for content, and a marker search in the current tree to decide whether a feature is already
present under a different commit. No merge, cherry-pick or working-tree change was made to any branch.

## Inventory

| Branch | Tip | Unique commits | Diff vs merge-base |
| --- | --- | --- | --- |
| `pr-40` | `2b4723b` | 1 | 3 files, +104/−5 |
| `pr36` | `b287e93` | 2 | 4 files, +74/−1 |
| `stack-test` | `2792d30` | 3 | 4 files, +140/−7 |

## `pr-40`: HDR GT tone curve

Commit: `2b4723b HDR: extend Fortune's Weave's GT tone curve to the display peak`.
Files: `gfx/extras/hdr_nvn.cpp`, `hdr_tonemap_patch.cpp`, `hdr_tonemap_patch.h`.

`git cherry` reports the commit as patch-equivalent to work in `main`, and the content confirms it is **strictly
superseded**. `main` already has the same function at a later revision:

- `hdr_tonemap_patch.h:41`: `spirv_hdr_tonemap_gt(in, peak_ratio, out, bool keep_wide = false)`.
- `hdr_tonemap_patch.cpp:316`: the implementation, including the `keep_wide` block that converts the three
  `FMax(0, x)` output clamps to `FMax(x, x)` so BT.2020 colours outside BT.709 survive as negative components.
- `hdr_nvn.cpp:44/48/65/109`: call sites that pass `wide` from `[hdr] wide_gamut`, and a third path at `:65`.

`pr-40` carries the *earlier* 3-argument version and lacks `keep_wide` entirely. Merging it would **remove** the
wide-gamut handling that `main` has. The textual conflict in `hdr_tonemap_patch.cpp/.h` is therefore not a merge
problem to solve; the branch is obsolete.

**Recommendation:** do not merge. Delete the `pr-40` label (or close the PR as superseded) and keep `2b4723b` as the
historical reference for the GT curve. Nothing in it needs re-landing.

## `pr36`: mod hook-site reporting

Commits:

- `38e1114 Overview: warn next to Play when code mods need a rebuild`
- `b287e93 Mod hook sites: report .sites problems before a build; keep the engine's override to one run`

Files: `apps/emblemweave/src/sites.rs` (+64), `pages/mods.rs` (+4), `pages/overview.rs` (+5),
`engine/fefw-recomp/recomp/games/fefw/CMakeLists.txt` (2 lines).

This is **unique, still relevant work not present in `main`**. `sites.rs` in `main` has `prepare()`, `stale()`,
`mod_missing()` and `bid_matches()`, but not the branch's two additions:

- `problem(root, mods) -> Option<String>`: reads the main build ID and returns the first `.sites` error with the build's
  own message, so a malformed mod is reported *before* a build is started rather than after it fails.
- `installed_note() -> Option<String>`: a two-second-cached answer for per-frame pages, returning either the `.sites`
  error or "code mods bring hook sites the built game does not have yet: press Rebuild". `pages/mods.rs` and
  `pages/overview.rs` consume it.

The branch also includes tests (`problems_are_reported`) that build a synthetic NSO with a valid `@nsobid` line and
assert both the accepted and rejected cases. That is the right shape for this code.

Why it conflicts:

- `pages/overview.rs`: `main` has moved on (and is currently dirty in FCC's working tree).
- `pages/mods.rs`: same page churn.
- `recomp/games/fefw/CMakeLists.txt`: a two-line change whose intent ("keep the engine's override to one run") is
  textual, not structural; `main`'s copy changed around it.

**Recommendation:** keep the branch, do not merge it now. Re-land it as **two fresh commits on current `main`** once
FCC's `overview.rs` work is committed:

1. `sites.rs` + `mods.rs` + tests (`problem()`, `installed_note()`), no UI change.
2. the `overview.rs` warning and the CMakeLists line.

Landing `sites.rs` first means the core value (fail before the build, not after) is testable on its own, and the UI hunk
is a small, reviewable second step. Do not cherry-pick across the dirty `overview.rs`.

## `stack-test`: replay/HDR engine work

Commits:

- `3814abb Replay: skip the full held-result check after a program bind`
- `81dd076 HDR: keep R11G11B10F render targets as float16`
- `2792d30 Replay: linear sampler for texel-fetch handles sampled with filtering`

All three features are **already present in `main`**, arrived at through the engine merge rather than these commits. Each
commit's distinguishing symbol is in the current tree:

| Commit | Marker | Present in `main` |
| --- | --- | --- |
| `3814abb` | `out_prog_dirty`, `out_memo_held` | `gfx/nvn/nvn_replay.cpp` |
| `81dd076` | `float16_scene` ("an R11G11B10F image … is remade as RGBA16F") | `gfx/nvn/nvn_gpu_internal.h`, `nvn_gpu_texture.cpp`, `nvn_replay.cpp` |
| `2792d30` | `reserved_sampler` | `nvn_gpu_internal.h`, `nvn_gpu_texture.cpp`, `nvn_replay.cpp` |

`git cherry` marked two of the three as already applied; the marker search shows the third is functionally present too,
with `main` carrying a later revision (the `nvn_gpu_internal.h` comment documents the `float16_scene` contract and its
fallback rule).

The branch still conflicts because it also edits `runtime/docs/CONFIG.md` and `nvn_replay.cpp`, both of which have moved,
and `nvn_replay.cpp` is **currently modified in FCC's working tree**. Per the coordination rule, that file must not be
touched.

**Recommendation:** do not merge; the content is redundant. Keep the branch and its `ew-pr52` worktree until FCC
confirms the current `nvn_replay.cpp` state covers all three behaviours, then delete the label. If any behaviour is
missing, re-implement it *after* FCC commits, never by merging this branch over the dirty file.

## Conflict summary and integration order

| Branch | Unique value | Status | Action |
| --- | --- | --- | --- |
| `pr-40` | none (older GT curve) | superseded, would regress wide gamut | delete/close |
| `pr36` | `.sites` pre-build reporting + Play warning | unique, relevant | re-land as 2 fresh commits after `overview.rs` is committed |
| `stack-test` | none (all three behaviours in `main`) | redundant, touches a dirty file | keep, then delete after confirmation |

Nothing here justifies an automatic merge. The only branch with work worth recovering is `pr36`, and its value is in the
`sites.rs` logic, not in the two UI lines that cause most of the conflict.

## Dependencies on FCC's active work

- `apps/emblemweave/src/pages/overview.rs`: dirty. Blocks the `pr36` UI hunk.
- `engine/fefw-recomp/runtime/src/gfx/nvn/nvn_replay.cpp`: dirty. Blocks any `stack-test` comparison beyond the marker
  check.
- `engine/fefw-recomp/runtime/src/kernel/svc_memory.cpp`: dirty, unrelated to these branches but on the same exit/memory
  path as #77; do not touch.

## Worktrees

Retained deliberately: `EmblemWeave` (main), `target/metrist` (detached `9d59afe`), `target/pr49-ui` (`64d6bf1`),
`target/release-eng` (`f7bc6c0`), `EmblemWeave-integration` (`ew-restore`, `640ed55`), `ew-pr52` (`stack-test`,
`2792d30`). `release-engineering` is merged into `main` but its label cannot be deleted while the worktree has it checked
out; that is expected and not a cleanup failure.
