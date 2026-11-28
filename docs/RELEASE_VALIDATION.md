# Release validation

Every release can be checked without the development tree. The checks below use only the Python standard library and
work on Windows, Linux and macOS (the PowerShell packaging step is Windows-only).

## The tools

| Tool | What it proves |
| --- | --- |
| `tools/check_release.py` | `docs/release.json`, the site download button, the README current-release line and that release's changelog row all name the published version. `Cargo.toml` may name a newer development version only when its draft changelog and index row exist; with `--version` (packaging), every pointer must name that version. |
| `tools/verify_archive.py` | Archive paths are relative and `/`-separated, no traversal, no exact duplicate, no case or Unicode-normalization collision, no Windows-reserved name, no symlink/encrypted entry, required files exist, and extraction stays inside the chosen folder. |
| `tools/release_manifest.py` | Every file in the zip is listed with its size and SHA-256; the archive hash, source commit, dirty flag and launcher build stamp are recorded. `verify` recomputes all of it. |
| `tools/verify_install.py` | An unpacked install matches a release manifest. Missing or changed release files fail; unknown extras are reported, while documented user data and generated build output are ignored. |
| `tools/verify_engine_tree.py` | Packaging-side: enumerates the `git ls-files` engine tree, excludes fetched/generated trees, allows only the generated Windows translator, and proves the copied engine has no untracked files. |

The three verifier scripts are also included at `tools/` inside the release zip, so a player can check a download
without cloning the repository.

## Run the tooling tests

```sh
python -m unittest discover -s tools/tests -v
python -m py_compile tools/*.py
python tools/check_release.py
```

The tests build temporary zips; they do not touch a real installation. They cover unsafe paths, exact duplicates,
case and Unicode collisions, symlinks, required files, traversal extraction, manifest creation/verification, tampering,
extra/missing files, tracked-engine copying/exclusions, install matching and user-data exemptions.

On Windows, validate the packaging scripts without running them:

```powershell
foreach ($file in 'tools/package_release.ps1', 'tools/release.ps1', 'tools/install_local.ps1') {
    $errors = $null
    [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $file), [ref]$null, [ref]$errors) | Out-Null
    if ($errors) { $errors | ForEach-Object { Write-Error $_.Message }; exit 1 }
}
```

## Build a local package

From a committed tree (the package script refuses a dirty tree unless you explicitly pass `-AllowDirty`):

```powershell
pwsh -File tools/package_release.ps1 -Version 0.1.41 -RequireConfigureCheck
```

Useful flags:

| Flag | Effect |
| --- | --- |
| `-SkipBuild` | Reuse `target\release\EmblemWeave.exe`; it must match the build stamp for the version and commit. |
| `-AllowStaleBuild` | Warn and proceed when the stamp is missing or mismatched. The manifest records the stale build. Do not use for a published release. |
| `-SkipConfigureCheck` | Skip the extracted-engine CMake configure. |
| `-RequireConfigureCheck` | Fail if LLVM/Visual Studio are unavailable. The release workflow uses this. |
| `-Deterministic` | Pin zip entry timestamps to 1980-01-01. |
| `-AllowDirty` | Package a tree with tracked or untracked changes; the manifest records `source.dirty: true`. |

Outputs:

```text
target/pkg/EmblemWeave-v<version>-windows-x64.zip
target/pkg/release-manifest.json
target/pkg/manifest.json                 # small summary used by tools/release.ps1
```

## Verify an archive

```sh
python tools/verify_archive.py EmblemWeave-v0.1.41-windows-x64.zip \
  --require EmblemWeave-v0.1.41/EmblemWeave.exe \
  --require EmblemWeave-v0.1.41/engine/fefw-recomp/runtime/CMakeLists.txt
```

Add `--extract <empty-folder>` to unpack and check every size. Use a new empty folder; the verifier does not clear an
existing one.

## Verify the engine copy

The engine tree is copied from Git, not from the working directory. To inspect the list or check a copied tree by hand:

```sh
python tools/verify_engine_tree.py list --source .
python tools/verify_engine_tree.py verify --source . \
  --root target/pkg/EmblemWeave-v0.1.41/engine/fefw-recomp \
  --allow tools/shaders/prebuilt/win-x64/spirv_harness.exe
```

`list` emits a JSON array of paths relative to `engine/fefw-recomp/`. The package script uses that same list to copy,
then runs `verify`; a missing tracked file or any extra file (other than the explicitly allowed translator and its
notices) fails the package before the zip is written. Fetched texture/shader/download trees, `.github`, `game`, `work`
and the Linux translator are excluded by the tool's shared policy. The Windows translator folder must contain
`LICENSE.Ryujinx.MIT.txt`, `DOTNET-LICENSE.txt` and `DOTNET-THIRD-PARTY-NOTICES.txt`; archive and CI verification require
those paths explicitly.

```sh
python tools/release_manifest.py verify \
  --archive EmblemWeave-v0.1.41-windows-x64.zip \
  --manifest release-manifest.json
```

A manifest problem means the zip is not the artifact the manifest describes. Do not publish or install it.

## Manifest schema

`release-manifest.json` is schema 1:

```json
{
  "schema": 1,
  "kind": "emblemweave-release-manifest",
  "product": "EmblemWeave",
  "version": "0.1.41",
  "archive": {
    "name": "EmblemWeave-v0.1.41-windows-x64.zip",
    "sha256": "<zip sha256>",
    "size": 117000000,
    "files": 10000,
    "directories": 1200,
    "prefix": "EmblemWeave-v0.1.41/"
  },
  "source": {"commit": "<full git sha>", "dirty": false, "branch": "main"},
  "build": {
    "configuration": "Release",
    "target": "windows-x64",
    "packager": "tools/package_release.ps1",
    "launcher": {"commit": "<full git sha>", "dirty": false}
  },
  "files": [
    {"path": "EmblemWeave-v0.1.41/EmblemWeave.exe", "size": 14500000, "sha256": "<file sha256>"}
  ]
}
```

The manifest is a sidecar, not a zip entry, so it can hash the zip without a circular dependency. It is published next to
the zip and named in the release notes. The full file list makes the JSON a few megabytes; that is intentional.

## Verify an installed folder

```sh
python tools/verify_install.py --root D:\EmblemWeave --manifest D:\EmblemWeave\release-manifest.json
```

The manifest is normally beside the downloaded zip, not inside the install. Keep a copy in the install or its parent if
you want to re-check later.

- Missing or changed manifest files: exit 1.
- Unknown extras: warnings by default; `--strict` makes them failures.
- Ignored as user data/generated output: `data/`, `backups/`, `logs/`, `upscalers/`, `engine/.../game/`, `work/`,
  build folders, `tools/download/`, fetched `third_party` trees and the Linux translator placed under
  `tools/shaders/prebuilt/linux-x64/`.
- A stale file under `docs/`, `recomp/`, `runtime/src/`, or the launcher root is reported; that is how a mixed install is
  found.

This check proves file identity, not that the game launches. Launch testing and GPU/audio behavior remain separate.

## CI

`.github/workflows/release-validation.yml` runs the Python tests, `check_release.py` and PowerShell parsing on every pull
request and on pushes touching the release tools. `.github/workflows/release.yml` runs the same tests, builds the
Windows zip with `-RequireConfigureCheck`, uploads `release-manifest.json`, verifies the manifest again in the publish
job, and writes the exact three-entry `SHA256SUMS` for the Windows zip, Linux translator and manifest with
`tools/release_sums.py finalize` before uploading.

## Release checklist

1. Version pointers agree: `python tools/check_release.py`.
2. Tests pass: `python -m unittest discover -s tools/tests -v`.
3. Build from a clean tree with `-RequireConfigureCheck`.
4. Verify the zip with `verify_archive.py`.
5. Verify `release-manifest.json` against the zip.
6. Verify an extracted copy with `verify_install.py`.
7. Confirm the release notes carry the zip SHA-256.
8. Publish only if every step above passed. The workflow repeats steps 2, 4 and 5 on the downloaded artifact.
