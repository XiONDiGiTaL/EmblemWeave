# EmblemWeave technical feasibility report

**Target:** *Fire Emblem: Fortune's Weave* (Nintendo Switch 2), Title ID `04007EF01F8AE000`, v131072
**Goal:** a native Windows x64 build that executes the game's own code, built by [XiØN](https://github.com/XiONDiGiTaL)
**Report date:** 8 November 2026
**Status of this report:** every claim below was measured against the user's own dump. Numbers are reproducible with the tools in this repository.

---

## 1. What the dump actually is

The supplied package is **not** a Switch 1 title. Three independent facts establish this:

| Evidence | Observation |
| --- | --- |
| Package magic | `NUP\x01` at offset 0, the Nintendo Switch 2 package container |
| Title ID | `04007EF0...`, where the `0x0400` application prefix is reserved for Switch 2 titles (Switch 1 uses `0x0100`) |
| Executable format | NSO0 modules with 64 MiB+ rodata and Switch 2 process metadata, segmented differently from Switch 1 |

The `NUP` is an encrypted retail package and was **not** usable directly. A decrypted view of the content already existed on this machine at
`E:\Switch2\Games\.nup-convert-h_e2mfw5\.nup-nsp-f_aicmw_\`, containing a decrypted PFS0 (NSP) and an extracted ExeFS/romfs. EmblemWeave consumes that extracted material and never touches, modifies, or redistributes the original package.

### Executable modules (measured, `ew-inspect`)

| ExeFS file | Module name | File size | Image size | Flags | Compression | Status |
| --- | --- | --- | --- | --- | --- | --- |
| `main` | `app` | 11.5 MiB | 34.7 MiB | `0xbf` | **ZBIC** | decoded (see §2.2) |
| `subsdk0` | `dlss` | 40.4 MiB | 43.6 MiB | `0x3f` | LZ4 | decoded + verified |
| `subsdk1` | `cuda` | 6.0 MiB | 20.3 MiB | `0x3f` | LZ4 | decoded + verified |
| `subsdk2` | `movie` | 520.9 KiB | 890.9 KiB | `0x3f` | LZ4 | decoded + verified |
| `rtld` | `nnrtld` | 10.3 KiB | 124.3 KiB | `0x3f` | LZ4 | decoded + verified |
| `sdk` | `nnSdk` | 7.5 MiB | 16.4 MiB | `0x3f` | LZ4 | decoded + verified |

`main.npdm` reports process category **Application**, confirming `main` is the game program. The game's own logic therefore lives in `app`: **27,546,352 bytes of AArch64 text**, 5,532,760 bytes of rodata and 2,265,808 bytes of data, plus `0x109d30` of bss.

## 2. Format findings

### 2.1 NSO0 segment layout

The Switch 1 header layout does **not** describe these files. The layout below was derived empirically and then confirmed by construction: an independent LZ4 decode of every segment reproduces the header's declared boundaries exactly, and the final segment ends precisely at end-of-file for all five LZ4 modules.

```
0x00  "NSO0"                 0x20  rodata.file_offset
0x04  u32 version            0x24  rodata.mem_offset
0x08  u32 reserved           0x28  rodata.size
0x0C  u32 flags              0x2C  u32 name_len
0x10  u32 text.file_offset   0x30  data.file_offset
0x14  u32 text.mem_offset    0x34  data.mem_offset
0x18  u32 text.size          0x38  data.size
0x1C  u32 header_size        0x3C  u32 bss_size
                             0x40  u8 module_id[0x20]
module name at (text.file_offset - name_len), NUL terminated
```

Flag bits `0..2` mark the text/rodata/data segments as compressed, `3..5` mark segment hashes, and **bit 7 selects ZBIC instead of LZ4**.

Verification (`rtld`, the smallest module):

| Segment | Declared size | Stored bytes consumed | Ends at | Header next-offset |
| --- | --- | --- | --- | --- |
| text | `0x2740` | `0x1f67` | `0x206e` | `0x206e` ✓ |
| rodata | `0x9e4` | `0x6eb` | `0x2759` | `0x2759` ✓ |
| data | `0xc008` | `0x1bb` | `0x2914` | EOF `0x2914` ✓ |

The decoded `rtld` rodata yields real symbols, e.g. `_ZN2nn4init5StartEmmPFvvES2_S2_` and `nnrtld.nss`, so the decode is not merely plausible-looking bytes.

### 2.2 ZBIC: Nintendo's modified Zstandard

`app` uses compression flag bit 7. Its text segment begins with the ASCII magic `ZBIC` where a standard Zstandard frame would carry `28 B5 2F FD`. The frame header that follows declares a content size of `0x01a452f0` = 27,546,352, which matches the header's text size exactly.

Public documentation and implementations agree that ZBIC is otherwise Zstandard (1.5.7) with `FSE_readNCount` replaced by a Binary Interpolative Coding reader. Because a decoder is publicly available (`kinnay/zbic`, Atmosphère, `alula/beetool`, and the Ghidra Switch Loader), the game module **is** recoverable today:

```
main: module='app' flags=0xbf zbic=True bss=0x109d30
  text   stored=  9917392 decoded= 27546352 (declared  27546352) OK
  rodata stored=  1809272 decoded=  5532760 (declared   5532760) OK
  data   stored=   298704 decoded=  2265808 (declared   2265808) OK
```

Every decoded size matches the declaration, so `tools/zbic_extract.py` is a trustworthy bridge until a Rust ZBIC decoder replaces it (see §6).

### 2.3 romfs

252,686 files totalling **27.0 GiB** across 17 top-level directories, dominated by audio:

| Extension | Files | Likely content |
| --- | --- | --- |
| `.opus` | 108,201 | voice/music streams |
| `.lips` | 107,436 | lip-sync data |
| `.zst` | 34,122 | standard Zstandard-wrapped assets (`.bfres`, `.bfsha`, `.msbt`, …) |
| `.matanimb` | 604 | material animation |
| `.webm` | 82 | movies |
| `.msbt.zst` | 6,289 (`.zst` total) | dialogue/message tables |

`Shaders/*.bfsha.zst` and `Graphics/**/*.bfres.zst` confirm the expected NintendoWare pipeline: BFRES models, BFSHA shader archives, BAKE audio, MSBT text. `rodata` in `app` contains `Emblem`/`Fortune` strings and NintendoSDK symbols, further confirming identity.

## 3. Is static recompilation the right approach?

| Option | Assessment |
| --- | --- |
| **A. Static recompilation (chosen)** | The only route that yields a native executable with no original binary at run time. Requires an AArch64→C lifter plus a Horizon/NVN2 runtime. |
| **B. Full decompilation to source** | Not tractable: 27.5 MB of text with stripped symbols and heavy inlining. Viable only for small, hot subsystems later. |
| **C. Hybrid** | **Recommended end state.** Static recompilation for bulk code, hand-written native implementations for the platform layer (graphics, audio, input, filesystem), and reconstructed high-level code where behaviour must be cleanly reimplemented. |

Frameworks examined: N64Recomp and XenonRecomp target different ISAs (MIPS/PPC) and cannot be reused; McSema/Remill lift to LLVM IR but supply no Switch loader, no service layer and no graphics; Ghidra is the right tool for *analysis*, not for producing runnable translations. Public Switch recompilation projects (`ssbu-recomp`, `smo-recomp`, `dkb-pc-recomp`) are title-specific and confirm the shape of the work: lifter + per-title runtime + shader translation.

**Recommendation: Option C**, with the lifter and runtime built here as the reusable core.

## 4. Verified progress

| Milestone | Result |
| --- | --- |
| Container parsing | All 6 modules parsed; segment boundaries verified to EOF |
| Code extraction | 6/6 modules decoded (app via ZBIC bridge), sizes match declarations |
| Decoder coverage | **86.18 %** of `app` text decodes to supported instructions (6,886,588 words; 951,908 unsupported) |
| Lifting | 1,352 register-only blocks (2,783 instructions) lifted with 0 unsupported instructions in the lifted set |
| Emitted C compiles | `zig cc -std=c99 -Wall`: clean build of `app_lift.c` |
| Execution | Lifted guest `0x81468` executed natively; results **bit-identical** to the independent interpreter (x5=`cf1bbcdcbfa56300`, x6=`cf1bbcdcbf9b0000`, x7=`cf1bbcdcbb000000`, pc=`0x814dc`) |
| Memory/HLE layer | Range-checked guest bus and explicit HLE registry, 9 tests |

## 5. Major obstacles (honest list)

1. **CFG recovery.** Blocks currently end at the first control-flow instruction; conditional branches mark a block partial and the runtime halts instead of guessing. Jump tables and indirect branches are the next lift milestone.
2. **14 % undecoded text.** SIMD/FP, bitfield-extract and system instructions are not yet modelled. Each becomes either an emitter case or a documented trap.
3. **No service layer.** Nothing in `nn::os`, `nn::mem`, `nn::fs`, `nn::hid`, `nn::audio`, `nn::vi` or `nn::nvn` is implemented. Nine blocked services are listed in the runtime registry, each with a reason.
4. **NVN2 graphics.** The renderer needs Switch 2 (Ampere-class) shader translation to SPIR-V. Public work exists but is incomplete; mesh shaders, VRS and sparse virtual textures are open problems.
5. **Threading and scheduling.** The guest's synchronisation primitives must be mapped onto the host without changing observable ordering.
6. **No C toolchain on this machine by default.** Verification used `zig cc` from a Python wheel. Release engineering needs a pinned LLVM/MSVC toolchain.

## 6. Milestones ahead

| Phase | Work | Exit criterion |
| --- | --- | --- |
| 3a | Port ZBIC decoding to Rust (remove Python bridge) | `ew-format` decodes `app` with no external tool |
| 3b | CFG recovery: basic blocks, conditional successors, jump tables | ≥95 % of text reachable; blocks no longer partial |
| 3c | Widen instruction coverage (SIMD/FP, bitfield, system) | ≥99 % decodable |
| 3d | `nn::os`/`nn::mem` HLE with a guest heap and threads | `app` reaches its own initialisation and reports a clean failure at the first unimplemented service |
| 4 | NVN2 device + shader translation to Vulkan | First clear colour rendered from the game's own init path |
| 5 | Filesystem, audio, input HLE | First boot to title screen |
| 6 | Gameplay validation matrix | Save/load, battle, cutscene, transition parity |
| 7 | PC enhancements (widescreen, frame pacing, remapping) | Feature-complete settings surface |

## 7. Legal position

EmblemWeave ships **no** Nintendo code, assets, keys or firmware. It requires the user's own dump and never modifies it. See [LEGAL.md](LEGAL.md).
