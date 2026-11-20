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

## Copyright and takedown

EmblemWeave does not operate a user-uploaded content service. Copyright complaints should go to the hosting provider first, because the provider controls the repository and release hosting. The project's review policy, the elements of a formal notice and the counter-notice path are in [DMCA.md](DMCA.md). This project does not claim protection under 17 U.S.C. §512, and this page is not legal advice.

## Analysis of the supplied package

The supplied `.nup` package is encrypted retail content. This project consumed an already-decrypted, already-extracted view present on the machine (ExeFS + romfs). No circumvention of the package's protection was performed by this project, and no keys are stored or embedded here.

## Licence

EmblemWeave is licensed under the GNU General Public License, version 3 only (`GPL-3.0-only`). It was MIT-licensed until
9 November 2026; it moved to the GPL so that code from fefw-recomp (GPL-3.0, by Goldoire and its contributors) can be
adapted into it. That project's repository has since been removed; its final release was preserved by Obersteinn, and
[UPSTREAM.md](UPSTREAM.md) records what EmblemWeave keeps from it. Adapted files keep fefw-recomp's attribution, and
[THIRD_PARTY.md](THIRD_PARTY.md) lists them.

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
| `zip`, `flate2` | MIT / MIT OR Apache-2.0 | the diagnostic report archive the Troubleshooter writes |
| `winresource` | MIT | build-time embedding of the launcher icon and version information |
| FFmpeg public headers | LGPL-2.1-or-later | optional movie decoder: the libavcodec/libavutil headers used to compile the dynamic loader |
| FFmpeg 8.1 shared libraries | GPL-2.0-or-later | optional movie decoder: `avcodec-62.dll`, `avutil-60.dll` and `swresample-6.dll` ship in `upscalers/ffmpeg`; they are loaded at run time, not linked. The folder carries the GPLv3 text, SHA-256 hashes, build/source links and upstream notices |
| Unicorn | GPL-2.0 | development-only CPU oracle (`tools/oracle_lockstep.py`), used from Python as a separate process; not linked, not shipped |
| `zstandard`, Pillow | BSD-3-Clause / HPND | development-only helpers for checking assets and generating the icon; not shipped |
| `zbic` (kinnay/zbic) | **GPL-2.0** | development-only ZBIC bridge, invoked as a separate process by `tools/zbic_extract.py`; not linked, not vendored, not shipped |
| Ghidra | Apache-2.0 | interactive analysis (not redistributed) |

`zbic` is used out-of-process and is not a dependency of any shipped binary. Because it is GPL-2.0, an in-tree port must be written from the format description rather than copied, and the resulting licensing decision recorded before any port lands.

## Trademarks and presentation

The launcher's visual language is inspired by the game's tactical-fantasy aesthetic and by [XiØN](https://github.com/XiONDiGiTaL)'s own identity. It uses no Nintendo artwork and no official Fire Emblem logo.

## What the shipped build will contain

Source code, recompiled translations of the user's own binary (generated locally, never distributed), a native runtime, a launcher, documentation, and the FFmpeg 8.1 shared DLLs with their GPL text and source links. Distribution of translated game code is not permitted; the generated `build/lift/*.c` files are build artifacts derived from the user's dump and must be produced locally by the user.
