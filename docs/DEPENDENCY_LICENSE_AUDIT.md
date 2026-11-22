# Dependency and licensing audit

Date: 2026-11-22

Scope: the launcher, Rust workspace, engine/runtime, shader translator, build tools and the Windows release package. This
is an inventory and distribution-compliance review, not legal advice. It does not change engine dependencies.

## Method

- Read every `Cargo.toml` in the workspace and the direct dependency list in `Cargo.lock`.
- Ran `cargo metadata --locked --format-version 1` against the locked tree: 464 packages, all with a declared license
  expression or license file.
- Read the license files under `engine/fefw-recomp/runtime/third_party/`, `upscalers/ffmpeg/`, the root `LICENSE`, and
  `docs/LEGAL.md`, `docs/THIRD_PARTY.md`, and the translator notices under
  `engine/fefw-recomp/tools/shaders/licenses/`.
- The Microsoft .NET files are the SDK's `LICENSE.txt` and `ThirdPartyNotices.txt`, copied as
  `DOTNET-LICENSE.txt` and `DOTNET-THIRD-PARTY-NOTICES.txt`. The Ryujinx MIT text is copied from the pinned translator
  source's licence.
- Compared the package layout in `tools/package_release.ps1` with those files. The package ships engine sources and the
  runtime third-party licenses; it does not ship the fetched texture/shader third-party source trees.

## Rust workspace

Direct dependencies (workspace and launcher):

| Dependency | License | Use | Distribution |
| --- | --- | --- | --- |
| `anyhow` | MIT OR Apache-2.0 | error propagation | statically linked |
| `serde`, `serde_json` | MIT OR Apache-2.0 | configuration, reports | statically linked |
| `lz4_flex` | MIT | LZ4 segment decompression | statically linked |
| `ruzstd` | MIT | Zstandard probing | statically linked |
| `sha2` | MIT OR Apache-2.0 | hashes and integrity | statically linked |
| `libm` | MIT OR Apache-2.0 | console-matching math | statically linked |
| `ash` | Apache-2.0 OR MIT | Vulkan device layer | statically linked |
| `eframe`/`egui` | MIT OR Apache-2.0 | launcher UI | statically linked |
| `rfd` | MIT | native file dialogs | statically linked |
| `image` | MIT OR Apache-2.0 | PNG icons/screenshots | statically linked |
| `tar`, `zip`, `flate2` | MIT/Apache-2.0 family | package extraction | statically linked |
| `zstd` | MIT | package extraction | statically linked |
| `roxmltree` | MIT OR Apache-2.0 | controls.ini parsing | statically linked |
| `chrono` | MIT OR Apache-2.0 | timestamps | statically linked |
| `regex` | MIT OR Apache-2.0 | text matching | statically linked |
| `uuid` | Apache-2.0 OR MIT | identifiers | statically linked |
| `clap` | MIT OR Apache-2.0 | CLI front ends | statically linked |
| `walkdir` | Unlicense OR MIT | directory walks | statically linked |
| `winresource` | MIT | Windows icon/version resource | build-time only |
| `windows-sys` | MIT OR Apache-2.0 | Win32 bindings | platform only |
| `r-efi` (transitive, optional) | MIT OR Apache-2.0 OR LGPL-2.1-or-later | Windows platform metadata | the permissive option is selected; no LGPL code is linked |

`cargo metadata` counts for the locked tree: 464 packages. The most common expressions are 205 `MIT OR Apache-2.0`,
110 `MIT`, 39 `Apache-2.0 OR MIT`, 18 `Unicode-3.0`, 15 `MIT/Apache-2.0`, 14 `Apache-2.0`, 7
`Zlib OR Apache-2.0 OR MIT`, 7 `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT`, and 6 workspace crates under
`GPL-3.0-only`. No package had an empty license field and no `license_file` fallback was needed. `Unlicense OR MIT`
packages (`aho-corasick`, `memchr`, `walkdir`, `winapi-util`, `termcolor`, `same-file`, `byteorder-lite`) can be used
under MIT. The `r-efi` expression offers a permissive option; retain the chosen option in any future notice file.

The workspace itself is `GPL-3.0-only` (`Cargo.toml` and `LICENSE`). The launcher and every shipped Rust crate are
statically linked into `EmblemWeave.exe`, so the GPL applies to the combined launcher distribution. Source is provided
by the public repository and the release zip's engine sources. Keep the root `LICENSE` in the package.

## Shipped engine/runtime components

| Component | Version/source | License | Shipped in the Windows zip | Notes |
| --- | --- | --- | --- | --- |
| fefw-recomp engine | vendored source, GPL-3.0 | GPL-3.0 | yes, full source | Engine `LICENSE`, `LEGAL.md` and notices must stay. |
| zstd | 1.5.7, fetched by `runtime/cmake/zstd_deps.cmake` | BSD-3-Clause | compiled into runtime | Upstream license; no separate notice currently packaged. |
| dav1d | vendored `runtime/third_party/dav1d` | BSD-2-Clause | source + `COPYING` | Keep `COPYING`. |
| SDL3 | fetched 3.4.18 | Zlib | compiled into runtime | Upstream license. |
| Vulkan-Headers / volk / VMA | fetched | Apache-2.0 / MIT / MIT | compiled/source | Notices in upstream. |
| Dear ImGui | fetched | MIT | compiled into runtime | Upstream license. |
| AMD FidelityFX SDK (`amd_ffx_api`, `amd_ffx_api_fg`, `ffx_fsr1`) | vendored headers | MIT-style FidelityFX license | source + `LICENSE.txt` | MIT-like; keep the AMD copyright and permission notice. |
| NVIDIA NIS | vendored | MIT | source + `LICENSE.txt` | Keep notice. |
| NVIDIA Streamline | vendored | MIT | source + `license.txt` | Keep notice. |
| Intel XeSS SDK | vendored | Intel Simplified Software License (Oct 2022) | source + `LICENSE.txt` | **Not an OSI license.** Redistribution is allowed unmodified with the notice; no reverse engineering, decompilation or modification. The XeSS runtime DLL itself is user-supplied, not in the zip. |
| renhdrum | vendored | MIT OR Apache-2.0 | source + both licenses + NOTICE | Keep all three. |
| FFmpeg public headers | `runtime/third_party/ffmpeg` | LGPL-2.1-or-later | source + `COPYING.LGPLv2.1`/`COPYING.LGPLv3` | Used to compile the dynamic loader. |
| FFmpeg 8.1 shared libraries | `upscalers/ffmpeg` | GPL-2.0-or-later | `avcodec-62.dll`, `avutil-60.dll`, `swresample-6.dll` + `LICENSE.txt`, `COPYING.GPLv3`, `README.md`, `SHA256SUMS` | The build includes libx264/libx265 (GPL). The folder must include the GPL text and the corresponding-source links/offer. |
| Ryujinx shader translator sources | pinned Ryujinx commit, MIT | MIT | translator binary + `LICENSE.Ryujinx.MIT.txt` beside it | The binary is built from Ryujinx code. The notice is now shipped and required by package verification. |
| .NET runtime | self-contained single-file translator | Microsoft .NET Library terms + Microsoft third-party notices | inside `spirv_harness.exe` + `DOTNET-LICENSE.txt`, `DOTNET-THIRD-PARTY-NOTICES.txt` beside it | The Microsoft .NET redistribution terms and third-party notices are now shipped and required by package verification. |

AMD, NVIDIA and Intel upscaler DLLs are **not** shipped; the player supplies them. The engine's headers and notices are
shipped because the engine source tree is in the zip.

## Build/development-only components

| Component | License | Use | Shipped? |
| --- | --- | --- | --- |
| Python 3 | PSF-2.0 | lifter, tools, shader harness | no |
| LLVM/Clang, lld | Apache-2.0 WITH LLVM-exception | compiler/linker | no |
| CMake, Ninja | BSD-3-Clause / Apache-2.0 | build | no |
| Visual Studio C++ tools and Windows SDK | Microsoft | build | no |
| .NET SDK | MIT | publish translator | no |
| Unicorn | GPL-2.0 | separate-process CPU oracle | no |
| `zbic` (kinnay/zbic) | GPL-2.0 | separate-process ZBIC bridge | no |
| Pillow | HPND | icon/asset helpers | no |
| zstandard (Python) | BSD-3-Clause | asset checks | no |
| Ghidra | Apache-2.0 | interactive analysis | no |
| Ryujinx `Ryujinx.Common`, `Spv.Generator`, `Ryujinx.Graphics.Shader` | MIT | staged translator sources | only as compiled translator |
| `astc-encoder`, `bc7enc_rdo`, `ispc` (fetched by texture tools) | Apache-2.0 / MIT / BSD-3-Clause | texture transcoders | source fetched at build; not in the player zip |

`Unicorn` and `zbic` are GPL-2.0 but are invoked as separate processes for development; they are not linked into a
shipped binary. Do not copy their code into a shipped component without a licensing decision. The `tools/oracle_lockstep.py`
and `tools/zbic_extract.py` paths are development tools, not package contents.

## Redistribution checklist

- [x] Root `LICENSE` (GPL-3.0) in the zip.
- [x] Engine source contains `LICENSE`, `LEGAL.md`, and the runtime third-party license files (`dav1d`, FFmpeg headers,
      AMD, NVIDIA, Intel, renhdrum).
- [x] FFmpeg DLL folder contains `LICENSE.txt`, `COPYING.GPLv3`, `README.md` with source/build links, and `SHA256SUMS`.
- [x] Package verifies the three FFmpeg DLL hashes and that `SHA256SUMS` pins the same values.
- [x] `docs/LEGAL.md` and `docs/THIRD_PARTY.md` are copied as tracked docs.
- [x] Translator folder contains `LICENSE.Ryujinx.MIT.txt`, `DOTNET-LICENSE.txt` and
      `DOTNET-THIRD-PARTY-NOTICES.txt` beside `spirv_harness.exe`; the package script and CI require all three.
- [ ] Add upstream notices for fetched-but-compiled zstd/SDL3/Vulkan/volk/VMA/ImGui if the project wants a single
      consolidated notice file; the upstream licenses already apply.
- [ ] Keep the corresponding-source offer for the GPL FFmpeg build available; a source link that disappears must be
      replaced with an offer to provide the source (the FFmpeg README says so).

## Gaps and recommendations

**G1 — FFmpeg GPL source offer.** The DLLs are GPL-2.0-or-later and the build includes libx264/libx265. Shipping the GPL
text and source links is not the same as shipping the corresponding source; the current README offers links and says a
source request will be honoured. Keep that offer working, and consider attaching the exact FFmpeg/BtbN build scripts and
source archive or a written offer to the release assets. This is the highest-risk distribution item because the codec
build is GPL, not LGPL.

**G2 — Intel XeSS is not open source.** The license permits unmodified redistribution with the notice but forbids
reverse engineering/decompilation and modification. It is not compatible with adding the XeSS SDK code to a GPL-covered
work in a way that ignores those terms. Keep it as a separate vendored SDK/header with its `LICENSE.txt`; do not modify
it. The actual XeSS runtime DLL is user-supplied and is not in the zip.

**G3 — Ryujinx translator notice — resolved.** The Windows translator is built from MIT-licensed Ryujinx sources. The
package now places `LICENSE.Ryujinx.MIT.txt` next to `spirv_harness.exe`; the text is the Ryujinx MIT copyright and
permission notice, also kept under `tools/shaders/licenses/` for source builds. The archive and CI verification require
the file.

**G4 — .NET self-contained runtime notices — resolved.** The translator folder now includes `DOTNET-LICENSE.txt`
(Microsoft .NET Library terms) and `DOTNET-THIRD-PARTY-NOTICES.txt` (Microsoft's .NET runtime third-party notices),
beside `spirv_harness.exe`. Both are tracked source files and are required by archive and CI verification.

**G5 — Upstream licenses for fetched build inputs.** zstd, SDL3, Vulkan-Headers, volk, VMA and ImGui are fetched during a
build rather than vendored as license files in this repository. Their licenses are compatible (BSD/MIT/Apache/Zlib), but
a consolidated `THIRD_PARTY_NOTICES` file would make the shipped runtime notices auditable. The source URLs/versions are
in `runtime/cmake/`.

**G6 — No missing Cargo license metadata.** `cargo metadata` found no package without a license expression. The
`Unlicense OR MIT` and `r-efi` expressions have permissive options; keep using those options. Re-run the inventory when
`Cargo.lock` changes.

## Reproduction

```sh
cargo metadata --locked --format-version 1 > metadata.json
python - <<'PY'
import json, collections
m = json.load(open('metadata.json'))
print(len(m['packages']), 'packages')
print(collections.Counter(p.get('license') or '(none)' for p in m['packages']).most_common(20))
PY
```

```sh
git ls-files upscalers/ffmpeg
git ls-files engine/fefw-recomp/runtime/third_party | rg 'LICENSE|COPYING|NOTICE'
python tools/verify_archive.py <zip> --require <prefix>/upscalers/ffmpeg/LICENSE.txt
python tools/verify_archive.py <zip> \
  --require <prefix>/engine/fefw-recomp/tools/shaders/prebuilt/win-x64/LICENSE.Ryujinx.MIT.txt \
  --require <prefix>/engine/fefw-recomp/tools/shaders/prebuilt/win-x64/DOTNET-LICENSE.txt \
  --require <prefix>/engine/fefw-recomp/tools/shaders/prebuilt/win-x64/DOTNET-THIRD-PARTY-NOTICES.txt
```

This audit is a snapshot. Re-run it when a packaged dependency, codec build, SDK header or translator source changes.
