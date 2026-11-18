# Development status

Measured on the retail game (version 1.1.0, update 131072) on 16 November 2026.


## Playable build

The game builds from your own dump and plays. The launcher's **Build the game** runs fefw-recomp's lifter, compiles the
six modules and links `fefw.exe`; **Play** starts it with the chosen graphics, audio and controller settings.

- **0.1.17 play test**, AMD Radeon RX 7900 XT: about 60 fps with sound, the opening movie and cutscenes, the title
  screen, an existing save loaded through **Continue** into chapter 4, the hub walked with `W`, the in-game menu on
  `Tab`, the engine's settings overlay on `F1`, 47 to 68 fps and no errors in the log.
- **0.1.31 headless run**, `.nx` archive: the engine built a 252,693-file RomFS image and ran 25 s, exit 0.
- **0.1.32 headless run**, `.nup` package that packs 65,074 of its 252,686 files with zstd: full image, 25 s, 12,163
  presents, 85 textures, no crash, no unreadable file, with no unpacked copy on disk.
- **0.1.32 code-mod test**: a DLL built with clang-cl against `nxrt/mod_api.h` loaded from `mods/TestMod/exefs`, ran
  `nxmod_init`, saw 30 frames, and was told when one of its `.sites` entries was not in the build.

Requirements are listed under [System requirements](REQUIREMENTS.md); failures and their messages under
[TROUBLESHOOTING.md](TROUBLESHOOTING.md).

## Engine integration

- `engine/fefw-recomp` is the integrated primary engine: lifter, generated code, runtime, services, graphics, audio and
  its own documentation. It is built from source in this repository, not launched as an external program.
- The launcher owns the build around it: toolchain probe, CMake cache and build folder handling, the Python decode step,
  the lift, the per-module compile and the link, plus the hook-site merge for code mods.
- The engine's configuration (`fefw.ini`, `controls.ini`, `NXRT_*` variables) is written by the launcher's pages and
  documented in `engine/fefw-recomp/runtime/docs/CONFIG.md`.
- Crash reports from the engine are read back by the launcher, which shows the decoded stop, the modules and the log.

## Compiler and toolchain

- The lifter, the generated C++ and the runtime are compiled with LLVM clang-cl and lld-link through CMake and Ninja. The
  launcher validates the exact executables and the Visual Studio environment before a build starts, records their
  fingerprint, and refuses to mix builds from different compilers.
- **Two processor targets.** The lifted code is built as `fma` (FMA3 + AVX) or `compat` (SSE4.2 + POPCNT, no FMA3).
  `apps/emblemweave/src/cpu.rs` probes the processor and passes `FEFW_CPU_VARIANT`; the engine re-checks at start and
  writes `fefw_fatal.txt` and exit code 4 naming anything missing. Both configure paths are verified: the `compat` tree
  contains no `-mfma`, and its stub compiles with `FEFW_CPU_NEEDS_COMPAT_BUILD`.
- Build folders are reused when the compiler, generator and toolchain file match, including a toolchain path spelled
  with `..`; a folder that lost its compiler is reset instead of failing.
- An optional `sccache` is used when present. The build takes a lock so two builds cannot write the same objects.

## Runtime

- The engine maps the six modules, runs the console SDK's start-up, and serves the game's system calls natively: kernel
  objects, threads and synchronization, `fsp-srv` with RomFS (folder, `.nsp`, `.nx`, `.nup`, plaintext `.xci`, zstd entries) and save data,
  `aoc:u`, `hid`, `time`, `pctl`, `lm`, `csrng`, applet and audio services.
- NVN is served on the host GPU through Vulkan. Movies decode with dav1d and libvpx on the host.
- Saves live in the launcher's profile directory and are imported from Switch emulators.

## Graphics

- Vulkan 1.3 NVN shell, HDR output, swap-chain management, shader translation with a disk cache, GPU texture transcoding
  (BC, ASTC) and frame pacing.
- Upscaling: built-in TAAU, FSR 1, NVIDIA Image Scaling, FSR 3.1, DLSS / DLAA and XeSS, each with its own availability
  check. **FSR 3 frame generation works**: with the FidelityFX SDK 1.1.4 DLLs, the engine turns 60 rendered fps into 120
  presented fps on an RX 7900 XT. The Graphics page has a one-press AMD preset for FSR 3.1 + frame generation.
- Optional **ReShade**, verified end to end with the Add-on build of 6.8.0: the layer loads into the game, the overlay
  opens with `Home`, and the launcher's switch keeps it out of the game process. The launcher detects the 64-bit Vulkan
  layer, checks this build in `ReShadeApps.ini`, opens the official setup for `fefw.exe` with Vulkan preselected, and
  clears the modifier rule 6.8 adds to the overlay key. ReShade stays a separate user install; the add-on build supports
  `.addon64` files and user presets.
  Expand **ReShade (optional)** on Graphics, press **Set up ReShade…**, select the Add-on build if you use add-ons, and
  install the effect packs you want. Put presets, shader packs and `.addon64` files beside `fefw.exe`; press `Home` in
  game for the overlay. The **Load ReShade** switch disables the layer for this game without uninstalling it.
- The evaluation of further options (FSR 4, OptiScaler, vkBasalt) and their status are in
  [GRAPHICS.md](GRAPHICS.md).

## Audio

- The audio renderer runs on its 5 ms clock through XAudio2, and the game's voice buffers, sound effects and music reach
  it; the runtime log reports underruns, accepted and dropped buffers, and queue depth.

## Input

- Keyboard and up to four controllers through SDL 3 and XInput, with mappings in `controls.ini`, editable in the
  launcher's Controls page. The in-game menu opens on `F10` (or Back + RB) and can pause, screenshot, restart or stop.

## Launcher features

- Game data validation and preparation, builds with eight visible stages, save profiles and a save editor, Atmosphere
  cheats and Cheat Engine tables, quick cheats, an in-game menu, RomFS and code mods with profiles and conflict listing,
  GameBanana browsing, graphics and control settings, diagnostics with export, crash reports, six colour schemes and
  portable mode.

## Experimental components

The Rust crates in `crates/` are development tools, not the engine that runs the game:

| Area | Result |
| --- | --- |
| `ew-lift` decoder | 99.97% of 6,714,569 instruction words, including Advanced SIMD, system registers and supervisor calls |
| Control-flow recovery | 304,909 basic blocks in 3,620 complete functions (4,000-function pass); 151 switch tables resolved |
| Verification | 400 recovered functions and 2,400 vector-instruction runs identical to the reference interpreter, and the interpreter checked against Unicorn |
| `ew-boot` / `ew-gfx` | A run that reaches the first map load in the interpreter; a Vulkan device and presenter that serves NVN calls but does not yet draw the game's own geometry |

They are kept because they verify the recompiler's output and provide format tooling. They are not a replacement engine
and are not on the shipping path.

## Known limitations

- Encrypted retail `.xci` images are not decrypted: EmblemWeave reads plaintext/decrypted `.xci` files, and names the first encrypted NCA instead of unpacking garbage. A plaintext `.xci` is accepted like a plaintext `.nsp`; a folder, `.nsp`, `.nx` or `.nup` also works.
- FSR 4 is not integrated. The FidelityFX path calls FSR 3.1 upscaling through the SDK 2.4 Vulkan loader.
- FSR 3 frame generation is validated on the RX 7900 XT; other AMD GPUs and drivers still need a run. DLSS Frame Generation
  is implemented but has not been tested on RTX 40 hardware in this project.
- ReShade's layer, allowlist, overlay key and disable switch are verified with 6.8.0 Add-on on Vulkan. Individual shader
  packs are the user's to choose, and depth-based effects depend on the depth buffer the engine exposes.
- Code mods are tested with the loader and site checks; a mod hooking a site that exists in the build, `make_function`,
  `make_writable` and `edit_file` have not been exercised end to end.
- The compatibility matrix is small: a Ryzen 7 5700X3D with an RTX 5060 (development PC) and the 0.1.17 test PC. Other
  CPUs and GPUs are supported in code but not each verified; [PLATFORMS.md](PLATFORMS.md) records what is
  known.

The compatibility matrix and the milestone notes live in [STATUS.md](STATUS.md);
the detailed roadmap is in [ROADMAP.md](ROADMAP.md).
