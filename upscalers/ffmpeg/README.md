# FFmpeg shared libraries

This folder is shipped in the release zip at `upscalers\ffmpeg`. The launcher detects it beside
`EmblemWeave.exe` and points the engine at it, so the optional **FFmpeg (libavcodec)** movie decoder
works without a separate download. The engine also searches the play folder's `upscalers\ffmpeg` and
`upscalers` folders, so a player can replace these files there.

## What is here

| File | Version | Size | SHA-256 |
| --- | --- | ---: | --- |
| `avcodec-62.dll` | 62.29.101 | 97,584,640 | `c202c131bb051e2298aaf5aca2571fc65d2e781ac07568c63c925b6d4e128877` |
| `avutil-60.dll` | 60.29.100 | 2,930,688 | `1645ea1d5774d07304731f842e6b16507f5000c331cca3cf6d71346ebc02541b` |
| `swresample-6.dll` | 6.4.100 | 722,944 | `ce4dce3d71ea83c49cc332437fd61fe847fa055f4f25a9cf120fd4f41294f1c4` |

The DLLs report FFmpeg build `N-123619-g44ad73031d-20260325`. They are a 64-bit Windows GPL shared
build: it contains libx264 and libx265 even though EmblemWeave itself uses only FFmpeg's AV1 decoder
(the software path uses its bundled libdav1d; the hardware path uses native AV1 with D3D11VA).
FFmpeg's libavcodec major is 62 and libavutil major is 60, which is the ABI the runtime is compiled
against. A different major pair is refused with a log line and the engine falls back to its built-in
dav1d decoder.

## Licence and source

FFmpeg is free software. This GPL build is distributed under the GNU General Public License,
version 2 or later; `LICENSE.txt` is the GPLv3 text that applies to this distribution, and
`COPYING.GPLv3` is a second copy beside it. The FFmpeg public headers used to compile the loader are
under `engine/fefw-recomp/runtime/third_party/ffmpeg/`.

Corresponding source and build information:

- FFmpeg source at the reported revision:
  <https://github.com/FFmpeg/FFmpeg/commit/44ad73031d>
- FFmpeg project: <https://ffmpeg.org/> and <https://github.com/FFmpeg/FFmpeg>
- The shared Windows build and its pinned dependency/build scripts:
  <https://github.com/BtbN/FFmpeg-Builds>
- GPL components in this build include libx264
  (<https://code.videolan.org/videolan/x264>) and libx265
  (<https://bitbucket.org/multicoreware/x265_git>). Their own source and licence notices are
  available from those projects and are identified by the build scripts above.

`SHA256SUMS` contains the hashes of the three DLLs. The release packaging script checks these files
and hashes before it builds the zip, so a release cannot silently ship a missing or different set.

If any source link above stops working, open an issue in
<https://github.com/XiONDiGiTaL/EmblemWeave> and the corresponding source for this build will be
provided.
