# Portable installation

EmblemWeave is distributed as a folder, not an MSI or an installer. There is no machine-wide registration and nothing is
written outside the folders below unless the user chooses profile storage.

## What is in the release

```text
EmblemWeave-vX.Y.Z/
├── EmblemWeave.exe        launcher; the program to run
├── portable.txt           keep settings, saves and build output in data/ beside the exe
├── LICENSE
├── README.md
├── docs/                  guides, changelog and screenshots
├── tools/                 offline archive/manifest/install verifiers (Python standard library)
├── upscalers/ffmpeg/      pinned FFmpeg shared libraries and their licences
└── engine/fefw-recomp/    engine sources, runtime, game-build projects and tools
```

The archive has one top-level `EmblemWeave-vX.Y.Z/` folder. Do not flatten it into an existing install; unzip it as a
sibling first, or overlay it deliberately as described below.

## First install

1. Download the Windows zip and its `release-manifest.json`.
2. Check the zip before extracting:

   ```powershell
   python tools/verify_archive.py EmblemWeave-vX.Y.Z-windows-x64.zip --extract verify
   python tools/release_manifest.py verify --archive EmblemWeave-vX.Y.Z-windows-x64.zip --manifest release-manifest.json
   ```

   The verifier scripts are in the release zip under `tools/`; run them from the repository or from an already
   extracted copy. Use a new empty `verify` folder.
3. Extract into a folder of its own, for example `D:\Games\EmblemWeave-vX.Y.Z`.
4. Run `EmblemWeave.exe`.
5. On **Game Setup**, choose your dump. On **Overview**, press **Build the game** once, then **Play**.

The build needs Visual Studio C++ tools, LLVM, CMake, Ninja and Python. The launcher checks them and offers install
commands where it can. The shader translator ships in the zip and carries its own .NET runtime; the .NET SDK is not
required to play.

## Portable and profile storage

`portable.txt` beside `EmblemWeave.exe` selects portable mode:

| Data | Portable mode | Profile mode (no marker) |
| --- | --- | --- |
| Settings, saves, mods, logs | `<install>\data\` | `%APPDATA%\EmblemWeave\` (Windows) or `$XDG_CONFIG_HOME/EmblemWeave/` (Linux) |
| Built game and engine output | `<install>\engine\fefw-recomp\game-build\fefw\build\` and `work\` | Same, beside the engine checkout |
| Game dump | The folder chosen on Game Setup; never copied into the release | Same |
| Fetched tools/third-party sources | `engine\fefw-recomp\tools\` | Same |

**About > Storage** shows the folder in use. Do not move `data/` while the launcher is running.

## What must never be deleted during an update

- `data\` (in a portable install): saves, settings, profiles, mods, logs, decoded modules.
- `engine\fefw-recomp\game\` or a separately chosen game folder: the user's own dump. The release never contains it.
- `engine\fefw-recomp\work\`: lifted and decoded output.
- `engine\fefw-recomp\game-build\fefw\build\`: the compiled game and `fefw.exe`.
- `engine\fefw-recomp\tools\.venv`, `tools\bin`, `tools\download`, `tools\textures\third_party`,
  `tools\shaders\third_party`, `tools\shaders\build`: fetched or generated tools; expensive to recreate.
- `backups\`: the previous launcher and save/profile backups.
- `upscalers\` files the player supplied (DLSS/FSR/XeSS DLLs are not all shipped).
- `portable.txt`: its presence or absence is the user's storage choice.

An update replaces the launcher, docs and engine **sources**; it must not remove the list above. The launcher updater
skips `data`, `portable.txt`, the executable and the generated engine paths. See
[UPGRADE_AND_RECOVERY.md](UPGRADE_AND_RECOVERY.md).

## Updating

Use the launcher's **Update available / Update now** action on Overview or About > Updates. It downloads the release,
writes the new launcher, docs and engine sources over the install, keeps the built game and user data, and restarts once.
A launcher version change alone does not force a rebuild; the existing build is recorded as carried forward.

Do not use a generic zip updater that deletes the install folder first. If updating by hand:

- Unzip the new release into the existing folder, allowing it to replace `EmblemWeave.exe`, `docs/` and
  `engine/fefw-recomp/` sources.
- Do **not** delete `data/`, the built game, `work/`, fetched tools or the game dump.
- Or unpack the new release into a fresh folder and keep the old `data/` and `engine/` as described in
  [INSTALL.md](INSTALL.md).

## Verifying an installed copy

If the release manifest is available:

```powershell
python tools\verify_install.py --root D:\Games\EmblemWeave-vX.Y.Z --manifest release-manifest.json
```

The verifier checks every file in the manifest and reports extra files. User data and generated build trees are ignored;
unknown extra source/docs files are warnings unless `--strict` is used. Missing or changed release files always fail.
Use this after a manual overlay, a failed update or a support report.

## Windows path notes

- The archive verifier rejects backslashes, drive letters, `..`, exact duplicates, case/Unicode collisions,
  Windows-reserved names, trailing dots/spaces and characters Win32 cannot create. A zip that passes can be extracted
  on a normal Windows filesystem without producing a flat or truncated tree.
- Spaces, ampersands and Unicode in the install path are supported by the launcher and engine. The build path has
  special handling for `& | < > ^ ( ) %`; see [BUILDING.md](BUILDING.md) and [TROUBLESHOOTING.md](TROUBLESHOOTING.md).
- Very long paths can still hit the Windows 260-character default in third-party tools. Keep the install path moderate
  (for example `D:\EmblemWeave`), or enable long-path support in Windows.
- Do not put the install under a synced folder that rewrites timestamps or locks files during an update.

## Safety

The release tools never delete user data. They verify and report; they do not clean an install. If a check reports a
missing or changed release file, repair it by overlaying the same release or reinstalling the launcher, not by deleting
`data/` or the build.

On a shared or managed PC, coordinate before changing a testing installation that another process is using. Check for a
running `EmblemWeave.exe` and active builds first; replacing a running launcher fails by design.
