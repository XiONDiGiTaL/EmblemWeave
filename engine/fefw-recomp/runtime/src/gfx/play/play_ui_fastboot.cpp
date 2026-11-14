// SPDX-License-Identifier: GPL-3.0-or-later
// The F1 page's start-up and menu tweaks (S46, play_fastboot.h): a group at the end of the Extras tab.
#include "imgui.h"
#include "play_fastboot.h"

namespace nxrt::play {

bool fastboot_ui(FastbootSettings& f) {
    bool changed = false;
    auto tip = [](const char* text) {
        if (ImGui::IsItemHovered()) ImGui::SetItemTooltip("%s", text);
    };
    ImGui::SeparatorText("Start-up and menus");
    if (ImGui::Checkbox("Skip the opening cutscene of a new game", &f.skip_intro)) changed = true;
    tip("New Game starts in the Frigate's docking hangar after the game's own cutscene skip, in the same state the full cutscene leaves. "
        "Other cutscenes play as usual.");
    if (ImGui::Checkbox("Skip the title intro movie", &f.skip_boot_logos)) changed = true;
    tip("The title screen comes up at once instead of after the 14 second logo movie.");
    if (ImGui::Checkbox("Faster menus", &f.fast_menus)) changed = true;
    tip("The title, main menu, save slots and options run their animations, fades and waits faster. Movies and the game itself keep their speed.");
    if (f.fast_menus) {
        int speed = f.menu_speed;
        if (ImGui::SliderInt("Menu speed", &speed, 1, 4, "%dx")) {
            f.menu_speed = speed;
            changed = true;
        }
    }
    return changed;
}

}  // namespace nxrt::play
