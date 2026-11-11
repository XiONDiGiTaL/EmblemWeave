# Code adapted from other projects

EmblemWeave is GPL-3.0-only. This file lists source code adapted from other free-software projects, with where it came from and
under which licence. Libraries used as dependencies are listed in [LEGAL.md](LEGAL.md).

## fefw-recomp

[Goldoire/fefw-recomp](https://github.com/Goldoire/fefw-recomp), GPL-3.0. A native PC port of Fire Emblem: Fortune's Weave by static
recompilation. Every file adapted from it carries a header naming the original file.

It is included whole as a git submodule at `engine/fefw-recomp`, pinned to a known commit, and is EmblemWeave's game engine: its
recompiler, runtime, renderer, audio and shader translator build and run the game. The EmblemWeave launcher drives it.

| EmblemWeave file | Adapted from |
| --- | --- |
| `engine/fefw-recomp/` (submodule, unmodified) | the whole repository at commit 501f7c0 |
