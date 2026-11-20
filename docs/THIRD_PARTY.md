# Code adapted from other projects

EmblemWeave is GPL-3.0-only. This file lists source code adapted from other free-software projects, with where it came from and
under which licence. Libraries used as dependencies are listed in [LEGAL.md](LEGAL.md).

## fefw-recomp

fefw-recomp by Goldoire and its contributors, GPL-3.0. A native PC port of Fire Emblem: Fortune's Weave by static
recompilation. Every file adapted from it carries a header naming the original file. Its repository has since been removed
from GitHub; Obersteinn preserved the final 0.2.0 release, and [UPSTREAM.md](UPSTREAM.md) records the handover.

It is included in this repository at `engine/fefw-recomp` and is EmblemWeave's game engine: its recompiler, runtime,
renderer, audio and shader translator build and run the game. The EmblemWeave launcher drives it.

| EmblemWeave file | Adapted from |
| --- | --- |
| `engine/fefw-recomp/` (vendored) | the repository at commit 501f7c0, plus EmblemWeave's own changes (zstd `.nup` entries served from a decompressed cache) |
| `apps/emblemweave/data/dump_hashes.json` | `tools/build/dump_hashes.json` in the 0.2.0 release package, unchanged: the module hashes of the build the engine was tested with |

## NXFormat

NXFormat by fab918 and its contributors, MIT. The NX container format (`.nx`) and its reference implementation, including
a header-only C++17 reader and a Rust packer. `docs/SPEC.md` in that repository is the definition of the wire format, and
the fixtures under `crates/ew-format/tests/nx-fixtures/` are its conformance fixtures, copied unchanged to test
EmblemWeave's reader.

| EmblemWeave file | Adapted from |
| --- | --- |
| `crates/ew-format/src/nx.rs` | an independent reader written from NXFormat's `docs/SPEC.md` |
| `crates/ew-format/tests/nx-fixtures/*.nx` | `tests/fixtures/` from NXFormat V1.0.3 and V1.0.4, unmodified (the 0.3 wire-format fixture is `source_resource_v103.nx`) |
| `engine/fefw-recomp/runtime/src/loader/nx.cpp` | the same reader in the engine, written from that specification |

## ObeliskChamber

ObeliskChamber by xinthefreefallxai, GPL-3.0, is another fork based on fefw-recomp with updates beyond 0.2.0. We are in
full support of others extending the life of the original engine beyond its development. Its import-freeze write-watch
race fix (ObeliskChamber issue 81, fixed in PR 93) is reimplemented in EmblemWeave's write-watch handler.

| EmblemWeave file | Adapted from |
| --- | --- |
| `engine/fefw-recomp/runtime/src/mem/write_watch.cpp` | the late heap-write retry for a page released while an import freeze was ending, reimplemented for EmblemWeave's handler |

## MAKO

MAKO by Eugenio Segala and contributors, GPL-3.0-or-later. Its single-pass spatial scaler (`spatial_scaling.comp`) is
adapted as EmblemWeave's optional `mako` upscaler, a sharpened-cubic reconstruction with contrast-aware anti-ringing and
bounded local sharpening. The engine's version is a Vulkan compute port that shares the runtime's HDR pre-scaling and
output-pass plumbing; it is not MAKO's frame-generation or layer code.

| EmblemWeave file | Adapted from |
| --- | --- |
| `engine/fefw-recomp/runtime/src/gfx/extras/mako_spatial.cpp`, `mako_spatial.h` | `engine/mako-render/src/shaders/spatial_scaling.comp` at MAKO commit `504611d42b73d33320456911e9bf92215dbb8cbf`, restructured as an nxrt `SpatialUpscaler` |
| `engine/fefw-recomp/runtime/scripts/gen_mako_shaders.py` | the adapted shader source and the build/validation convention of `gen_nis_shaders.py`; it writes `src/gfx/extras/mako_shaders.inc` |

MAKO is GPL-3.0-or-later; the adapted files are used under GPL-3.0, whose text is the repository-root `LICENSE`. Source:
https://github.com/eugeniosegala/MAKO, Copyright (C) 2025-2026 Eugenio Segala and contributors. The adaptation keeps the
algorithm's structure (clamped Catmull-Rom weights, anti-ringing, local sharpening) and changes the buffer and push-constant
layout to nxrt's.

## Eglantine

Eglantine by the Eglantine Community, GPL-3.0-or-later. Its renderer, texture-cache and upscaler work was reviewed for
portable findings. No source code is copied. Its Fire Emblem-specific DLSS finding, that NVIDIA's default Performance and
Ultra Performance presets draw thin horizontal lines at 2× scale while preset K does not, is why `[dlss] preset` now
defaults to `k` in EmblemWeave.
