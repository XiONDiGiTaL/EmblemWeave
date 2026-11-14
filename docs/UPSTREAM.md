# Where the engine comes from

`engine/fefw-recomp` is not EmblemWeave's own code. It is **fefw-recomp**, the
static recompilation of *Fire Emblem: Fortune's Weave* by
[Goldoire](https://github.com/Goldoire/fefw-recomp) and its contributors, GPL-3.0,
used here as the baseline engine that the launcher sets up, builds and runs.
EmblemWeave is the launcher around it: game data handling, the recompilation
pipeline, mods, saves, cheats, the in-game menu and the interface.

## The handover

In November 2026 the author of fefw-recomp said they would not continue the
project and removed the repository from GitHub. **Obersteinn** had saved the last
release, `fefw-recomp-0.2.0-win64.zip`, with its build tools and documentation,
and shared it so the work would not be lost
([archive](https://drive.google.com/file/d/1U8_QkcL7yxbW8eUTwGywHsynLCOCshHu/view)).
That archive is the only copy of the 0.2.0 package we know of.

## What we keep and use

- **The engine** in `engine/fefw-recomp`: the last upstream commit we had
  (`501f7c0`, 9 November 2026) plus EmblemWeave's own changes, such as serving
  zstd-packed `.nup` entries from a decompressed cache. It is committed here as
  ordinary files rather than as a submodule, since a submodule would point at a
  repository that no longer exists: a fresh clone is complete and needs no fetch
  step, and the vendored tree keeps its own `.gitignore` so game data and build
  output stay out. The copy carries the files, not the upstream commit history,
  which went with the repository; `git log` of EmblemWeave records when and why
  it was brought in.
- **`apps/emblemweave/data/dump_hashes.json`**, copied unchanged from the 0.2.0
  package's `tools/build/dump_hashes.json`. Game Setup hashes the user's ExeFS
  modules against it and reports whether they are the build the engine was tested
  with — update 131072, `main` build `db93b0125f48…` — or a different one.
- **Its build notes.** The 0.2.0 first-run build (verify the dump against the
  manifest, write an uncompressed copy of the ZBIC `main`, lift and compile with
  a resumable stamp) is the reference the launcher's own pipeline and its
  Build the game checks follow.

## What we do not ship

The packaged `fefw-launcher.exe`, `fefw_rt.dll`, `renhdrum.dll`, the bundled
Python and Zig runtimes and the translation files stay out of this repository.
EmblemWeave has its own launcher and its own runtime; nothing from the package
is redistributed here, and the archive itself contains no game data.

## Credit

- **Goldoire** and everyone who contributed to fefw-recomp: the engine, its
  runtime, its build tooling, and the research behind them. The baseline this
  project stands on.
- **Obersteinn**: preserving the final 0.2.0 release and passing it on, which
  kept the reference build, its build scripts and its tested-dump manifest
  available after the original repository was removed.

Both are credited in the [README](../README.md#credits) and on the launcher's
About page.
