# Engine stability and data-pipeline audit — 19 November 2026

**Status: Phase 1, read-only.** This document is the written audit required before
broad engine changes. No source file was changed in this pass. The working tree
already contains unrelated uncommitted work, which was left untouched.

This audit builds on [ENGINE_AUDIT.md](ENGINE_AUDIT.md) (18 November 2026) rather
than replacing it. It concentrates on the issues that are still open:

- **#70** — cinematics lag and voices cut; rebuilding from `.nup` reportedly fixed it.
- **#37** — freeze after roughly two hours, and `VK_ERROR_DEVICE_LOST` on an RTX 5060.
- **#26** — only TAUU is recognised; copying FSR files under `upscaler/fsr` did not help.
- **#29** — Z1 Extreme + RX 6850M XT: 720p drops, torch-lighting glitches, cutscene
  audio/video problems, upscalers not found.

The primary conclusion is narrow and testable: **the `.nup` and extracted-folder
builds do not currently guarantee equivalent runtime inputs.** Their content can
be identical, but nothing in the launcher proves that it is. Issue #70 is therefore
at least as likely to be a data-preparation or file-selection problem as an AV1
decoder problem. The decoder is not exonerated, but replacing it now would be
premature.

The primary engine remains `engine/fefw-recomp`. This audit does not propose the
Rust research runtime, a second renderer, a new allocator, or a replacement video
decoder.

---

## A. What already exists (do not duplicate)

The existing engine already has the mechanisms that a broad “stability rewrite”
would otherwise recreate:

| Area | Existing mechanism |
| --- | --- |
| GPU memory | VMA allocation, `VK_EXT_memory_budget`, pool mirrors, budget reporting |
| Pipelines | Pipeline cache, graphics pipeline libraries, background prewarm |
| Frame pacing | Frame limiter, VRR-aware caps, present modes, frames in flight |
| Upscalers | TAAU, FSR 1, FSR 3.1/4.1.2, DLSS/DLAA, XeSS, NIS, MAKO, frame generation |
| Video | AV1 worker in `media/movie_vdec.cpp`, bounded `video_decode_ahead` (default 30), PTS clock |
| Audio | XAudio2 sink with 64 slots, queue caps, underrun/drop counters, device recovery; audren on Pro Audio MMCSS |
| `.nup` RomFS | Direct package reader, packed-entry decompression, bounded `PackedCache` (1.5 GiB total, 256 MiB per entry) |
| `.nx` RomFS | Block reader with a small LRU block cache |
| Diagnostics | `fefw.log`, hitch lines, F1 overlay, gauge CSV, freeze report, crash report, device-loss report |
| Build | `vswhere`/toolchain probe, compiler/link probe, memory-aware job estimate, optional `sccache` |

The audit found no evidence that any of these should be removed. The work below is
about making their inputs and evidence trustworthy, not replacing them.

---

## B. Issue #70: do `.nup` and extracted-folder builds produce the same inputs?

### B.1 Exact launcher flow

#### `.nup` selected

`apps/emblemweave/src/gamedata.rs`:

1. `detect_with` recognises `NUP\x01` and calls `use_nup`.
2. `ew_format::nup::Nup::open` parses the package.
3. `nup.program_package()` selects **one** package: the first update if present,
   otherwise the first base package. Its comment says the update “carries the
   newest program and a full RomFS”.
4. `use_nup` walks that package’s ExeFS and copies it to
   `work/nup/<title-id>-v<version>/exefs`. The copy is reused when every file’s
   **length** matches the package entry.
5. Add-on packages for the base title are unpacked to `work/nup/.../dlc`.
6. `g.romfs` is set to the `.nup` file itself, not to the copied ExeFS folder.
7. `finish` reports the RomFS as “read in place from the `.nup`”.

#### Extracted folder selected

`detect_with` accepts an explicit `paths.exefs` / `paths.romfs`, or discovers
`exefs/` and `romfs/` under the selected root. It does **not** merge a base game
and an update, and it does not compare the folder against a package manifest.
`finish` checks that the RomFS is non-empty and looks for `Fs.bin.zst`; then
`check_known_dump` compares ExeFS modules against the known-dump hashes. A
different or missing module produces a **warning**, not a refusal.

This is already a material difference: the `.nup` path derives its ExeFS and
RomFS from one selected program package; the folder path trusts whatever tree the
user points at.

### B.2 Exact build flow

`apps/emblemweave/src/engine.rs`:

- `romfs_folder_for_play` returns `None` immediately when `romfs` is a directory.
- For a `.nup`, `gamedata::packed_romfs` counts the zstd-compressed RomFS entries.
  If there are any, `unpack_packed_romfs` writes **only those compressed entries**
  into `<engine-root>/work/romfs-overlay`.
- The overlay identity is a text key: the package path, its size and modification
  time, and a SHA-256 over every packed entry’s path, decompressed size and entry
  hash. `work/romfs-overlay/.source` records the identity the folder was written
  for and `.written` that the write finished.
- The overlay writer stages each file in `<overlay>/.emblemweave-parts` and renames
  it into place, so a file at its final path is complete; a destination at the
  package entry’s decompressed size is then a safe resume test within one identity.
  `--unpack-romfs` uses that same writer and identity, so rerunning clears orphans
  and a replaced package is rebuilt. It refuses a non-empty folder without the
  launcher’s `.source`/`.written` marker instead of clearing a folder the launcher
  did not write.
- A different identity clears `work/romfs-overlay` before the write, so a path the
  old package packed cannot survive as an orphan over the current package.
- `ini_text` writes:
  - `[fs] romfs = <folder-or-.nup>`
  - optional `[fs] romfs_overlay = <work/romfs-overlay>`
  - `[engine] packed_nup = 1`

A folder build writes no `romfs_overlay` line. A `.nup` build normally does.

### B.3 Exact runtime flow

`engine/fefw-recomp/runtime/src/services/fs.cpp`:

- `rom_image()` reads `[fs] romfs` and `[fs] romfs_overlay`.
- The package overlay is added **before** installed mods: base data, then mods.
- `RomfsImage::build` calls `make_layout`.

`services/fs_romfs.cpp`:

- `scan_nup` reads every RomFS entry from the package. A compressed entry gets
  `packed = true`, `pkg = f`, and is **included in the tree**. The current source
  comment in `loader/nup.cpp` says compressed entries are “left out of the served
  tree”, but `scan_nup` does not leave them out. That comment is stale and should
  not be used as the basis for a fix.
- `make_layout` scans each overlay folder and merges it into the package tree.
  A file at the same path replaces the package node; a new path is added.
- `read_host` serves:
  - a compressed `.nup` entry through `packed_content` (whole-file decompression,
    then `PackedCache`);
  - a stored `.nup` entry through a positioned `ReadFile` on the package;
  - an overlay/folder file through the ordinary 16-handle cache.

The current runtime therefore **can** serve a packed `.nup` without the overlay.
The launcher’s overlay is a compatibility path (and a legacy response to the
`main.npdm is compressed in the .nup` failure in #24), not a requirement of the
current `scan_nup`/`PackedCache` code.

### B.4 Equivalence verdict

**Content can be equivalent; inputs are not guaranteed equivalent.**

| Input | `.nup` build | Extracted-folder build | Equivalent only when |
| --- | --- | --- | --- |
| ExeFS | Copied from the selected update/base package, SHA-256 checked by `Nup::read` during extraction | User’s `exefs/`, only warned if hashes differ | Folder ExeFS has the same module bytes/build |
| RomFS tree | Selected program package’s RomFS (normally update’s full tree) | User’s `romfs/` tree, no update merge | Folder is the same merged base+update tree |
| Compressed RomFS entries | Overlay files, or direct package decode if overlay absent | Ordinary files on disk | Overlay is complete and current, or direct decode is used |
| Stored RomFS entries | Read from package | Files on disk | Package and folder contain the same bytes |
| Mods | Overlay below mods | Folder below mods | Overlay has no stale/orphan entries |
| Integrity | ExeFS copy is SHA-checked; runtime packed reads are not | No hash check | Folder has not been corrupted or partially replaced |
| Read behaviour | Mixed package reads, overlay files, and packed-cache decompression | Folder file reads | Content equivalence, not performance equivalence |

The runtime’s file tree can be identical in the happy path. The launcher does not
establish the happy path; it assumes it.

### B.5 Failure modes that fit #70

Ranked by how well they fit “some cinematics lag and voices cut, and rebuilding
from `.nup` fixes it”:

1. **The extracted folder is the base RomFS, or an incompletely merged update.**
   The launcher does not merge. The engine’s `romfs_files` check catches a folder
   below 95% of 252,686 files, but a folder at 95% or above can still be missing a
   small set of movies or voice banks. Those specific reads fail or return the
   wrong asset while the game boots and most scenes work. This is consistent with
   selective cutscene/voice symptoms.
2. **The ExeFS and RomFS come from different versions.** A folder can contain the
   update’s code with the base game’s data, or the reverse. `check_known_dump`
   warns about ExeFS hashes, but the warning does not block the build, and there
   is no RomFS manifest comparison. The launcher's `.nup` path derives both its
   ExeFS and its RomFS from one selected package, so rebuilding from it can
   remove the mismatch.
3. **The folder has corrupted or stale files.** The folder path has no per-file
   hash verification. The `.nup` path verifies SHA-256 while copying ExeFS and
   while extracting compressed RomFS entries through `ew_format::nup::Nup::read`,
   so a `.nup` rebuild can silently replace damaged files with correct ones.
4. **The `.nup` overlay is stale and shadows the package.** This is the opposite
   direction: a `.nup` build can be wrong if `work/romfs-overlay` contains files
   from an older package. The current stamp and skip rules make that possible.
   It should be checked before concluding that `.nup` is always the correct source.
5. **Large packed entries are decoded whole on every read.** If the overlay is
   absent and a packed audio/video file exceeds the 256 MiB per-entry cap, each
   read decompresses the whole entry and does not cache it. This can cause
   hitches, but it would affect a `.nup` build more than a folder build, which is
   the reverse of the reporter’s observation. It is a real asymmetry, not the
   first explanation for #70.

The AV1 worker already has a bounded decode-ahead queue and a dedicated thread.
A decoder defect would normally leave decoder-specific log lines, repeated frame
delivery failures, or audio-clock divergence. Nothing in the issue text proves
that. The first evidence to collect is the data identity described below.

### B.6 Two package-selection algorithms

There is a second, independent divergence:

- Launcher `ew_format::nup::Nup::program_package()` selects one package: first
  update, else base.
- Runtime `loader/nup.cpp::open()` selects the highest-ranked ExeFS and the
  highest-ranked RomFS **independently**. Its own comment says “ExeFS and RomFS
  may come from different packages (an update without one).”

So a `.nup` with an update that has an ExeFS but no RomFS can be interpreted
differently by the launcher and the runtime.

This was checked against the current parser, not inferred only from the runtime.
`Nup::open` validates the TOC offsets, directory child indices and file extents,
but it does not validate a package’s `root`, `exefs` or `romfs` indices.
`program_package()` returns the first update without checking them, and `walk()`
indexes `self.dirs[d as usize]` directly. A fixture with an update package, a
valid ExeFS and `romfs == 0xFFFFFFFF` therefore opens successfully;
`program_package()` returns that update; and `walk(pkg.romfs)` panics at
`nup.rs` with an out-of-bounds index. In `use_nup`, this happens while building
`listed` from `walk(pkg.romfs)`, before the later
`nup.dirs[pkg.romfs as usize]` assignment. `packed_romfs` and
`engine::shader_romfs_nup` have the same unguarded `walk`. The runtime handles
the layout by skipping the sentinel and pairing the update ExeFS with the base
RomFS, or by failing cleanly when no RomFS exists. The parser does **not**
reject this layout, so this finding is not softened.

This should be treated as a robustness bug in the data path, independent of #70.

### B.7 Overlay correctness gaps

The overlay was the weakest link in the `.nup` path. The first three items are now
addressed by `gamedata::overlay_identity` and `gamedata::write_romfs_overlay`;
the last two remain open:

1. **Identity (fixed).** The identity now covers the package path, its size and
   modification time, and a SHA-256 over every packed entry’s path, decompressed
   size and entry hash. A package replaced with one that has the same count and
   total size but different contents no longer reuses the old overlay.
2. **Orphan cleanup (fixed).** A different identity clears `work/romfs-overlay`
   before the write, so files from an older package cannot remain and, because the
   overlay is merged after the package, override a current stored entry.
3. **Resume (fixed).** Files are written to a staging folder and renamed into
   place, so a file at its final path is complete. A destination at the expected
   length is accepted only within the same identity, which covers the content of
   every packed entry.
4. **Narrow reuse of the ExeFS copy.** `use_nup` reuses the copied ExeFS when all
   lengths match; it does not verify hashes. The directory is keyed by title and
   version, so this is less exposed than the overlay, but a same-version repack
   can still leave stale modules.
5. **Disk cost is not surfaced after the build.** The overlay can be gigabytes;
   the launcher reports it during extraction, but there is no ongoing accounting
   or cleanup policy.

These are correctness issues in data preparation, not rendering or decoding
issues. The overlay items were fixed before any decoder work, as required.

### B.8 Minimum evidence needed before changing code

Run the same build and the same cutscene twice, once from the `.nup` and once
from the extracted folder that the reporter used, and record:

- `main` SHA-256 and the `main.npdm` version/build ID;
- ExeFS module hashes for all modules in `data/dump_hashes.json`;
- RomFS top-level count, total file count, and total bytes;
- SHA-256 of the affected `.webm` movies and `.opus` voice files, plus their
  sizes, from both sources;
- the `fefw.log` lines beginning `RomFS image from`, `package overlay`,
  `RomFS: ... packed`, and every `fs`/`movie`/`audio` error around the cutscene;
- whether the folder contains the update’s merged RomFS, the base RomFS, or only
  the update’s delta files;
- the exact launcher source path and the `.nup` path, so the two are not confused.

If the file sets and hashes match, the decoder becomes the leading suspect. If
they do not, #70 is a data-path issue and a decoder rewrite would not fix it.

### B.9 Phase 2A fixes (items 1-3 implemented, items 4-5 open)

These are deliberately small and evidence-preserving:

1. **Version the overlay directory by a strong source identity.** Include package
   size, modification time, and a digest of the TOC/manifest, not only packed
   count and bytes. Write into a new directory and switch the ini after success.
   Old directories can then be removed deliberately, not silently reused.
2. **Write and verify an overlay manifest.** Record every packed path, its
   decompressed size, and the source entry hash. On reuse, require the manifest to
   match the package; remove paths that are no longer packed, or rebuild the
   directory from scratch. Do not hash every file on every launch; validate the
   manifest and the files written by this launcher.
3. **Never let an orphan shadow the package.** If the overlay is retained, merge
   it only for paths that are currently packed, or clear the destination before a
   full rewrite. A manifest-driven rebuild is simpler and safer.
4. **Align package selection.** Make the launcher use the same independent
   ExeFS/RomFS selection as the runtime, or reject a package whose selected
   update has no RomFS with a clear message. Bounds-check `pkg.romfs` before
   indexing it.
5. **Add a data-source equivalence check.** When both a `.nup` and a folder are
   available, compare ExeFS hashes, RomFS file count/bytes, and a manifest of
   movie/voice files. When only a folder is available, state that it is
   unverified rather than implying it is the tested tree. This can be a diagnostic
   report first, not a blocking gate.
6. **Decide the overlay’s future.** The current runtime serves packed entries
   directly. The overlay is only needed for older engines or for a compatibility
   guarantee. Either version-gate it and make it safe, or remove it after an
   engine-version check and a measured comparison. Do not remove it without
   confirming the shipped/installed engine can read packed entries in place.

No Phase 2A code should start until the user approves the audit and the evidence
in B.8 exists.

---

## C. Issue #37: freeze and device loss

### C.1 Evidence in the issue

- After roughly two hours, during a scene/audio batch load, a freeze report says
  `FREEZE: no frame tick for 10.0 s (frame count 0)`.
- A later report on an RTX 5060 at 2560×1440 shows `VK_ERROR_DEVICE_LOST` after
  410,785 presented frames.

These may be two different failures. A freeze with frame count 0 is not the same
as a device loss after 410k frames, and treating them as one bug would hide the
distinction.

### C.2 Existing handling

- `diag/watchdog.cpp` counts frame ticks and a separate progress heartbeat. The
  default threshold is 10 s when no option is supplied; `ini_text` writes
  `[diag] watchdog_seconds = 30`, so the effective threshold depends on how the
  runtime is started. The freeze report includes thread stacks, process memory,
  system memory, and the last 30 s of frame times.
- `gfx/vk_renderer.cpp` has a terminal `device_lost()` path. It logs
  `GPU DEVICE LOST (reason: VK_ERROR_DEVICE_LOST)`, includes context, writes a
  report, and ends the process after one thread wins the once-flag. It is called
  for `vkWaitForFences`, `vkAcquireNextImageKHR`, `vkQueueSubmit2`, and
  `vkQueuePresentKHR`.
- `gfx/vk_device.cpp` has VMA, `VK_EXT_memory_budget`, debug utils, and optional
  extension checks. `VK_EXT_device_fault` was not found in the audited source.
- `nvn/nvn_astc_gpu.cpp` already documents that GPU ASTC transcoding on an AMD
  iGPU held frames for 1–2 s and produced a `VK_ERROR_DEVICE_LOST`; `auto` now
  uses the CPU path on integrated GPUs. This is directly relevant to #29.

### C.3 What is not known

The current evidence cannot distinguish:

- a driver TDR or GPU hang from an invalid submission;
- VRAM exhaustion from system commit/pagefile exhaustion;
- a long pool import or shader compile from a deadlock;
- a render-thread stall from a GPU wait;
- a real device loss from the process being killed while the report was written.

The freeze report’s `frame count 0` is ambiguous: it can mean no frame has been
presented yet, a counter was reset, or the report was emitted from a path that
does not see the render counter. It should not be read as “the game never
rendered” without the surrounding log. The watchdog also re-arms on
`diag_progress()` heartbeats, so a long operation that reports progress can keep
the watchdog quiet while no frame is presented; the freeze report must show both
counters to distinguish that from a true stall.

### C.4 Diagnostic plan (before any recovery code)

1. **Device fault capture when available.** If `VK_EXT_device_fault` is present,
   enable it and log the fault address/description/vendor binary dump on
   `VK_ERROR_DEVICE_LOST`. If it is absent, log that fact; do not require it.
2. **Breadcrumb ring.** Keep the last 30–60 s of: frame number, frame time,
   `VmaBudget` usage/budget, process working set/private commit, pagefile usage,
   packed-cache retained/peak, audio queue depth/underruns, last submitted command
   buffer/serial, and the current hitch phase. Emit it on device loss and freeze.
3. **Mark long operations.** Add explicit breadcrumbs around pool imports, ASTC
   transcodes, shader/pipeline creation, `.nup` whole-file decompression, and
   movie seeks. The existing hitch timer names some of these; the freeze report
   should show which one was active.
4. **Separate counters.** Log both the render frame counter and
   `diag_progress()` in the freeze header, and state whether the process was still
   making progress elsewhere.
5. **Reproduce with a bounded soak.** Use the same save and route, record the
   gauge CSV, and stop at the first device-loss/freeze. Do not add recovery or
   restart logic until the cause is known; a silent restart would hide the bug.

Possible fixes after evidence include a VRAM/commit guard, a pool-import budget,
or a device-loss recovery policy, but none should be chosen from the current
evidence.

---

## D. Issues #26 and #29: upscaler discovery and hybrid hardware

### D.1 The two upscaler vocabularies

The launcher and engine have two different settings:

- `[upscale] mode` selects the presentation upscaler: `fsr1`, `fsr4`, `fsr3`,
  `nis`, `mako`, `dlss`, `dlaa`, `xess`, or off.
- `[dlss] upscaler` selects the host implementation behind the game’s NVN2 DLSS
  API: `taau`, `dlss`, `fsr3`, `xess`, `bilinear`, or `auto`.

`taau` is the built-in temporal upscaler and the fallback. “Only TAUU is
recognised” therefore usually means “the vendor provider was not loaded and the
engine fell back to TAUU”, not “the engine has no other upscaler code”.

### D.2 Current discovery

- `gfx/extras/ffx_files.cpp` searches `upscalers\fsr`, `upscalers\fsr3`, the
  executable folder, and through `upscaler_paths::vendor_dirs("fsr"/"fsr3")`.
  The shared `upscaler_paths.h` walks the executable’s ancestors, the working
  directory, and the launcher directory, and tries both `upscalers` and
  `upscaler`, plus `bin`, `bin\x64`, `x64\bin`, and `x64` subfolders.
- The launcher’s `engine::sdk()` performs a similar search across the run folder,
  the game folder, the engine package folder, and the launcher folder. It accepts
  either the SDK 2.x loader+upscaler pair or the v1.1.4 single DLL. A lone loader
  is not accepted.
- `upscale_mode` maps the launcher labels to the engine’s modes. The launcher
  deliberately downgrades a requested XeSS/DLSS/FSR mode to FSR 1 when the SDK is
  absent, so the game does not start with a black picture and working sound.

The ancestor walk and `upscaler` spelling were added in response to #26/#29. That
is the right direction. The remaining gap is verification on the reporters’
actual layouts and GPUs.

### D.3 What to check on a report

Ask for, or extract from the diagnostics bundle:

- the exact `upscalers`/`upscaler` tree the player created;
- `fefw.log` lines from `fsr3`, `dlss`, `xess`, `upscale`, and `nvn`;
- the F1 page’s “found/missing” line;
- whether the game was started from the launcher or directly from `fefw.exe`
  (which changes the working directory and `NXRT_LAUNCHER_DIR`);
- the selected `[upscale] mode` and `[dlss] upscaler`;
- the GPU actually chosen on a hybrid system.

A copied DLL in the wrong folder is now logged with the searched directories. The
next report should include those lines rather than another screenshot of the
folder.

### D.4 #29 hardware and rendering

The Z1 Extreme + RX 6850M XT system has two GPUs and limited shared memory. Three
different questions must not be conflated:

1. **Which GPU is active?** The launcher writes `[gfx] gpu` (automatic, discrete,
   integrated). On a hybrid handheld, the wrong device or a driver-level
   preference can make 720p slow. Log the selected Vulkan device and its type at
   startup.
2. **Is the frame CPU-bound or GPU-bound?** The existing hitch and pool-import
   instrumentation can separate replay-thread work from GPU waits. Use a capture
   before changing culling, streaming, or draw batching.
3. **Are the torch-lighting glitches correctness or precision?** They need a
   RenderDoc frame capture on the same save and scene. The existing alpha/HDR
   layout caches and storage-write tracking are plausible places to inspect, but
   the audit found no evidence that they are the cause.

The cutscene slow/desynced/muted-voice report overlaps #70 and the movie/audio
path. It should be triaged with the same data-identity check first. If the file
sets match, then collect decoder/audio clock lines.

`nvn_astc_gpu.cpp` already avoids the GPU ASTC path on integrated GPUs because it
produced 1–2 s frames and a device loss. If the reporter forced `[gfx] astc_gpu =
on`, that is a concrete, testable cause for stutter or device loss. The audit did
not verify their setting.

---

## E. Memory and resource lifetime

### E.1 Bounds already present

- `PackedCache`: 1.5 GiB retained budget, 256 MiB per-entry cap, LRU eviction,
  explicit `skipped` counter for entries that cannot be retained.
- `RomfsImage`: 16 open handles (`kOpenFiles`), positioned reads, per-file
  `shared_ptr` handles.
- XAudio2: 64 slots, bounded queue, underrun/drop counters, device recovery.
- Vulkan: VMA, budget extension, pool mirrors, frames in flight.
- Watchdog/freeze report: process working set, peak, private commit, pagefile,
  guest heap, Vulkan budget.

The earlier audit found no unbounded per-read decoded-PCM growth. This pass found
no new unbounded allocation in the `.nup` path beyond the already documented
packed cache.

### E.2 Gaps

1. **Packed-cache statistics are not periodically logged.** `RomfsImage::stats()`
   exposes retained/peak/evictions/skips, but `fs.cpp` logs only the initial image
   size. A long session with packed audio cannot show whether the cache is
   thrashing or near its budget from the normal log.
2. **Overlay disk usage is not accounted for.** `work/romfs-overlay` can be
   gigabytes and is not part of the run-folder report.
3. **Integrity is uneven.** The Rust `.nup` reader verifies SHA-256; the runtime
   `nup::read_file` does not. A corrupted stored entry is served as bytes. This can
   look like a decoder or audio bug.
4. **Overlay orphan files can retain stale data indefinitely**, as described in
   B.7.
5. **No per-file handle attribution in the freeze report.** The 16-handle cache is
   bounded, but a stall inside `acquire`/`ReadFile` is not named.

### E.3 Measurement plan

- Log `RomfsImage::Stats` every 60 s when a package or overlay is in use, at
  `Info` only when values changed materially.
- Add packed-cache retained/peak/evictions/skips to the freeze and device-loss
  headers.
- Record overlay path, file count, and bytes at build and at first play.
- In the benchmark matrix, sample process working set, private commit, pagefile,
  VMA budget/usage, packed-cache bytes, and audio queue depth at the same points.

---

## F. Frame-time, graphics, audio, and video evidence

### F.1 Existing instrumentation

- `diag/hitch.cpp` logs frames over a threshold and a summary every 10 s.
- `diag/gauge.cpp` emits a CSV with frame time p50/p99/max, working set, private
  commit, pagefile, handles, threads, GDI/user objects, and CPU time.
- `gfx/frame_limiter.cpp` keeps an 8192-interval ring and reports pacing stats.
- `media/movie_vdec.cpp` has a dedicated AV1 worker, bounded decode-ahead, thread
  priority/power-throttling handling, and diagnostic registration.
- `services/audio_xaudio2.cpp` reports accepted/dropped frames, underruns, queue
  depth, and device recovery.

### F.2 What is missing for these issues

- A single capture format that combines frame time, GPU busy, decoder queue,
  audio queue, and memory at the same timestamps. The pieces exist; the issue
  reports do not contain them together.
- p95 is required by [GAMEPLAY_TESTING.md](GAMEPLAY_TESTING.md), while the gauge
  line shown in this audit exposes p50/p99/max. Confirm p95 is emitted elsewhere,
  or add it to the capture summary.
- No automatic before/after comparison. The benchmark plan is manual and
  environment-sensitive.
- No route/save identifier in the log, so two reports cannot be proven to use the
  same scene.

None of these justify a new profiler dependency. They justify consistent capture
and reporting.

---

## G. Build-pipeline reliability

### G.1 Current state

- The launcher probes `vswhere`, the compiler, the Windows SDK, CMake, Ninja, and
  Python; it compiles and links a probe file before the real build; it logs a
  toolchain fingerprint.
- `engine/fefw-recomp/runtime/scripts/env.cmd` still checks a hardcoded
  `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools` path **before**
  asking `vswhere`. If both VS 2022 Build Tools and VS 2026 are installed, the
  script can select the old toolchain while the launcher’s probe selected the new
  one.
- `runtime/scripts/build.ps1` defaults to 10 jobs (or `NX_JOBS`), ignoring the
  launcher’s memory-aware job estimate. `game-build/scripts/build_fefw.ps1`
  defaults to `min(16, cores-2)`. These can oversubscribe a 14.7 GB handheld.
- `sccache` is optional and not present on the audited machine; a clean build is
  therefore slower and more variable.
- The repository contains a `build-runtime-111.log` with `'vswhere.exe' is not
  recognized` near the top. The build continued using a Visual Studio Build Tools
  installation, but the warning shows the environment setup is not hermetic.
- Local VS 2026 Community `18.12.12412.261` with MSVC `14.52.37025` was detected,
  but the Windows SDK installation showed only `UnionMetadata` under the audited
  path and `--check-toolchain` produced no usable output. The launcher’s probe was
  not verified end-to-end on this machine in this pass.

### G.2 Recommended hardening (Phase 2D, after data/diagnostic work)

1. Make `env.cmd` use `vswhere` first, then fall back to the hardcoded VS 2022
   path only when `vswhere` returns nothing. Log which `vcvars64.bat` was chosen.
2. Pass the launcher’s chosen job count to the engine scripts explicitly, or make
   all scripts default to the same memory-aware formula.
3. Record the exact `vcvars64.bat`, MSVC version, Windows SDK version, CMake,
   Ninja, LLVM, and Python versions in the build log and the diagnostics bundle.
4. Add a small build-environment test that asserts the selected compiler’s
   `clang-cl --version` and the SDK’s `rc.exe` are the ones the probe validated.
5. Do not add a second build system. The existing CMake/Ninja/clang-cl path is
   adequate; the problem is selection and reporting.

---

## H. Prioritized roadmap and gates

### Phase 1 — this audit

- Read-only source and issue review.
- `.nup` vs folder data-path analysis.
- No source changes.

### Phase 2A — data correctness (highest priority)

1. Add the evidence capture in B.8, including the reporter’s two trees.
2. Fix the overlay identity/manifest/orphan rules (B.9).
3. Align package selection and bounds-check `.nup` directory indices.
4. Add a non-blocking data-source equivalence report.
5. Add regression tests with synthetic `.nup` packages:
   - compressed and stored entries;
   - overlay complete, incomplete, stale, and orphaned;
   - package replaced with same/different count and bytes;
   - update-without-RomFS layout;
   - `.nup` vs extracted folder with identical content produces the same file
     list and bytes.
6. Decide, with the engine’s compatibility guard, whether the overlay remains.

**Gate:** no decoder changes until B.8 shows the file sets and hashes match, or
shows a decoder-specific failure with matching inputs.

### Phase 2B — diagnostics for #37/#29

1. Device-fault capture when supported.
2. Breadcrumb ring and richer freeze/device-loss headers.
3. Packed-cache and audio/decoder counters in the same report.
4. Log selected GPU, driver, and upscaler provider at startup.
5. Reproduce the freeze and device loss with a bounded soak and capture.

**Gate:** no recovery/restart policy and no rendering changes until a capture
identifies the failing stage.

### Phase 2C — performance, capture-driven only

1. Same-route before/after captures in the hub, Dagsion crowd, battle, and
   cutscenes.
2. Attribute CPU render-thread time, GPU busy, pool import, ASTC transcode,
   pipeline creation, and texture streaming.
3. Only then consider culling, batching, streaming, or pacing changes.
4. Keep p50/p95/p99, 1% lows, memory plateau, and audio/video sync as acceptance
   metrics, not average FPS alone.

### Phase 2D — build hardening

1. `env.cmd` selection order and logging.
2. One job-count policy across launcher and scripts.
3. Toolchain/SDK provenance in the diagnostics bundle.
4. VS 2026 / Windows SDK end-to-end verification on a clean machine.

### Not recommended

- Replacing `fefw-recomp` with the Rust research runtime.
- Rewriting the AV1 decoder before the data path is proven equivalent.
- Adding a second renderer, a mandatory Vulkan layer, or a new allocator.
- Forcing device-loss recovery that hides the original failure.
- Shipping or bypassing vendor upscaler DLL signatures.
- Declaring a fix from a boot-only or short menu session.

---

## I. Verification plan

### I.1 Automated checks that exist and should remain green

- Launcher tests: `cargo test -p emblemweave`.
- Format tests: `cargo test -p ew-format`.
- Runtime tests: the CMake/CTest target used by `scripts/build.ps1 -Test`.
- Existing `.nup`/`.nx`/RomFS tests, including packed-cache accounting and the
  launcher’s synthetic package tests.

### I.2 New regression checks for Phase 2A

Use a synthetic package with at least:

- one stored RomFS file, one zstd-packed RomFS file, and one packed ExeFS file;
- a movie-sized entry and an audio-bank-sized entry;
- a base package and an update package;
- an update with an ExeFS but no RomFS.

Assert:

- launcher and runtime select the same ExeFS/RomFS;
- a complete overlay produces the same effective tree as the package alone;
- a stale overlay cannot override a current stored file;
- an orphan overlay file is removed or ignored;
- a same-size stale overlay file is not reused when the source identity changed;
- a package/overlay error is reported, not silently served.

### I.3 Real-data checks (require the user’s dump/hardware)

1. Build from the `.nup`; build from the extracted folder; record the B.8 manifest.
2. Play the same cutscene with the same save and settings; capture p50/p95/p99,
   decoder queue, audio underruns, and `fs` errors.
3. If the manifests differ, stop and fix the data path. If they match and the
   cutscene still fails, capture the decoder/audio path.
4. Run the #37 soak on the RTX 5060 with the new breadcrumb ring; capture the first
   device loss/freeze.
5. On the Z1 Extreme system, log the selected GPU and run 720p hub/village/
   colosseum routes with `astc_gpu` auto and forced, then compare.
6. For upscalers, test the exact `upscalers\fsr`, `upscaler\fsr`, `bin\x64`, and
   ancestor layouts and confirm the F1 page/log agree with the launcher.

### I.4 Limitations of this audit

- No real game dump, save, controller, handheld, or vendor upscaler DLLs were
  available in the development environment.
- No RenderDoc/GPU capture or process-RSS measurement was performed.
- The working tree is not a clean commit; this audit describes the files as read
  and did not modify them.
- Local VS 2026/Windows SDK verification was inconclusive.
- Issue symptoms are taken from the issue text and earlier notes; no new
  reproduction was possible.
- The audit does not claim that any candidate fix improves frame time. Phase 2C
  requires measurements before and after.

---

## J. Approval point

Phase 1 ends here. The recommended next step is **Phase 2A only**: capture the
`.nup` vs folder manifest, then fix the overlay identity/cleanup and package
selection. That work is small, testable, and directly addresses the class of
failure behind #70 without touching the decoder or renderer.

Broad code modifications should wait for:

1. approval of this audit;
2. the B.8 evidence from at least one real `.nup` and one extracted folder;
3. a confirmed decision on whether `work/romfs-overlay` remains a supported
   compatibility path or is retired for engines that can read packed entries
   directly.
