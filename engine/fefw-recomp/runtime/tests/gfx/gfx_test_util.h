// SPDX-License-Identifier: GPL-3.0-or-later
// Shared helpers of the scene tests (gfx_scene, gfx_present, gfx_pacing).
#pragma once
#include <windows.h>
#include <psapi.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>

#include "nxrt/gfx.h"
#include "nxrt/input.h"
#include "nxrt/log.h"
#include "nxrt/platform.h"
#include "nxrt/scene.h"
#include "test_framework.h"

namespace gfxtest {
using namespace nxrt;
using namespace nxrt::gfx;


inline const std::string kRefPath = std::string(NXRT_GFX_TEST_DATA_DIR) + "/test_scene_640x360_f60.png";

// Tolerance of the screenshot oracle: the same GPU gives the same image to the bit, other GPUs and drivers differ by rounding in
// the texture filter. These bounds were set from measurements on the RTX 3080 and the UHD 770 (see docs/S4b.md).
inline constexpr double kMeanAbsTolerance = 0.05;     // mean absolute difference per channel, 0..255
inline constexpr int kMaxAbsTolerance = 4;           // largest single channel difference
inline constexpr double kFractionOver8 = 0.0005;       // pixels with any channel off by more than 8

inline SceneRunDesc base_desc() {
    SceneRunDesc d;
    d.validation = 1;
    d.window_hidden = false;   // windows are hidden by NXRT_GFX_HIDDEN=1 (CTest sets it); run the exe by hand to see them
    return d;
}

inline void print_result(const char* what, const SceneRunResult& r) {
    std::printf("    %s: %llu frames on %s, %.2f fps, validation %llu errors / %llu warnings, %llu swapchains, %llu presents\n", what,
                static_cast<unsigned long long>(r.frames), r.gpu_name.c_str(), r.pacing.fps, static_cast<unsigned long long>(r.validation.errors),
                static_cast<unsigned long long>(r.validation.warnings), static_cast<unsigned long long>(r.renderer.swapchain_creates),
                static_cast<unsigned long long>(r.renderer.presents));
    if (!r.validation.first_error.empty()) std::printf("      first error: %s\n", r.validation.first_error.c_str());
}

inline bool capture_scene(SceneRunDesc d, std::vector<uint8_t>* px, uint32_t* w, uint32_t* h, SceneRunResult* out = nullptr) {
    const std::string tmp = "capture_tmp.png";
    d.screenshot_frame = 60;
    d.screenshot_path = tmp;
    d.frames = 61;
    const SceneRunResult r = run_scene(d);
    if (out) *out = r;
    if (!r.ok) {
        std::printf("    run failed: %s\n", r.error.c_str());
        return false;
    }
    return load_png_rgba(tmp, px, w, h);
}

inline DWORD handle_count() {
    DWORD n = 0;
    GetProcessHandleCount(GetCurrentProcess(), &n);
    return n;
}

}  // namespace gfxtest
