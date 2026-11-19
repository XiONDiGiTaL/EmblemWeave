// SPDX-License-Identifier: GPL-3.0-or-later
// The code mods' panels (mod API version 4, nxrt/mod_api.h add_panel): services/mods_dll.cpp keeps them, the overlay draws them in its
// Mods window (gfx/overlay.cpp, [gfx] mods_key).
#pragma once
#include <string>
#include <vector>

#include "nxrt/mod_api.h"

namespace nxrt::services::modapi {

struct PanelInfo {
    std::string title;           // as the mod gave it
    std::string mod;             // the mod that added it (folder/dll), for the tooltip
    NxModPanelFn fn = nullptr;
    void* user = nullptr;
};

// The panels of the mods that started, in the order added (a copy: any thread)
std::vector<PanelInfo> panels();

// The overlay says whether its Mods window is on screen (api->panel_visible)
void set_panel_visible(bool visible);

}  // namespace nxrt::services::modapi
