// SPDX-License-Identifier: GPL-3.0-or-later
// Play mode settings page (src/gfx/play/play_ui.cpp) drawn headless: ImGui without a renderer or platform backend, a few frames
// at a time, against a fake host. No input is simulated: tabs are opened through ImGui's tab bar (TabBarQueueFocus).
#include "test_framework.h"

#include <windows.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include "imgui.h"
#include "imgui_internal.h"
#include "nxrt/input_source.h"
#include "play/play_ui.h"

NXTEST_GROUP(play_ui)

namespace {

namespace play = nxrt::play;
namespace hid = nxrt::hid;

// Records what the page asks of the host and answers from fields the test sets.
class FakeHost final : public play::UiHost {
public:
    play::Settings settings_ = play::default_settings();
    play::LiveStatus status_;
    std::vector<play::InputId> queued;   // capture results, handed out one per poll
    bool capturing_ = false;
    int applies = 0, begins = 0, cancels = 0;
    play::Settings last_applied;

    play::Settings settings() const override { return settings_; }
    play::Settings factory() const override { return play::default_settings(); }
    void apply(const play::Settings& s) override {
        applies++;
        last_applied = s;
    }
    play::LiveStatus status() const override { return status_; }
    std::string settings_path() const override { return "test_settings.ini"; }
    void begin_capture() override {
        begins++;
        capturing_ = true;
    }
    bool capturing() const override { return capturing_; }
    bool capture_result(play::InputId* out) override {
        if (queued.empty()) return false;
        *out = queued.front();
        queued.erase(queued.begin());
        capturing_ = false;
        return true;
    }
    void cancel_capture() override {
        cancels++;
        capturing_ = false;
    }
};

// One ImGui context without a backend. ImGui 1.92 without RendererHasTextures builds the font atlas on request, so it is built here.
struct Headless {
    Headless() {
        IMGUI_CHECKVERSION();
        ImGui::CreateContext();
        ImGuiIO& io = ImGui::GetIO();
        io.DisplaySize = ImVec2(1280, 720);
        io.DeltaTime = 1.0f / 60.0f;
        io.IniFilename = nullptr;   // no imgui.ini in the working directory
        io.LogFilename = nullptr;
        unsigned char* px = nullptr;
        int w = 0, h = 0;
        io.Fonts->GetTexDataAsRGBA32(&px, &w, &h);
    }
    ~Headless() { ImGui::DestroyContext(); }
};

// Draws n frames of the Controls window with the settings page inside it.
void run_frames(play::UiHost& host, int n) {
    for (int i = 0; i < n; ++i) {
        ImGui::NewFrame();
        ImGui::SetNextWindowSize(ImVec2(640, 720), ImGuiCond_FirstUseEver);
        if (ImGui::Begin("Controls")) play::draw_settings_page(host);
        ImGui::End();
        ImGui::Render();
    }
}

// The tab bar of the settings page inside the Controls window, or nullptr before the first frame.
ImGuiTabBar* settings_tab_bar() {
    ImGuiWindow* win = ImGui::FindWindowByName("Controls");
    return win ? ImGui::TabBarFindByID(win->GetID("settings_tabs")) : nullptr;
}

// Asks the tab bar to open the named tab at the next frame. A tab's id is its label hashed with the tab bar's id as the seed (the tab
// bar is on the ID stack while its tabs are drawn). TabBarQueueFocus by name needs a current window, which there is not between frames.
void open_tab(const char* name) {
    ImGuiWindow* win = ImGui::FindWindowByName("Controls");
    ImGuiTabBar* bar = settings_tab_bar();
    if (win && bar) bar->NextSelectedTabId = ImHashStr(name, 0, win->GetID("settings_tabs"));
}

// Returns true when the open tab has this name. Checked through the tab bar's selected id.
bool tab_open(const char* name) {
    ImGuiTabBar* bar = settings_tab_bar();
    if (!bar) return false;
    for (int i = 0; i < bar->Tabs.Size; ++i) {
        if (bar->Tabs[i].ID == bar->SelectedTabId) return std::strcmp(ImGui::TabBarGetTabName(bar, &bar->Tabs[i]), name) == 0;
    }
    return false;
}

// Draws every tab: one frame so the tabs exist, then each tab is opened in turn with two frames, and checked to be the open one.
// Returns false when a tab did not open.
bool draw_all_tabs(play::UiHost& host) {
    static const char* const kTabs[] = {"Mouse", "Controller", "Display", "Bindings", "Switch layout", "Status"};
    bool ok = true;
    run_frames(host, 1);
    for (const char* t : kTabs) {
        open_tab(t);
        run_frames(host, 2);
        if (!tab_open(t)) {
            std::printf("    tab did not open: %s\n", t);
            ok = false;
        }
    }
    return ok;
}

}  // namespace

NXTEST(default_settings_draw_20_frames) {
    Headless ui;
    FakeHost host;
    run_frames(host, 20);
    CHECK_EQ(host.applies, 0);   // drawing never applies by itself
    CHECK_EQ(host.begins, 0);
}

NXTEST(every_tab_draws_with_default_settings) {
    Headless ui;
    FakeHost host;
    CHECK(draw_all_tabs(host));
    CHECK_EQ(host.applies, 0);
    CHECK_EQ(host.begins, 0);
}

// The page keeps its working copy for the life of the process, so the binding counts run in a fresh process each.
NXTEST_CHILD(play_ui_empty_bindings) {
    Headless ui;
    FakeHost host;
    host.settings_.bindings.clear();
    return draw_all_tabs(host) ? 0 : 1;
}

NXTEST_CHILD(play_ui_300_bindings) {
    Headless ui;
    FakeHost host;
    host.settings_.bindings.clear();
    for (int i = 0; i < 300; ++i) {
        play::Binding b;
        b.in.kind = play::InKind::Key;
        b.in.code = 4 + i % 26;   // SDL scancodes of the letters
        b.ctx = (i % 2) ? play::Ctx::Play : play::Ctx::Menu;
        b.act = play::Act::Primary;
        b.trig = play::Trig::Tap;   // a timed trigger: the ms column is enabled
        b.ms = 200;
        host.settings_.bindings.push_back(b);
    }
    return draw_all_tabs(host) ? 0 : 1;
}

NXTEST(bindings_empty_and_300_each_in_fresh_process) {
    char exe[MAX_PATH] = {};
    GetModuleFileNameA(nullptr, exe, MAX_PATH);
    const char* kids[] = {"play_ui_empty_bindings", "play_ui_300_bindings"};
    for (const char* k : kids) {
        // cmd /c strips the outer pair of quotes, so the whole command is wrapped in one more pair.
        const std::string cmd = std::string("\"\"") + exe + "\" --child " + k + "\"";
        const int rc = std::system(cmd.c_str());
        CHECK_MSG(rc == 0, std::string("child ") + k + " returned " + std::to_string(rc));
    }
}

NXTEST(status_values_draw) {
    Headless ui;
    FakeHost host;
    host.status_.fps = 59.94;
    host.status_.pads = 1;
    host.status_.pad_name = "Test pad";
    host.status_.pad_gyro = true;
    host.status_.refresh_hz = 60.0f;
    host.status_.window_mode = "windowed";
    host.status_.present_mode = "fifo";
    host.status_.width = 1280;
    host.status_.height = 720;
    host.status_.aim = "aim on";
    host.status_.input_calls = 0;
    CHECK(draw_all_tabs(host));
    host.status_.input_calls = 5;
    host.status_.out.buttons = hid::btn(hid::kBtnA);
    host.status_.out.ly = 1.0f;
    CHECK(draw_all_tabs(host));
    CHECK_EQ(host.applies, 0);
}

NXTEST(capture_hint_and_result_are_consumed) {
    Headless ui;
    FakeHost host;
    host.begin_capture();   // the host is capturing: the page shows the hint line
    run_frames(host, 3);
    play::InputId key;
    key.kind = play::InKind::Key;
    key.code = 4;
    host.queued.push_back(key);
    run_frames(host, 1);
    CHECK(host.queued.empty());   // the page takes the result even when it did not ask for one
    CHECK_EQ(host.applies, 0);
}

// The frame rate counter is live: the game's hotkey can flip fps.show after the page has loaded. Drawing every tab after that must not
// hand anything to the host, and the page keeps drawing.
NXTEST(fps_flip_after_load_draws_without_apply) {
    Headless ui;
    FakeHost host;
    CHECK(draw_all_tabs(host));
    host.settings_.fps.show = true;
    host.settings_.fps.corner = play::Corner::BottomRight;
    CHECK(draw_all_tabs(host));
    CHECK_EQ(host.applies, 0);
    CHECK_EQ(host.begins, 0);
}
