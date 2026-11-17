# Gameplay verification

This is the repeatable check for the integrated fefw-recomp engine. It is a test
plan, not a compatibility claim: a successful compile, a boot to the title
screen, or a short menu session is not a stable game.

**Tested game build: 1.1.0 only.** A run on 1.1.1 is recorded as unverified even
when it launches. A decrypted tree labelled 1.1.1 that was checked carries the
same seven ExeFS modules as the 1.1.0 manifest (`main` `616e7d90…`), so a 1.1.1
dump with those hashes runs the supported code and is still tested here as its
own build. The engine's own build and module checks are in
[STATUS.md](STATUS.md); this page covers what a person does with the game.

## Before a run

- Use a retail 1.1.0 dump and the engine built from this tree; record the
  launcher version, engine commit and `main` module hash.
- Record the GPU, driver version, Windows build, installed RAM, pagefile size
  and whether the GPU shares system memory.
- Start from a fresh `fefw.log`; keep `fefw_freeze.txt` and the launcher's
  exported diagnostics beside each result.
- Note the shader cache state (cold, warm) and the Graphics profile (default or
  Low Memory). Do not change either during a scenario.
- Use the same save for before/after comparisons where possible. Back it up;
  the test must not be the only copy.

## Scenarios

Run them in order on one session when practical, then repeat the long session
separately.

1. **Launch and title screen.** Start from the launcher, reach the title, leave
   it idle for two minutes. Check input, audio device and the first frame time.
2. **Opening movie and cutscenes.** Watch the opening movie through to gameplay.
   Record decoder messages, frame-time p99 during video, and whether audio and
   video stay in sync.
3. **Save load and save.** Load an existing save, save to a new slot, return to
   the title and load the new slot. Verify the save is unchanged outside the
   game and that the second load succeeds.
4. **Hub and town exploration.** Walk through the busiest hub area, open menus,
   and move the camera for five minutes.
5. **Battle transition.** Enter and leave a battle from the map at least three
   times, including one transition after a long dialogue.
6. **Large battle.** Play a battle with many characters and effects through at
   least three turns. Record p50/p95/p99 frame time, not only average FPS.
7. **Audio-heavy dialogue.** Play a voiced scene with music and effects, then
   skip and replay part of it. Listen for dropped or repeated voice lines and
   check the log for decoder or voice-allocation errors.
8. **Extended scene transitions.** After 30 minutes, change maps and open the
   preparation and support screens. Watch process working set, private commit,
   guest heap and Vulkan budget around each transition.
9. **Shader compilation and texture streaming.** Start with a cold shader cache
   and enter a new area. Confirm compilation finishes, the hitch report names
   the work, and no pipeline or texture upload stays queued indefinitely.
10. **Repeated restarts.** Close and restart the game five times without
    rebuilding. Check that settings, saves, mods and the shader cache survive.
11. **Mod activation and deactivation.** Enable one RomFS mod, verify its file
    wins over the base and package copies, disable it, and verify the base file
    returns. Repeat with two mods in their documented order.
12. **Graphics and resolution changes.** Change resolution, render scale and
    upscaler, restart as the UI requires, and return to the previous settings.
    Confirm audio, controls, saves and mods were not touched.
13. **Controller and menu interaction.** Use a physical controller for menus,
    camera, confirm/cancel and rumble. Disconnect and reconnect it while the
    game is running.

## What to record

| Measurement | Where it comes from | Why |
| --- | --- | --- |
| Frame time p50 / p95 / p99 and longest hitch | engine hitch lines and GPU timestamps | Averages hide the stutters players feel |
| Working set, peak, private commit, pagefile | `memory` lines and `fefw_freeze.txt` | Distinguishes a leak from a temporary spike |
| Guest heap and Vulkan budget/usage | `memory` and `nvn.census` lines | Shows whether the failure is guest, system or video memory |
| Audio voices, buffers, decoder activity | audio and movie lines | Separates a mixer defect from resource exhaustion |
| Pipeline queue depth and shader compiles | hitch lines and `nvn` lines | Identifies a compile storm |
| Device-loss / out-of-memory result | `vkQueueSubmit2`, `vkDeviceWaitIdle` and driver messages | A device loss is not a game crash and needs its own diagnosis |
| Time since last present, frame count | freeze report | Tells a stalled render thread from a dead process |

## Pass criteria

A scenario passes only when all of these hold:

- the process does not exit unexpectedly and no freeze report is written;
- no Vulkan device-loss or out-of-memory error appears in the log;
- p99 frame time and the longest hitch stay in the range recorded on the
  known-good Windows machine for that scene, and do not grow across repetitions;
- private commit and guest heap plateau instead of rising with each repetition;
- saves load, audio stays in sync, and the controller still works; and
- disabling a mod restores the underlying file.

## Long-session run

After the scenarios, leave the game in a hub or battle for at least 90 minutes,
then perform one scene transition every ten minutes. Save before the run and
again after it. Attach the complete log, the freeze report if one exists, the
diagnostics export and a note of the exact point where frame time or memory
changed. Do not describe a run as stable from a boot-only or 15-minute session.

## Not performed here

No real game dump, physical controller, handheld or 1.1.1 environment is
available to the development checks in this repository. The scenarios above are
the procedure for a person with that hardware; parser and unit tests do not
replace them.
