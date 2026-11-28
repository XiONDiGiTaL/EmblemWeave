# F1 menu guide

The **F1** overlay is the engine's own settings menu. It is drawn by the fefw-recomp runtime over the game, and it is separate from EmblemWeave's **F10** menu.

| Key | Menu | What it is for |
| --- | --- | --- |
| **F1** | Engine settings overlay | Graphics, performance, upscaling, pacing, audio, controls, diagnostics and system information. |
| **F10** (or **Back + RB**) | EmblemWeave in-game menu | Pause/resume, screenshot, restart at the title screen, stop and return to the launcher, quick cheats, cheat codes/tables, display settings and the controls in use. |

F1 does not replace F10, and F10 does not replace F1. Use F1 to tune the runtime; use F10 to control the running session.

## Opening and moving around

1. While the game is running, press **F1**. The overlay opens on top of the game.
2. Switch tabs with **L/R** on a controller or by clicking the tab strip. The 0.1.43 candidate fixes L/R tab switching so it no longer activates the first control.
3. Use the mouse, or the controller prompts shown by the panel, to move and change a value. The exact face-button glyphs follow the connected controller.
4. Close the overlay with the on-screen close/back action when you are finished. If the overlay does not open, click the game window once to make sure it has focus, then press F1 again.

The overlay is not a second launcher. It reads and writes the same `fefw.ini` and `controls.ini` values the launcher uses.

## The nine tabs

| Tab | What it contains |
| --- | --- |
| **Overview** | The active build/status, frame rate and a summary of the performance controller. |
| **Graphics** | Display mode, resolution/picture size, VSync, frame cap, sharpening and the graphics controls the engine can change while running. |
| **Performance** | The adaptive performance controller: operating mode, frame-time percentiles (average, 1% and 0.1% lows, P50/P95/P99), pressure classification and the event list. |
| **Upscaling** | The selected spatial or temporal upscaler, sharpening and the `Files found` report for FSR, DLSS/DLAA and XeSS. |
| **Pacing** | Presentation timing: frame-rate cap, VSync and the frame-generation/pacing controls that are available. |
| **Audio** | Master volume (0–200%), the ten-band equalizer, presets and **Audio Enhancements**. The launcher's Audio page reads the same master value back. |
| **Advanced** | **Controls**, **Game** and **Mods**: bindings, game/runtime options and the installed mod switches. |
| **Diagnostics** | Runtime diagnostics and the log/overlay information used when a frame-time, shader or device problem needs a report. |
| **System** | Version/build information, runtime/system status and the overlay's exit/close actions. |

The tab set is the one added in 0.1.41. The page is themed with the XiØN palette and uses Sora for Latin text, with CJK-capable faces when a CJK UI pack is installed.

## Common tasks

### Change the master volume or equalizer

1. Open **F1 > Audio**.
2. Move **Master** between 0% and 200%. Above 100% is a limited boost; the engine's peak limiter prevents digital clipping.
3. Choose an equalizer preset (Flat, Bass, Treble, Vocal, Warm, V-Shape, Night or Headphones Boost) or open **Audio Enhancements** for the band sliders.
4. Close the overlay. The launcher's Audio page reads the engine's value back, and a change made there appears in F1.

### Change the performance mode

1. Open **F1 > Performance**.
2. Choose **Automatic**, **Conservative**, **Balanced**, **Aggressive** or **Manual**.
3. Watch the percentile and pressure lines while you play. **Manual** never actuates the controller; it is for A/B tests and for keeping a fixed configuration.
4. The launcher's dedicated **Performance** page exposes the same controller and its live diagnostics.

### Change upscaling or sharpening

1. Open **F1 > Upscaling** (or **Graphics** for sharpening).
2. Choose the upscaler the engine and your GPU support. FSR 4.1.2/3.1, DLSS/DLAA and XeSS need their vendor DLLs in the folders named on the launcher's Graphics page.
3. Check **Files found**. If a required DLL is missing, the launcher's **Upscaler files** card and **Look for the files** action are the supported way to place it.
4. Some changes take effect when the overlay closes; a resolution or display-mode change may need a restart. The exact behavior is shown by the control and the launcher's **What the game will start with** panel.

### Rebind controls

1. Open **F1 > Advanced > Controls**.
2. Select the action and press the new key or controller input.
3. Save/apply, then test it in the game. The launcher's **Controls** page draws the same bindings; controller travel bars are there for ZL/ZR.
4. If a binding does not reach the game, confirm you are editing the active profile and that the game was not restarted with an older `controls.ini`.

### Check why the game is slow

1. Open **F1 > Performance** and read the average FPS, 1%/0.1% lows and CPU/GPU/upload/replay lines.
2. Read the pressure classification and the event list; it names CPU, GPU, VRAM, streaming, shader-compilation, scheduling or synchronization pressure.
3. Compare the same scene with the controller in **Manual** and with a lower preset to separate a real bottleneck from a transient compile/hitch.
4. If the result is a crash, a device loss or a repeated freeze, open **F10** and use EmblemWeave's diagnostics/report path instead; attach the report bundle.

## How F1 and the launcher share settings

- The launcher writes its display, graphics and audio keys to the run folder's `fefw.ini` and `controls.ini` before each start. It writes only values that changed since the last start, so a setting changed in F1 is not overwritten on the next Play.
- The launcher adopts `[perf]`, `[nvn]` and `[gfx]` values changed in F1 back into its pages. The Audio page reads the master value back; a 185% write and a 200% value adopted from the engine have a regression test.
- Some settings apply live (for example master gain or sharpening); others are read at the next start (resolution, display mode and some upscaler choices). **What the game will start with** on the Graphics/Audio pages is the authoritative "next start" view.
- The engine's full key reference is [runtime/docs/CONFIG.md](../engine/fefw-recomp/runtime/docs/CONFIG.md).

## Configuration files and backup

In a portable install the run folder is `data\play`; in a profile install it is `%APPDATA%\EmblemWeave\play`. The files are:

- `fefw.ini` — engine/graphics/performance/mod keys.
- `controls.ini` — display, audio and controller bindings.

Close the game before editing either file by hand. Copy the file to a backup first, change one key at a time, and start the game to test it. The launcher reads the files back on its pages, so a manual change that disagrees with a page is visible rather than silently overwritten.

## Limits and known issues

- HDR state changes were not exercised live in the 0.1.43 candidate; HDR-only and SDR-only Picture controls are hidden according to the current state.
- Opus decode/seek/pause and measured A/V drift remain unverified; AV1 playback is verified through dav1d and FFmpeg/D3D11VA.
- The in-game character switch in the Obelisk Chamber was not visually confirmed in the release-candidate retest. The chamber freeze fix itself is recorded in the changelog.
- If a control is unavailable, the overlay states the limit rather than showing a dead switch. Use the launcher page or the changelog's known issues for the current status.

## Related

- [Documentation hub](GUIDES.md)
- [Launcher guide](LAUNCHER_GUIDE.md)
- [Using EmblemWeave](USAGE.md)
- [Graphics](GRAPHICS.md)
- [Troubleshooting](TROUBLESHOOTING.md)
- [0.1.43 release notes](changelog/0.1.43.md)
- [0.1.43 issue map](release/0.1.43/ISSUE_MAP.md)
