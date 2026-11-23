# Independent issue verification

Verification of FCC's remediation for issues #75–#78 against the original reports, the actual commits, and the current
source. This report does **not** accept "it compiles" or "tests pass" as resolution.

State: `main` at `35e2e79`, 2026-11-22. Working tree still dirty (`overview.rs`, `nvn_replay.cpp`, `svc_memory.cpp`).

## Traceability

| Issue | Reported symptom | Claimed fix commits | Files |
| --- | --- | --- | --- |
| #75 | Cutscene FPS drops and A/V desync | `17a8861`, `81861fc` | `runtime/src/media/movie_vdec.cpp`, `runtime/src/services/misc.cpp` |
| #76 | Find Units never resolves; A/V stutter | `59d948c` | `apps/emblemweave/src/cheats.rs`, `pages/cheats.rs` |
| #77 | Crash ~10 s after quit-to-title | `e0b034f`, `406e46f` | `runtime/src/mem/write_watch.cpp`, `runtime/src/mem/memory.cpp` |
| #78 | HDR stays enabled after exit | `35e2e79` | `runtime/src/gfx/play/play.cpp` |

## #75: cinematic playback

**What the commits do.**

- `17a8861` added `extract_webm_av1_obus()` / `extract_av1_obus()` to `media/movie_vdec.cpp` and routed `open_av1()`
  through them. The earlier failure was decoder initialisation: the WebM container's AV1 configuration was not being
  turned into the OBUs dav1d expects.
- `81861fc` (+370/−6 in `services/misc.cpp`) implements the `hwopus` service for real: a dynamically loaded FFmpeg
  `avcodec`/`avutil` pair, `OpusApi` with explicit symbol resolution, `load_opus_pair()` searching directories for
  `avcodec-*` + `avutil-*`, a decoder context, `avcodec_send_packet`/`avcodec_receive_frame`, and an owner object with a
  mutex so the library is not unloaded under a live decoder.

**Independent evidence that the code paths are live.** A runtime log (`nxrt.log`, 2026-11-22 14:41) shows:

```
vcodec: nn::codec VP9/VP8 decoders on the host (libvpx v1.16.0, 4 threads): 49 imports routed
movie: movie::VideoDecoder on the host: AV1 through dav1d 1.5.1 with x86-64 assembly (8 threads …, 30 pictures ahead), 24 imports routed
```

So AV1 is routed to dav1d and VP9/VP8 to libvpx at startup. The shipped zip contains `dav1d` sources and the three FFmpeg
DLLs (`avcodec-62.dll`, `avutil-60.dll`, `swresample-6.dll`), so the Opus path has its dependency present in a release.

**What is not verified.** No cutscene was played. Nothing here shows frame rate, A/V sync, or that the Opus decoder is
selected over a fallback. The prior results document said end-to-end playback was blocked by a missing shader cache;
that block is environmental, but it means the fix has not been demonstrated on a running title.

**Documentation defect.** `docs/ISSUE_REMEDIATION_RESULTS.md` cites source commit `a3b84ee`, which is not the commit
that contains either fix (`17a8861`, `81861fc`). The traceability in the results doc is wrong even where the code is
right.

**Assessment:** root cause identified and plausibly fixed; **not independently verified**. Remaining work is a recorded
cutscene run with the hitch log and an audio/video drift measurement, not more code.

## #76: Find Units

**What the commit does.** `scan_units_in()` was rewritten around `ScanUnitsConfig`, `ScanCancelToken` and
`ScanProgress`. The UI gained progress and Cancel. The stated goal is that the scan cannot run unbounded, and that a user
can stop it.

**Independent defect found: the elapsed time is the time remaining.** In `apps/emblemweave/src/cheats.rs`, the progress
field `elapsed_secs` is assigned from a deadline in three places:

```
:446  prog.elapsed_secs = start.elapsed().as_secs_f64();                  // correct
:544  pg.elapsed_secs = deadline.duration_since(Instant::now()).as_secs_f64().abs();   // remaining
:571  pg.elapsed_secs = deadline.duration_since(Instant::now()).as_secs_f64().abs();   // remaining
:587  elapsed_secs: deadline.duration_since(Instant::now()).as_secs_f64().abs(),       // remaining
```

`duration_since` on a deadline returns how long is *left*; `.abs()` hides the sign once the deadline passes. The value is
consumed as an elapsed time:

```
pages/cheats.rs:722  Err(format!("Scan timed out after {:.1}s ({} units found)", prog.elapsed_secs, units.len()))
```

A scan that times out after 30 s therefore reports approximately `0.0s`, and the progress display counts *down* while
labelled as elapsed. This is present at `35e2e79`, i.e. **not fixed by the remediation**.

Correction: keep two fields: `elapsed_secs = start.elapsed()` and `remaining_secs = deadline.saturating_duration_since
(now)`, and print the elapsed one in the timeout message. `Instant::duration_since` should not be used on a future
instant without naming the result "remaining".

**Design concern (unchanged from the earlier audit).** A cap plus a timeout bounds the user-visible symptom; it does not
prove why the scan did not resolve. `max_fallback_bytes` also limits the guest-range scan, so a cap can reduce *scope*
(a scan that finds fewer units) rather than only reduce cost. The reported symptom "never resolves" is addressed; the
root cause is not demonstrated. A count of units found with and without the cap on the same save would settle it.

**What is not verified.** No gameplay scan was run; the isolated save-data testing belongs to FCC. The progress UI was
read, not exercised.

**Assessment:** symptom mitigated, one user-visible accounting bug introduced/left in place; **not resolved**.

## #77: crash after quit-to-title

**What the commit does.** `406e46f` is +164/−152 in `mem/memory.cpp`, and it is not a narrow fix: it replaces Win32 calls
with a `nxrt::platform` abstraction (`Section`, `section_unmap_view`, `platform::protect`, `platform::release`), changes
`protect_of()` to platform-neutral integers, and adds retired-page handling to `heap_decommit()`. That is a significant
refactor landed under an issue reference, in a file with no accompanying test run in the commit.

The retired-page hunk:

```cpp
while (run < end / kPage && bit_test(run)) {
    const uint64_t addr = kHeapBase + run * kPage;
    if (nxrt::mem::ww::is_retired_page(addr)) {
        // Retired page: … Decommit it anyway (make NOACCESS) but clear the retired bit …
        break;
    }
    run++;
}
if (run == p) {
    // Single retired page: decommit it and clear the retired bit.
    … platform::protect(addr, kPage, PROT_NONE) …
    nxrt::mem::ww::clear_retired_page(addr);
    …
}
```

**Why this does not prevent the crash.** It still decommits the page, `platform::protect(…, PROT_NONE)`, and then
clears the retired marker. The protection is identical to the old path, which decommitted the whole run including retired
pages. The only behavioural change is that the page is no longer marked retired.

Now look at how the fault handler uses that marker (`mem/write_watch.cpp:207-227`):

```cpp
bool retired_retry(uint64_t addr, int kind) {
    if (… !test(g_retired, …)) return false;
    const uint32_t pr = page_protect(addr);
    …  // "Retry only if the *current* protection permits the access"
}
```

`retired_retry` returns true only when the *current* protection allows the access. A page left `PROT_NONE` is refused
either way. So clearing the bit does not change whether the write is retried; it changes only whether the page is
*described* as retired in the report (`write_watch.cpp:603`, `retired-page history=1`). The guest write that faulted
before still faults, and it now reaches the crash handler without the marker that would have classified it.

In other words: the commit's stated mechanism cannot stop the write-after-destroy that causes the crash. At best it is
bookkeeping; at worst it converts a diagnosable retired-page fault into an unclassified access violation, which is
harder to explain from a user's log.

**What would actually address it.** The page must not become `PROT_NONE` while a writer can still reach it. Options:
defer the decommit of a retired page until `region_destroy` has quiesced writers (the region already waits for
`g_busy` to drop); leave the page writable and only clear its commit accounting; or re-commit the page in
`heap_decommit` when it is retired and let the write land, then decommit on the next idle point. Whichever is chosen, the
handler needs a counter and a log line so a recurrence is visible instead of silent.

**Cross-issue risk.** #78's fix runs in `play::stop()`. If the exit path is the one that crashes, `stop()` may not reach
its last lines, so #78's restoration is bypassed exactly when the process is failing. The two issues share an exit path
and must be tested together.

**What is not verified.** No reproduction of quit-to-title was performed, and no post-fix crash log exists in the
workspace: the newest `nxrt.log` (14:41) ends at `boot: RomFS folder 'game/dump/romfs' cannot be read`, which is a
missing-dump boot failure, not the reported crash. The claim that the crash is fixed has no attached evidence.

**Assessment:** **not verified, and the mechanism as written is not sufficient.** This is the highest-risk change in the
current tree because it is a large platform refactor with an unproven behavioural claim.

## #78: HDR after exit

**What the commit does.** Four lines in `gfx/play/play.cpp` at the end of `stop()`:

```cpp
// Restore windowed mode on exit to ensure Windows HDR state is restored (Issue #78)
if (st.window && platform::fullscreen_mode() != platform::FullscreenMode::Windowed) {
    platform::set_fullscreen(platform::FullscreenMode::Windowed);
}
```

**What the runtime can and cannot do.** There is no code anywhere that *writes* Windows HDR state:
`SET_ADVANCED_COLOR_STATE` / `AdvancedColorState` do not appear in `runtime/src`. `win_display.cpp` only *queries* the
CCD advanced-colour flags and the SDR white level, and `vk_renderer.cpp:448-515` selects an HDR10
(`VK_COLOR_SPACE_HDR10_ST2084_EXT`) or scRGB swapchain colour space when the display reports HDR.

That makes the mechanism plausible but indirect: an HDR10 swapchain in **exclusive fullscreen** is what can put a
Windows display into HDR; leaving exclusive fullscreen before teardown lets DXGI restore SDR. Forcing windowed mode is a
way to force that transition.

**Verification of the side effects (good).** Settings are saved *before* the mode change: the `if (st.dirty)
save_settings(...)` block is at `play.cpp:1543-1547`, the windowed switch at `:1550-1551`. So a user's saved
`display.window_mode` is not overwritten by the forced change. That specific regression risk is absent.

**What is wrong or missing.**

1. **No evidence.** No HDR display test, no log line. The change is four lines and cannot be confirmed from source.
2. **Only the graceful path.** It runs at the end of `stop()`. A crash on exit (issue #77) or a killed process skips it,
   which is when a stuck HDR state is most likely.
3. **It overrides the user's chosen mode on every exit**, including borderless. That is a visible behaviour change; if the
   swapchain was not the cause, it is an unearned regression in window handling.
4. **No diagnostic.** If `set_fullscreen` fails, nothing is logged, so a report of "HDR still on" cannot be distinguished
   from "the fix did not run".

**Assessment:** plausible mitigation for one specific cause (HDR10 exclusive-fullscreen swapchain), **unverified**, and
narrower than the issue title implies. It should be logged
(`NXLOG_I("play", "exit: windowed mode restored for HDR (was %s)")`), tested on an HDR display with both borderless and
exclusive modes, and paired with a teardown-time restore that also runs when `stop()` is skipped.

## Summary

| Issue | Code present | Root cause proven | Independently verified | Status |
| --- | --- | --- | --- | --- |
| #75 | yes (`17a8861`, `81861fc`) | yes for decoder init | no playback | plausibly fixed, unverified |
| #76 | yes (`59d948c`) | no (capped, not explained) | no | mitigated, with a confirmed elapsed-time bug |
| #77 | yes (`e0b034f`, `406e46f`) | no | no | **unproven; mechanism insufficient** |
| #78 | yes (`35e2e79`) | partly | no | plausible mitigation, unverified |

## Cross-issue regression risks

- **#77 ↔ #78:** both depend on the exit path. A crash after quit-to-title skips the HDR restore. Test the pair together.
- **#77 ↔ saves:** `stop()` saves settings before the window-mode change; if the crash happens earlier in `stop()`,
  settings may not be written. Verify a save survives quit-to-title.
- **#76 ↔ #75:** both touch the scan/decoder thread lifetimes (`OpusDecoderOwner`, `ScanCancelToken`). A cancel during a
  movie or a scan must not leave a decoder or a worker alive.
- **#75 ↔ packaging:** the Opus path needs the FFmpeg DLLs at runtime; they are shipped, but a user who deletes
  `upscalers/ffmpeg` silently falls back. `verify_install.py` will report it as a changed/missing release file, which is
  the correct behaviour: make sure support uses it.

## Evidence level of this report

- Source reading: complete for the files named.
- Executed: `py_compile`, the 30 release-tooling tests, `check_release.py`, `verify_engine_tree list`,
  `verify_archive` on the 0.1.41 zip.
- Not executed: any C++ build, any gameplay, any cutscene, any HDR display test, any quit-to-title reproduction.
