// SPDX-License-Identifier: GPL-3.0-or-later
// Outputs wider than the game's 3840x2160 display (32:9 5120x1440, 21:9 5120x2160, 7680x2160, ...) drawn at their own size. A game
// module sets the sizes once at start (games/fefw/fefw_res.cpp, [fefw] wide_textures = auto) and arms the override once it has checked
// the game will use its 2160p display; everything here is inert until then. The NVN back end then:
//   - makes the game's display-sized render targets (3840x2160) and its output-fitted ones (the output fitted into 3840 wide, 3840x1080
//     for 5120x1440, and their halves) the output's size, with GPU memory of their own (api_texture.cpp; only 2D render targets in a
//     pool the CPU cannot access, which the overflow path can back, else the texture is made as the game asked);
//   - while such a texture is bound as the colour target (else the depth target) in the same recording, widens a viewport or scissor
//     from (0, 0) of its game size, a fitted size or its game width (api_cmd_state.cpp; decided when recorded, so a rect recorded
//     in another command buffer than its SetRenderTargets is left as it is);
//   - widens the window's present crop from (0, 0) of a fitted size or 3840 wide (api_device.cpp).
// The game module widens the game's own display size once the window textures exist and frames are being presented.
#pragma once
#include <atomic>
#include <cstdint>

namespace nxrt::display_override {
inline std::atomic<int> output_w{0};          // 0: off
inline std::atomic<int> output_h{0};
inline std::atomic<int> texture_h{0};         // the display-sized targets' height (2160)
inline std::atomic<bool> armed{false};        // the game module checked the game's display answer: widen from now on
inline std::atomic<int> display_widened{0};   // window textures (NVN_TEXTURE_FLAGS_DISPLAY) made wider so far
inline std::atomic<uint64_t> presents{0};     // frames presented (counted only while armed)

inline bool active() { return armed.load(std::memory_order_acquire) && output_w.load(std::memory_order_relaxed) > 3840; }

// The output fitted into 3840 wide is (3840, oh * 3840 / ow): the game module only takes outputs where that is exact.
inline int ceil_shr(int v, int k) { return (v + (1 << k) - 1) >> k; }

// (w, h) is a fitted size at level k (0 = the fitted size, 1..3 = its halves, rounded down or up), for an output (ow, oh).
inline int fitted_level(int ow, int oh, int w, int h) {
    if (ow <= 3840 || oh <= 0) return -1;
    const int fh = static_cast<int>(static_cast<long long>(oh) * 3840 / ow);
    for (int k = 0; k < 4; ++k)
        if ((w == (3840 >> k) && h == (fh >> k)) || (w == ceil_shr(3840, k) && h == ceil_shr(fh, k))) return k;
    return -1;
}

// A fitted size becomes the output's size halved as often (rounded up). True when (w, h) was one.
inline bool widen_fitted(int* w, int* h) {
    if (!active()) return false;
    const int ow = output_w.load(std::memory_order_relaxed), oh = output_h.load(std::memory_order_relaxed);
    const int k = fitted_level(ow, oh, *w, *h);
    if (k < 0) return false;
    *w = ceil_shr(ow, k);
    *h = ceil_shr(oh, k);
    return true;
}

// A game render size the back end would take for a size it widens: itself or one of its first three halves is a fitted size, or it is
// the display size (3840x2160).
inline bool collides_with_fitted(int ow, int oh, int w, int h) {
    if (ow <= 3840 || oh <= 0) return false;
    if (w == 3840 && h == 2160) return true;
    for (int j = 0; j < 4; ++j)
        if (fitted_level(ow, oh, ceil_shr(w, j), ceil_shr(h, j)) >= 0 || fitted_level(ow, oh, w >> j, h >> j) >= 0) return true;
    return false;
}

// The render height kept apart from those sizes: 16 rows less at a time, even (the SSAO pass works on 2x2 blocks), at least 64. The
// game module uses it last, after its own caps, and the F1 menu predicts with it.
inline int keep_apart_h(int ow, int oh, int w, int h) {
    for (int n = 0; n < 8 && h > 64 && collides_with_fitted(ow, oh, w, h); ++n) h = (h - 16 > 64 ? h - 16 : 64) & ~1;
    return h;
}
}  // namespace nxrt::display_override

namespace nxrt {
// Called with the answer just before vi's QueryDisplayBufferMaxResolution returns it, on the game's thread (src/core/switch2.cpp; set once
// by a game module at start, before the game runs).
extern void (*g_vi_buffer_answer_hook)(uint32_t answer);
}  // namespace nxrt
