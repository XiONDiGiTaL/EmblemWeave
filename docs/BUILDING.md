# Building, testing and project structure

The player release is built from the same tree. This page covers the Rust launcher, the engine build, the verification scripts and the layout of the repository.

## Building from source

| Tool | Version | Used for |
| --- | --- | --- |
| [Rust](https://rustup.rs) | stable, 1.85 or newer | everything |
| Python | 3.10 or newer | the ZBIC decode helper and verification tools |
| `ziglang` (Python package) | any recent | compiling the emitted C during verification |

```powershell
git clone https://github.com/XiONDiGiTaL/EmblemWeave.git
cd EmblemWeave
cargo build --release -p emblemweave
```

The launcher is `target\release\EmblemWeave.exe`. The command-line tools are
built alongside it:

| Tool | Purpose |
| --- | --- |
| `ew-inspect` | Inspect an ExeFS and RomFS |
| `ew-lift` | Run the recompiler and emit C, verification campaigns and reports |
| `ew-run` | Run the game in the console and print a detailed stop report (`--native` runs the compiled library, `--window` shows its frames) |
| `ew-present` | Exercise the window and swap chain on their own (`ew-present 30` presents 30 frames) |
| `ew-native` | Lift, emit and compile the whole main module into `ew_native.dll` |

For example, to run the game from the command line:

```powershell
target\release\ew-run --module work\decoded\main --romfs D:\Games\FortunesWeave\romfs --nacp D:\Games\FortunesWeave\data\control\control.nacp
```

The baseline engine in `engine/` is a separate checkout with its own build notes (`engine/fefw-recomp/docs/BUILDING.md`).
It builds `fefw.exe` with CMake, Ninja and `clang-cl`; when a vcpkg `libvpx:x64-windows-static` install is present
(under `NXRT_LIBVPX_ROOT`, or in `C:\vcpkg`), the runtime also decodes the title's VP9/VP8 movies on the host.

## Testing and verification

```powershell
cargo test --workspace
```

The repository also carries end-to-end checks that run on your own game data:

| Script | What it proves |
| --- | --- |
| `tools/verify_cfg.sh` | 400 recovered functions produce identical registers in the interpreter and in the compiled C |
| `tools/verify_simd.sh` | every distinct vector encoding behaves identically in both, across edge-case inputs |
| `tools/oracle_lockstep.py` | the interpreter agrees with an independent CPU model instruction by instruction, from a snapshot taken by `ew-run --snapshot-at` |

The verification tools need `pip install ziglang unicorn`.

## Project structure

```
apps/emblemweave/     the EmblemWeave launcher (egui)
assets/branding/     the EmblemWeave logo
crates/ew-format/    container and asset formats (supporting tool)
crates/ew-lift/      decoder, recompiler and reference interpreter (verification)
crates/ew-boot/      experimental runtime work: image, startup, scheduler, host services
crates/ew-gfx/       experimental host Vulkan device and presenter
crates/ew-runtime/   experimental guest memory and service registry
docs/                status, architecture, graphics, legal, upstream, changelog
engine/fefw-recomp/  the integrated primary engine (GPL-3.0, its own licence and notices)
tools/               decoding helper, release and verification scripts
```
