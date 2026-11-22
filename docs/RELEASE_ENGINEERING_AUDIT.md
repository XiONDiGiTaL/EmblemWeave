# Release engineering audit

Date: 2026-11-22

Scope: packaging, archive integrity, release metadata, artifact provenance, install verification and release CI.

Branch: `release-engineering`, cut from `2732561` (release 0.1.41). The main checkout was left untouched because an
active engine build and uncommitted engine/launcher work were present there.

This audit does not cover renderer architecture, frame pacing, CPU parallelism, guest-thread scheduling, audio decoding,
or the build configuration being changed by the engine owner. No engine or launcher implementation file was edited.

## Pipeline

The release path is:

1. `tools/check_release.py` checks that `Cargo.toml`, `docs/release.json`, the site download button, the README and the
   changelog index all name the same version.
2. `tools/package_release.ps1` builds the launcher (`cargo build --release -p emblemweave`), publishes the shader
   translator if needed, copies the tracked release tree, pins the three FFmpeg DLLs, writes the zip, verifies it,
   optionally configures the extracted engine, and writes `manifest.json`.
3. `.github/workflows/release.yml` (tag or manual dry run) repeats the package build, verifies both archives, writes the
   release notes and publishes the GitHub release. A dry run stops before publishing.
4. The launcher's updater (`apps/emblemweave/src/updater.rs`) downloads the Windows zip, unpacks it to a temporary cache,
   overlays the release over the install, keeps `data/` and generated build output, backs up the old executable and swaps
   the launcher after exit.

## Findings

| ID | Severity | Finding | Evidence | Status |
| --- | --- | --- | --- | --- |
| R1 | High | A release had no per-file integrity record. The zip hash proves the download but not which files were inside or whether one was replaced after unpacking. | `package_release.ps1` wrote only `manifest.json` with version/zip/hash/size. | Fixed: `tools/release_manifest.py` writes a sidecar SHA-256 for every file, plus source commit, dirty flag and launcher build stamp. Package, local release and CI verify it. |
| R2 | High | `-SkipBuild` could package `target\release\EmblemWeave.exe` from an older version or tree. The only check was that the file existed. | No build stamp or version comparison in the package script. | Fixed: a successful build writes `target\release\emblemweave-build.json`; `-SkipBuild` compares version, commit and dirty state and fails unless `-AllowStaleBuild` is explicit. |
| R3 | High | The package copied every `docs/*.md` file, including untracked experimental or another agent's notes. | `Get-ChildItem docs -Filter *.md` copied the working tree, not the release commit. | Fixed: the package reads the list from `git ls-files` for `docs/*.md`, `docs/changelog` and `docs/screenshots`; the engine tree is now tracked-only as well (R12). |
| R4 | High | A dirty tree could be packaged as if it were the tagged source. | No dirty-tree check. | Fixed: packaging fails on tracked or untracked non-ignored changes unless `-AllowDirty` is passed; the manifest then records `source.dirty: true`. |
| R5 | Medium | The engine CMake configure check was best-effort: a machine without LLVM/Visual Studio printed a warning and the package still succeeded. CI could therefore claim a verified zip without configuring it. | `package_release.ps1` warning path. | Fixed: `-RequireConfigureCheck`, used by the release workflow, turns a missing toolchain into a failure. |
| R6 | Medium | The archive verifier did not reject exact duplicate entries, Windows-reserved names, trailing dots/spaces, alternate-data-stream colons, symlink entries or encrypted entries. | `tools/verify_archive.py` checked only backslashes, traversal, case collisions and required files. | Fixed in the verifier; standard-library tests cover each case. |
| R7 | Medium | `upscalers/ffmpeg/SHA256SUMS` was copied to players without being compared with the hard-coded DLL hashes. | The package script verified the DLLs, then copied the sums file. | Fixed: every pinned DLL must appear in `SHA256SUMS` with the same hash. |
| R8 | Medium | Zip entry timestamps came from the checkout, so two identical trees produced different zip bytes. | Zip writer used each file's `LastWriteTime`. | Fixed for the metadata: `-Deterministic` pins entry times to 1980-01-01. .NET's deflate output can still differ between runtime versions. |
| R9 | High | The launcher updater does not verify a manifest or per-file hash before overlaying a download, and the overlay is not a single transaction. A disk-full or permission failure can leave some release files replaced and others old while the old launcher starts. Only the executable is backed up. | `updater.rs`: `download()` uses `curl` and `tar`; `overlay_tree()` copies file by file; `backups\EmblemWeave.exe.old` is the only saved release component. | Not changed here. This is launcher update logic; the owner should add manifest preflight and an all-file staging/rollback step. Until then, use the recovery procedure in [UPGRADE_AND_RECOVERY.md](UPGRADE_AND_RECOVERY.md) and verify the install afterward. |
| R10 | Medium | There was no way to answer "does this folder match the published release?" after an update, a manual overlay or a support report. | No install-side checker. | Fixed: `tools/verify_install.py` checks every manifest file, reports missing/changed files and lists unknown extras while ignoring documented user-data/generated paths. |
| R11 | Medium | The release tooling had no automated tests and CI ran none. | No `tools/tests`; release workflow built and published only. | Fixed: 29 standard-library tests and a fast `.github/workflows/release-validation.yml` for Python and PowerShell syntax. |
| R12 | Medium | The engine source tree is copied with `robocopy`, not from Git's tracked file list. An ignored but present generated file is excluded by name, but a non-ignored untracked engine file is only caught by the dirty-tree guard. | `robocopy 'engine/fefw-recomp' ...`. | Fixed: `tools/verify_engine_tree.py` supplies the tracked list, explicitly adds the generated Windows translator, and verifies the copied tree has no untracked files. `.github` and fetched/generated trees are excluded by one shared policy. |
| R13 | Low | `verify_archive.py --extract` reuses an existing directory, so a stale file can be mistaken for an extracted one. | `os.makedirs(extract, exist_ok=True)`. | Remaining. The package script removes its extraction directory first; standalone users should use a new empty path. |
| R14 | Low | Required-file lists are duplicated between the package script and the release workflow. | Two hand-written `--require` lists. | Partially reduced: the package now uses one list for verify and manifest. CI still repeats a subset. |
| R15 | Low | The zip's Windows version resource is not cross-checked against `Cargo.toml`. The source stamp catches a stale build, but a hand-copied executable with the same stamp would not be caught by version metadata alone. | `build.rs` embeds icon and description; no package-side `VersionInfo` check. | Remaining. The build stamp and manifest are the current provenance guard. |

## What changed

- `tools/release_manifest.py` creates and verifies a schema-1 release manifest. It refuses an archive that
  `verify_archive.py` rejects and enforces one `EmblemWeave-v<version>/` prefix.
- `tools/verify_install.py` compares an unpacked folder with the manifest. User data, backups, upscalers, game data,
  lifted output, build folders, fetched toolchains and third-party downloads are ignored as expected extras.
- `tools/verify_archive.py` now rejects exact duplicates, Unicode-normalization collisions, Windows-reserved names,
  illegal Win32 characters, trailing dots/spaces, symlink entries and encrypted zip entries. It catches extraction errors
  instead of aborting with a traceback.
- `tools/package_release.ps1` adds the build stamp, dirty-tree and stale-build guards, deterministic timestamps,
  `-RequireConfigureCheck`, tracked-doc copying, FFmpeg sums validation, the sidecar manifest and the three verifier
  scripts in the package.
- The engine tree is copied from `tools/verify_engine_tree.py list`, never with `robocopy`. The list comes from
  `git ls-files`; `.github`, the dump/work trees, fetched texture/shader/download trees and the Linux translator are
  excluded. The generated Windows translator is added explicitly and the copied tree is verified before archiving.
- `tools/release.ps1` writes `SHA256SUMS`, verifies the manifest before upload and publishes the zip, manifest and sums.
- `tools/install_local.ps1` stages the new launcher beside the target, verifies its hash, atomically replaces the target
  and restores the pre-install copy if the replacement or final hash check fails.
- `.github/workflows/release.yml` runs the release tests, requires the configure check, uploads the manifest and verifies
  it before publish. `.github/workflows/release-validation.yml` runs the fast checks on pull requests.

## Reproducibility

The manifest records:

```text
version, archive name/size/sha256, top-level prefix, file count
source.commit, source.dirty, source.branch
build.configuration = Release, build.target = windows-x64, build.packager
build.launcher.commit, build.launcher.dirty
files[]: path, size, sha256
```

`-Deterministic` removes checkout mtimes from the zip and entries are written in sorted path order. The compression
stream is still .NET's; a byte-identical zip across different .NET versions is not promised. The manifest and source
stamp are the stronger identity: they name the exact tree that produced the zip.

A release is independently checkable with:

```powershell
python tools/verify_archive.py EmblemWeave-v0.1.41-windows-x64.zip --extract verify
python tools/release_manifest.py verify --archive EmblemWeave-v0.1.41-windows-x64.zip --manifest release-manifest.json
python tools/verify_install.py --root verify/EmblemWeave-v0.1.41 --manifest release-manifest.json
```

## Verification performed

- `python -m unittest discover -s tools/tests -v`: 29 tests passed.
- `python -m py_compile tools/*.py`: passed.
- `python tools/check_release.py`: release metadata agrees on 0.1.41.
- PowerShell parser: `package_release.ps1`, `release.ps1` and `install_local.ps1` parse.
- A two-thread probe of the zstd recent-file helper from the preceding fix passed separately (main thread read its own
  save entry; worker read its own RomFS entry).

A synthetic end-to-end package was also run with a fake launcher and fake translator, the real tracked docs and engine
tree, `-SkipConfigureCheck`, `-AllowDirty` and `-AllowStaleBuild`: the engine check accepted 1,735 tracked files plus the
translator, the archive had 1,841 files (46.7 MB), and archive verification, manifest creation/verification and a strict
install check all passed. This exercised the packaging/verifier wiring, not a real launcher or translator.
`tools/install_local.ps1` was tested against temporary installs for both a replacement
(with a pre-install backup) and a fresh target (no backup). The first test found a real bug: `File.Replace` cannot take a
null backup path; it now uses the pre-install copy and passed both paths.

Not run in this worktree: a full `cargo build`/engine build, a real package with the actual launcher and translator, the
CMake configure check, and the launcher Rust test suite. The main checkout had an active engine build and uncommitted
work; the release scripts were validated without competing for that build tree. The release workflow is the place to run
the full packaging check.

## Remaining work

1. Make the launcher updater verify `release-manifest.json` before overlaying and stage the whole release for rollback
   (R9). This is the highest-value follow-up and belongs with the updater owner.
2. Add a package-side Windows `VersionInfo` check once the resource version is guaranteed to match `Cargo.toml` (R15).
3. Require an empty extraction directory in `verify_archive.py`, or add `--clean-extract` (R13).
4. Generate the required-file list from one checked-in source for both PowerShell and CI (R14).
