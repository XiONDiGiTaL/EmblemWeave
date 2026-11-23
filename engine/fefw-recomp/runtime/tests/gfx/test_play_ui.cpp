// SPDX-License-Identifier: GPL-3.0-or-later
// Play mode settings page (src/gfx/play/play_ui.cpp) drawn headless: ImGui without a renderer or platform backend, a few frames
// at a time, against a fake host. No input is simulated: tabs are opened through ImGui's tab bar (TabBarQueueFocus).
#include "play_test_game.h"
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
#include "play/play_audio_ui.h"
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

// The page's tab bars (play_ui.cpp draw_settings_page): "settings_pages" in the Controls window, and "controls_tabs" inside its Controls
// page. A tab's id is its label hashed with its tab bar's id as the seed (the bar is on the ID stack while its tabs are drawn), and an open
// tab item pushes its own id, so the inner bar's id is seeded with the Controls tab's id.
ImGuiID pages_bar_id() {
    ImGuiWindow* win = ImGui::FindWindowByName("Controls");
    return win ? win->GetID("settings_pages") : 0;
}
ImGuiID controls_bar_id() {
    const ImGuiID pages = pages_bar_id();
    return pages ? ImHashStr("controls_tabs", 0, ImHashStr("Controls", 0, pages)) : 0;
}

// Asks the tab bar to open the named tab at the next frame. TabBarQueueFocus by name needs a current window, which there is not between frames.
void open_tab(ImGuiID bar_id, const char* name) {
    if (ImGuiTabBar* bar = bar_id ? ImGui::TabBarFindByID(bar_id) : nullptr) bar->NextSelectedTabId = ImHashStr(name, 0, bar_id);
}

// Returns true when the open tab of the bar has this name. Checked through the tab bar's selected id.
bool tab_open(ImGuiID bar_id, const char* name) {
    ImGuiTabBar* bar = bar_id ? ImGui::TabBarFindByID(bar_id) : nullptr;
    if (!bar) return false;
    for (int i = 0; i < bar->Tabs.Size; ++i) {
        if (bar->Tabs[i].ID == bar->SelectedTabId) return std::strcmp(ImGui::TabBarGetTabName(bar, &bar->Tabs[i]), name) == 0;
    }
    return false;
}

// Opens one tab with two frames and checks that it is the open one.
bool open_and_check(play::UiHost& host, ImGuiID bar_id, const char* name) {
    open_tab(bar_id, name);
    run_frames(host, 2);
    if (tab_open(bar_id, name)) return true;
    std::printf("    tab did not open: %s\n", name);
    return false;
}

// Draws every tab: one frame so the tabs exist, then each page is opened in turn, and inside the Controls page each of its tabs.
// Returns false when a tab did not open.
bool draw_all_tabs(play::UiHost& host) {
    static const char* const kPages[] = {"Display", "Graphics", "Controls", "Audio", "About"};
    static const char* const kControls[] = {"Mouse", "Keyboard and mouse buttons", "Gamepad", "Switch layout", "Status"};
    bool ok = true;
    run_frames(host, 1);
    for (const char* t : kPages) ok = open_and_check(host, pages_bar_id(), t) && ok;
    ok = open_and_check(host, pages_bar_id(), "Controls") && ok;
    for (const char* t : kControls) ok = open_and_check(host, controls_bar_id(), t) && ok;
    return ok;
}

}  // namespace

NXTEST(default_settings_draw_20_frames) {
    const nxrt::play::test::PlayTestGame play_game;
    Headless ui;
    FakeHost host;
    run_frames(host, 20);
    CHECK_EQ(host.applies, 0);   // drawing never applies by itself
    CHECK_EQ(host.begins, 0);
}

// The floating panel is a window of its own drawn while the settings window is current; this catches a Begin/End or
// table/draw-list mistake without a renderer.
NXTEST(audio_enhancements_panel_draws_beside_the_settings_page) {
    Headless ui;
    play::Settings settings = play::default_settings();
    settings.audio.eq_enabled = true;
    settings.audio.eq_bands[0] = 6.0;
    play::audio_ui::State state;
    state.eq_open = true;
    for (int i = 0; i < 3; ++i) {
        ImGui::NewFrame();
        ImGui::Begin("Controls");
        const play::audio_ui::Edit edit = play::audio_ui::enhancements_window(settings.audio, state);
        CHECK(edit == play::audio_ui::Edit::None);
        ImGui::End();
        ImGui::Render();
    }
    CHECK(state.eq_open);
}

NXTEST(audio_volume_can_return_above_100_after_lowering_from_a_boost) {
    play::AudioSettings a;
    a.volume = 2.0;
    play::audio_ui::State state;
    CHECK(play::audio_ui::apply_volume(a, state, 100.0));
    CHECK(a.volume == 1.0);
    CHECK(!state.boost_warning);
    CHECK(play::audio_ui::apply_volume(a, state, 150.0));
    CHECK(a.volume == 1.5);
    CHECK(!state.boost_warning);
    CHECK(play::audio_ui::apply_volume(a, state, 200.0));
    CHECK(a.volume == 2.0);
}

NXTEST(audio_volume_above_100_applies_without_a_confirmation) {
    play::AudioSettings a;
    play::audio_ui::State state;
    CHECK(play::audio_ui::apply_volume(a, state, 150.0));
    CHECK(a.volume == 1.5);
    CHECK(!state.boost_warning);
    CHECK(play::audio_ui::apply_volume(a, state, 175.0));
    CHECK(a.volume == 1.75);
    CHECK(play::audio_ui::apply_volume(a, state, 200.0));
    CHECK(a.volume == 2.0);
}

NXTEST(audio_boost_warning_draws_in_the_settings_window) {
    Headless ui;
    play::Settings settings = play::default_settings();
    play::audio_ui::State state;
    state.boost_warning = true;
    state.boost_pending = 150.0;
    for (int i = 0; i < 3; ++i) {
        ImGui::NewFrame();
        ImGui::Begin("Controls");
        CHECK(!play::audio_ui::boost_warning(settings.audio, state));
        ImGui::End();
        ImGui::Render();
    }
    CHECK(state.boost_warning);
    CHECK(settings.audio.volume == 1.0);
}

NXTEST(every_tab_draws_with_default_settings) {
    const nxrt::play::test::PlayTestGame play_game;
    Headless ui;
    FakeHost host;
    CHECK(draw_all_tabs(host));
    CHECK_EQ(host.applies, 0);
    CHECK_EQ(host.begins, 0);
}

// The page keeps its working copy for the life of the process, so the binding counts run in a fresh process each.
NXTEST_CHILD(play_ui_empty_bindings) {
    const nxrt::play::test::PlayTestGame play_game;
    Headless ui;
    FakeHost host;
    host.settings_.bindings.clear();
    return draw_all_tabs(host) ? 0 : 1;
}

NXTEST_CHILD(play_ui_300_bindings) {
    const nxrt::play::test::PlayTestGame play_game;
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
    const nxrt::play::test::PlayTestGame play_game;
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
    const nxrt::play::test::PlayTestGame play_game;
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
    const nxrt::play::test::PlayTestGame play_game;
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
    const nxrt::play::test::PlayTestGame play_game;
    Headless ui;
    FakeHost host;
    CHECK(draw_all_tabs(host));
    host.settings_.fps.show = true;
    host.settings_.fps.corner = play::Corner::BottomRight;
    CHECK(draw_all_tabs(host));
    CHECK_EQ(host.applies, 0);
    CHECK_EQ(host.begins, 0);
}
