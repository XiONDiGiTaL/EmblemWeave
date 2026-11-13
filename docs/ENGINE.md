# EmblemWeave's own engine

EmblemWeave plays the game today through [fefw-recomp](https://github.com/Goldoire/fefw-recomp). The goal is to replace it with
EmblemWeave's own runtime, in Rust, built on the recompiler in `crates/ew-lift` and the host services in `crates/ew-boot`, so the
whole path from your dump to a running game is EmblemWeave's. fefw-recomp stays the reference while that happens: where it has
already worked something out (how a service behaves, how NVN maps to Vulkan), EmblemWeave reads its code and docs, which its
GPL-3.0 licence allows, and writes its own version.

The order is set by what blocks playing, not by what is most visible.

## 1. Native execution (in progress)

The runtime ran the game's code on the reference interpreter, roughly a hundred times too slow to play. The game's code is now
compiled ahead of time:

* `ew-native --module work/decoded/main --out build/native` lifts the whole main module, emits it as C in shards, compiles them
  in parallel with clang and links `ew_native.dll`.
* `ew-run --native build/native/ew_native.dll` runs every function the library has natively and interprets the rest. A native
  function can be entered at any of its blocks, so calls return into native code.
* Guest memory is read and written directly; every block charges its instructions to a budget, so guest threads are still
  rotated by the scheduler; supervisor calls, the system counter, exclusive monitors and halts behave as in the interpreter.
* `EW_NATIVE_CHECK=1` runs each native call on the side and compares its registers and stores with the interpreter, narrowing a
  difference down to the first block where the two part. It found the first native bug (a function entered mid-way restarted
  at its entry) and now passes the boot's first 300 million instructions.

Where it stands (10 November 2026, RX 7900 XT PC, 16 threads, 32 GB):

* The whole main module builds: 807,604 functions, 10.6 million instructions, 530 C files, compiled in about 16 minutes
  (8 jobs at below-normal priority, bounded by memory) into a 757 MB library; later builds only recompile files that changed.
* `EW_NATIVE_CHECK=1` finds no difference from the interpreter over the boot's first billion instructions. On the way it
  found two bugs in the generated code: functions restarting at their entry, and the LSE atomics (`ldadd` and friends)
  storing and returning the wrong values.
* A billion boot instructions take 12 s native against 22 s interpreted. Native functions call each other directly; what is
  left is the run loop's work around host calls: hot loops in the game call host functions (a shuffle calls the C
  library's random number generator every iteration), and every such call goes through the full run loop.

Next: serve simple host calls from inside native code without the full run loop (keeping the scheduler's view of new
threads correct); compile the SDK module too (it needs the lifter to work at the module's load address); `-O2` for hot
code.

## 2. Frame loop at speed

With native code, run the game headless to the title screen and into play, fixing the services it reaches (the audio
renderer's voice release is the last known blocker), as the interpreter runs did, but in minutes rather than hours.

## 3. Graphics

The game draws with NVN. EmblemWeave already tracks NVN's devices, memory pools, textures, programs, state and command buffers
(`nvn_*.rs`). Next:

1. ~~A Vulkan device and window (the `ew-gfx` crate) presenting the game's colour buffers.~~ **Window and presentation are in.** `ew-gfx::present` owns a
   Windows window, its Vulkan surface and swap chain; the NVN layer untiles the presented swap texture straight out of its block-linear storage (RGBA8,
   BGRA8 and RGB10A2 so far) and uploads it on every present, scaled to the window. `ew-run --window` turns it on. What is on screen is still whatever
   the game last left in that texture, because clears and draws are not executed yet: that is the next step, and the first frame that is the game's own.
2. Textures: the Switch's block-linear layouts and its compressed formats, uploaded to Vulkan.
3. Shaders: the game's Maxwell/Ampere shader binaries translated to SPIR-V. This is the largest single piece; fefw-recomp uses
   Ryujinx's translator (MIT) offline, and EmblemWeave can do the same as a build step before writing its own.
4. Draw calls, render targets, blending, depth and the rest of the pipeline state.

## 4. Audio and input

The audio renderer already runs the game's voices; its mixed output goes to the sound device. Keyboard and controller input go
to the game's HID state (`hid.rs`), with the bindings from the launcher's Controls page.

## 5. Launcher

When the runtime can play, the launcher's **Play** starts it instead of fefw-recomp, and **Build the game** builds
`ew_native.dll` and the shader cache. Saves and mods carry over as they are (the save layout and the mods folder are shared).
Cheats need one change: fefw-recomp maps the game's memory at the game's own addresses, while EmblemWeave's runtime holds it in
one host block, so the runtime will publish where that block is and the cheat engine will add the offset.
