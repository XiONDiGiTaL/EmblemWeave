# Roadmap

This is a working order, not a release promise. It starts from the current tree
and the 0.1.40 public release; anything still marked unreleased below needs a
build and real-hardware checks before it can be described as shipped.

Priority means what should be done first to make the next build trustworthy, not
how interesting the work is. P0 is a release gate.

| Priority | Area | Why now | Effort |
| --- | --- | --- | --- |
| P0 | Real-hardware release validation | The tree contains memory and patch changes that have only synthetic tests; 1.1.1 is still unverified | M |
| P1 | Game 1.1.1 compatibility | Issue #41 is running 1.1.1 and cannot be answered with a supported build ID yet | L |
| P1 | Controller trigger verification and diagnostics | ZL/ZR and vibration could not be exercised on this machine; a report must be actionable on a user's PC | M |
| P2 | Low Memory profile validation | The profile exists, but only emulated hardware and log fixtures test it | M |
| P2 | ExeFS patch system hardening | `.ips`, `.ips32` and `.pchtxt` support has shipped since 0.1.37; wider game-version validation remains | M |
| P3 | Save profile compatibility and recovery | Save failures are high-impact and hard for users to diagnose | M |
| P3 | Upscaler discovery and failure messages | Issue #29 still needs a reliable answer about missing files and SDK layouts | S |
| P3 | Build diagnostics and report bundles | Issues #37 and #41 need measurements that separate game, driver and launcher faults | S |
| P4 | Shader-compile resource controls | The CPU spike is improved, but not proven across CPUs and driver versions | M |
| P4 | Linux / Steam Deck guidance | Issue #7 is a real request, but a native port is not a packaging change | L |
| P5 | Long-session soak and performance matrix | Memory pressure on shared-memory handhelds needs evidence over time | M |

## A. Real-hardware release validation (P0)

**Description.** Build the current tree as a release candidate and run the
same matrix on physical hardware: a 16 GB shared-memory handheld, a 24 GB
handheld if available, one NVIDIA discrete system, one AMD discrete system, and
a clean Windows profile with no build tools on `PATH`.

**Why first.** The Low Memory recommendation, patch application and the current
UI have unit tests, but no real game dump, no physical controller and no 1.1.1
dump were available here. Shipping either change without that evidence would
repeat the compatibility mistakes this task is meant to correct.

**Who needs it.** Every user, and especially anyone on a 16 GB handheld or a
game update newer than 1.1.0.

**Requirements.** A legal 1.1.0 and, if possible, 1.1.1 dump; one controller
with analog triggers; the Visual Studio build tools; the release zip layout
including `portable.txt`.

**Dependencies and risks.** Availability of hardware and dumps; 1.1.1 may not be
supported by the upstream recompiler; test time is dominated by shader
compilation and a full build.

**Effort.** M (one to two days of machine time plus triage).

**Success criteria.** One clean install builds and plays on each system; the
title bar version and build revision are recorded in the report; no test claims
"verified" without a log, screenshot or build ID; the release notes list exactly
which systems and which game versions were exercised.

## B. Game 1.1.1 compatibility (P1)

**Description.** Determine whether the 1.1.1 `main` build ID is supported by the
lifted recompiler configuration. If it is, add the build ID to the tested
manifest and run a full lift, compile, boot and save-load test. If it is not,
report the unsupported build clearly at Game Setup instead of failing an hour
into a build.

**Why.** Issue #41 is a 1.1.1 run. The reply cannot promise compatibility, and
the launcher currently cannot distinguish "wrong update" from "bad dump"
early enough for a useful answer.

**Who needs it.** Anyone who updated the game, including the #41 reporter.

**Requirements.** A decrypted 1.1.1 dump; the upstream build manifest; a full
build; a save made by 1.1.1; the 1.1.0 baseline for comparison.

**Dependencies and risks.** Upstream support may not exist; module hashes and
function layout can change; a partial lift can produce a binary that crashes
later, so a boot is not enough. This work must not modify the user's dump.

**Effort.** L.

**Success criteria.** The build manifest lists the 1.1.1 build ID and the
version it maps to; Game Setup accepts that dump or refuses it before a build
with the supported IDs named; one full build boots, loads a 1.1.1 save, reaches
the hub and exits cleanly; the 1.1.0 path still passes.

**Checked 17 November 2026, launcher updated 18 November.** A decrypted tree
presented as 1.1.1 was hashed against the manifest: all seven ExeFS modules agree
(`main` `616e7d90…`, build `db93b012…`), so its executable is the update 131072
build the engine was tested with. What remains is data and save compatibility,
not a different `main`. The status stays *unverified* until a full build of that
tree boots and loads a save. The checked tree held no AddOnContent. The launcher
now finds dumped DLC in `dlc/<title id>/romfs` and writes `[aoc] dir`; add-on
packages still inside a `.nup` (or unextracted NCA) are not unpacked by it, and
no DLC payload was available to boot here.

## C. Controller trigger verification and diagnostics (P1)

**Description.** Close the verification gap around analog triggers and
vibration. The Controls page already polls XInput each frame and draws
`ZL travel` and `ZR travel`, and **Test vibration** sends both motors for half a
second. What is missing is a recorded hardware pass and a raw report a user can
send when a trigger does not reach the game.

**Why.** A trigger that only reaches 60 % is visible on the bar, but without a
controller this machine could not confirm the bars, the threshold, or rumble.
"Controller does not work" is otherwise a guess.

**Who needs it.** Controller users, especially on handhelds where ZL/ZR are the
primary shoulder inputs.

**Requirements.** An XInput-compatible pad with analog triggers; the XInput
state read by `pad::Pad::read`; the Controls page trigger bars; the
`controls.ini` threshold and vibration keys; a diagnostics export that records
the pad name, trigger range and XInput packet number without starting the game.

**Dependencies and risks.** SDL 3 and XInput can report different device names
and trigger ranges; Bluetooth versus USB can change the report rate; the
launcher has no controller on the development machine.

**Effort.** M.

**Success criteria.** With a physical pad, the ZL and ZR bars track the full
travel, the configured threshold changes when ZL/ZR register, **Test vibration**
runs both motors, and a saved report names the device and the observed range.
A pad that reports a short range produces a specific message rather than a
generic failure.

## D. Low Memory profile validation (P2)

**Description.** Validate the Low Memory profile (shipped in 0.1.37) on
16 GB and 24 GB shared-memory systems. Confirm that detection uses installed
RAM and the engine's Vulkan budget, that the recommendation does not appear on
a discrete GPU with ample memory, that **Apply** changes only graphics, and that
**Restore** returns the exact previous values. Add a memory-pressure line to the
report bundle and compare it before and after the preset.

**Why.** The selection logic has tests, but the profile has not been run against
the real engine on the hardware class it is meant to help. The #41 log shows a
16 GB shared-memory profile losing the device after about 400 seconds; the
preset is a mitigation, not a proven fix.

**Who needs it.** Handheld and integrated-GPU users, particularly 16 GB devices.

**Requirements.** A 16 GB shared-memory machine and a 24 GB machine; the
engine's memory and Vulkan budget lines; a repeatable long session that reaches
the failing scene; the restore snapshot; telemetry fields for working set,
commit, budget and driver allocations.

**Dependencies and risks.** The game may leak independently of the preset; a
lower resolution can shift a bottleneck from GPU memory to CPU; driver
allocations are not fully controllable; results on one handheld do not
generalise to every shared-memory device.

**Effort.** M.

**Success criteria.** On the 16 GB machine the profile is recommended, applies
without touching audio/controls/saves/mods, survives a restart, and restores
exactly; a session that previously lost the device either completes or produces
a report showing which limit was still reached. On the 24 GB machine it is not
recommended unless the measured budget is genuinely constrained. The release
notes call it a stability preset, not a leak fix.

## E. ExeFS patch system hardening (P2)

**Description.** Harden the `.ips`, `.ips32` and IPSwitch `.pchtxt` support that
shipped in 0.1.37: install patch-only mods, validate the build ID, apply
deterministically to the launcher's uncompressed `main`, refuse conflicts,
record a fingerprint, and require a rebuild when the active set changes. Before
calling it supported on more game versions, exercise real patches on real dumps
and document the supported subset.

**Why.** 0.1.36 and older do not install patch-only mods at all. This code is
the first thing that writes the code the lifter consumes; a wrong offset or a
silently ignored build ID is worse than a clear refusal.

**Who needs it.** Mod authors and users of code patches, including #39 if
Weapon Durability turns out to be a patch rather than a RomFS mod.

**Requirements.** A legal dump and its 40-hex build ID; at least one real IPS and
one real pchtxt patch; a patch-only mod layout; the uncompressed `main` produced
by `prepare_exefs.py`; the audit and state files in the run folder; a synthetic
test for each parser and conflict case (already present).

**Dependencies and risks.** Patch formats have variants (final-size records,
16-hex build IDs, vendor-specific directives); a patch made for another game
version must be refused; two patches can write the same byte; writing the dump
must never happen. Only `main` is supported because it is the module the
pipeline writes uncompressed.

**Effort.** M.

**Success criteria.** A real patch changes the expected bytes in the prepared
`main`, the build consumes it, the game starts, and removing the patch triggers
a rebuild that restores the original bytes. Wrong build IDs, other modules,
truncated records, out-of-bounds writes, RLE errors and conflicting patches are
refused before the lifter. The dump's hash is unchanged. The Mods page shows
the patch state and the rebuild reason. The feature has shipped since 0.1.37;
this validation is required before calling it supported on 1.1.1 or widening the
supported formats.

## F. Save profile compatibility and recovery (P3)

**Description.** Broaden the save-loader matrix and make recovery explicit:
which emulator produced a profile, which slot was chosen, what is missing, and
how to restore a backup. Keep the title's fixed `account/DEFAULT_USER` path and
the current backup rotation.

**Why.** A save that does not load is indistinguishable from a lost save to a
user. The current fixes cover the known path and profile cases but not every
emulator layout.

**Who needs it.** Users importing saves from yuzu, suyu, sudachi, citron, Eden,
Fritchii or torzu.

**Requirements.** Representative save trees from each supported emulator; the
probe and import tests; a read-only source path; atomic writes and backups;
clear slot/profile labels.

**Dependencies and risks.** Emulator layouts change; encrypted or incomplete
saves cannot be repaired; overwriting a good profile is unacceptable.

**Effort.** M.

**Success criteria.** Each supported layout is detected by title and account;
the imported profile loads in the game; a failed import names the missing file
and leaves both source and existing profile untouched; a backup can be restored
from the Saves page.

## G. Upscaler discovery and failure messages (P3)

**Description.** Keep the current search paths for FSR, Streamline and XeSS,
but make the Graphics page state exactly which DLL is missing, which folder was
searched, and whether the selected mode can run. Add a "search again" action
after the user copies files in.

**Why.** Issue #29 shows that "upscaler files not found" is not enough when an
SDK archive has several nested layouts. The engine already searches several
roots; the UI should explain the result.

**Who needs it.** Users of FSR 1/3.1, DLSS/DLAA and XeSS.

**Requirements.** The engine's upscaler path conventions; a scanner result per
mode; vendor DLL names and bitness; the play-folder layout; no redistribution of
vendor binaries.

**Dependencies and risks.** DLL ABI and driver requirements vary by GPU; a file
can be present but unusable; frame generation still needs motion vectors this
title does not provide.

**Effort.** S.

**Success criteria.** With a complete FSR SDK the page reports the found files
and the mode runs; with a missing DLL it names the file and folder; with a
wrong-architecture DLL it says so; no unsupported mode is offered as working.

## H. Build diagnostics and report bundles (P3)

**Description.** Extend the report bundle with a stable summary of the build
stage, toolchain fingerprint, CPU target, game build ID, active patch
fingerprint, memory pressure and the last engine stop. Keep the bundle local and
make the contents visible before it is attached.

**Why.** Issues #37 and #41 contain logs that cannot distinguish a heap
exhaustion from an audio stall or a device loss. A report should answer that
without a round trip.

**Who needs it.** Everyone filing a build, freeze or crash report; maintainers
triaging them.

**Requirements.** The existing job logs and crash report; the engine memory
lines; a redaction pass for user paths; a zip writer (already used by the
Troubleshooter); no automatic upload.

**Dependencies and risks.** Logs can contain a user's folder names; oversized
bundles; engine logs may be incomplete after a hard hang.

**Effort.** S.

**Success criteria.** One bundle from a failed build and one from a freeze both
open on a clean machine and name the stage, the failing command and the memory
or device state; no secrets, save contents or game files are included.

## I. Shader-compile resource controls (P4)

**Description.** Measure the Quiet/Balanced/Fast presets on several CPUs and
make the default depend on measured core count and memory, not a single fixed
choice. Keep idle priority and efficiency mode.

**Why.** The 6 GHz report in #10 was addressed with a conservative default, but
the preset has not been measured across the supported CPU range.

**Who needs it.** Users building on laptops and shared machines.

**Requirements.** A timing harness around the shader step; per-preset thread
counts; a build log line with elapsed time and peak working set; a way to
restore the default.

**Dependencies and risks.** Build time is dominated by shader translation and
compilation; too few threads can make a build feel hung; efficiency mode is
Windows-specific.

**Effort.** M.

**Success criteria.** Each preset has a measured time and peak memory on at
least three CPUs; the default does not saturate every core; the build log
records the preset and the result.

## J. Linux / Steam Deck guidance (P4)

**Description.** Treat issue #7 as two separate requests. The archive separator
fix is in 0.1.36 and later. A native Linux launcher and engine build is a port,
not a packaging change. For Steam Deck, document and test the supported path:
build on Windows, copy the result, run under Proton, and edit `fefw.ini` as
`docs/STEAMDECK.md` describes.

**Why.** Users conflate "the zip extracts on Linux" with "the game runs on
Linux". A clear, tested Proton path is useful now; a native port is a larger
project.

**Who needs it.** Linux and Steam Deck users.

**Requirements.** A Steam Deck or Linux desktop with Proton; a Windows-built
`fefw.exe`; the prefix paths; a known-good controller layout; logs from the
Proton run.

**Dependencies and risks.** Vulkan drivers, XAudio2 through Proton, case-sensitive
paths, and anti-cheat-free single-player assumptions; a native port would need a
new window/input/audio layer and is not scoped here.

**Effort.** L for a native port; S for the documented Proton path.

**Success criteria.** The Proton steps produce a boot to the title screen and a
save load on a Steam Deck; the documentation names the tested Proton version,
the launch options and the known failures; the issue distinguishes packaging
from porting.

## K. Long-session soak and performance matrix (P5)

**Description.** Run repeated 30–60 minute sessions on the supported GPU classes
while recording frame time, working set, commit, Vulkan budget and driver
allocations. Compare the baseline with the Low Memory profile and with shader
cache warm and cold.

**Why.** A single crash log is an anecdote; a soak shows whether memory grows
and whether the profile changes the slope or only the starting point.

**Who needs it.** Handheld users, performance-sensitive users, and maintainers
deciding whether the next work is a leak fix or a budget fix.

**Requirements.** A repeatable save and route; the engine's telemetry; a script
that records the samples; enough disk for captures; a machine that can be left
alone.

**Dependencies and risks.** Game progression changes the scene; driver updates
change results; thermal throttling on handhelds; no automated in-game route yet.

**Effort.** M.

**Success criteria.** At least three runs per configuration; the report shows
frame-time percentiles and memory over time; a leak appears as a slope that
does not plateau; the Low Memory profile's effect is stated in measured terms,
not as a guarantee.

## What is deliberately not on this list

- Replacing fefw-recomp with the Rust research runtime. fefw-recomp is the
  permanent primary engine; the crates remain verification and tooling.
- Automatic application of the Low Memory profile. It stays an explicit
  recommendation with a one-press restore.
- A native Linux port presented as a small follow-up to the archive fix.
- A promise that any patch or profile fixes 1.1.1 before it has been built and
  played on that version.
