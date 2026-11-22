# Building and running the Linux prototype

This page is command-first: what to install, how to build and test the platform layer, how to type-check the launcher,
and how the optional Proton path is launched. It describes the isolated `linux/compat-prototype` branch, not a
supported release.

## 1. Host requirements

* Ubuntu 26.04 (tested), Ubuntu 24.04 or Debian 13 should work with the same package names.
* x86-64. AArch64 is not claimed.
* For the platform layer: no GPU is required. For the engine: a Vulkan 1.3 driver (Mesa RADV, NVIDIA, Intel).
* For Proton mode: a real Steam installation with the Proton Hotfix installed. WSL is not a supported host for it.

## 2. Dependency manifest

| Package | Why | Upstream licence |
| --- | --- | --- |
| `clang`, `lld` | C/C++ compiler and linker for the runtime | Apache-2.0 WITH LLVM-exception |
| `cmake` (≥ 3.25) | build configuration | BSD-3-Clause |
| `ninja-build` | build executor | Apache-2.0 |
| `pkg-config` | library discovery for SDL3 and friends | GPL-2.0-or-later (pkgconf: ISC) |
| `libvulkan-dev` | Vulkan headers and loader soname | Apache-2.0 / MIT |
| `libzstd-dev`, `zlib1g-dev` | shader pack and RomFS compression | BSD-3-Clause / Zlib |
| `libsdl3-dev` (optional; CMake can fetch) | window, gamepad, Vulkan surface | Zlib |
| `libpipewire-0.3-dev`, `libasound2-dev` | audio backend (P4) | MIT / LGPL-2.1 |
| `libavcodec-dev`, `libavformat-dev`, `libavutil-dev`, `libswresample-dev` | video/audio decode path (P4) | LGPL-2.1-or-later |
| `rustup` + stable Rust | launcher | MIT / Apache-2.0 |
| `dotnet-sdk-10.0` (optional) | shader translator from source; releases ship a prebuilt ELF | MIT |

`libxkbcommon-dev`, `libwayland-dev`, `libx11-dev`, `libxcursor-dev`, `libxi-dev`, `libxrandr-dev`,
`libegl1-mesa-dev`, `wayland-protocols` are needed to **link** the launcher on a desktop; they are not needed for
`cargo check`.

Install:

```bash
sudo apt update
sudo apt install -y clang lld cmake ninja-build pkg-config \
                    libvulkan-dev libzstd-dev zlib1g-dev \
                    libsdl3-dev libpipewire-0.3-dev libasound2-dev
# desktop link only:
sudo apt install -y libxkbcommon-dev libwayland-dev libx11-dev libxcursor-dev \
                    libxi-dev libxrandr-dev libegl1-mesa-dev wayland-protocols
```

## 3. Platform layer: build, test, probe

Build outside the repository so no artifact lands in the tree:

```bash
W=$PWD/engine/fefw-recomp/runtime
cmake -S "$W" -B ~/ew-linux/build-platform -G Ninja \
      -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ \
      -DCMAKE_BUILD_TYPE=Release
cmake --build ~/ew-linux/build-platform
ctest --test-dir ~/ew-linux/build-platform --output-on-failure
```

Expected: `1/1 Test #1: platform ... Passed`. The build prints
`nxrt: platform layer only (.../src/platform/posix); the full runtime is Windows-only, see docs/LINUX.md`.

Run the diagnostic:

```bash
~/ew-linux/build-platform/nxrt_linux_probe
```

Expected shape (the device differs by host):

```
nxrt linux probe
kernel: Linux ... x86_64
page size: 4096
cores: 8 physical, 16 online
config root: /home/<user>/.config/EmblemWeave
cache root: /home/<user>/.cache/EmblemWeave
vulkan loader: libvulkan.so.1 opened
loader api version: 1.4.341
vulkan devices: 1
  [0] <device> | discrete | api 1.3.xxx | driver 0x... | vendor 0x... device 0x...
```

Exit codes: `0` loader and at least one device, `1` loader but no device, `2` loader missing, `3` instance/entry
points failed.

## 4. Launcher: type-check and build

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
source ~/.cargo/env
rustup target add x86_64-unknown-linux-gnu      # native; harmless if already present
CARGO_TARGET_DIR=~/ew-linux/cargo-target \
  cargo check -p emblemweave --target x86_64-unknown-linux-gnu
```

Expected: `Finished dev profile ...`, exit 0. This is verified on Ubuntu 26.04 with rustc 1.100.0.

To link and run on a desktop (not verified in WSL):

```bash
cargo build --release -p emblemweave --target x86_64-unknown-linux-gnu
./target/x86_64-unknown-linux-gnu/release/EmblemWeave
```

On Linux the launcher's Game Setup **refuses Build the game** by design: the engine build path does not exist yet.
That refusal is the correct current behaviour, not a bug to work around.

## 5. Proton compatibility mode (unverified)

Use the Proton that Steam installed; do not copy or rebuild it. In Steam:

1. Add the Windows launcher (`EmblemWeave.exe`) as a non-Steam game, or launch the installed build if it is already in
   the library.
2. Properties → Compatibility → Force the use of a specific Steam Play compatibility tool → **Proton Hotfix**.
3. Launch from the library. No terminal is required.

The manual equivalent, for diagnosis only, needs the variables Steam sets:

```bash
export STEAM_COMPAT_CLIENT_INSTALL_PATH="$HOME/.steam/steam"
export STEAM_COMPAT_DATA_PATH="$HOME/.steam/steam/steamapps/compatdata/<appid>"
export STEAM_COMPAT_APP_ID=<appid>
"$HOME/.steam/steam/steamapps/common/Proton Hotfix/proton" run /path/to/EmblemWeave.exe
```

Notes:

* `proton` expects the Steam Linux Runtime (pressure-vessel); outside Steam it exits or hangs during prefix setup.
* `PROTON_LOG=1` writes a log; `PROTON_LOG_DIR` chooses the directory.
* Wine's XAudio2 and Media Foundation are what make audio and cinematics work in this mode. DXVK/VKD3D are not used by
  EmblemWeave's Vulkan renderer.
* This path has **not** been verified in this workstream. See the test matrix.

## 6. Troubleshooting

| Symptom | Check |
| --- | --- |
| `no Vulkan loader found` | `ldconfig -p \| grep libvulkan.so.1`; install `libvulkan1` |
| Loader present, no device | `vulkaninfo --summary`; install the Mesa/NVIDIA ICD for the GPU |
| Probe reports llvmpipe only | The host has no GPU driver; software Vulkan is not a gameplay target |
| `cargo check` fails on a system library | Install the desktop `-dev` packages above; check `pkg-config --modversion xkbcommon` |
| Proton prefix hangs | Expected outside Steam; use a real Steam installation |
| CMake picks up Windows flags | You are on the full runtime path; the Linux prototype builds only the platform layer today |

## 7. What is deliberately not here

No Proton fork, no bundled Wine, no DXVK/VKD3D dependency, no second engine, no launch script that hides a failure.
The prototype is the platform layer, its test, the probe, and the launcher compile; everything else is in
`docs/LINUX_IMPLEMENTATION_PLAN.md`.
