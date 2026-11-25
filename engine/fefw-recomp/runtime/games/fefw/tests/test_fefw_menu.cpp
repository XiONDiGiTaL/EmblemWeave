// SPDX-License-Identifier: GPL-3.0-or-later
// The F1 menu on a gamepad: R and L walk through every tab and back. A tab's first control takes the cursor without being activated:
// an activated slider is a text box, and while a control is active L / R do not switch tabs.
#include <cstdio>
#include <set>
#include <string>

#include "fefw_game.h"
#include "gfx/gfx_test_util.h"
#include "imgui.h"
#include "imgui_internal.h"
#include "nxrt/input_source.h"
#include "nxrt/overlay.h"
#include "play/play_ui.h"
#include "test_framework.h"

NXTEST_GROUP(fefw_menu)

using namespace gfxtest;

namespace {

namespace play = nxrt::play;
namespace hid = nxrt::hid;

class FakeHost final : public play::UiHost {
public:
    play::Settings settings_ = play::default_settings();
    play::Settings settings() const override { return settings_; }
    play::Settings factory() const override { return play::default_settings(); }
    void apply(const play::Settings& s) override { settings_ = s; }
    play::LiveStatus status() const override { return {}; }
    std::string settings_path() const override { return "test_settings.ini"; }
    void begin_capture() override {}
    bool capturing() const override { return false; }
    bool capture_result(play::InputId*) override { return false; }
    void cancel_capture() override {}
};

}  // namespace

NXTEST(l_and_r_walk_every_tab) {
    DeviceDesc dd;
    dd.validation = 1;
    dd.headless = 1;
    const auto dev = Device::create(dd);
    REQUIRE(dev != nullptr);
    RendererDesc rd;
    rd.headless_width = 1280;
    rd.headless_height = 720;
    const auto rend = Renderer::create(*dev, rd);
    REQUIRE(rend != nullptr);
    Overlay::Context oc;
    oc.device = dev.get();
    oc.renderer = rend.get();
    const auto overlay = Overlay::create(oc);
    REQUIRE(overlay != nullptr);
    overlay->install_on_renderer();

    FakeHost host;
    std::string tab;        // the open tab's label, as the last frame drew it
    bool active = false;    // a control is active (held, or being typed into)
    Overlay& o = *overlay;
    o.set_menu([&] {
        nxrt::games::fefw::settings_menu(host, o);
        tab.clear();
        if (ImGuiWindow* w = ImGui::FindWindowByName("##fefw_menu"))
            if (ImGuiTabBar* bar = ImGui::TabBarFindByID(w->GetID("##tabs")))
                for (int i = 0; i < bar->Tabs.Size; ++i)
                    if (bar->Tabs[i].ID == bar->SelectedTabId) tab = ImGui::TabBarGetTabName(bar, &bar->Tabs[i]);
        active = ImGui::IsAnyItemActive();
    });
    const auto frames = [&](int n) {
        for (int i = 0; i < n; ++i) {
            FrameInfo fi;
            if (rend->begin_frame(&fi)) rend->end_frame();
        }
    };
    const auto press = [&](hid::NpadButtonBit b) {
        o.set_menu_pad(hid::btn(b), 0, 0);
        frames(2);
        o.set_menu_pad(0, 0, 0);
        frames(6);
    };

    o.set_visible(true);
    o.set_menu_pad(0, 0, 0);
    frames(6);
    REQUIRE(!tab.empty());

    std::set<std::string> seen = {tab};
    int tabs = 1;
    // forwards until the first tab comes round again, then the same number of steps backwards
    const std::string first = tab;
    for (int step = 0; step < 20; ++step) {
        const std::string before = tab;
        press(hid::kBtnR);
        std::printf("    R: %s -> %s%s\n", before.c_str(), tab.c_str(), active ? "  (a control is active)" : "");
        CHECK_MSG(tab != before, std::string("R did not leave the tab ") + before);
        CHECK_MSG(!active, std::string("a control is active on the tab ") + tab);
        // the cursor did arrive: on Audio it is on the volume slider, so right steps the volume
        if (!active && tab.find("###tab3") != std::string::npos) {
            const double volume = host.settings_.audio.volume;
            press(hid::kBtnRight);
            std::printf("    right on %s: volume %.2f -> %.2f\n", tab.c_str(), volume, host.settings_.audio.volume);
            CHECK(host.settings_.audio.volume > volume);
            CHECK(!active);
        }
        if (tab == before || tab == first) break;
        seen.insert(tab);
        ++tabs;
    }
    CHECK(tab == first);
    CHECK(tabs >= 7);
    for (int step = 0; step < tabs; ++step) {
        const std::string before = tab;
        press(hid::kBtnL);
        std::printf("    L: %s -> %s%s\n", before.c_str(), tab.c_str(), active ? "  (a control is active)" : "");
        CHECK_MSG(tab != before, std::string("L did not leave the tab ") + before);
        CHECK_MSG(!active, std::string("a control is active on the tab ") + tab);
        if (tab == before) break;
    }
    CHECK(tab == first);
}
