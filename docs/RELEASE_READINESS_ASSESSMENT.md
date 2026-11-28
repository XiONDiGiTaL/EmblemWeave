# Release readiness assessment

Consolidated judgement of whether EmblemWeave can be called a dependable public release, as of 2026-11-22,
`main` at `35e2e79`. Each area gets a status and the evidence behind it. "Green" means demonstrated by an artifact or a
test in this audit; "amber" means the mechanism exists but is unproven; "red" means a known gap that a user can hit.

## Scorecard

| Area | Status | Evidence |
| --- | --- | --- |
| Build reliability | Amber | Release tooling tests pass (30/30); no runtime compile in the release gate |
| Installation experience | Red | VS C++ tools, LLVM, CMake, Ninja and Python still required; no provisioning |
| Packaging completeness | Green (engine), Amber (proof) | Tracked-tree copy + verifier; the only verified zip predates the merge |
| Runtime stability | Red | #77's fix is unproven and its mechanism does not prevent the fault; #78 unverified |
| Hardware compatibility | Amber | 8 NVIDIA Windows configurations reported working; 2 startup failures unexplained |
| Performance consistency | Amber | Pipeline cache exists; no frame-time evidence for the Z1 Extreme report |
| Documentation | Green | Install, package, portable, upgrade/recovery and validation pages are accurate and specific |
| Licensing | Amber | All vendored trees ship notices; GPL source links for FFmpeg unverified; fetched libs unnoted |
| Automated QA | Amber | 47 tooling tests + launcher skip baseline; launch and GPU behavior are still field checks |
| Outstanding critical defects | Red | Exit-path crash (#77) and HDR restore (#78) are not demonstrated |

## Build reliability: Amber

The release path is reproducible in principle: a tag triggers a pinned build, `check_release.py` forces the version
pointers to agree, and `package_release.ps1` refuses a dirty tree or a stale `-SkipBuild` executable. The 30 tooling
tests and the PowerShell parse gate run on every PR.

What is missing is proof that the *product* builds. `-RequireConfigureCheck` configures the extracted engine with CMake
and stops there. A tag can therefore publish an archive whose C++ does not compile or link. `launcher-tests.yml` does
compile `mpr_launcher_tests`, but it is a separate workflow, filtered by path, and not a dependency of `release.yml`.

Gate to green: make the runtime compile a required step of the release job (even a single translation-unit build or the
existing launcher-test target), and make `launcher-tests.yml` a `needs:` dependency of `publish`.

## Installation experience: Red

The documented first run is: install VS C++ tools, LLVM, CMake, Ninja and Python; choose a dump; build; play. The shader
translator is now self-contained (no .NET SDK), which removes one dependency, but the C++ toolchain and Python remain.

The evidence that this is a user-facing problem is in the issue tracker, not in theory: #43 is a stale `tools\.venv`
whose `python311.dll` survived an update and broke the build; #34 is a build that compiled but would not launch and was
resolved by a fresh extraction. Both are recovery problems caused by a complex host toolchain.
`tools/install_local.ps1` installs a launcher into an existing folder; it does not provision a toolchain.

The new `verify_install.py`, the release manifest and `UPGRADE_AND_RECOVERY.md` improve *diagnosis* substantially. They
do not reduce the number of things a new user must install.

Gate to green: automated provisioning (detect, install, verify versions, recover a half-installed toolchain) or a
prebuilt runtime that removes the build step for players. Either is a project, not a patch.

## Packaging completeness: Green for engine dependencies, Amber for proof

Structurally, this is the strongest area. The engine is copied from `git ls-files`, the copied tree is verified to have
no extras, and the zip is re-opened and extracted with a file-count comparison. The six folders missing from v0.1.11
(`dav1d`, `amd_ffx_api`, `amd_ffx_api_fg`, `ffx_fsr1`, `nvidia_nis`, `nvidia_streamline`) are tracked under
`runtime/third_party/`, are enumerated (1,738 engine files), and are present in the shipped 0.1.41 zip (13/8/11/5/28/153
entries including directories).

The amber is about proof, not mechanism: the 0.1.41 zip was built on 2026-11-20, before the merge, and does **not**
contain the new validation docs or `tools/verify_*.py`. No artifact produced by the new packager exists yet.

Gate to green: run `package_release.ps1 -RequireConfigureCheck` from a clean `main`, verify the archive and manifest, and
publish that zip, and then the claims in `PACKAGE.md` are demonstrated rather than described.

## Runtime stability: Red

Two issues on the same exit path are not demonstrated:

- **#77.** `406e46f` still decommits retired pages to `PROT_NONE` and then clears the retired marker. Because
  `retired_retry()` only retries when the current protection permits the access, clearing the marker does not change the
  outcome of the fault; it only removes the classification. The commit bundles this with a 316-line platform refactor and
  has no reproduction log. The newest runtime log in the workspace ends at a missing-RomFS boot error, not at a
  quit-to-title crash.
- **#78.** `35e2e79` forces windowed mode at the end of `stop()`. No code in the runtime writes Windows HDR state, so the
  mechanism relies on leaving an HDR10 exclusive-fullscreen swapchain letting DXGI restore SDR. Plausible, unverified,
  and skipped entirely if the process crashes on exit, the #77 scenario.

A release should not claim these are fixed. They are "changed, awaiting evidence".

Gate to green: a recorded quit-to-title run with the post-fix log, and an HDR-display test in both borderless and
exclusive modes.

## Hardware compatibility: Amber

Eight configurations are reported working, all of them NVIDIA discrete GPUs on Windows, including a laptop 5090 and
several 50-series cards. One AMD integrated configuration (Z1 Extreme) runs with shader-loading stutter. Two startup
failures, Steam Deck under Proton (0.1.40) and Intel Arc B390 (0.1.36), are unexplained because neither report
includes a log.

The code shows both vendors are considered: Intel has `igpu_compat`, memory budget and pipeline-cache-lock quirks; the
Deck has a documented Proton route and a Wine-specific `heap_section` switch. Neither failure is evidence of a missing
vendor path.

Gate to green: logs. Until then, "eight configurations work" must not be restated as "Windows is supported".

## Performance consistency: Amber

The runtime persists a `VkPipelineCache` (`pipeline_cache.bin`, `pipeline_keys.bin`) and logs frames over 50 ms, which is
the right instrumentation. The unresolved question is whether the Z1 Extreme stutter is first-run compilation (expected,
mitigable) or a persistent stall. AMD integrated GPUs do not receive the memory `budget_detail`/`pressure_detail` quirks
that Intel iGPUs get, and the default heap is 12 GiB: both are credible contributors on a unified-memory device.

Gate to green: a cold/warm hitch-log comparison, then a decision on extending the memory quirks beyond Intel.

## Documentation: Green

`PACKAGE.md`, `PORTABLE_INSTALLATION.md`, `UPGRADE_AND_RECOVERY.md`, `RELEASE_VALIDATION.md` and
`DEPENDENCY_LICENSE_AUDIT.md` describe what the code actually does, including the parts that are not finished. The
upgrade page states plainly that the overlay is not transactional and that the rollback journal is future work; that is
the correct way to document a known limitation. `docs/STEAMDECK.md` says the route is untested, which is also correct.

One documentation defect remains: `docs/ISSUE_REMEDIATION_RESULTS.md` cites `a3b84ee` for #75 when the fixes are
`17a8861` and `81861fc`.

## Licensing: Amber

Every vendored tree ships its licence (`LICENSE.txt` for the AMD/NVIDIA/Intel SDKs, `COPYING` for dav1d,
`COPYING.LGPLv2.1`/`v3` for FFmpeg headers, MIT/Apache/NOTICE for renhdrum). The GPL FFmpeg DLLs ship with the GPL text,
hashes and source notes, and the translator ships the Ryujinx MIT notice and both Microsoft .NET notice files, all
required by archive verification.

Two items are not closed:

- the FFmpeg corresponding-source links in `upscalers/ffmpeg/README.md` were not followed, so the GPL offer is asserted
  rather than verified;
- zstd, SDL3, Vulkan-Headers/volk/VMA and Dear ImGui are fetched and compiled with no packaged notice. That is acceptable
  while the runtime is built on the user's machine; it becomes a defect the moment a prebuilt runtime is distributed.

## Automated QA: Amber

Present: 30 release-tooling tests (archive paths, duplicates, case/Unicode collisions, symlinks, traversal, manifest
create/verify/tamper, install matching, engine-tree exclusions), a PowerShell parse gate, `check_release.py`, and a
launcher-test skip baseline that rejects any new skip.

Absent: a runtime compile in the release job, a launch smoke test, any GPU or frame-time test, and any automated check of
the exit path that #77/#78 modify. The suite proves the *packaging* is well formed; it says nothing about whether the
game runs.

## Critical defects to clear before calling a release dependable

1. The release job does not compile the runtime, so an unbuildable archive can be published.
2. #77's fix does not prevent the write-after-decommit it claims to; it removes the diagnostic marker.
3. #78's restore is unverified and is skipped on the crash path it is most needed on.
4. A new user must install a C++ toolchain, LLVM, CMake, Ninja and Python; recovery from a half-installed toolchain
   (#43) is manual.
5. Startup failures on Deck and Arc are unexplained only because no log was collected; support cannot triage them.

## Verdict

**Not ready to be called dependable.** The repository is in a much better state than it was: the release tooling is
real, the archive is verified rather than trusted, dependency omission is structurally prevented, and the documentation
is honest. That is release-engineering progress, not release readiness.

The gap is evidence at the two ends of the lifecycle: nothing proves a published zip builds and launches, and nothing
proves the current exit-path fixes work. Both are cheap relative to their impact (a compile-and-launch gate, and two
logged test runs), and both should land before the next version is described as stable.

Suggested order:

1. Reproduce quit-to-title and capture the log; correct #77 or withdraw the claim.
2. Test #78 on an HDR display in borderless and exclusive; add the log line.
3. Add a runtime compile (and ideally a launch smoke test) to the release workflow.
4. Build and verify one artifact with the new packager; publish its manifest.
5. Request logs for Deck and Arc; retest on 0.1.41+.
6. Fix the #76 elapsed/remaining accounting and the stale commit reference in the results doc.
7. Plan toolchain provisioning; it is the largest remaining barrier for a new user.
