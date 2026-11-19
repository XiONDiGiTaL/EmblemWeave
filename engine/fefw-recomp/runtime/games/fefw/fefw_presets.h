// SPDX-License-Identifier: GPL-3.0-or-later
// Fire Emblem: Fortune's Weave: the graphics presets (Low, Medium, High, Ultra).
//
// A preset is not a key of its own: picking one writes seven [fefw] keys, and the preset shown is the one whose seven values the file
// holds now ("Custom" when none matches). So a file edited by hand, the launcher and the F1 menu always agree. Output size, the upscaler
// and frame generation are left alone: they depend on the display and the GPU, not on how much detail is wanted.
//
//            render_ratio          shadow_size          quality
//   Low      0.5 (half per side)   1024                 handheld (the handheld config's lighter shadows and effects)
//   Medium   0 (the game's own)    0 (the game's own)   console       = the defaults: the console's picture
//   High     0.67                  4096                 docked
//   Ultra    1.0 (native, DLAA)    8192                 docked
//
// and the graphics data (fefw_detail.cpp, applies at the next start):
//            reflections           ambient_occlusion    crowd_detail    scenery_detail
//   Low      reduced (8 rays)      0                    reduced         reduced
//   others   full                  1                    full            full
//
// Picking a preset also clears [fefw] render (a fixed render size wins over render_ratio).
//
// The launcher has the same table (launcher/src/pages/settings.js PRESETS) and runtime/docs/CONFIG.md describes it: keep the three in step.
#pragma once

#include <cstdlib>
#include <string>

#include "nxrt/config.h"

namespace nxrt::games::fefw {

inline constexpr int kPresetKeyCount = 7;
// The seven [fefw] keys a preset writes, with the runtime's default of each.
inline constexpr const char* kPresetKeys[kPresetKeyCount] = {"render_ratio", "shadow_size", "quality", "reflections", "ambient_occlusion",
                                                             "crowd_detail", "scenery_detail"};
inline constexpr const char* kPresetDefaults[kPresetKeyCount] = {"0", "0", "console", "full", "1", "full", "full"};

struct GraphicsPreset {
    const char* name;
    const char* values[kPresetKeyCount];   // in kPresetKeys order
};
inline constexpr GraphicsPreset kGraphicsPresets[] = {
    {"Low", {"0.5", "1024", "handheld", "reduced", "0", "reduced", "reduced"}},
    {"Medium", {"0", "0", "console", "full", "1", "full", "full"}},
    {"High", {"0.67", "4096", "docked", "full", "1", "full", "full"}},
    {"Ultra", {"1.0", "8192", "docked", "full", "1", "full", "full"}},
};
inline constexpr int kGraphicsPresetCount = static_cast<int>(sizeof kGraphicsPresets / sizeof kGraphicsPresets[0]);

namespace preset_detail {
inline std::string lower(std::string s) {
    for (char& c : s) c = static_cast<char>(c >= 'A' && c <= 'Z' ? c + 32 : c);
    return s;
}
inline bool on(const std::string& v) {
    const std::string l = lower(v);
    return !(l == "0" || l == "false" || l == "off" || l == "no");
}
// Compares by meaning: render_ratio and shadow_size as numbers, ambient_occlusion as a switch, the rest as text in any case.
inline bool same(int key, const std::string& have, const char* want) {
    if (key == 0 || key == 1) return std::atof(have.c_str()) == std::atof(want);
    if (key == 4) return on(have) == (want[0] == '1');
    return lower(have) == want;
}
}  // namespace preset_detail

// The preset fefw.ini holds now, -1 for none (Custom). A fixed [fefw] render size wins over render_ratio, so with one set no preset is in
// force; picking a preset clears it.
inline int graphics_preset_current() {
    if (!config_get_str("fefw", "render", "").empty()) return -1;
    std::string have[kPresetKeyCount];
    for (int k = 0; k < kPresetKeyCount; ++k) have[k] = config_get_str("fefw", kPresetKeys[k], kPresetDefaults[k]);
    for (int i = 0; i < kGraphicsPresetCount; ++i) {
        bool all = true;
        for (int k = 0; k < kPresetKeyCount && all; ++k) all = preset_detail::same(k, have[k], kGraphicsPresets[i].values[k]);
        if (all) return i;
    }
    return -1;
}

}  // namespace nxrt::games::fefw
