# External compatibility audit

Evidence from RecompLauncher user reports for EmblemWeave plus a code-level investigation of the startup and
shader-loading paths. This document separates what users reported, what the code shows, and what could not be tested.

## Evidence basis

The RecompLauncher page (`https://recomplauncher.com/game/emblemweave`) serves its review list from client-side code: a
static fetch returns only the page shell (title, description, "no game files here" notice). The configurations and
failures below are therefore taken from the report inventory supplied for this audit, not re-scraped. They are
**user-reported**, not independently reproduced.

No Steam Deck, Intel Arc, or Ryzen Z1 machine was available. Nothing in this document is a hardware test result.

## Compatibility matrix

| Configuration | GPU | OS | Version | Reported state | Confidence |
| --- | --- | --- | --- | --- | --- |
| Ryzen 7 5700X3D | RTX 3090 | Windows | not reported | 60 FPS at 1440p | user-reported working |
| Ryzen 7 5700X | RTX 4060 | Windows | not reported | working | user-reported working |
| Ryzen 7 9800X3D | RTX 5090 | Windows | not reported | working | user-reported working |
| Ryzen 7 9800X3D | RTX 5080 | Windows | not reported | working | user-reported working |
| Ryzen 9 5950X | RTX 5070 Ti | Windows | not reported | working | user-reported working |
| Core i5-12400F | RTX 3060 | Windows | not reported | working | user-reported working |
| Core i7-12700K | RTX 3070 | Windows | not reported | working | user-reported working |
| Ryzen AI Z2 Extreme | RTX 5090 Laptop | Windows | not reported | working | user-reported working |
| Ryzen Z1 Extreme | Radeon integrated | Windows | 0.1.37 | runs; frame drops while shaders load | user-reported degraded |
| Steam Deck (RDNA 2, Zen 2 4c/8t, 16 GB) | integrated | SteamOS/Proton | 0.1.40 | **crashes on startup** | user-reported failure |
| Core Ultra 5 245K, 32 GB | Intel Arc B390 | Windows | 0.1.36 | **crashes on startup** | user-reported failure |
| Unspecified | not reported | Windows 10/11 | not reported | build/install friction; manual source edits to build | user-reported friction |

What this does and does not establish:

- NVIDIA discrete GPUs on Windows are the well-attested path (8 configurations, including laptop and high-end parts).
- AMD integrated (Z1 Extreme) runs but stutters during shader loading; it is not a startup failure.
- Two startup failures are **unexplained**, not "unsupported". Neither report in the inventory includes a log.
- No report confirms Linux, macOS, Intel discrete success, or AMD discrete beyond the maintainer's RX 7900 XT.

Do not convert "eight configurations work" into "Windows is supported": every working report is an NVIDIA discrete GPU.

## Startup failures

### Steam Deck / Proton (v0.1.40)

The repository's own position is in `docs/STEAMDECK.md`: EmblemWeave is a Windows program, the Deck runs it under
Proton, **the build cannot be done on the Deck** (it needs winget, PowerShell 7 and MSVC), and "this page has not been run
on a Deck by the maintainer". The documented route is: build on Windows, copy the whole folder including
`game-build/fefw/build/release/fefw.exe`, translated shaders and the play folder.

That means a Deck crash on startup is a *Proton execution* failure of a Windows build, not evidence that native Linux
support is broken (there is none). Candidate causes, in the order worth checking:

1. **A copy that is missing build output or shaders.** The guide's copy list is long and easy to get wrong; a missing
   `fefw.exe`, `fefw.map` or translated shader corpus produces a startup failure before any rendering.
   `nxrt.log` would show `runtime: host symbols: … from the linker map …` and then the boot line.
2. **Memory.** The runtime reserves `default heap 12288 MiB` (`mem: init` in `nxrt.log`). On a 16 GB Deck the GPU shares
   the same pool, so a 12 GiB heap is a plausible commit failure under Proton. `[memory] heap_section = auto` already
   switches to private memory under Wine because a shared section produced a black picture (`runtime/docs/CONFIG.md`),
   which shows this path has been adapted before.
3. **Missing Vulkan/VC++ runtime inside the prefix.** Proton supplies both, but a hand-copied folder launched outside
   Steam does not.
4. **Shader translator selection.** `tools/shaders/prebuilt/win-x64/spirv_harness.exe` is the Windows binary and is used
   under Wine/Proton by design. A Linux `spirv_harness` placed in `prebuilt/linux-x64` is *not* what a Proton run looks
   for.

None of these is confirmed. The decisive artifact is the Deck's `nxrt.log`, `fefw_fatal.txt` and the Proton version;
until one is attached to the report, "Steam Deck crashes" cannot be turned into a defect.

### Intel Arc B390 (v0.1.36)

The runtime has explicit Intel handling, so this is not a never-seen vendor:

- `vk_renderer.cpp:179-185`: `present_guard_applies()` returns true only for `vendorID == 0x8086` **and**
  `deviceType != DISCRETE_GPU`. An Arc B390 is discrete, so the present guard does **not** apply to it.
- `vk_device.cpp:1078`: `q.igpu_compat = quirk_key("igpu_compat", q.intel)` with the comment "#5, #18; Arc A770 and
  B580 too (#45)". Intel compatibility quirks are known to be needed on Arc, and the code says so.
- `vk_device.cpp:1071-1077`: Intel gets `budget_detail` and `pressure_detail` because "Intel's driver hands out about
  80%" of the heap once the OS budget is reached.

So Arc is a supported-with-quirks vendor, and a B390 startup crash is more likely a driver-version or
feature-enumeration problem than a missing code path. The report is against **0.1.36**, which predates the current
0.1.41 and several Intel changes. Candidate causes: a driver too old for the required Vulkan feature set, a
`VK_EXT_full_screen_exclusive`/present-mode path, or a device-creation failure whose message is only in the log.

The actionable request is the same: `nxrt.log` up to the failure, `fefw_fatal.txt`, GPU driver version, and whether
0.1.41 still fails. Without that, the report cannot be distinguished from an old build.

### Diagnostic gap (both cases)

`fefw_fatal.txt` and `nxrt.log` are written, and the launcher has a Troubleshooter that packages a report (issues #34,
#43 show the flow working). The RecompLauncher reports contain neither. The single highest-value change for external
support is not a code fix: it is asking reporters for the Troubleshooter zip, and making the crash path write a
one-line reason to `fefw_fatal.txt` even when the Vulkan device cannot be created.

## Shader-loading performance (Ryzen Z1 Extreme, v0.1.37)

The report is frame drops *during shader loading*, on an AMD integrated GPU. The runtime already has the machinery to
explain this:

- **Pipeline cache.** `nvn_pipeline.cpp` persists a `VkPipelineCache` to `pipeline_cache.bin`
  (`[nvn] pipeline_cache`), keeps `pipeline_cache.bin`/`pipeline_keys.bin` between runs, and saves on the prepare step
  (`g_prepare_cache` → `pipeline_cache_save()`). A first run compiles; later runs hit the cache. The repository root
  currently holds a 23 MB `pipeline_cache.bin` and a 170 KB `pipeline_keys.bin`, which is consistent with that design.
- **Cache serialisation is a known hazard.** `nvn_pipeline.cpp:927` notes a draw's link must not wait on the cache lock
  while a saver thread serialises "hundreds of MB", and `pipeline_cache_locked()` exists because of Intel (#124). That
  lock is **Intel-only by default**; AMD integrated GPUs do not get it.
- **Memory quirks are Intel-only.** `q.budget_detail = quirk_key("budget_detail", q.intel)` and
  `q.pressure_detail = quirk_key("pressure_detail", q.intel)` mean an AMD iGPU, where the game and the compositor share
  one 16 GB pool, gets neither the OS budget detail nor the pressure handling that Intel iGPUs get. On a Z1 Extreme
  with the runtime reserving a 12 GiB heap, memory pressure during pipeline creation is a credible cause of hitches
  that look like shader-loading stalls.
- **Hitch logging already exists.** `nxrt.log` shows `diag: hitch log: frame summary every 10 s, frames over 50 ms
  logged (at most one line per second)`. That is exactly the instrument needed to separate first-run compilation from
  steady-state stutter.

Assessment: the reported behaviour is **most likely first-run pipeline compilation plus memory pressure**, not an
unavoidable architectural limit, but it cannot be confirmed without the hitch log. Average FPS is the wrong metric here;
the question is the p99 frame time and whether it improves on the second run with a warm `pipeline_cache.bin`.

Recommended investigations, in order:

1. Ask for two runs: cold cache and warm cache, with the hitch log. If the hitches vanish on run 2, it is compilation.
2. If they persist, check whether `pipeline_cache_save()` is serialising a large cache on the render thread during
   gameplay, and whether the save can be deferred to a non-rendering moment.
3. Consider extending `budget_detail`/`pressure_detail` to AMD integrated GPUs (`deviceType == INTEGRATED_GPU`), not just
   `vendorID == 0x8086`, and reducing the default heap on low-memory unified systems.
4. Do not promise a fix from a single report; a frame-time trace is required.

## Build and installation friction

The first-run experience still requires Visual Studio C++ tools, LLVM, CMake, Ninja and Python, and the game build
itself. Reports in the inventory say users had to edit code to build, and that installing every program on the host is
not user-friendly.

Repository evidence:

- `docs/PORTABLE_INSTALLATION.md` states the requirement plainly and says the launcher "checks them and offers install
  commands where it can". The shader translator ships self-contained, so the .NET SDK is no longer needed, which is a real
  improvement over earlier releases.
- `tools/install_local.ps1` is *not* a provisioning tool: it requires an existing install folder (`if (-not (Test-Path
  -LiteralPath $InstallDir -PathType Container)) { throw 'no install folder at …' }`) and installs the launcher into it.
  Toolchain detection lives in the Rust launcher, not in this script.
- Issue #43 is the clearest friction data point: a stale `tools\.venv` with a `python311.dll` survived an update and the
  build failed with `ImportError: Module use of python311.dll conflicts with this version of Python`. The maintainer's
  answer confirms the cause (the build "previously skipped the environment just because that file existed"). That is a
  recovery defect in the toolchain layer, not a documentation problem.
- Issue #34 shows the same shape: a build that compiles but fails to launch, resolved by a fresh extraction rather than
  an in-place repair, evidence that mixed installs are common.

Assessment: the packaging and verification work in `20396fc` materially improves *diagnosis* (a manifest, an install
verifier, documented recovery) but does not reduce the *number of tools* a user must install. The friction is real and
will persist until toolchain provisioning is automated or a prebuilt runtime is offered.

## Prioritised recommendations

| Priority | Action | Why |
| --- | --- | --- |
| 1 | Collect Troubleshooter logs for the Deck and Arc B390 reports; retest both on 0.1.41+ | Two startup failures are unexplained; every other conclusion is speculation until logs exist |
| 2 | Make device-creation failure write a one-line reason to `fefw_fatal.txt` | Turns "crashes on startup" into an actionable defect |
| 3 | Request cold/warm hitch logs for the Z1 Extreme report | Distinguishes compilation cost from a persistent stutter |
| 4 | Extend memory budget/pressure quirks beyond Intel iGPUs; review the 12 GiB default heap | Unified-memory systems are the weakest attested configuration |
| 5 | Add a real launch smoke test to the release pipeline | Prevents shipping a build that cannot start at all |
| 6 | Automate toolchain provisioning, or ship a prebuilt runtime | The largest remaining gap between "works for developers" and "works for a new user" |

## Limitations

- No hardware testing was performed.
- The RecompLauncher review list could not be read directly (client-rendered page); the report inventory is second-hand.
- Version numbers in the reports (0.1.36, 0.1.37, 0.1.40) predate the current 0.1.41 in several cases, so a failure may
  already be fixed, or may have regressed. Neither can be assumed.
