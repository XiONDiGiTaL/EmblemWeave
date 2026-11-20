# Using EmblemWeave

The rail groups pages by what you are doing. This page is the full tour: every page, the display settings, upscaling files, and the in-game menu.

A new installation starts in **Standard** mode. It has **Home** (Overview), **Game** (Game Setup, Requirements), **Library**
(Saves, Mods, Cheats), **Runtime** (Graphics, Audio, Controls), **System** (Troubleshooter) and **Settings** (Appearance,
About). **Appearance → Interface → Advanced** adds **Recompilation**, **Diagnostics** and **Game Log** without moving files
or changing any setting; a configuration from an older release stays in Advanced. A direct Build details, Diagnostics or
Game Log link reveals Advanced automatically. The choice is saved.

The top strip is the launcher's own shell, with the window controls, version and build revision; the strip along the bottom
carries the current job and the readiness line.

## The rail and the pages

The rail groups the pages by mode. **Standard** has **Home** (Overview), **Game** (Game Setup, Requirements), **Library**
(Saves, Mods, Cheats), **Runtime** (Graphics, Audio, Controls), **System** (Troubleshooter) and **Settings** (Appearance,
About). **Advanced** restores **Recompilation**, **Diagnostics** and **Game Log**; a direct link to one of those pages
reveals Advanced automatically. The strip along the top carries
the wordmark, the page you are on, and the version with the build revision; the strip along the bottom carries the current
job's progress or the readiness line, and the last thing the launcher did. On **Overview**, the column on the right shows
where the game data, the recompiler and the runtime stand, and what the launcher has just done.

The launcher opens at 1440×900 and works down to 1120×720. The page column stops at a fixed measure and is centred, so
a wide window does not stretch the text; below 1240 wide the status column steps aside so the page keeps its width; the
rail scrolls when the window is too short for every page; and the lists inside pages grow and shrink with the window. Every
button is the same size, and every control (buttons, sliders, switches, cards) is one liquid-glass material that
takes the chosen scheme's accent, glow and rim.

| Page | Purpose |
| --- | --- |
| **Overview** | **Play** starts the game; **Build the game** compiles it from your dump. Load a save profile and jump to the save editor, mods and cheats. Also readiness, the boot test of the Rust runtime, next steps, and recent activity. |
| **Game Setup** | Choose and validate your game data (a `.nup`, `.nx`, `.nsp` package, a plaintext `.xci` game card or a folder) and set the language and profile. |
| **Saves** | Save profiles: keep the current save under a name and load any profile later (the save it replaces goes to the backups folder). Lists the slots in the current save, with an editor for gold, renown and each unit's level, experience, max HP, stat gains, weapon masteries, items and spells; names come from your own game data. Imports saves from yuzu, suyu, sudachi, citron, eden and torzu, from any folder, and exports to the emulator layout. |
| **Mods** | Installed RomFS mods, switched on and off with a toggle. Installs from a .zip, .7z or .rar archive, a folder, a GameBanana link, or the game's GameBanana list browsed in the launcher. Code mods made for the engine (a DLL in the mod's `exefs`, `engine/fefw-recomp/runtime/include/nxrt/mod_api.h`) are installed and load when the game starts; one that brings hook sites (`exefs/*.sites`) asks for Build the game once after it is installed, updated or removed. Patch-only mods with `.ips`, `.ips32` or IPSwitch `.pchtxt` files are installed and applied to the launcher's uncompressed copy of `main` before lifting, so they also need a build; the [ExeFS code patches](PATCHES.md) page has the naming, syntax, conflict and rebuild rules. Patch support has shipped since 0.1.37. |
| **Cheats** | Atmosphere cheat codes, pasted or imported as `.txt`, each switched on and off and applied about 60 times a second while the game runs (data writes, pointer chains, conditionals, loops and controller-button tests work; instruction patches do not). Cheat Engine tables (`.CT`): `aobscan`/`aobscanregion` entries search the game's memory, and the values below them show live, can be typed in, and can be frozen. |
| **In-game menu** | While the game runs, F10 (or Back + RB on a controller) opens EmblemWeave's menu on top of it: pause and resume, a screenshot of the game window (saved to Pictures\EmblemWeave), restart at the title screen, stop and return to the launcher, quick cheats, cheat codes and tables, display settings (window mode, picture size, frame rate counter) and the controls in use. |
| **Quick cheats** | Per-unit switches for the units found in the game's memory, using the record layout of the community Cheat Engine tables (stats are single bytes, experience is a running total): infinite HP (current HP held at the unit's max HP every tick, including from 0 HP), quick level-up (the level rises one step every quarter second, up to 40) and max stats (max HP and every stat set to 200). The same switches are on the launcher's Cheats page and the in-game menu's Quick tab (F10); search again after a map loads. |
| **Recompilation** | Run the nine-stage pipeline (parse, decompress, decode, recover, analyse, emit C, compile, verify, prepare) with live progress. |
| **Graphics, Audio, Controls** | In **Standard**, **Graphics** offers Performance, Balanced and Quality presets plus resolution, display mode, VSync, frame cap, render scale, FSR 1.0 or native upscaling, sharpening and shader precompiling. It writes the settings the engine can act on into `fefw.ini` before every start; only values that changed since the last start are written, so a setting changed in the game's own F1 menu stands until you change it here. **Advanced** restores the full page, including FSR 4.1.2 / 3.1, NVIDIA Image Scaling, DLSS/DLAA, XeSS, frame generation, ReShade, upscaler DLL folders and shader-compile tuning. **Audio**'s master volume goes to the same file and can now go to 200% (a move above 100% asks **I know the risks** or **Cancel**); the ten-band **Equalizer** offers Flat, Bass, Treble, Vocal, Warm, V-Shape and Night presets plus a Custom curve from the F1 menu, and music, effects and voice wait for mixing. Advanced Graphics and Audio panels fold into drawers with a one-line summary, and **Controls** draws ZL/ZR travel as bars. While playing, display settings are in the in-game menu and full rebinding is in the engine's F1 menu. |
| **Appearance** | **Interface** switches between Standard and Advanced at any time; both modes use the same installation and settings. Choose between the [XiØN](https://github.com/XiONDiGiTaL) scheme (XiØN's own palette, from xiondigital.net), Arcade (a retro video-game palette of hot pink, arcade yellow and cyan on violet, over a neon grid), FE (gold, white silver and royal blue), Ember (ember orange and molten gold over oxblood steel), Verdant (emerald and pale gold over moss-black) and Frost (glacier blue and ice silver over midnight). The logo artwork is recoloured to match each scheme, and every control takes its accent, its glow and its rim from it. |
| **Diagnostics** | Stop reason, guest threads, host events, imports reached and the final events before a stop, with search and export. Its **Log** tab lists the log files: every task writes its own under the launcher's `logs` folder (`data\logs` in a portable install), with the compiler or script it ran, that tool's whole output, and why it stopped, so a failure can be sent on instead of described. |
| **Troubleshooter** | The real game-data and build-tool checks in one place, the safe repairs the launcher can run (rebuild, reset graphics, open the logs), a plain explanation of long-session memory use, and **Create report bundle**. The bundle is a real `.zip` under the launcher's `reports` folder with settings, logs, the game log and any freeze or crash report; nothing is uploaded. The page also opens a prefilled GitHub issue after you write what happened. |
| **Requirements** | Every build dependency, what it is used for, whether it is installed, the official install command, and a **Check again** button. The bundled shader translator removes the .NET SDK requirement. **Updates and existing builds** names the settings/saves/mods, engine/build and game-run folders, and explains that updates keep them and Build reuses a valid build unless the engine revision, toolchain or settings changed. |
| **Game Log** | The game engine's log, followed live while the game runs: info, warnings and errors (each counted and filterable), search, copy, this run or the previous one, and **Pop out** to keep it in a window of its own. |
| **About** | Version, build revision, what each runtime is, credits and legal information. |

Press <kbd>Ctrl</kbd> + <kbd>1</kbd> to <kbd>9</kbd> to jump between pages. From the command line,
`EmblemWeave.exe --page Diagnostics` opens a page, `--build-engine` builds the game as soon as your game data is detected,
`--boot-test` runs the boot test, `--screenshot <file.png>` saves the launcher's own frame and closes, and `--tour` walks through the features by itself (for recording showcase footage; it never saves settings).

The game runs from the launcher's `play` folder (`data\play` in a portable install, `%APPDATA%\EmblemWeave\play` otherwise),
where the launcher writes its `fefw.ini`. RomFS mods go in that folder's `mods` folder, one folder per mod holding
`romfs/` (the layout emulators use). Every engine option is described in
`engine/fefw-recomp/runtime/docs/CONFIG.md`.

## ExeFS code patches

Code patches are installed like any other mod, as `mods/<mod name>/exefs/*.ips`,
`*.ips32` or `*.pchtxt`. They are **not** applied to your game dump and they are
not applied by the engine at run time. After **Build the game** has copied and
uncompressed `main` into the launcher's `game/dump/exefs`, the launcher validates
the active patch files, applies them to that copy, and only then starts the
lifter. The built `fefw.exe` therefore contains the patched code. A change to the
active set makes Play unavailable until the game is built again.

Only `main` is supported. The other ExeFS modules are refused because the build
pipeline does not write them uncompressed. An `.ips`/`.ips32` file must be named
with the 40-hex build ID of `main` (a 32-hex prefix is also accepted); a generic
`main.ips` is refused because it cannot be tied to a build. A `.pchtxt` must
start with `@nsobid-<build id>` and may use `@enabled` / `@disabled`, `@stop`,
comments and `@flag offset_shift`. Full syntax, the conflict rules and the
troubleshooting table are in [docs/PATCHES.md](PATCHES.md).

The game must be rebuilt after installing, editing, enabling, disabling or
removing a patch. The Mods page shows the patch state, the build reason and any
conflict before Play. This support has shipped since 0.1.37; the game version 1.1.1 is not verified.

## Display settings

**Graphics** also carries the display options the game takes from its own files. **Resolution** sets the window and the
picture the game renders (1280×720 to 3840×2160, or *Native display* to keep the game's own console size), **Render
scale** the picture relative to it (0.50× to 2.00×, with the upscaler filling the rest), **Display mode** the window
mode, **Aspect ratio** the shape the game renders, **VSync** the presentation sync, and **Frame-rate limit** the
presentation cap. They are written to `controls.ini` (and the picture size to `[fefw] output` in `fefw.ini`) before
each start, so they apply the next time the game opens. **Audio**'s **Master** volume goes to `[audio] volume` in the
same file, with the ten-band **Equalizer** (`[audio] eq`, `eq_preset`, `eq_preamp`, `eq_bands`); **Music**, **Effects** and **Voice** are the game's own mix and have no key there yet. In the game, **F1 > Audio > Audio Enhancements** opens the glass panel and applies the master gain, presets and band sliders live.

Both pages read the files back. A **What the game will start with** panel lists what the engine will actually do on
the next start (upscaler, output, window and picture size, frame limit, vsync and aspect ratio on **Graphics**; master
volume and equalizer on **Audio**) and names the settings still waiting for that start, so a value changed after the last run
is never mistaken for one already in force, and a setting the game's own F1 menu changed since is taken back onto
the page, so the two always agree unless you have changed the page yourself and not started the game yet.

## Upscaling

The engine upscales the picture itself, and **Graphics > Upscaling** chooses how: **FSR 1.0** (AMD's spatial upscaler,
any GPU), **NVIDIA Image Scaling** (spatial, any GPU, up to 2× per side), **MAKO (spatial)** (spatial, any GPU and any
scale, a single-pass sharpened-cubic filter), **FSR 4.1.2 / FSR 3.1** (AMD's temporal upscaler through the FidelityFX API;
FSR 4.1.2 on supported Radeon GPUs, FSR 3.1 otherwise), **DLSS**
/ **DLAA** (NVIDIA's temporal upscaler through Streamline) and **XeSS** (Intel's temporal upscaler, on any GPU with
DP4a). The launcher writes the choice, and the folder it found the DLLs in, into the run
folder's `fefw.ini` before it starts the game, so it applies at the game's next start.

**Graphics > Sharpening** sets how crisp the final picture is: **Off**, **Low**, **Default** (the engine's own 0.8) or
**High**. It is the engine's `[upscale] sharpness`, it applies live rather than at the next start, and the game's own
F1 menu sees the same value.

AMD's, NVIDIA's and Intel's files cannot be shipped, so those upscalers need you to add them, in the run folder, beside the game, or in an `upscalers` or `upscaler` folder anywhere from the game executable up through the engine checkout:

- **FSR 4.1.2 / FSR 3.1**: `amd_fidelityfx_loader_vk.dll` and `amd_fidelityfx_upscaler_vk.dll` from the FidelityFX SDK
  (v2.4.0, Vulkan), in the play folder's `upscalers\fsr` (an `fsr3` folder is read as well). The SDK 2.4 provider
  exposes FSR 4.1.2 on supported Radeon GPUs and FSR 3.1 on older ones; the Graphics page has a mode for each. FSR 3
  frame generation needs the v1.1.4 `amd_fidelityfx_vk.dll` there too, so the loader pair alone cannot make frames.
- **DLSS / DLAA**: `sl.interposer.dll`, `sl.common.dll`, `sl.dlss.dll` and `nvngx_dlss.dll` from NVIDIA's
  [Streamline SDK](https://github.com/NVIDIAGameWorks/Streamline) release `bin/x64`, in
  the play folder's `upscalers\streamline`. DLSS Frame Generation adds `sl.dlss_g.dll`, `sl.reflex.dll`,
  `sl.pcl.dll`, `nvngx_dlssg.dll` and `NvLowLatencyVk.dll` to the same folder, and wants an RTX 40 class GPU and
  Windows' hardware-accelerated GPU scheduling.
- **XeSS**: `libxess.dll` from Intel's XeSS SDK, in   the play folder's `upscalers\xess`, with the Visual C++
  2015-2022 runtime installed.

**Graphics > Upscaler files** shows one row per upscaler: the folder its files were found in, or the exact files that are
missing. **Open the upscalers folder** creates those three folders and opens them; **Look for the files** searches the
launcher's folder, the engine folder, Downloads and the Desktop one folder deep, copies what it finds into the run
folder and says what is still missing, so a ZIP that landed in Downloads does not have to be arranged by hand. The game's
own F1 menu, Graphics tab, reports the same under `Files found`; at startup the runtime also walks the folders above
`fefw.exe`, so a folder left in `engine/fefw-recomp/upscaler` works without moving it (#26). **FSR 4.1.2** is now
integrated through that native Vulkan FidelityFX provider; it was verified on the RX 7900 XT with Adrenalin 26.10.1,
and FSR 3.1 is the automatic fallback when the driver does not expose 4.x. The separate DX12 FSR 4 shader-override
builds, INT8 model swaps and **OptiScaler** still cannot drive this renderer: they hook DirectX or replace a DX12 DLL,
while this engine calls its own FidelityFX, Streamline and XeSS interfaces over Vulkan. [GRAPHICS.md](GRAPHICS.md)
records that boundary.

For **AMD Fluid Motion Frames 2.1**, add `fefw.exe` to AMD Software: Adrenalin Edition (Gaming > Add) and enable AFMF
in the driver. AFMF works with Vulkan on RX 7000 series in borderless fullscreen, needs Adrenalin 25.3.1 or newer, and
requires VSync off in both the game and the driver. The launcher cannot toggle a driver-side feature, but the Graphics
page's note repeats the setup. FSR 4.1.2, FSR 3.1, DLSS / DLAA and XeSS are the temporal upscalers the engine runs
itself.
