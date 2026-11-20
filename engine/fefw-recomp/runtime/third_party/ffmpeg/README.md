# FFmpeg public headers

These are the public `libavcodec` and `libavutil` headers of **FFmpeg n8.1**, used only to compile
`src/media/movie_ffmpeg.cpp`, the optional dynamic loader for the cutscene movie decoder. FFmpeg is not linked. The release
ships an FFmpeg 8.1 GPL shared build (`avcodec-62.dll`, `avutil-60.dll` and the `swresample-6.dll` those DLLs import) in
`upscalers\ffmpeg` beside the launcher; the engine loads it at run time and refuses a build whose libavcodec or libavutil
major version differs from these headers.

The upstream source is <https://github.com/FFmpeg/FFmpeg/tree/n8.1>; the headers were taken from the
`ffmpeg-n8.1-latest-win64-lgpl-shared-8.1` package published by BtbN at
<https://github.com/BtbN/FFmpeg-Builds/releases>. `libavutil/avconfig.h` is the generated Windows x64 configuration from
that package.

FFmpeg is licensed under the GNU Lesser General Public License, version 2.1 or later. The licence texts are
`COPYING.LGPLv2.1` and `COPYING.LGPLv3`; FFmpeg's own notice is in `LICENSE.md` upstream. The headers are used here under
LGPL-2.1-or-later, which is compatible with this runtime's GPL-3.0-or-later licence.

The shipped DLLs are the GPL shared build `N-123619-g44ad73031d` (libavcodec 62.29.101, libavutil 60.29.100,
swresample 6.4.100), pinned for this release. Its GPLv3 text, SHA-256 hashes, source links and component notices are in
the release's `upscalers\ffmpeg` folder. That GPL build is compatible with this runtime's
GPL-3.0-or-later licence; no FFmpeg library is linked into `fefw.exe`.

The hardware path uses FFmpeg's native `av1` decoder with D3D11VA. FFmpeg's native AV1 decoder has no software path, so
the software path uses FFmpeg's bundled `libdav1d` decoder. The engine's own dav1d remains the default decoder.
