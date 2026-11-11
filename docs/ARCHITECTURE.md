# EmblemWeave architecture

## Principles

1. **Evidence over assumption.** Format layouts in this project were derived from the dump and then verified by construction (decode boundaries must land exactly where the header says).
2. **Loud failure.** Unsupported instructions and unimplemented services trap with an address and a reason. No silent no-ops, no fake successes.
3. **Two implementations of instruction semantics, and an outside judge.** The C emitter is the product; the interpreter exists to disagree with it when it is wrong. Because both share one decoder, a third, independent CPU model (Unicorn, development only) checks the interpreter instruction by instruction.
4. **The launcher is not the runtime.** They share a format layer and nothing else.

## Components

```
                +----------------------------------------------+
                |  apps/eblemweave   (launcher, egui/eframe)   |
                |  data detection, settings, diagnostics       |
                +---------------------+------------------------+
                                      |
                +---------------------v------------------------+
                |  crates/ew-boot                               |
                |  dispatcher, module image, imports, host calls|
                +---------------------+------------------------+
                                      |
                +---------------------v------------------------+
                |  crates/ew-runtime                            |
                |  guest memory bus, module plan, HLE registry  |
                +---------------------+------------------------+
                                      |
   +--------------------+   +---------v----------+   +---------------------+
   | crates/ew-format   |   | crates/ew-lift     |   | tools/              |
   | NSO0, ExeFS, romfs |-->| decoder, interpreter|  | zbic_extract.py     |
   | inventory, probes  |   | C emitter, CFG scan |  | (ZBIC bridge)       |
   +--------------------+   +--------------------+   +---------------------+
                                      |
                                      v
                       build/lift/app_lift.c + ew_aarch64.h
                       (compiled by the host C toolchain)
```

### `ew-format` containers and assets

* `nso`: the verified NSO0 header, per-segment compression detection, `.bss`, `MOD0` discovery.
* `exefs`: module discovery by magic (renamed dumps still work), NPDM parsing, liftability classification.
* `romfs`: metadata-only inventory and content probing that unwraps standard Zstandard to report the *inner* container.

### `ew-lift` static recompilation

* `aarch64`: decoder producing a typed `Op`. Unknown words become `Op::Unsupported`, never a guess.
* `scan`: discovery of dispatchable addresses (entry, branch targets, call targets, call return sites) and linear block lifting.
* `lift`: C emission, one statement per guest instruction, return-to-dispatcher at every control-flow edge.
* `interpreter`: reference semantics for differential testing.

Generated code shape:

```c
int ew_fn_00081468(Cpu *cpu)
{
    cpu->pc = 0x81468ULL;
    /* 0x81468: sub sp, sp, #0x40 */
    { ew_u64 r_ = (EW_R(cpu, 31, 1) - 0x40ULL); EW_W(cpu, 31, 1, r_); }
    ...
}
```

`x31` is context-dependent (XZR or SP); the `EW_R`/`EW_W` macros encode that rule, and the index expression clamps 31 so the unused branch of the ternary is never an out-of-bounds access.

### `ew-boot` the whole-program dispatcher

* `Module`: maps `text`, `rodata`, and `data` at the addresses the code expects,
  and derives the entry stub from the loader header at the start of `.text`.
* `imports`: reads `MOD0`, the dynamic array, the symbol table, and the jump slot
  relocations, which is how every external call gets a name.
* `hle`: the host service registry and the C memory and string routines. A
  handler either returns a value in `x0` or reports why it cannot serve the call.
* `services`: the single-threaded `nn::os` families (mutex, event, reader/writer
  lock, semaphore, page allocation, thread-local storage, thread queries). State
  lives in the caller's guest object with a magic tag, so no handler needs global
  state and two runs cannot see each other.
* `math`: the C math library, whose results are exact because IEEE 754 is.
* `cxx`: the libc++ entry points the module imports (`std::random_device`,
  the static-initialisation guards, `__hash_memory`, `__next_prime`, nothrow
  `operator new`).
* `pmr`: `std::pmr` with a real `new_delete_resource` object whose vtable
  forwards into the game's own `operator new` and `operator delete`.
* `threads` and `sched`: multi-threaded `nn::os` (owner-tracking mutexes,
  condition variables, events, timer events, thread lifecycle, sleep) and the
  round-robin scheduler that runs guest threads on one interpreter.
* `fs`: `nn::fs` over host directories. RomFS is mounted read-only; save data
  lives in a separate writable folder; paths cannot leave their mount.
* `lmem`: `nn::lmem` frame heaps, state kept in the caller's heap head.
* `system`: `nn::oe`, `nn::account`, `nn::time`, `nn::aoc` and `nn::diag`,
  answered from launcher settings and the title's `control.nacp`.
* `libc`: the musl C runtime: formatted I/O with AAPCS64 varargs, `sscanf`,
  `strto*`, stdio over `nn::fs`, pthreads on the scheduler, `rand`, `errno`.
* `math`: musl's libm (through the `libm` crate), arguments in `s`/`d`
  registers, results in `v0`.
* `boot`: the dispatch loop. It follows nnSdk's startup order
  (`nninitStartup`, module entry, `nnMain`), runs recovered functions, executes
  host services, switches threads, and stops with a typed reason.

The stop reason is the product of this crate. A run that ends with
`called std::__1::__hash_memory` has proved which service to build next.

A host service must state a contract that is true in the host's model. Mutexes
can be exact because exactly one guest thread exists; `CreateThread` cannot be,
so it is absent and stops the run by name instead of dropping work.

### `ew-runtime` host services

* `image::GuestMemory`: region-bounded guest bus. Reads/writes outside a mapped region are errors; `.text` is not writable.
* `hle::HleRegistry`: service table where every entry is `Implemented` or `Unimplemented(reason)`. The default set is drawn from symbol names actually present in the dump.
* `readiness`: single source of truth for "can this boot yet?", currently **no**.
  Its implemented entries mirror the handlers `ew-boot` registers, so the list
  cannot claim less than the runtime delivers.

### Blocking host services and the scheduler

A host handler is `fn(&mut State) -> HostResult`. Besides returning a value or
reporting that it cannot serve a call, it can:

* `Jump(addr)`: tail-call guest code with the registers it prepared, which is
  how a host vtable forwards to the game's own functions;
* `Block { resume, timed }`: park the calling thread. The scheduler saves the
  thread's registers and later calls `resume` with them loaded. Continuations
  keep private state in caller-saved registers, which the procedure call
  standard lets a callee clobber.

Each guest thread has its own register file, TLS table and exclusive monitor
(cleared on every switch, as an exception return does). Threads rotate every
200,000 instructions, when a call blocks, or when a function exhausts its
budget in a spin-wait. When every thread waits on something only another thread
could provide, the run stops as a deadlock that names each waiter.

### Verification layers

| Layer | Compares | Catches |
| --- | --- | --- |
| `tools/verify_cfg.sh` | interpreter vs compiled C, whole functions | emitter mistakes |
| `tools/verify_simd.sh` | interpreter vs compiled C, every vector encoding on edge-case inputs | vector-unit mistakes in either implementation |
| `tools/oracle_lockstep.py` | interpreter vs Unicorn, one instruction at a time from a real game snapshot | decoder mistakes, which the other two cannot see |

### Dispatcher contract

Lifted modules export `ew_func_entry <module>_functions[]` mapping guest addresses to functions. The dispatch loop is:

```
while !cpu->halted:
    fn = lookup(cpu->pc)
    if fn is None: halt("unmapped pc")
    fn(&cpu)
```

Each function returns `1` to continue dispatch and `0` after a trap, so a missing implementation stops the guest instead of corrupting state.

## The launcher

`apps/eblemweave` is organised by responsibility: `config` (versioned, atomic
settings), `theme` (palettes, type and spacing tokens), `widgets` (shared
components), `gamedata` (detection and validation), `jobs` (background work with
staged progress and cancellation) and one module per page under `pages/`. Long
operations never run on the interface thread.

## Planned platform layer

| Area | Plan |
| --- | --- |
| Graphics | NVN2 device emulation over **Vulkan** (primary) with D3D12 as a later option; BFSHA shader translation to SPIR-V; BFRES resource upload |
| Audio | `nn::audio::AudioOut` over WASAPI, Opus decode for the 108k `.opus` streams |
| Input | `nn::hid` mapped to SDL3/XInput with per-device profiles and glyph detection |
| Save data | `nn::fs` save mount backed by atomic files under the user's config directory |
| Timing | Guest tick derived from a monotonic host clock; render rate decoupled from simulation rate |

## Directory layout

```
crates/ew-format    containers, romfs, probes
crates/ew-lift      decoder, interpreter, C emitter, ew-lift CLI
crates/ew-boot      dispatcher, module image, import naming, host services
crates/ew-runtime   guest memory, HLE registry, readiness
apps/eblemweave     launcher
tools/              zbic_extract.py, verification scripts, assembler probes
docs/               feasibility, architecture, status, legal
```
