# Legal and distribution position

## Independent project

EmblemWeave is an independent [XiØN](https://github.com/XiONDiGiTaL) project. It is **not** affiliated with, endorsed by, or sponsored by Nintendo, Intelligent Systems, or any of their subsidiaries. "Fire Emblem" and "Fortune's Weave" are trademarks of their respective owners and are used here only to identify the software this tool interoperates with.

## Bring your own game data

EmblemWeave ships no Nintendo content. It contains:

* no game executables, NSO modules or NCA/NSP/NUP containers,
* no textures, models, shaders, audio, movies or fonts from the game,
* no encryption keys, tickets, certificates or firmware,
* no links to download any of the above.

The application consumes an extracted dump that the user has produced from a copy they legally own. Every tool in this repository is read-only with respect to that dump: `ew-inspect`, `ew-lift` and the runtime never modify game files. Decoded output (`work/decoded/…`, `build/lift/…`) is derived from the user's own data and must not be redistributed; `work/` and `build/` are excluded from version control.

## Analysis of the supplied package

The supplied `.nup` package is encrypted retail content. This project consumed an already-decrypted, already-extracted view present on the machine (ExeFS + romfs). No circumvention of the package's protection was performed by this project, and no keys are stored or embedded here.

## Licence

EmblemWeave is licensed under the GNU General Public License, version 3 only (`GPL-3.0-only`). It was MIT-licensed until
9 November 2026; it moved to the GPL so that code from [fefw-recomp](https://github.com/Goldoire/fefw-recomp) (GPL-3.0) can be
adapted into it. Adapted files keep fefw-recomp's attribution, and [THIRD_PARTY.md](THIRD_PARTY.md) lists them.

## Third-party software

| Component | License | Use |
| --- | --- | --- |
| `lz4_flex` | MIT | LZ4 segment decompression |
| `ruzstd` | MIT/Apache-2.0 | standard Zstandard asset probing |
| `serde`, `serde_json` | MIT/Apache-2.0 | reports and configuration |
| `anyhow` | MIT/Apache-2.0 | error propagation |
| `eframe`/`egui` | MIT/Apache-2.0 | launcher UI |
| `libm` | MIT/Apache-2.0 | the C math library, a port of musl's libm, so results match the console's |
| `rfd` | MIT | native folder and file dialogs in the launcher |
| `winresource` | MIT | build-time embedding of the launcher icon and version information |
| Unicorn | GPL-2.0 | development-only CPU oracle (`tools/oracle_lockstep.py`), used from Python as a separate process; not linked, not shipped |
| `zstandard`, Pillow | BSD-3-Clause / HPND | development-only helpers for checking assets and generating the icon; not shipped |
| `zbic` (kinnay/zbic) | **GPL-2.0** | development-only ZBIC bridge, invoked as a separate process by `tools/zbic_extract.py`; not linked, not vendored, not shipped |
| Ghidra | Apache-2.0 | interactive analysis (not redistributed) |

`zbic` is used out-of-process and is not a dependency of any shipped binary. Because it is GPL-2.0, an in-tree port must be written from the format description rather than copied, and the resulting licensing decision recorded before any port lands.

## Trademarks and presentation

The launcher's visual language is inspired by the game's tactical-fantasy aesthetic and by [XiØN](https://github.com/XiONDiGiTaL)'s own identity. It uses no Nintendo artwork and no official Fire Emblem logo. The EmblemWeave logo is original [XiØN](https://github.com/XiONDiGiTaL) artwork: `assets/branding/logo-fe.png` is the render cut out of its background by `tools/cutout_logo.py`, and the XiØN and Arcade colourways are produced from it by `tools/recolor_logo.py`.

## What the shipped build will contain

Source code, recompiled translations of the user's own binary (generated locally, never distributed), a native runtime, a launcher, and documentation. Distribution of translated game code is not permitted; the generated `build/lift/*.c` files are build artifacts derived from the user's dump and must be produced locally by the user.
