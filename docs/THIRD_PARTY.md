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
