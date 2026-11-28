# Release-engineering validation

Independent review of merge `20396fc` (`release-engineering` into `main`), the release tooling it brought in, and
whether the packaging path still permits the v0.1.11 class of failure (a zip missing `dav1d`, `amd_ffx_api`,
`amd_ffx_api_fg`, `ffx_fsr1`, `nvidia_nis`, `nvidia_streamline`).

Audit date: 2026-11-22. Repository state: `main` at `35e2e79`, four commits past the merge under review
(`81861fc`, `406e46f`, `35e2e79`, plus the merge itself), 7 ahead of `origin/main`, not pushed.

## What was actually executed

| Check | Command | Result |
| --- | --- | --- |
| Tooling compiles | `python -m py_compile tools/*.py` | exit 0 |
| Tooling tests | `python -m unittest discover -s tools/tests` | 47 tests, OK |
| Version pointers | `python tools/check_release.py` | `release metadata agrees on 0.1.41 (v0.1.41)` |
| Engine file list | `python tools/verify_engine_tree.py list --source .` | 1,738 tracked files, JSON array |
| Shipped archive | `python tools/verify_archive.py target/pkg/EmblemWeave-v0.1.41-windows-x64.zip --require ...` | ok, 10,167 files in 592 folders |

Not executed: any C++ build, `package_release.ps1` end to end, a clean install on a machine without build tools, an
upgrade of an existing install, or a launch test. Those require a machine and a dump this audit must not use while
FCC is testing, so they are reported as untested rather than assumed.

## The merge

`20396fc` is a two-parent merge of `2b8e960` and `f7bc6c0`, containing `a789301` (artifacts, packaging, recovery),
`9e61f76` (copy the engine tree from tracked files) and `f7bc6c0` (ship translator notices): 25 files, +3,715/−66.
The merge was conflict-free. The working tree was dirty before and after it, and FCC's diff hash was unchanged by the
merge; no runtime source file was touched by these commits.

## Release workflows

`.github/workflows/release.yml` is tag-driven (`push: tags: ['v*']`) with a manual `workflow_dispatch` that defaults to
`dry_run: true`. The manual path refuses to publish (`exit 1` unless `dry_run` is true), so a release can only become
public from a tag. `permissions: contents: write` is scoped to the workflow, and `concurrency: release-${{ github.ref }}`
with `cancel-in-progress: false` prevents two publishes for one tag from interleaving.

The job graph is `prepare → windows + linux-translator → publish → site`:

- **prepare** rejects a version that disagrees with `docs/changelog/<version>.md` or `Cargo.toml`, then runs
  `check_release.py` and the 47 tooling tests. A missing changelog or a stale site pointer therefore fails before
  anything is built.
- **windows** stages the pinned Ryujinx commit (`RYUBING_REV`) into `tools/shaders/third_party/ryubing`, builds the
  launcher and zip with `-RequireConfigureCheck`, and uploads `*.zip`, `manifest.json` and `release-manifest.json`.
- **linux-translator** publishes the self-contained `linux-x64/spirv_harness`, checks that it exists and is executable,
  tars it, and verifies the tar (including the executable bit) before upload.
- **publish** downloads both artifacts, re-verifies the zip against an explicit `--require` list, verifies
  `release-manifest.json`, writes the exact three-entry `SHA256SUMS` with `tools/release_sums.py finalize`, then creates
  or edits the GitHub release.
- **site** stamps `docs/index.html` and `docs/release.json`, commits them to `main`, and runs a post-deploy smoke check
  against `https://xiondigital.net/EmblemWeave/`.

`.github/workflows/release-validation.yml` is the fast gate: `py_compile`, the same 47 tests, `check_release.py`, and a
PowerShell parse of the three release scripts. It runs on every pull request and on pushes touching `tools/**`,
`release*.yml`, `docs/release.json`, `Cargo.toml` or `README.md`, with `contents: read`.

`.github/workflows/launcher-tests.yml` builds `mpr_launcher_tests` and asserts a skip baseline with
`tools/check_launcher_skips.py`, so "exactly two skips" is checked rather than assumed.

### Workflow defects

1. **The release gate never compiles the runtime.** `-RequireConfigureCheck` runs the engine's CMake *configure* of the
   extracted zip. Configure success proves the source tree is complete enough for CMake; it does not prove the C++
   compiles, links, or starts. A tag can therefore publish a zip whose runtime does not build. `launcher-tests.yml`
   does compile `mpr_launcher_tests`, but only on PRs/pushes matching its `paths` filter, and it is not a dependency of
   `release.yml`. Severity: high, because it is the one check that would have caught an unbuildable tree before users did.
2. **No launch or smoke test of the packaged game.** The only post-publish check is the website version stamp. Nothing
   starts `EmblemWeave.exe`, opens the launcher, or verifies that a build from the extracted zip reaches a window.
3. **The `site` job writes to `main`.** `git pull --rebase origin main && git push origin HEAD:main` after publishing
   means a developer push racing the job fails the release *after* the GitHub release exists. The release survives; the
   job goes red. Low severity, but it makes a green run depend on branch timing.
4. **`-RequireConfigureCheck` depends on runner tooling.** It needs VS C++ tools and `clang-cl` on `windows-latest`;
   `env.cmd` is what resolves them. This audit did not run the step, so the assumption that the hosted image satisfies
   `env.cmd` is unverified.

## Packaging and dependency completeness

`tools/package_release.ps1` was reviewed line by line. Its guards are materially better than a folder copy:

- Refuses a dirty tree unless `-AllowDirty`; refuses `-SkipBuild` when `target/release/emblemweave-build.json` is
  missing or names another version/commit/dirty state unless `-AllowStaleBuild`.
- Rejects a version that is not `major.minor.patch`, and proves the package path stays under `target\pkg`.
- Copies the engine from `git ls-files` via `verify_engine_tree.py list`, then runs `verify` on the copied tree so an
  untracked object file or experimental source cannot enter the zip. Only the generated Windows translator and its three
  notices are allowed as extras.
- Writes the zip entry by entry with `/` separators, a `EmblemWeave-v<version>/` prefix, explicit directory entries and
  optional fixed timestamps, then re-opens the written zip with `verify_archive.py --extract` and compares the extracted
  file count to the number written.
- Pins the three FFmpeg DLLs by SHA-256, requires `SHA256SUMS` to pin the same hashes, and ships `LICENSE.txt`,
  `COPYING.GPLv3`, `README.md` and `SHA256SUMS` beside them.
- Produces `release-manifest.json` with a SHA-256 and size for every entry, the source commit, dirty flags and the
  launcher build stamp, then verifies the manifest against the archive it just described.

### The v0.1.11 failure class

The six folders named in the v0.1.11 report now live under `engine/fefw-recomp/runtime/third_party/` and are tracked:

| Folder | Tracked files | In the shipped v0.1.41 zip |
| --- | ---: | ---: |
| `amd_ffx_api` | 7 | 13 |
| `amd_ffx_api_fg` | 6 | 8 |
| `ffx_fsr1` | 9 | 11 |
| `nvidia_nis` | 4 | 5 |
| `nvidia_streamline` | 26 | 28 |
| `dav1d` | 140 | 153 |

(The zip counts include directory entries; the file counts agree with `verify_engine_tree.py list` plus the packaged
docs and tools.)

`runtime/third_party/` is deliberately *not* in `verify_engine_tree.EXCLUDED_PREFIXES`; only the fetched
`tools/textures/third_party/`, `tools/shaders/third_party/` and `tools/download/` trees are excluded. So anything tracked
under `runtime/third_party/` is copied, and a tracked file missing from the working tree aborts the package
(`tracked engine file is missing from the working tree`). `release.yml` additionally requires
`runtime/third_party/dav1d/CMakeLists.txt` and `runtime/third_party/ffmpeg/include/libavcodec/avcodec.h` plus the FFmpeg
licence by name in the publish job.

**Conclusion:** for engine-side dependencies, the v0.1.11 class of failure is structurally prevented. It is no longer
possible to add a tracked third-party directory and have it silently omitted; the list, the copy and the verification all
come from the same `git ls-files` enumeration. This is the strongest part of the merge.

### Remaining packaging gaps

1. **`upscalers/` is not generic.** The script copies `upscalers\ffmpeg` only. Today that is the only vendor folder in
   the repository, and the AMD/NVIDIA/Intel upscaler DLLs are documented as user-supplied. But if a future release ships
   `upscalers\fsr` or `upscalers\dlss`, nothing copies it and no check requires it. A `git ls-files upscalers` copy with
   an explicit allowlist, or a `--require` for each shipped vendor folder, would close the same class one level up.
2. **The v0.1.41 zip predates this merge.** It was written on 2026-11-20 14:45, before `20396fc` (2026-11-22 14:26). It
   contains the FFmpeg DLLs and the six third-party trees but **not** `docs/RELEASE_VALIDATION.md`,
   `docs/PORTABLE_INSTALLATION.md`, `docs/UPGRADE_AND_RECOVERY.md` or `tools/verify_*.py`, because the old packager did
   not ship them. Any claim that "the new packaging is proven by 0.1.41" is false: the first post-merge package has not
   been produced. Everything above about the new script is a reading of the script plus its tests, not an artifact.
3. **The translator notices are in two places on purpose.** `tools/shaders/licenses/` holds the tracked copies, and
   `prebuilt/win-x64/` receives them at package time. Both are verified; this is correct, but it means editing one
   without the other fails the package, which is the intended behaviour and should stay documented.
4. **`release-manifest.json` is a sidecar.** It is not inside the zip, so a user who downloads only the zip cannot verify
   per-file hashes later. `docs/RELEASE_VALIDATION.md` says so; the release notes must keep naming the manifest or the
   guarantee is only theoretical.

## Installation verification and recovery

`tools/verify_install.py` compares an unpacked folder against a manifest: missing or changed files fail, unknown extras
warn (fail with `--strict`), and `data/`, `backups/`, `logs/`, `upscalers/`, `engine/.../game/`, `work/`, build folders,
`tools/download/`, the fetched `third_party` trees and `prebuilt/linux-x64/` are treated as user or generated content.
`release-manifest.json` and `SHA256SUMS` are ignored as release-owned extras. Tests cover a matching install, user data,
tampering, extras, and a missing file.

The updater in `apps/emblemweave/src/updater.rs` matches the documentation at the level checked: `clean_cache()`,
`finish_interrupted_update()`, `BACKUP_FILE = "EmblemWeave.exe.old"`, `carried_by` in `fefw.ini`, and
`update-failed.txt` all exist, and the updater has its own tests (including `carried_by = 0.1.40` and the restored
`backups/EmblemWeave.exe.old`).

`docs/UPGRADE_AND_RECOVERY.md` is honest that the overlay "is not a filesystem transaction" and that the
snapshot/rollback journal is not implemented. That is a real residual risk for an interrupted update: the previous
launcher starts and `data/` is preserved, but the source tree can be mixed, and the only remedy is a manual overlay plus
`verify_install.py`. A manifest preflight in the updater (verify staged files before replacing any) is the highest-value
follow-up and is already described as such in the document.

**Clean install, upgrade install, and recovery from a corrupted install were not performed.** They need a machine
without VS/LLVM/CMake/Ninja/Python and a disposable install folder; neither was used, in keeping with the coordination
restriction on FCC's testing.

## Legal and attribution review

Every vendored third-party tree ships its licence:

| Tree | Notice file(s) tracked |
| --- | --- |
| `amd_ffx_api`, `amd_ffx_api_fg`, `ffx_fsr1`, `nvidia_nis` | `LICENSE.txt` (+ `README.txt`) |
| `nvidia_streamline` | `license.txt` |
| `intel_xess` | `LICENSE.txt` (Intel Simplified Software License, not OSI) |
| `renhdrum` | `LICENSE-APACHE`, `LICENSE-MIT`, `NOTICE` |
| `dav1d` | `COPYING` |
| `ffmpeg` (headers) | `COPYING.LGPLv2.1`, `COPYING.LGPLv3` |

The FFmpeg shared libraries in `upscalers/ffmpeg` are GPL-2.0-or-later builds and ship with `LICENSE.txt`,
`COPYING.GPLv3` and a `README.md` recording version, hashes and source links. The translator binary ships
`LICENSE.Ryujinx.MIT.txt`, `DOTNET-LICENSE.txt` and `DOTNET-THIRD-PARTY-NOTICES.txt` beside it, and archive verification
requires all three.

Residual legal gaps:

- `docs/DEPENDENCY_LICENSE_AUDIT.md` correctly records that zstd, SDL3, Vulkan-Headers/volk/VMA and Dear ImGui are
  fetched and compiled into the runtime with "no separate notice currently packaged". That is acceptable while the
  runtime is compiled on the user's machine from source, but any future prebuilt runtime would need those notices.
- The GPL corresponding-source obligation for the shipped FFmpeg DLLs rests on the links in
  `upscalers/ffmpeg/README.md`. The links were not followed in this audit; they must resolve to the exact build, and the
  version/hashes in `SHA256SUMS` are what make that checkable.

## Outstanding defects, by severity

| # | Defect | Evidence | Severity |
| --- | --- | --- | --- |
| 1 | Release publishes without compiling the runtime; configure-only gate | `.github/workflows/release.yml` windows job; `package_release.ps1` step 6 | High |
| 2 | No launch/smoke test of the packaged launcher or game | `release.yml` publish/site jobs | High |
| 3 | The only verified zip (0.1.41) predates the merge and lacks the new docs/tools | `target/pkg/...0.1.41...zip`, mtime 2026-11-20 14:45 | High (for any "validated" claim) |
| 4 | `upscalers/` copy is hard-coded to `ffmpeg` | `package_release.ps1` step 3b | Medium (latent) |
| 5 | Updater overlay is not transactional; no manifest preflight | `updater.rs`, `docs/UPGRADE_AND_RECOVERY.md` | Medium |
| 6 | Site job commits to `main` after publishing | `release.yml` site job | Low |
| 7 | FFmpeg corresponding-source links unverified | `upscalers/ffmpeg/README.md` | Medium (legal) |

## Verdict

The merge is a genuine improvement: packaging is enumerated from Git, the archive is checked rather than assumed, and a
per-file manifest makes an install auditable. The previously omitted dependency folders are tracked and mechanically
included, and the shipped 0.1.41 zip confirms their presence.

It is not yet a *release gate*. The workflow proves a zip is well formed and that its engine tree configures; it does not
prove the runtime compiles, that the launcher starts, or that the game reaches a frame. Until those exist, "the release
workflow passed" means "the archive is internally consistent", not "the download works".
