# Key capabilities

The deeper technical capabilities behind the feature list.

- **Static recompiler** for AArch64: integer, floating point, Advanced SIMD,
  atomics and exclusives, with an independent reference interpreter for every
  instruction.
- **Three layers of verification:** the emitted C against the interpreter, the
  vector unit against seeded edge cases (NaNs, infinities, signed zeros), and the
  interpreter against an independent CPU model.
- **Guest thread scheduler** with real blocking waits: mutexes, condition
  variables, events, timers, joins and sleeps, plus deadlock detection that names
  every waiting thread.
- **Native platform services:** the SDK's startup sequence, heap and frame-heap
  allocators, a read-only RomFS filesystem with a separate save directory, the
  musl C library (formatted I/O, parsing, a bit-exact math library), libc++ runtime
  pieces, and system settings such as language and profile.
- **Native execution:** the recompiled code compiled ahead of time into one
  library, entered at any block, checked call by call against the interpreter
  (`EW_NATIVE_CHECK=1`).
- **Its own presenter:** a Win32 window, a Vulkan surface and swap chain, and the
  game's swap texture untiled per frame and presented, independent of the
  baseline engine.
- **A complete launcher** with game-data detection, save profiles and editing,
  mod management and cheats, an in-game enhancements menu, a staged
  recompilation pipeline, diagnostics with search and export, and six colour
  schemes, all built from one set of liquid-glass controls that take their
  colour and glow from the scheme.
