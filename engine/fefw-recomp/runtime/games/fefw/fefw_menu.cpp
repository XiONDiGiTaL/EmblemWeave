// SPDX-License-Identifier: GPL-3.0-or-later
// Fire Emblem: Fortune's Weave: the F1 menu (GamePlay::settings_menu, in place of the base settings page and the Diagnostics window).
// notes/menu.md has the design and how it was tested.
//
// One window over the dimmed game, tabs Display, Graphics, Frame rate, Audio, Controls, Game and Debug. F1 or Minus + Plus on the pad open
// and close it ([gfx] overlay_key, [gfx] overlay_pad); while it is open the game gets no input (play.cpp holds the keyboard, the mouse and
// the pad back). The mouse clicks and drags as usual; the pad navigates: the d-pad or the left stick moves, left / right change the value of
// the focused row, A opens a list or holds a slider, B goes back (and closes the menu when nothing is open), L / R switch tabs.
//
// Every change is applied at once and saved at once:
//   controls.ini (play mode's settings: window, vsync, frame limit, counter, audio, pad, HDR, picture) goes through UiHost::apply, which the
//   main thread applies at its next frame and writes back (play_config.cpp keeps the file's comments and unknown keys, nxrt::ini_merge);
//   fefw.ini (the game's [fefw] [dlss] [media] ... keys) is written with nxrt::config_write, which changes the value in place and keeps the
//   rest of the file as it is.
// What the runtime can change while running is live. What is read once at start (the game picks its render config once, the upscaler and
// frame generation are set up once, the frame-rate hooks are installed at start) says "restart", and the row shows the value in force until
// then; nothing pretends to be live. A key set by an environment variable (NXRT_...) wins over the file: the row says so and is locked.
#include "fefw_game.h"
#include "fefw_presets.h"

#include <algorithm>
#include <array>
#include <cctype>
#include <cfloat>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <iterator>
#include <map>
#include <string>
#include <vector>

#include "imgui.h"
#include "imgui_internal.h"   // NavMoveRequestCancel: left / right change a value instead of moving the cursor
#include "extras/framegen.h"
#include "extras/framegen_ffx.h"
#include "extras/temporal.h"
#include "nxrt/audio_sink.h"
#include "nxrt/config.h"
#include "nxrt/cpu_info.h"
#include "nxrt/extras.h"
#include "nxrt/frame_limiter.h"
#include "nxrt/gauge.h"
#include "nxrt/gfx.h"
#include "nxrt/input_source.h"
#include "nxrt/jc2_mouse.h"
#include "nxrt/kernel.h"
#include "nxrt/log.h"
#include "nxrt/lowlat.h"
#include "nxrt/overlay.h"
#include "nxrt/menu_i18n.h"
#include "nxrt/haptic.h"
#include "nxrt/platform.h"
#include "nxrt/render_status.h"
#include "play_selftest.h"   // play::exit_process (Quit game)
#include "play_types.h"
#include "play_ui.h"

namespace nxrt::nvn::be {
const char* dlss_upscaler_in_use();   // nvn/api_dlss_replay.cpp
}

namespace nxrt::games::fefw {

namespace {

constexpr const char* kTag = "menu";
namespace hid = nxrt::hid;
namespace haptic = nxrt::gfx::haptic;
using nxrt::gfx::tr;
using play::UiHost;

const ImVec4 kOrange(1.0f, 0.66f, 0.26f, 1.0f);
const ImVec4 kRestartTag(0.86f, 0.62f, 0.34f, 0.85f);
const ImVec4 kGreen(0.55f, 0.86f, 0.55f, 1.0f);
const ImVec4 kRed(1.0f, 0.42f, 0.38f, 1.0f);
const ImVec4 kAccent(0.42f, 0.78f, 0.96f, 1.0f);
const ImVec4 kInk(0.93f, 0.94f, 0.96f, 1.0f);
const ImVec4 kBody(0.74f, 0.77f, 0.82f, 1.0f);
const ImVec4 kMuted(0.53f, 0.57f, 0.63f, 1.0f);
const ImVec4 kAccentDark(0.15f, 0.34f, 0.70f, 1.0f);

enum Tab { kTabDisplay, kTabGraphics, kTabFrameRate, kTabAudio, kTabControls, kTabGame, kTabMods, kTabDebug, kTabCount };
constexpr const char* kTabNames[kTabCount] = {"Display", "Graphics", "Frame rate", "Audio", "Controls", "Game", "Mods", "Debug"};

// ---- the fefw.ini keys the menu edits -------------------------------------------------------------------------------------------------
// The value read at start is kept (boot) so a restart-only row can show what is in force and the bottom bar can list what waits.
enum class Norm { Text, Bool, Number };
struct IniKey {
    const char* sec;
    const char* key;
    const char* def;     // the default the runtime uses when the key is missing
    Norm norm;
    bool restart;        // read once at start
    const char* label;   // for the "waits for a restart" list
};
constexpr IniKey kIni[] = {
    {"fefw", "output", "console", Norm::Text, true, "output size"},
    {"fefw", "ultrawide", "auto", Norm::Text, true, "ultrawide"},
    {"fefw", "render_ratio", "0", Norm::Number, true, "render scale"},
    {"fefw", "render", "", Norm::Text, true, "fixed render size"},
    {"fefw", "shadow_size", "0", Norm::Number, true, "shadow resolution"},
    {"fefw", "quality", "console", Norm::Text, true, "quality"},
    {"fefw", "reflections", "full", Norm::Text, true, "reflections"},
    {"fefw", "ambient_occlusion", "1", Norm::Bool, true, "ambient occlusion"},
    {"fefw", "crowd_detail", "full", Norm::Text, true, "crowd detail"},
    {"fefw", "scenery_detail", "full", Norm::Text, true, "scenery detail"},
    {"fefw", "fps60", "0", Norm::Bool, true, "60 fps"},
    {"fefw", "fps120", "0", Norm::Bool, true, "120 fps"},
    {"fefw", "smooth_step", "auto", Norm::Text, true, "smooth time step"},
    {"fefw", "frame_fixes", "1", Norm::Bool, false, "frame-rate fixes"},
    {"fefw", "clock_log", "10", Norm::Number, false, "game clock log"},
    {"dlss", "upscaler", "taau", Norm::Text, true, "upscaler"},
    {"dlss", "frame_gen", "0", Norm::Text, true, "frame generation"},
    {"dlss", "preset", "k", Norm::Text, false, "DLSS model"},
    {"dlss", "sharpness", "0", Norm::Number, true, "FSR 3 sharpening"},
    {"dlss", "history_weight", "12", Norm::Number, true, "TAAU history"},
    {"media", "movies", "host", Norm::Text, true, "movies"},
    {"log", "level", "info", Norm::Text, false, "log level"},
    {"gfx", "log_fps", "1", Norm::Bool, true, "frame rate log"},
    {"play", "log_status", "0", Norm::Bool, true, "play status log"},
    {"input", "jc2_mouse", "auto", Norm::Text, false, "mouse controls"},
    {"diag", "hitch_summary_s", "10", Norm::Number, true, "frame summary log"},
};

const IniKey* ini_key(const char* sec, const char* key) {
    for (const IniKey& k : kIni)
        if (!std::strcmp(k.sec, sec) && !std::strcmp(k.key, key)) return &k;
    return nullptr;
}

std::string lower(std::string s) {
    for (char& c : s) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    return s;
}

// The value as it compares: booleans and numbers by meaning, text case-insensitively.
std::string normalized(const IniKey& k, const std::string& v) {
    const std::string l = lower(v);
    switch (k.norm) {
        case Norm::Bool: return (l == "1" || l == "true" || l == "yes" || l == "on") ? "1" : "0";
        case Norm::Number: {
            char buf[32];
            std::snprintf(buf, sizeof buf, "%g", std::atof(l.c_str()));
            return buf;
        }
        default: return l;
    }
}

// ---- the menu's state (one per process; the presenting thread only) --------------------------------------------------------------------
struct Menu {
    bool loaded = false;
    play::Settings edit;          // the working copy of controls.ini's settings: resynced from the host whenever no widget is held
    play::Settings boot;          // in force since the start: what the restart-only rows compare against
    std::map<std::string, std::string> boot_ini;   // "sec.key" -> the value at start (kIni)
    int tab = kTabDisplay, tab_request = -1;
    bool focus_first = false;     // the first row of focus_tab takes the gamepad cursor (a tab switched by L / R)
    int focus_tab = -1;
    int last_frame = -100;        // ImGui frame of the last draw: a gap means the menu was just opened
    bool busy_last = false;       // a popup was open or a widget held at the end of the last frame (B / Escape belonged to it)
    std::string row_help;         // the current row's description
    std::string help;             // the focused or hovered row's description (bottom bar)
    std::string notice;
    bool notice_err = false;
    double notice_until = 0;
    std::vector<std::string> monitors;
    double monitors_at = -1e9;
    std::vector<std::string> audio_devices;
    bool audio_loaded = false;
    // Debug page
    double phase_prev[kHpCount] = {}, phase_ms[kHpCount] = {};
    uint64_t frames_prev = 0;
    double sample_at = -1, frame_ms_avg = 0;
    // Debug page, CPU: sampled twice a second while the page is open
    cpu::Load cpu_load;
    std::vector<cpu::ThreadLoad> cpu_threads;
    double cpu_at = -1;
};

Menu& menu() {
    static Menu m;
    return m;
}

float px(float v) { return v * ImGui::GetFontSize() / 16.0f; }

// The menu gets its own compact, dark surface and blue accent. Keeping the
// style in one scope means the rest of the overlay (notifications, the hint
// bar) keeps the look it had before.
struct StyleScope {
    ImGuiStyle old;
    StyleScope() : old(ImGui::GetStyle()) {
        ImGuiStyle& s = ImGui::GetStyle();
        s.WindowRounding = px(16);
        s.ChildRounding = px(12);
        s.FrameRounding = px(8);
        s.PopupRounding = px(10);
        s.TabRounding = px(8);
        s.GrabRounding = px(8);
        s.ScrollbarRounding = px(8);
        s.WindowBorderSize = 1.0f;
        s.ChildBorderSize = 1.0f;
        s.FrameBorderSize = 1.0f;
        s.TabBarBorderSize = 0.0f;
        s.TabBarOverlineSize = 2.0f;
        s.WindowPadding = ImVec2(px(22), px(18));
        s.FramePadding = ImVec2(px(12), px(7));
        s.ItemSpacing = ImVec2(px(10), px(9));
        s.CellPadding = ImVec2(px(12), px(8));
        s.ScrollbarSize = px(12);
        const auto col = [&s](ImGuiCol which, float r, float g, float b, float a = 1.0f) { s.Colors[which] = ImVec4(r, g, b, a); };
        col(ImGuiCol_WindowBg, 0.031f, 0.039f, 0.051f, 0.99f);
        col(ImGuiCol_ChildBg, 0.043f, 0.055f, 0.071f, 1.0f);
        col(ImGuiCol_PopupBg, 0.055f, 0.071f, 0.094f, 1.0f);
        col(ImGuiCol_Border, 1.0f, 1.0f, 1.0f, 0.08f);
        col(ImGuiCol_FrameBg, 0.09f, 0.11f, 0.14f);
        col(ImGuiCol_FrameBgHovered, 0.13f, 0.17f, 0.23f);
        col(ImGuiCol_FrameBgActive, 0.17f, 0.24f, 0.34f);
        col(ImGuiCol_TableRowBg, 1.0f, 1.0f, 1.0f, 0.0f);
        col(ImGuiCol_TableRowBgAlt, 1.0f, 1.0f, 1.0f, 0.025f);
        col(ImGuiCol_Separator, 1.0f, 1.0f, 1.0f, 0.09f);
        col(ImGuiCol_Tab, 1.0f, 1.0f, 1.0f, 0.0f);
        col(ImGuiCol_TabHovered, 0.18f, 0.28f, 0.42f, 0.6f);
        col(ImGuiCol_TabSelected, 0.12f, 0.22f, 0.38f);
        col(ImGuiCol_TabSelectedOverline, 0.48f, 0.65f, 1.0f);
        col(ImGuiCol_TabDimmed, 1.0f, 1.0f, 1.0f, 0.0f);
        col(ImGuiCol_TabDimmedSelected, 0.08f, 0.14f, 0.24f);
        col(ImGuiCol_Button, 0.10f, 0.13f, 0.17f);
        col(ImGuiCol_ButtonHovered, 0.15f, 0.20f, 0.27f);
        col(ImGuiCol_ButtonActive, 0.18f, 0.32f, 0.55f);
        col(ImGuiCol_Header, 0.12f, 0.24f, 0.42f);
        col(ImGuiCol_HeaderHovered, 0.16f, 0.30f, 0.52f);
        col(ImGuiCol_HeaderActive, 0.18f, 0.34f, 0.60f);
        col(ImGuiCol_CheckMark, 0.48f, 0.65f, 1.0f);
        col(ImGuiCol_SliderGrab, 0.35f, 0.55f, 0.90f);
        col(ImGuiCol_SliderGrabActive, 0.55f, 0.72f, 1.0f);
        col(ImGuiCol_ScrollbarBg, 0.02f, 0.025f, 0.035f);
        col(ImGuiCol_ScrollbarGrab, 0.20f, 0.24f, 0.31f);
        col(ImGuiCol_TextSelectedBg, 0.24f, 0.42f, 0.75f, 0.45f);
    }
    ~StyleScope() { ImGui::GetStyle() = old; }
};

// A small rounded tag in a table row, for "restart" and the file checks.
void pill(const char* text, const ImVec4& tint) {
    const ImVec2 pad(px(7), px(3));
    const ImVec2 text_size = ImGui::CalcTextSize(text);
    const float height = ImGui::GetTextLineHeight() + pad.y * 2;
    const ImVec2 p = ImGui::GetCursorScreenPos();
    ImDrawList* dl = ImGui::GetWindowDrawList();
    dl->AddRectFilled(ImVec2(p.x, p.y + px(1)), ImVec2(p.x + text_size.x + pad.x * 2, p.y + px(1) + height),
                      ImGui::GetColorU32(ImVec4(tint.x, tint.y, tint.z, 0.16f)), px(999));
    dl->AddText(ImGui::GetFont(), ImGui::GetFontSize(), ImVec2(p.x + pad.x, p.y + px(1) + pad.y), ImGui::GetColorU32(tint), text);
    ImGui::Dummy(ImVec2(text_size.x + pad.x * 2, height + px(1)));
}

// The EmblemWeave title block: logo, page title, game name, FPS and the pad hint (the custom shell of 0.1.34).
void draw_header(const play::LiveStatus& live) {
    const float width = ImGui::GetContentRegionAvail().x;
    const float height = px(58);
    const ImVec2 p = ImGui::GetCursorScreenPos();
    ImDrawList* dl = ImGui::GetWindowDrawList();
    dl->AddRectFilled(p, ImVec2(p.x + width, p.y + height), ImGui::GetColorU32(ImVec4(0.055f, 0.071f, 0.094f, 1.0f)), px(12));
    dl->AddRect(p, ImVec2(p.x + width, p.y + height), ImGui::GetColorU32(ImVec4(1.0f, 1.0f, 1.0f, 0.08f)), px(12), 0, 1.0f);

    const float mark = px(30);
    const ImVec2 mark_at(p.x + px(14), p.y + (height - mark) * 0.5f);
    dl->AddRectFilled(mark_at, ImVec2(mark_at.x + mark, mark_at.y + mark), ImGui::GetColorU32(kAccentDark), px(9));
    const char* logo = "EW";
    const ImVec2 logo_size = ImGui::GetFont()->CalcTextSizeA(px(11), FLT_MAX, 0.0f, logo);
    dl->AddText(ImGui::GetFont(), px(11), ImVec2(mark_at.x + (mark - logo_size.x) * 0.5f, mark_at.y + (mark - logo_size.y) * 0.5f),
                IM_COL32(255, 255, 255, 255), logo);

    dl->AddText(ImGui::GetFont(), px(18), ImVec2(p.x + px(56), p.y + px(11)), ImGui::GetColorU32(kInk), tr("Settings"));
    dl->AddText(ImGui::GetFont(), px(10.5f), ImVec2(p.x + px(57), p.y + px(34)), ImGui::GetColorU32(kMuted), tr("Fire Emblem: Fortune's Weave"));

    const float small = px(10.5f);
    char fps[48];
    std::snprintf(fps, sizeof fps, "%.0f FPS", live.fps);
    const ImVec2 fps_size = ImGui::GetFont()->CalcTextSizeA(small, FLT_MAX, 0.0f, fps);
    dl->AddText(ImGui::GetFont(), small, ImVec2(p.x + width - px(16) - fps_size.x, p.y + px(13)), ImGui::GetColorU32(kBody), fps);
    const char* hint = tr("L / R tabs   A select   B close");
    const ImVec2 hint_size = ImGui::GetFont()->CalcTextSizeA(small, FLT_MAX, 0.0f, hint);
    dl->AddText(ImGui::GetFont(), small, ImVec2(p.x + width - px(16) - hint_size.x, p.y + px(34)), ImGui::GetColorU32(kMuted), hint);
    dl->AddRectFilled(ImVec2(p.x + px(14), p.y + height - px(2)), ImVec2(p.x + width - px(14), p.y + height - px(1)),
                      ImGui::GetColorU32(kAccent), px(1));
    ImGui::Dummy(ImVec2(0.0f, height));
    ImGui::Spacing();
}

void notify(Menu& m, const std::string& text, bool err = false) {
    m.notice = text;
    m.notice_err = err;
    m.notice_until = ImGui::GetTime() + (err ? 8.0 : 4.0);
}

std::string ini_now(const IniKey& k) { return config_get_str(k.sec, k.key, k.def); }
bool ini_env(const IniKey& k) { return std::strcmp(config_source(k.sec, k.key), "env") == 0; }
const std::string& ini_boot(Menu& m, const IniKey& k) {
    const std::string id = std::string(k.sec) + "." + k.key;
    auto it = m.boot_ini.find(id);
    if (it == m.boot_ini.end()) it = m.boot_ini.emplace(id, ini_now(k)).first;
    return it->second;
}
bool ini_pending(Menu& m, const IniKey& k) { return k.restart && normalized(k, ini_now(k)) != normalized(k, ini_boot(m, k)); }

void ini_put(Menu& m, const IniKey& k, const std::string& value) {
    ini_boot(m, k);   // the value in force is known before the first write
    std::string err;
    if (config_write(k.sec, k.key, value, &err)) {
        NXLOG_I(kTag, "[%s] %s = %s written to %s%s", k.sec, k.key, value.c_str(), config_file_path().c_str(),
                k.restart ? " (applies on restart)" : "");
        notify(m, std::string("Saved [") + k.sec + "] " + k.key + " = " + value + (k.restart ? "  (applies on restart)" : ""));
    } else {
        NXLOG_W(kTag, "cannot write [%s] %s: %s", k.sec, k.key, err.c_str());
        notify(m, "Could not save [" + std::string(k.sec) + "] " + k.key + ": " + err, true);
    }
}

void apply(UiHost& host, Menu& m) { host.apply(m.edit); }

// ---- widgets: a two-column table of rows, label left, the control right ----------------------------------------------------------------
enum class When { Live, Restart, Info };

// ImGui reads a key's state at io.KeysData[key - ImGuiKey_NamedKey_BEGIN] without a range check (asserts are off in this build), so
// ImGuiKey_None must never reach IsKeyPressed: it read 8 KiB before the ImGui context, and crashed when that page was not mapped.
bool pressed(ImGuiKey a, ImGuiKey b) { return ImGui::IsKeyPressed(a, true) || ImGui::IsKeyPressed(b, true); }

// Left / right on the focused, not held, row: -1 / +1, and the cursor stays where it is.
int step_dir() {
    if (!ImGui::IsItemFocused() || ImGui::IsItemActive()) return 0;
    int d = 0;
    if (pressed(ImGuiKey_GamepadDpadLeft, ImGuiKey_LeftArrow)) d = -1;
    else if (pressed(ImGuiKey_GamepadDpadRight, ImGuiKey_RightArrow)) d = 1;
    if (d) ImGui::NavMoveRequestCancel();
    return d;
}

void after_item(Menu& m) {
    if (ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenDisabled) || ImGui::IsItemFocused()) m.help = m.row_help;
}

bool begin_rows(const char* id) {
    if (!ImGui::BeginTable(id, 2, ImGuiTableFlags_SizingStretchProp | ImGuiTableFlags_PadOuterX | ImGuiTableFlags_RowBg |
                                      ImGuiTableFlags_BordersInnerH)) return false;
    ImGui::TableSetupColumn("##label", ImGuiTableColumnFlags_WidthStretch, 0.40f);
    ImGui::TableSetupColumn("##value", ImGuiTableColumnFlags_WidthStretch, 0.60f);
    return true;
}
void end_rows() { ImGui::EndTable(); }

void section(const char* title) {
    ImGui::Spacing();
    ImGui::PushStyleColor(ImGuiCol_Text, kAccent);
    ImGui::SeparatorText(tr(title));
    ImGui::PopStyleColor();
    ImGui::Spacing();
}

// Starts a row: the label (with a "restart" tag for what applies at the next start), then the control's cell, full width.
void row(Menu& m, const char* label, When when, const char* help) {
    ImGui::TableNextRow(ImGuiTableRowFlags_None, px(38));
    ImGui::TableSetColumnIndex(0);
    ImGui::AlignTextToFramePadding();
    ImGui::TextUnformatted(tr(label));
    if (when == When::Restart) {
        ImGui::SameLine();
        pill(tr("restart"), kRestartTag);
        if (ImGui::IsItemHovered()) ImGui::SetTooltip("%s", tr("Applies on restart: the game reads this once, when it starts."));
    }
    ImGui::TableSetColumnIndex(1);
    ImGui::SetNextItemWidth(-FLT_MIN);
    m.row_help = help ? help : "";
    if (m.focus_first && m.tab == m.focus_tab) {
        ImGui::SetKeyboardFocusHere();
        ImGui::SetNavCursorVisible(true);   // A acts on the focused row only while the cursor shows
        m.focus_first = false;
    }
}

// Under a restart-only control: what runs until the restart.
void pending_note(bool pending, const std::string& running) {
    if (!pending) return;
    ImGui::TextColored(kOrange, tr("Applies on restart (running: %s)"), running.empty() ? tr("default") : running.c_str());
}

// A value row that is set by an environment variable: shown, locked.
bool env_locked(const IniKey& k) {
    if (!ini_env(k)) return false;
    ImGui::TextDisabled(tr("%s (set by %s)"), ini_now(k).c_str(), config_env_name(k.sec, k.key).c_str());
    return true;
}

// A list. *idx may be -1 (a value no entry holds): `other` is shown then. Left / right step through the entries.
bool combo(Menu& m, const char* id, int* idx, const char* const* names, int n, const char* other = "Custom",
           bool other_is_text = false) {  // tr() at the sites
    bool changed = false;
    const char* preview = *idx >= 0 && *idx < n ? tr(names[*idx]) : other_is_text ? other : tr(other);
    if (ImGui::BeginCombo(id, preview, ImGuiComboFlags_HeightLarge)) {
        for (int i = 0; i < n; ++i) {
            const bool sel = i == *idx;
            if (ImGui::Selectable(tr(names[i]), sel) && !sel) {
                *idx = i;
                changed = true;
            }
            if (sel) ImGui::SetItemDefaultFocus();
        }
        ImGui::EndCombo();
    }
    after_item(m);
    if (const int d = step_dir()) {
        const int to = std::clamp((*idx < 0 ? 0 : *idx) + d, 0, n - 1);
        if (to != *idx) {
            *idx = to;
            changed = true;
        }
    }
    return changed;
}

bool check(Menu& m, const char* id, bool* v) {
    bool changed = ImGui::Checkbox(id, v);
    after_item(m);
    if (const int d = step_dir()) {
        const bool to = d > 0;
        if (to != *v) {
            *v = to;
            changed = true;
        }
    }
    return changed;
}

// A slider; true when the value is to be applied (released after a drag, or stepped with left / right). *dragging: it moved this frame.
bool slider(Menu& m, const char* id, double* v, double lo, double hi, double step, const char* fmt, bool* dragging = nullptr,
            ImGuiSliderFlags flags = 0) {
    float f = static_cast<float>(*v);
    const bool moved = ImGui::SliderFloat(id, &f, static_cast<float>(lo), static_cast<float>(hi), fmt, flags | ImGuiSliderFlags_AlwaysClamp);
    after_item(m);
    if (moved) *v = f;
    bool commit = ImGui::IsItemDeactivatedAfterEdit();
    if (const int d = step_dir()) {
        const double to = std::clamp(std::round((*v + d * step) / step) * step, lo, hi);
        if (to != *v) {
            *v = to;
            commit = true;
        }
    }
    if (dragging) *dragging = moved;
    return commit;
}

// A slider whose 0 means "auto" (zero_label) and whose other values start at lo: 1 .. lo-1 becomes lo, except when the value came down
// from lo or above (a step left from the minimum), which goes to 0, so 0 is reachable with a gamepad or the keyboard too.
bool zero_or_min_slider(Menu& m, const char* id, double* v, double lo, double hi, double step, const char* zero_label) {
    const double before = *v;
    if (!slider(m, id, v, 0, hi, step, *v <= 0 ? zero_label : "%.0f nits")) return false;
    if (*v > 0 && *v < lo) *v = before >= lo && *v < before ? 0 : lo;
    return true;
}

bool islider(Menu& m, const char* id, int* v, int lo, int hi, int step, const char* fmt) {
    int x = *v;
    const bool moved = ImGui::SliderInt(id, &x, lo, hi, fmt, ImGuiSliderFlags_AlwaysClamp);
    after_item(m);
    if (moved) *v = x;
    bool commit = ImGui::IsItemDeactivatedAfterEdit();
    if (const int d = step_dir()) {
        const int to = std::clamp(*v + d * step, lo, hi);
        if (to != *v) {
            *v = to;
            commit = true;
        }
    }
    return commit;
}

bool button(Menu& m, const char* label) {
    const bool b = ImGui::Button(tr(label));
    after_item(m);
    return b;
}

// A fefw.ini list row: the entries' values in `vals`, the label of the running value when it is pending.
void ini_combo_row(Menu& m, const IniKey& k, const char* label, const char* help, const char* const* names, const char* const* vals, int n,
                   const char* other = "Custom (set in the ini)") {
    row(m, label, k.restart ? When::Restart : When::Live, help);
    if (env_locked(k)) return;
    const std::string now = normalized(k, ini_now(k));
    int idx = -1;
    for (int i = 0; i < n; ++i)
        if (normalized(k, vals[i]) == now) idx = i;
    ImGui::PushID(k.key);
    if (combo(m, "##v", &idx, names, n, other)) ini_put(m, k, vals[idx]);
    ImGui::PopID();
    if (k.restart && ini_pending(m, k)) {
        const std::string boot = normalized(k, ini_boot(m, k));
        std::string running = ini_boot(m, k);
        for (int i = 0; i < n; ++i)
            if (normalized(k, vals[i]) == boot) running = names[i];
        pending_note(true, running);
    }
}

// ---- Display ----------------------------------------------------------------------------------------------------------------------------
constexpr const char* kWindowModeNames[] = {"Windowed", "Borderless fullscreen", "Exclusive fullscreen"};
constexpr const char* kWindowModeKeys[] = {"windowed", "borderless", "exclusive"};
constexpr int kSizes[][2] = {{1280, 720}, {1600, 900}, {1920, 1080}, {2560, 1440}, {3840, 2160}};
constexpr const char* kSizeNames[] = {"1280 x 720", "1600 x 900", "1920 x 1080", "2560 x 1440", "3840 x 2160"};
constexpr const char* kCornerNames[] = {"Top left", "Top right", "Bottom left", "Bottom right"};
constexpr const char* kDetailNames[] = {"Frame rate", "Frame rate and frame time", "Plus 1% low and the worst frame"};
constexpr const char* kScaleNames[] = {"Bilinear (plain)", "FSR 1 (AMD, sharpened)", "NIS (NVIDIA Image Scaling)"};
constexpr extras::UpscaleMode kScaleModes[] = {extras::UpscaleMode::Off, extras::UpscaleMode::Fsr1, extras::UpscaleMode::Nis};

void refresh_monitors(Menu& m) {
    const double now = ImGui::GetTime();
    if (now - m.monitors_at < 2.0) return;
    m.monitors_at = now;
    m.monitors.clear();
    const int n = platform::display_count();
    for (int i = 1; i <= n; ++i) {
        char buf[192];
        if (platform::display_name(i, buf, sizeof buf)) m.monitors.emplace_back(buf);
    }
}

void display_page(UiHost& host, Menu& m, const play::LiveStatus& live) {
    play::DisplaySettings& d = m.edit.display;
    section("Window");
    if (begin_rows("win")) {
        row(m, "Window mode", When::Live, "Borderless fullscreen keeps the desktop resolution and switches at once; exclusive takes over the display. Alt+Enter flips windowed and fullscreen.");
        int mode = 0;
        for (int i = 0; i < 3; ++i)
            if (d.window_mode == kWindowModeKeys[i]) mode = i;
        if (combo(m, "##wm", &mode, kWindowModeNames, 3)) {
            d.window_mode = kWindowModeKeys[mode];
            apply(host, m);
        }

        refresh_monitors(m);
        row(m, "Monitor", When::Live, "The display the window moves to; the fullscreen modes use it.");
        {
            std::vector<const char*> names = {"Automatic (where the window is)"};
            for (const std::string& s : m.monitors) names.push_back(s.c_str());
            int idx = d.monitor <= static_cast<int>(m.monitors.size()) ? d.monitor : -1;
            if (combo(m, "##mon", &idx, names.data(), static_cast<int>(names.size()), "Not connected")) {
                d.monitor = idx;
                apply(host, m);
            }
        }

        row(m, "Window size", When::Live, "The client size in windowed mode. It changes at once while windowed; in fullscreen it is the size the window goes back to.");
        int sz = -1;
        for (int i = 0; i < 5; ++i)
            if (kSizes[i][0] == d.width && kSizes[i][1] == d.height) sz = i;
        char custom[48];
        std::snprintf(custom, sizeof custom, "%d x %d", d.width, d.height);
        if (combo(m, "##ws", &sz, kSizeNames, 5, custom)) {
            d.width = kSizes[sz][0];
            d.height = kSizes[sz][1];
            apply(host, m);
        }

        row(m, "Keep 16:9, letterbox the rest", When::Live, "Off stretches the picture to the window.");
        if (check(m, "##ka", &d.keep_aspect)) apply(host, m);
        end_rows();
    }
    ImGui::TextDisabled(tr("Now: %s, %d x %d, %.2f Hz display, present mode %s"), live.window_mode, live.width, live.height,
                        static_cast<double>(live.refresh_hz), live.present_mode);

    section("Scaling to the window");
    if (begin_rows("scale")) {
        extras::UpscaleSettings& u = m.edit.extras.upscale;
        row(m, "Filter", When::Live,
            "How the game's output (Graphics > Output size) is scaled to the window when they differ. FSR 1 and NIS sharpen while they enlarge; "
            "a smaller window is always averaged.");
        int idx = -1;
        for (int i = 0; i < 3; ++i)
            if (u.mode == kScaleModes[i]) idx = i;
        if (combo(m, "##flt", &idx, kScaleNames, 3, "Other (set in controls.ini)")) {
            u.mode = kScaleModes[idx];
            apply(host, m);
        }
        if (u.mode == extras::UpscaleMode::Fsr1 || u.mode == extras::UpscaleMode::Nis) {
            row(m, "Sharpness", When::Live, "1 is the sharpest.");
            if (slider(m, "##shp", &u.sharpness, 0.0, 1.0, 0.05, "%.2f")) apply(host, m);
        }
        end_rows();
    }

    section("Frame rate counter");
    if (begin_rows("fps")) {
        play::FpsSettings& f = m.edit.fps;
        row(m, "Show the counter", When::Live, "A small frame rate counter in a corner. The hotkey flips it in the game.");
        if (check(m, "##show", &f.show)) apply(host, m);
        row(m, "Corner", When::Live, nullptr);
        int corner = static_cast<int>(f.corner);
        if (combo(m, "##corner", &corner, kCornerNames, 4)) {
            f.corner = static_cast<play::Corner>(corner);
            apply(host, m);
        }
        row(m, "Detail", When::Live, nullptr);
        int detail = std::clamp(f.detail, 0, 2);
        if (combo(m, "##detail", &detail, kDetailNames, 3)) {
            f.detail = detail;
            apply(host, m);
        }
        row(m, "Also in screenshots", When::Live, "Screenshots leave the counter out unless this is on.");
        if (check(m, "##shots", &f.in_screenshots)) apply(host, m);
        row(m, "Hotkey", When::Info, "[fps] key in controls.ini.");
        const std::string key = play::input_name(f.key);
        ImGui::AlignTextToFramePadding();
        ImGui::TextUnformatted(key.empty() ? tr("none") : key.c_str());
        end_rows();
    }
}

// ---- Graphics ---------------------------------------------------------------------------------------------------------------------------
constexpr const char* kOutputNames[] = {"Console (the game's choice: 1440p docked)", "1280 x 720", "1920 x 1080", "2560 x 1440", "3840 x 2160",
                                        "2560 x 1080 (21:9)", "3440 x 1440 (21:9)", "5120 x 1440 (32:9, drawn at 3840 x 1080)", "The window's size at start"};
constexpr const char* kOutputVals[] = {"console", "720p", "1080p", "1440p", "2160p", "2560x1080", "3440x1440", "5120x1440", "window"};
constexpr int kOutputCount = static_cast<int>(std::size(kOutputVals));
constexpr const char* kWideNames[] = {"Auto (outputs wider than 16:9)", "Off (16:9, stretched)"};
constexpr const char* kWideVals[] = {"auto", "off"};
constexpr const char* kRatioNames[] = {"Game default (half per side)", "33% (ultra performance)", "50% (performance)", "59% (balanced)",
                                       "67% (quality)", "75%", "85%", "100% (native: anti-aliasing only)"};
constexpr const char* kRatioVals[] = {"0", "0.33", "0.5", "0.59", "0.67", "0.75", "0.85", "1.0"};
constexpr const char* kShadowNames[] = {"Game default (2048 docked)", "1024", "1536", "2048", "3072", "4096", "6144", "8192"};
constexpr const char* kShadowVals[] = {"0", "1024", "1536", "2048", "3072", "4096", "6144", "8192"};
constexpr const char* kQualityNames[] = {"Console (handheld configs keep their cuts)", "Docked quality in every config",
                                        "Handheld quality in every config (lighter)"};
constexpr const char* kQualityVals[] = {"console", "docked", "handheld"};
constexpr const char* kReflectionNames[] = {"Full", "Reduced (8 rays, a little grainier)", "Off"};
constexpr const char* kReflectionVals[] = {"full", "reduced", "off"};
constexpr const char* kDetailOnOffNames[] = {"On", "Off"};
constexpr const char* kDetailOnOffVals[] = {"1", "0"};
constexpr const char* kDetailLevelNames[] = {"Full", "Reduced"};
constexpr const char* kDetailLevelVals[] = {"full", "reduced"};
constexpr const char* kPresetNames[] = {"Low", "Medium (console)", "High", "Ultra"};
static_assert(std::size(kPresetNames) == kGraphicsPresetCount);
constexpr const char* kUpscalerNames[] = {"TAAU (built in, temporal)", "DLSS (NVIDIA RTX, through Streamline)", "FSR 3 (AMD FidelityFX)",
                                          "XeSS (Intel, any GPU with DP4a)", "Bilinear (no anti-aliasing)", "Auto (DLSS, else XeSS, else FSR 3, else TAAU)"};
constexpr const char* kUpscalerVals[] = {"taau", "dlss", "fsr3", "xess", "bilinear", "auto"};
constexpr const char* kFgNames[] = {"Off", "DLSS Frame Generation (RTX 40 or newer)", "FSR 3 frame generation"};
constexpr const char* kFgVals[] = {"0", "dlss", "fsr3"};
constexpr const char* kHdrModeNames[] = {"Off", "Auto", "On", "Native"};
constexpr const char* kHdrFormatNames[] = {"Auto", "HDR10", "scRGB"};

// The output size an [fefw] output value means (0 x 0: the config's own), as fefw_res.cpp works it out: a window is fitted to 16:9 unless
// it is wider and ultrawide is on; with ultrawide on a narrower size gets 16:9 inside it.
void output_size(const std::string& v, bool wide, int* w, int* h) {
    *w = *h = 0;
    const std::string o = lower(v);
    if (o == "720p") *w = 1280, *h = 720;
    else if (o == "1080p") *w = 1920, *h = 1080;
    else if (o == "1440p") *w = 2560, *h = 1440;
    else if (o == "2160p" || o == "4k") *w = 3840, *h = 2160;
    else if (o == "window") {
        platform::pixel_size(w, h);
        if (*w > 0 && *h > 0 && !(wide && wide_aspect(*w, *h))) {
            const int fw = std::min(*w, *h * 16 / 9);
            *w = fw & ~1;
            *h = (fw * 9 / 16) & ~1;
        }
    } else std::sscanf(o.c_str(), "%dx%d", w, h);
    if (wide && *w > 0 && *h > 0 && static_cast<double>(*w) * 9.0 * 1.01 < static_cast<double>(*h) * 16.0) *h = (*w * 9 / 16) & ~1;
    if (wide && wide_aspect(*w, *h) && (*w > 3840 || *h > 2160)) {   // the shape within the 3840x2160 window textures
        const double s = std::min(3840.0 / *w, 2160.0 / *h);
        *w = 2 * static_cast<int>(std::lround(*w * s / 2));
        *h = 2 * static_cast<int>(std::lround(*h * s / 2));
    }
}

void resolution_group(Menu& m) {
    section("Resolution");
    ImGui::TextWrapped("%s", tr("The game chooses its render configuration once, at start: these apply on restart (notes/resolution.md)."));
    if (begin_rows("res")) {
        // the output: the entries, or a WxH from the ini shown as it is
        const IniKey& out = *ini_key("fefw", "output");
        {
            const std::string v = lower(ini_now(out));
            const char* cur = v == "4k" ? "2160p" : v.c_str();
            row(m, "Output size", When::Restart,
                "The size the game renders its picture, UI and post effects at (its DLSS output). Up to 3840 x 2160 at 16:9; wider shapes "
                "need Ultrawide. The window scales it (Display > Scaling to the window).");
            if (!env_locked(out)) {
                int idx = -1;
                for (int i = 0; i < kOutputCount; ++i)
                    if (cur == std::string(kOutputVals[i])) idx = i;
                const std::string other = std::string(tr("Custom")) + ": " + ini_now(out);
                if (combo(m, "##out", &idx, kOutputNames, kOutputCount, other.c_str(), true)) ini_put(m, out, kOutputVals[idx]);
                pending_note(ini_pending(m, out), ini_boot(m, out));
            }
        }
        ini_combo_row(m, *ini_key("fefw", "ultrawide"), "Ultrawide",
                      "An output wider than 16:9 (21:9, 32:9, or a wide window with Output size = the window's size) widens the 3D view and "
                      "keeps the menus, HUD, text and movies in the centred 16:9 area, after Kompas9's ultrawide patch. A narrower output "
                      "(16:10) gets the 16:9 picture letterboxed. Off: the game's 16:9 picture is stretched to a wide output.",
                      kWideNames, kWideVals, 2);
        ini_combo_row(m, *ini_key("fefw", "render_ratio"), "Render scale",
                      "The 3D render size per side as a share of the output; the game's upscaler (Graphics > Upscaler) fills the rest. "
                      "100% renders at the output size and only anti-aliases.",
                      kRatioNames, kRatioVals, 8);
        const IniKey& fixed = *ini_key("fefw", "render");
        if (!ini_now(fixed).empty() || ini_pending(m, fixed)) {
            row(m, "Fixed render size", When::Restart, "[fefw] render = WxH wins over the render scale. Clear it to use the scale.");
            if (!env_locked(fixed)) {
                ImGui::AlignTextToFramePadding();
                ImGui::TextUnformatted(ini_now(fixed).empty() ? tr("(cleared)") : ini_now(fixed).c_str());
                if (!ini_now(fixed).empty()) {
                    ImGui::SameLine();
                    if (button(m, "Clear##render")) ini_put(m, fixed, "");
                }
                pending_note(ini_pending(m, fixed), ini_boot(m, fixed));
            }
        }
        ini_combo_row(m, *ini_key("fefw", "shadow_size"), "Shadow resolution", "The shadow map's base size. Larger is sharper and costs GPU time and memory (8192: about 1.3 GB more).",
                      kShadowNames, kShadowVals, 8);
        ini_combo_row(m, *ini_key("fefw", "quality"), "Quality",
                      "Docked gives every render configuration the docked one's shadows and effects, so a handheld or 1080p answer keeps them. "
                      "Handheld gives every one the handheld configuration's lighter shadows and effects, for a slower GPU.",
                      kQualityNames, kQualityVals, 3);
        end_rows();
    }
    // what the chosen sizes mean
    int ow = 0, oh = 0;
    const bool wide_next = lower(ini_now(*ini_key("fefw", "ultrawide"))) != "off";
    output_size(ini_now(*ini_key("fefw", "output")), wide_next, &ow, &oh);
    const double ratio = std::atof(ini_now(*ini_key("fefw", "render_ratio")).c_str());
    if (ow > 0 && oh > 0) {
        int rw = ratio > 0 ? 2 * static_cast<int>(std::lround(ow * std::clamp(ratio, 0.25, 1.0) / 2)) : (ow + 1) / 2;
        int rh = ratio > 0 ? 2 * static_cast<int>(std::lround(oh * std::clamp(ratio, 0.25, 1.0) / 2)) : (oh + 1) / 2;
        const bool capped = !res_rt_pool_extended() && static_cast<int64_t>(ow) * oh > 2560 * 1440 && (rw > (ow + 1) / 2 || rh > (oh + 1) / 2);
        if (capped) rw = (ow + 1) / 2, rh = (oh + 1) / 2;
        ImGui::TextDisabled(tr("Next start: %d x %d rendered into %d x %d%s%s"), rw, rh, ow, oh,
                            capped ? " (above 1440p the game's render-target pool holds half per side at most)" : "",
                            wide_next && wide_aspect(ow, oh) ? ", ultrawide" : "");
    }
    const WideInfo wi = wide_info();
    if (wi.active)
        ImGui::TextDisabled(tr("Ultrawide now: %d x %d, %.2fx a 16:9 picture's width; UI in the centred %d x %d"), wi.out_w, wi.out_h,
                            static_cast<double>(wi.k), static_cast<int>(std::lround(wi.out_h * 16.0 / 9.0)), wi.out_h);
}

void upscaler_group(Menu& m) {
    section("Upscaler and frame generation");
    if (begin_rows("ups")) {
        const IniKey& up = *ini_key("dlss", "upscaler");
        ini_combo_row(m, up, "Upscaler",
                      "What answers the game's DLSS calls. TAAU needs nothing; DLSS needs an RTX GPU and the Streamline DLLs ([upscale] dlss_dll); "
                      "FSR 3 runs on any GPU with AMD's FidelityFX DLLs (upscalers\\fsr next to the launcher, or [upscale] fsr_dll); "
                      "XeSS runs on any GPU with DP4a (best on Intel Arc) and needs libxess.dll (upscalers\\xess next to the launcher, or "
                      "[upscale] xess_dll) and the Visual C++ 2015-2022 runtime. One that cannot run falls back to TAAU and the log names the "
                      "missing file. Choosing DLSS or XeSS (or Auto with the DLLs present) takes effect at the next start.",
                      kUpscalerNames, kUpscalerVals, 6);
        const std::string u = lower(ini_now(up));
        if (u == "dlss" || u == "auto") {
            const IniKey& pk = *ini_key("dlss", "preset");
            ini_combo_row(m, pk, "DLSS model",
                          "The neural network DLSS runs. The transformer models (K, J, L, M) look best; L and M are the second generation, M the faster "
                          "of them. Default lets the DLSS file decide (K at full render size). A DLSS file without the chosen model uses its default. "
                          "Applies at once.",
                          std::array<const char*, 5>{"Default (the DLSS file decides)", "K (transformer)", "J (transformer)", "L (transformer, 2nd generation)",
                                                     "M (transformer, 2nd generation, faster)"}.data(),
                          std::array<const char*, 5>{"default", "k", "j", "l", "m"}.data(), 5);
            extras::set_dlss_preset(std::max(0, extras::dlss_preset_from_word(ini_now(pk))));
        }
        if (u == "fsr3")
            ini_combo_row(m, *ini_key("dlss", "sharpness"), "FSR 3 sharpening", nullptr,
                          std::array<const char*, 5>{"Off", "0.25", "0.5", "0.75", "1 (sharpest)"}.data(),
                          std::array<const char*, 5>{"0", "0.25", "0.5", "0.75", "1"}.data(), 5);
        if (u == "taau" || u.empty())
            ini_combo_row(m, *ini_key("dlss", "history_weight"), "TAAU history",
                          "How much of the history TAAU keeps: higher is smoother and slower to follow change.",
                          std::array<const char*, 5>{"6 (sharper in motion)", "8", "12 (default)", "16", "24 (smoother)"}.data(),
                          std::array<const char*, 5>{"6", "8", "12", "16", "24"}.data(), 5);
        const IniKey& fg = *ini_key("dlss", "frame_gen");
        {
            row(m, "Frame generation", When::Restart,
                "One generated frame per rendered frame; the HUD is taken from the real frame. Keep vsync and low latency off. DLSS-G needs "
                "an RTX 40 or newer GPU, the Streamline DLLs (with the five frame generation adds) and Windows' hardware-accelerated GPU "
                "scheduling; FSR 3 runs on any GPU and needs amd_fidelityfx_vk.dll v1.1.4 (in upscalers\\fsr next to the launcher).");
            if (!env_locked(fg)) {
                std::string v = lower(ini_now(fg));
                if (v == "1") v = "dlss";
                if (v == "3") v = "fsr3";
                int idx = -1;
                for (int i = 0; i < 3; ++i)
                    if (v == kFgVals[i]) idx = i;
                if (combo(m, "##fg", &idx, kFgNames, 3, ("Other: " + ini_now(fg)).c_str())) ini_put(m, fg, kFgVals[idx]);
                pending_note(ini_pending(m, fg), ini_boot(m, fg));
            }
        }
        end_rows();
    }
    const char* in_use = nvn::be::dlss_upscaler_in_use();
    ImGui::TextDisabled(tr("Running: %s; frame generation %s."), in_use ? in_use : "no DLSS call yet",
                        extras::framegen_vsync_paused() ? "paused: vsync is on and the display is under twice the game's frame rate"
                        : extras::framegen_running()    ? "on"
                        : extras::framegen_requested()  ? "asked for, not running (see the log)"
                                                        : "off");
    // "found" only when every file the feature needs is there (the log names a missing one)
    ImGui::Spacing();
    ImGui::TextDisabled("%s", tr("Files found"));
    if (ImGui::BeginTable("##ups_files", 2, ImGuiTableFlags_SizingStretchSame | ImGuiTableFlags_PadOuterX)) {
        const struct {
            const char* name;
            bool found;
        } checks[] = {
            {"DLSS", extras::temporal_dlss_available()},
            {"DLSS-G", extras::streamline_framegen_files()},
            {"FSR 3 upscaling", extras::temporal_ffx_available()},
            {"FSR 3 frame generation", extras::ffx_fg_files()},
            {"XeSS", extras::temporal_xess_available()},
        };
        for (const auto& check_row : checks) {
            ImGui::TableNextColumn();
            pill(check_row.found ? tr("ready") : tr("missing"), check_row.found ? kGreen : kRed);
            ImGui::SameLine();
            ImGui::TextUnformatted(tr(check_row.name));
        }
        ImGui::EndTable();
    }
    ImGui::PushTextWrapPos(0.0f);
    ImGui::TextDisabled("%s", tr("Put the vendor files beside the game, in upscalers\\<name>, or anywhere in the engine folder tree. The runtime checks the game, launcher and engine folders at startup."));
    ImGui::PopTextWrapPos();
    if (extras::framegen_requested() && (m.edit.display.vsync || m.edit.extras.latency.mode != extras::LatencyMode::Off))
        ImGui::TextColored(kOrange, "%s", tr("Frame generation works best with vsync and low latency off (Frame rate tab). With vsync on, the game "
                                             "renders at most half the display's refresh rate."));
}

void hdr_group(UiHost& host, Menu& m) {
    extras::HdrSettings& h = m.edit.extras.hdr;
    section("HDR");
    if (begin_rows("hdr")) {
        row(m, "HDR output", When::Restart, "Off keeps SDR; Auto uses HDR when the display and Windows offer it; On runs the HDR pipeline anyway; Native uses the levels the game declares for an HDR TV.");
        int mode = static_cast<int>(h.mode);
        if (combo(m, "##hm", &mode, kHdrModeNames, 4)) {
            h.mode = static_cast<extras::HdrMode>(mode);
            apply(host, m);
        }
        pending_note(h.mode != m.boot.extras.hdr.mode, kHdrModeNames[static_cast<int>(m.boot.extras.hdr.mode)]);
        row(m, "Signal", When::Restart, "HDR10 or scRGB; Auto picks HDR10 when the display offers it.");
        int fmt = static_cast<int>(h.format);
        if (combo(m, "##hf", &fmt, kHdrFormatNames, 3)) {
            h.format = static_cast<extras::HdrFormat>(fmt);
            apply(host, m);
        }
        pending_note(h.format != m.boot.extras.hdr.format, kHdrFormatNames[static_cast<int>(m.boot.extras.hdr.format)]);
        row(m, "Peak brightness", When::Live, "Nits of the brightest highlights; 0 = what the display reports.");
        if (zero_or_min_slider(m, "##pk", &h.peak_nits, 200, 4000, 50, "auto")) apply(host, m);
        row(m, "Paper white", When::Live, "Nits of ordinary white; 0 = Windows' SDR content brightness (203 without it).");
        if (zero_or_min_slider(m, "##pw", &h.paper_white_nits, 80, 500, 10, "auto")) apply(host, m);
        row(m, "Highlight boost", When::Live, "0 keeps the SDR look, 1 lets highlights reach the peak, 2 pushes them further.");
        if (slider(m, "##hb", &h.highlight_boost, 0.0, 2.0, 0.1, "%.2f")) apply(host, m);
        row(m, "UI brightness", When::Live, "Nits of white in menus, text and the HUD while the 3D scene keeps paper white; 0 = same as paper white. HDR output only.");
        if (zero_or_min_slider(m, "##ui", &h.ui_nits, 40, 500, 10, "same as paper white")) apply(host, m);
        row(m, "Highlight saturation", When::Live, "0 = the game's highlights, which turn white as they brighten; higher keeps the scene's own colour in bright highlights (fire, magic, sunsets). Below paper white nothing changes.");
        if (slider(m, "##hs", &h.highlight_saturation, 0.0, 1.0, 0.05, "%.2f")) apply(host, m);
        row(m, "Hue preservation", When::Live, "0 = the game's tone curve, which shifts strong colours as they brighten (orange toward yellow, deep blue toward cyan). Higher keeps the scene's own hues through the whole range. HDR.");
        if (slider(m, "##hp", &h.hue_preserve, 0.0, 1.0, 0.05, "%.2f")) apply(host, m);
        row(m, "Wide colour", When::Restart, "HDR enhancement: keeps the colours the game itself renders beyond the standard range (it lights the scene in BT.2020), which SDR has to clip. Everything inside the standard range stays as in SDR. Off = SDR colours exactly.");
        if (check(m, "##wg", &h.wide_gamut)) apply(host, m);
        row(m, "Dither output", When::Live, "Adds very fine noise before the picture goes to the display, hiding faint steps in smooth dark gradients (night skies, fades).");
        if (check(m, "##dt", &h.dither)) apply(host, m);
        row(m, "Colour expansion", When::Live, "0 keeps the game's SDR colours exactly; higher pushes strongly saturated colours into the wider HDR range. Taste, not accuracy.");
        if (slider(m, "##ge", &h.gamut_expand, 0.0, 1.0, 0.05, "%.2f")) apply(host, m);
        end_rows();
    }
    const extras::HdrState hs = extras::hdr_state();
    ImGui::TextDisabled(tr("Now: HDR scene %s, swapchain %s, paper white %.0f, peak %.0f nits; the display reports HDR: %s"), hs.scene_hdr ? "on" : "off",
                        extras::output_kind_name(hs.output), static_cast<double>(hs.paper_white_nits), static_cast<double>(hs.peak_nits),
                        hs.display_hdr ? "yes" : "no");
}

void grading_group(UiHost& host, Menu& m) {
    extras::HdrSettings& h = m.edit.extras.hdr;
    section("Scene grading");
    if (!extras::hdr_scene_active()) {   // the grade runs in the replacement HDR tone map only
        ImGui::TextDisabled(tr("Needs the HDR scene (HDR on): the grading runs in the HDR tone map. In SDR use Picture below."));
    } else if (begin_rows("grade")) {
        row(m, "Exposure", When::Live, "Brightness of the 3D scene before the tone map, in stops. 0 = the game's.");
        if (slider(m, "##ge", &h.grade_exposure, -2.0, 2.0, 0.1, "%+.1f")) apply(host, m);
        row(m, "Contrast", When::Live, "Scene contrast around mid grey, before the tone map. 1 = the game's.");
        if (slider(m, "##gc", &h.grade_contrast, 0.5, 1.5, 0.05, "%.2f")) apply(host, m);
        row(m, "Shadows", When::Live, "Above 1 lifts the scene below mid grey, below 1 deepens it. 1 = the game's.");
        if (slider(m, "##gsh", &h.grade_shadows, 0.5, 1.5, 0.05, "%.2f")) apply(host, m);
        row(m, "Highlights", When::Live, "Above 1 brightens the scene above mid grey, below 1 tames it. 1 = the game's.");
        if (slider(m, "##ghl", &h.grade_highlights, 0.5, 1.5, 0.05, "%.2f")) apply(host, m);
        row(m, "Saturation", When::Live, "Scene colour intensity before the tone map (the UI is not touched). 1 = the game's.");
        if (slider(m, "##gsa", &h.grade_saturation, 0.0, 2.0, 0.05, "%.2f")) apply(host, m);
        end_rows();
    }
    section("Post effects");
    if (begin_rows("post")) {
        row(m, "Bloom", When::Live, "Strength of the game's glow around bright lights. 1 = the game's, 0 = none. SDR and HDR.");
        if (slider(m, "##bl", &h.bloom, 0.0, 4.0, 0.05, "%.2f")) apply(host, m);
        row(m, "Sharpening", When::Live, "Strength of the game's own sharpening of the finished picture. 1 = the game's, 0 = none. SDR and HDR.");
        if (slider(m, "##sp", &h.sharpen, 0.0, 3.0, 0.05, "%.2f")) apply(host, m);
        row(m, "Motion blur", When::Live, "Strength of the game's camera motion blur. 1 = the game's, 0 = none, 2 = twice the blur length. SDR and HDR.");
        if (slider(m, "##mb", &h.motion_blur, 0.0, 2.0, 0.05, "%.2f")) apply(host, m);
        row(m, "Depth of field", When::Live, "Strength of the game's out-of-focus blur. 1 = the game's, 0 = everything sharp, 2 = a wider aperture: more blur, starting closer to the focus. SDR and HDR.");
        if (slider(m, "##df", &h.depth_of_field, 0.0, 2.0, 0.05, "%.2f")) apply(host, m);
        row(m, "Vignette", When::Live, "Strength of the darkened screen edges in the scenes that have one (the world map, some story scenes). 1 = the game's, 0 = none. SDR and HDR.");
        if (slider(m, "##vg", &h.vignette, 0.0, 2.0, 0.05, "%.2f")) apply(host, m);
        row(m, "Compare with the game", When::Live, "Testing aid: the left half of the screen as the game draws it, the right half with your scene grading, highlight saturation and sharpening (HDR needed for grading and highlight saturation).");
        if (check(m, "##cmp", &h.compare)) apply(host, m);
        end_rows();
    }
}

void picture_group(UiHost& host, Menu& m) {
    extras::PictureSettings& c = m.edit.extras.picture;
    section("Picture");
    if (begin_rows("pic")) {
        row(m, "Preset", When::Live, "Neutral is the game's own picture; Vivid adds a little contrast and colour.");
        int preset = c.neutral() ? 0 : -1;
        extras::PictureSettings vivid;
        extras::picture_preset("vivid", &vivid);
        if (c.contrast == vivid.contrast && c.saturation == vivid.saturation && c.black == vivid.black && c.gamma == vivid.gamma) preset = 1;
        constexpr const char* kPresets[] = {"Neutral", "Vivid"};
        if (combo(m, "##pp", &preset, kPresets, 2, "Custom")) {
            extras::picture_preset(preset == 0 ? "neutral" : "vivid", &c);
            extras::set_picture_live(c);
            apply(host, m);
        }
        struct S { const char* label; const char* id; double* v; double lo, hi, step; const char* fmt; const char* help; };
        const S rows[] = {{"Contrast", "##ct", &c.contrast, 0.5, 1.5, 0.05, "%.2f", "Spreads the tones from mid grey; 1 is the game's own."},
                          {"Saturation", "##sa", &c.saturation, 0.0, 2.0, 0.05, "%.2f", "Colour intensity; 0 is grey, 1 the game's own."},
                          {"Black level", "##bl", &c.black, -0.5, 0.25, 0.01, "%.2f", "Below 0 pulls dark tones to black, above 0 lifts them."},
                          {"Gamma", "##gm", &c.gamma, 0.5, 2.0, 0.05, "%.2f", "Mid-tone brightness: above 1 brightens."}};
        for (const S& s : rows) {
            row(m, s.label, When::Live, s.help);
            bool d = false;
            const bool commit = slider(m, s.id, s.v, s.lo, s.hi, s.step, s.fmt, &d);
            if (d || commit) extras::set_picture_live(c);   // shows while dragged
            if (commit) apply(host, m);
        }
        end_rows();
    }
}

// The tier a running render scale reads as, named the way the ratio list names it. The percent comes from the pass's own extents, so a
// hand-written render_ratio (0.7) or a resolution the game picked itself shows its nearest tier beside the exact percent.
const char* live_tier_name(int percent) {
    if (percent <= 0) return "unknown";
    if (percent >= 99) return "native (DLAA)";
    if (percent >= 67) return "quality";
    if (percent >= 59) return "balanced";
    if (percent >= 50) return "performance";
    if (percent >= 33) return "ultra performance";
    return "below ultra performance";
}

// Graphics > Live render state (render_status.h): what ran, at which sizes, and whether frame generation really inserts frames. Read only:
// no row writes anything, and the snapshot is a plain copy taken here (the render thread publishes at most four times a second).
void live_status_group() {
    const render_status::Snapshot s = render_status::snapshot();
    // The same numbers in the log, once a second, while the page is open: what the rows say can be checked against the render thread's own
    // lines (nvn.dlss: network 2: first evaluation ...) without a screenshot.
    if (nxrt::log_enabled(nxrt::LogLevel::Debug)) {
        static uint64_t last = 0;
        const uint64_t now = FrameLimiter::now_ns();
        if (!last || now - last >= 1000000000ull) {
            last = now;
            NXLOG_D(kTag,
                    "live render state: seen %d, upscaler %s -> %s (fallback %d)%s%s, render %dx%d -> %dx%d (%d%%), screen %dx%d, "
                    "frame generation %s (dlss %d, fsr3 %d), pause %s, status %u, %u game frames, %u shown, %u generated (%.1f -> %.1f fps), measured %d",
                    s.seen ? 1 : 0, render_status::backend_name(s.configured), render_status::backend_name(s.actual), s.fallback ? 1 : 0,
                    s.reason[0] ? ", reason: " : "", s.reason, s.render_w, s.render_h, s.output_w, s.output_h, s.percent, s.screen_w, s.screen_h,
                    render_status::framegen_name(s.fg), s.fg_dlss ? 1 : 0, s.fg_ffx ? 1 : 0, render_status::pause_name(s.pause), s.status,
                    s.game_frames, s.shown_frames, s.generated, s.game_fps, s.shown_fps, s.measured ? 1 : 0);
        }
    }
    section("Live render state");
    if (!s.seen && s.fg == render_status::FrameGen::Unknown) {
        ImGui::TextDisabled("%s", tr("Nothing has been rendered yet."));
        return;
    }
    if (s.seen) {
        // The backend that really ran, and the one the configuration asked for when they differ.
        if (s.fallback)
            ImGui::Text(tr("Upscaler: %s -> %s (fell back)"), tr(render_status::backend_name(s.configured)),
                        tr(render_status::backend_name(s.actual)));
        else
            ImGui::Text(tr("Upscaler: %s (%s)"), tr(render_status::backend_name(s.actual)),
                        s.configured == render_status::Backend::Auto ? tr("chosen by Auto") : tr("as configured"));
        if (s.fallback && s.reason[0]) ImGui::TextWrapped("%s", s.reason);
        // The pass's own extents: the input it upscaled and the output it wrote.
        if (s.output_w > 0)
            ImGui::Text(tr("Render %d x %d -> %d x %d (%d%%, %s)"), s.render_w, s.render_h, s.output_w, s.output_h, s.percent,
                        tr(live_tier_name(s.percent)));
        if (s.screen_w > 0 && (s.screen_w != s.output_w || s.screen_h != s.output_h))
            ImGui::Text(tr("Screen %d x %d"), s.screen_w, s.screen_h);
    }
    // Frame generation: the backend, its lifecycle, and the presents it counted in the last window.
    const char* backend = s.fg_dlss ? "DLSS Frame Generation" : s.fg_ffx ? "FSR 3 frame generation" : tr("none");
    ImGui::Text(tr("Frame generation: %s (%s)"), tr(render_status::framegen_name(s.fg)), backend);
    if (s.fg == render_status::FrameGen::Paused && s.pause != render_status::Pause::None)
        ImGui::TextWrapped(tr("Why it is paused: %s"), tr(render_status::pause_name(s.pause)));
    if (s.measured && s.game_frames > 0) {
        // Three separate numbers, because the backend's own count covers every present: the game's frames, what reached the display, and
        // the extra frames the difference proves.
        ImGui::Text(tr("Last window: %u game frames, %u frames shown, %u generated (%.1f -> %.1f fps)"), s.game_frames, s.shown_frames, s.generated,
                    s.game_fps, s.shown_fps);
        if (s.fg == render_status::FrameGen::Generating && s.generated == 0)
            ImGui::TextColored(kOrange, "%s", tr("Set up, but no extra frame was presented in the last window: not confirmed to be inserting frames."));
    } else if (s.fg == render_status::FrameGen::Ready) {
        ImGui::TextColored(kOrange, "%s", tr("Set up, actual frame insertion not confirmed yet."));
    }
    if (s.fg_dlss && s.status) ImGui::TextDisabled(tr("Streamline status %u"), s.status);
}

// [display] npc_distance (fefw_crowd.cpp): the hub crowd's model limit, display distance and draw budget
constexpr const char* kNpcNames[] = {"Console (the game's own)", "1.5x", "2x", "3x", "4x"};
constexpr double kNpcVals[] = {1.0, 1.5, 2.0, 3.0, 4.0};

std::string npc_name(double k) {
    for (size_t i = 0; i < std::size(kNpcVals); ++i)
        if (std::fabs(k - kNpcVals[i]) < 1e-6) return tr(kNpcNames[i]);
    char b[32];
    std::snprintf(b, sizeof b, "%.2fx", k);
    return b;
}

// Graphics preset: the seven keys of fefw_presets.h together; Custom when they hold anything else.
void preset_group(Menu& m) {
    section("Preset");
    if (!begin_rows("preset")) return;
    row(m, "Graphics preset", When::Restart,
        "Sets Render scale, Shadow resolution, Quality and the four detail settings together. Low is for a slower GPU or a handheld PC; "
        "Medium is the console's picture (the defaults); High and Ultra render sharper. Output size and the upscaler are not changed; a fixed "
        "render size is cleared.");
    int idx = graphics_preset_current();
    const IniKey& fixed = *ini_key("fefw", "render");
    // a key set by an environment variable wins over the file: a preset could not apply, so the row is locked and says which one
    const IniKey* locked = ini_env(fixed) ? &fixed : nullptr;
    for (int k = 0; k < kPresetKeyCount && !locked; ++k)
        if (ini_env(*ini_key("fefw", kPresetKeys[k]))) locked = ini_key("fefw", kPresetKeys[k]);
    if (locked) {
        ImGui::AlignTextToFramePadding();
        ImGui::TextDisabled(tr("%s ([fefw] %s is set by %s)"), tr(idx >= 0 ? kPresetNames[idx] : "Custom"), locked->key,
                            config_env_name(locked->sec, locked->key).c_str());
    } else if (combo(m, "##preset", &idx, kPresetNames, kGraphicsPresetCount)) {
        const GraphicsPreset& p = kGraphicsPresets[idx];
        if (!ini_now(fixed).empty()) ini_put(m, fixed, "");
        for (int k = 0; k < kPresetKeyCount; ++k) {
            const IniKey& key = *ini_key("fefw", kPresetKeys[k]);
            if (normalized(key, ini_now(key)) != normalized(key, p.values[k])) ini_put(m, key, p.values[k]);
        }
        NXLOG_I(kTag, "graphics preset %s", p.name);
    }
    end_rows();
}

void detail_group(UiHost& host, Menu& m) {
    section("Graphics detail");
    ImGui::TextWrapped(tr("The game's own graphics settings files, edited as they load: these apply on restart."));
    if (!begin_rows("detail")) return;
    ini_combo_row(m, *ini_key("fefw", "reflections"), "Reflections",
                  "Screen-space reflections on floors and water. Reduced takes 8 rays instead of 32 and mostly helps in the hubs.",
                  kReflectionNames, kReflectionVals, 3);
    ini_combo_row(m, *ini_key("fefw", "ambient_occlusion"), "Ambient occlusion",
                  "The soft shading in corners. Off saves GPU time everywhere, with flatter lighting.", kDetailOnOffNames, kDetailOnOffVals, 2);
    ini_combo_row(m, *ini_key("fefw", "crowd_detail"), "Crowd detail",
                  "Reduced: crowd characters switch to their simpler models twice as soon, lightening busy towns and battles.",
                  kDetailLevelNames, kDetailLevelVals, 2);
    ini_combo_row(m, *ini_key("fefw", "scenery_detail"), "Scenery detail",
                  "Reduced: scenery switches to its simpler versions at 60% of the distance (50 m becomes 30 m).", kDetailLevelNames,
                  kDetailLevelVals, 2);
    // [display] npc_distance (fefw_crowd.cpp): the hub crowd's model limit, display distance and draw budget
    row(m, "NPC limit", When::Restart,
        "How many townspeople a town shows. The game keeps at most 70 different NPC models loaded, shows them within 80 m and under a "
        "draw budget; past those limits townspeople are hidden, even close by, and pop in as you walk. This multiplies all three. "
        "More memory and CPU work. Console = exactly the game's behaviour.");
    int nidx = -1;
    for (int i = 0; i < static_cast<int>(std::size(kNpcVals)); ++i)
        if (std::fabs(m.edit.display.npc_distance - kNpcVals[i]) < 1e-6) nidx = i;
    const std::string nother = std::string(tr("Custom")) + ": " + npc_name(m.edit.display.npc_distance);
    if (combo(m, "##npclimit", &nidx, kNpcNames, static_cast<int>(std::size(kNpcNames)), nother.c_str(), true)) {
        m.edit.display.npc_distance = kNpcVals[nidx];
        apply(host, m);
    }
    pending_note(std::fabs(m.edit.display.npc_distance - npc_distance_applied()) > 1e-6, npc_name(npc_distance_applied()));
    end_rows();
}

void graphics_page(UiHost& host, Menu& m) {
    preset_group(m);
    resolution_group(m);
    detail_group(host, m);
    upscaler_group(m);
    live_status_group();
    hdr_group(host, m);
    grading_group(host, m);
    picture_group(host, m);
}

// ---- Frame rate -------------------------------------------------------------------------------------------------------------------------
constexpr int kFpsChoices[] = {-1, 0, 30, 60, 72, 90, 120, 144, 165, 240};
constexpr const char* kLatencyNames[] = {"Off", "Auto", "On", "Boost"};

void fps_name(int v, char* buf, size_t n) {
    if (v < 0) std::snprintf(buf, n, "Refresh rate");
    else if (v == 0) std::snprintf(buf, n, "Unlimited");
    else std::snprintf(buf, n, "%d fps", v);
}

void framerate_page(UiHost& host, Menu& m, const play::LiveStatus& live, gfx::Overlay& ov) {
    const ClockInfo ck = clock_info();
    section("The game's frame rate");
    ImGui::TextWrapped("%s", tr("The game steps its world by its own frame rate; the hooks that change it are installed at start."));
    if (begin_rows("game")) {
        const IniKey& k60 = *ini_key("fefw", "fps60");
        const IniKey& k120 = *ini_key("fefw", "fps120");
        row(m, "Gameplay frame rate", When::Restart,
            "30 is the game's own. 60 runs gameplay at 60 at the right speed. 120 needs a frame limit of 120 or more and a fast PC. "
            "Frame-rate fixes below keep frame-counted effects at their 30 fps speed.");
        if (!(ini_env(k60) || ini_env(k120))) {
            const auto is_on = [](const IniKey& k) { return normalized(k, ini_now(k)) == "1"; };
            int idx = is_on(k120) ? 2 : is_on(k60) ? 1 : 0;
            constexpr const char* kNames[] = {"30 (the game's own)", "60", "120"};
            if (combo(m, "##rate", &idx, kNames, 3)) {
                if (idx == 2) ini_put(m, k120, "1");
                else {
                    ini_put(m, k60, idx == 1 ? "1" : "0");
                    if (is_on(k120)) ini_put(m, k120, "0");
                }
            }
            const int running = ck.fps120 ? 2 : ck.fps60 ? 1 : 0;   // the hooks installed at start
            pending_note(idx != running, kNames[running]);
        } else {
            ImGui::TextDisabled(tr("set by the environment (%s / %s)"), config_env_name("fefw", "fps60").c_str(), config_env_name("fefw", "fps120").c_str());
        }
        constexpr const char* kSmooth[] = {"Auto (on at 60 and 120)", "On", "Off"};
        constexpr const char* kSmoothVals[] = {"auto", "1", "0"};
        ini_combo_row(m, *ini_key("fefw", "smooth_step"), "Smooth time step",
                      "Keeps the game clock exact when frames are uneven: the game's step plus a quarter of the time it still owes.", kSmooth,
                      kSmoothVals, 3);
        const IniKey& kfix = *ini_key("fefw", "frame_fixes");
        row(m, "Frame-rate fixes", When::Live,
            "At 60 and 120 fps: physics, capes and hair, cutscene shakes, map markers, menu animations and dialogue typing that count frames move at "
            "their 30 fps speed. [fefw] frame_fixes_skip in the ini turns single groups off.");
        if (!env_locked(kfix)) {
            constexpr const char* kFix[] = {"On", "Off"};
            int fidx = normalized(kfix, ini_now(kfix)) == "1" ? 0 : 1;
            if (combo(m, "##fix", &fidx, kFix, 2)) {
                frame_set_enabled(fidx == 0);   // applies at once (fefw_frame.cpp)
                ini_put(m, kfix, fidx == 0 ? "1" : "0");
            }
        }
        end_rows();
    }
    if (ck.valid)
        ImGui::TextDisabled(tr("Game clock %.3fx real time over %d s (1.000 is right), target %d fps, display rate %d, step %.2f frames"), ck.rate,
                            ck.window_s, ck.target, ck.display, static_cast<double>(ck.step));
    else
        ImGui::TextDisabled("%s", tr("Game clock: not running yet (title or loading)"));

    play::DisplaySettings& d = m.edit.display;
    section("Presentation");
    if (begin_rows("pres")) {
        row(m, "Vsync", When::Live, "Off: no forced vsync, the lowest latency; works with G-Sync / FreeSync and the frame limit. On: the display paces.");
        if (check(m, "##vs", &d.vsync)) apply(host, m);
        row(m, "Frame limit", When::Live, "Refresh rate follows the display. The game's own pace (30 / 60 / 120) still holds under it.");
        std::vector<std::string> names;
        int idx = -1;
        for (int i = 0; i < static_cast<int>(std::size(kFpsChoices)); ++i) {
            char b[32];
            fps_name(kFpsChoices[i], b, sizeof b);
            names.emplace_back(b);
            if (kFpsChoices[i] == d.fps_cap) idx = i;
        }
        std::vector<const char*> ptrs;
        for (const std::string& s : names) ptrs.push_back(s.c_str());
        char cur[32];
        fps_name(d.fps_cap, cur, sizeof cur);
        if (combo(m, "##cap", &idx, ptrs.data(), static_cast<int>(ptrs.size()), cur)) {
            d.fps_cap = kFpsChoices[idx];
            apply(host, m);
        }
        {
            const int want = ck.fps120 ? 120 : ck.fps60 ? 60 : 0;      // the gameplay rate the hooks installed at start run at
            const FrameLimiter* l = ov.context().limiter;
            if (want > 0 && d.fps_cap > 0 && d.fps_cap < want)
                ImGui::TextColored(kOrange, tr("%d fps gameplay raises this limit to %d fps while it is on (the setting itself stays %d)."), want, want, d.fps_cap);
            else if (want > 0 && l && l->target_fps() > 0 && l->target_fps() < want - 1)
                ImGui::TextColored(kOrange, tr("%d fps gameplay is held to %.0f fps by %s: use a faster display, or turn vsync off and set a limit of %d or more."),
                                   want, l->target_fps(), d.vsync ? "vsync and the display's refresh rate" : "the display's refresh rate (limit: Refresh rate)", want);
        }
        row(m, "Low latency", When::Restart, "NVIDIA Reflex or AMD Anti-Lag 2, whichever the GPU has. Keep it off with frame generation.");
        int lat = static_cast<int>(m.edit.extras.latency.mode);
        if (combo(m, "##lat", &lat, kLatencyNames, 4)) {
            m.edit.extras.latency.mode = static_cast<extras::LatencyMode>(lat);
            apply(host, m);
        }
        pending_note(m.edit.extras.latency.mode != m.boot.extras.latency.mode, kLatencyNames[static_cast<int>(m.boot.extras.latency.mode)]);
        end_rows();
    }
    const FrameLimiter* lim = ov.context().limiter;
    ImGui::TextDisabled(tr("Now: %.1f fps presented; frame limit %.1f, pacing to %.1f fps (the lower of the limit and the game's rate); present mode %s"),
                        live.fps, lim ? lim->target_fps() : 0.0, lim ? lim->effective_fps() : 0.0, live.present_mode);
}

// ---- Audio ------------------------------------------------------------------------------------------------------------------------------
void audio_page(UiHost& host, Menu& m) {
    play::AudioSettings& a = m.edit.audio;
    section("Sound");
    if (begin_rows("aud")) {
        row(m, "Master volume", When::Live, "Everything the game plays.");
        double pct = a.volume * 100.0;
        bool drag = false;
        const bool commit = slider(m, "##vol", &pct, 0, 200, 5, "%.0f%%", &drag);   // 0 to audio::kMaxVolume (200%)
        if (drag || commit) {
            a.volume = std::clamp(pct / 100.0, 0.0, 2.0);
            audio::set_master_volume(static_cast<float>(a.volume));   // heard while dragged
        }
        if (commit) apply(host, m);
        row(m, "Mute in the background", When::Live, "Silent while the window is not in focus.");
        if (check(m, "##mute", &a.mute_unfocused)) apply(host, m);

        if (!m.audio_loaded) {
            m.audio_devices.clear();
            for (const audio::OutputDevice& dev : audio::list_output_devices())
                if (!dev.name.empty()) m.audio_devices.push_back(dev.name);
            m.audio_loaded = true;
        }
        row(m, "Output device", When::Restart, "The game opens its audio device once.");
        std::vector<const char*> names = {"System default"};
        for (const std::string& s : m.audio_devices) names.push_back(s.c_str());
        int idx = a.device.empty() || a.device == "default" ? 0 : -1;
        for (size_t i = 0; i < m.audio_devices.size(); ++i)
            if (a.device == m.audio_devices[i]) idx = static_cast<int>(i) + 1;
        if (combo(m, "##dev", &idx, names.data(), static_cast<int>(names.size()), a.device.c_str())) {
            a.device = idx == 0 ? "default" : m.audio_devices[static_cast<size_t>(idx - 1)];
            apply(host, m);
        }
        pending_note(a.device != m.boot.audio.device, m.boot.audio.device);
        row(m, "Speakers", When::Restart,
            "Stereo, or 5.1 surround (front left and right, centre, LFE, rear left and right). Windows downmixes 5.1 on a stereo device, so pick 5.1 "
            "only for a 5.1 or 7.1 output. [audio] channels in fefw.ini or NXRT_AUDIO_CHANNELS wins over this.");
        const char* const kSpeakerNames[] = {"Stereo", "5.1 surround"};
        int spk = a.channels == 6 ? 1 : 0;
        if (combo(m, "##spk", &spk, kSpeakerNames, 2)) {
            a.channels = spk == 1 ? 6 : 2;
            apply(host, m);
        }
        pending_note(a.channels != m.boot.audio.channels, m.boot.audio.channels == 6 ? "5.1 surround" : "Stereo");
        end_rows();
    }
}

// ---- Controls ---------------------------------------------------------------------------------------------------------------------------
constexpr const char* kPadSrcLabels[play::kPadSrcCount] = {
    "A / Cross (bottom)", "B / Circle (right)", "X / Square (left)", "Y / Triangle (top)", "LB / L1", "RB / R1", "LT / L2", "RT / R2",
    "Menu / Options", "View / Create", "Left stick click", "Right stick click", "D-pad up", "D-pad down", "D-pad left", "D-pad right"};
constexpr const char* kPadChoiceNames[] = {"Default", "Nothing", "A", "B", "X", "Y", "L", "R", "ZL", "ZR", "Plus", "Minus",
                                           "D-pad up", "D-pad down", "D-pad left", "D-pad right", "Left stick click", "Right stick click"};
constexpr hid::NpadButtonBit kPadChoiceBits[] = {hid::kBtnA,    hid::kBtnB,    hid::kBtnX,    hid::kBtnY,     hid::kBtnL,
                                                 hid::kBtnR,    hid::kBtnZL,   hid::kBtnZR,   hid::kBtnPlus,  hid::kBtnMinus,
                                                 hid::kBtnUp,   hid::kBtnDown, hid::kBtnLeft, hid::kBtnRight, hid::kBtnStickL,
                                                 hid::kBtnStickR};
constexpr int kPadChoiceCount = 18;
constexpr const char* kPromptNames[] = {"Switch (the game's own)", "Xbox", "PlayStation", "Keyboard and mouse", "Auto (the last device used)"};

uint64_t pad_choice_value(int c) { return c == 0 ? play::kPadDefault : c == 1 ? 0 : static_cast<uint64_t>(hid::btn(kPadChoiceBits[c - 2])); }

// The keys of controls.ini's [bindings.*] sections, by the Switch button or stick they drive (read only: they are changed in the file).
struct KeyRowName {
    play::Act act;
    const char* label;
};
constexpr KeyRowName kKeyRows[] = {
    {play::Act::MoveForward, "Move up (left stick)"},  {play::Act::MoveBack, "Move down (left stick)"},
    {play::Act::MoveLeft, "Move left (left stick)"},   {play::Act::MoveRight, "Move right (left stick)"},
    {play::Act::LookUp, "Camera up (right stick)"},    {play::Act::LookDown, "Camera down (right stick)"},
    {play::Act::LookLeft, "Camera left (right stick)"}, {play::Act::LookRight, "Camera right (right stick)"},
    {play::Act::BtnA, "A"}, {play::Act::Confirm, "A"}, {play::Act::BtnB, "B"}, {play::Act::Back, "B"}, {play::Act::BtnX, "X"},
    {play::Act::BtnY, "Y"}, {play::Act::BtnL, "L"}, {play::Act::BtnR, "R"}, {play::Act::BtnZL, "ZL"}, {play::Act::BtnZR, "ZR"},
    {play::Act::BtnPlus, "+ (menu)"}, {play::Act::BtnMinus, "-"}, {play::Act::DpadUp, "D-pad up"}, {play::Act::DpadDown, "D-pad down"},
    {play::Act::DpadLeft, "D-pad left"}, {play::Act::DpadRight, "D-pad right"}, {play::Act::BtnLStick, "Left stick click"},
    {play::Act::BtnRStick, "Right stick click"},
};

void keyboard_section(Menu& m) {
    section("Keyboard");
    ImGui::TextWrapped("%s", tr("The keys each Switch button or stick has. To change them, close the game and edit [bindings.all] in controls.ini."));
    if (!begin_rows("keys")) return;
    std::vector<std::pair<std::string, std::string>> rows;   // label, keys
    auto add = [&](const std::string& label, const std::string& key) {
        for (auto& r : rows)
            if (r.first == label) {
                r.second += ", " + key;
                return;
            }
        rows.push_back({label, key});
    };
    for (const KeyRowName& k : kKeyRows)
        for (const play::Binding& b : m.edit.bindings)
            if (b.act == k.act) {
                std::string key = play::input_name(b.in);
                if (b.ctx != play::Ctx::All) key += std::string(" (") + play::ctx_name(b.ctx) + ")";
                add(k.label, key);
            }
    for (const play::Binding& b : m.edit.bindings) {   // anything else bound (by its settings-file name)
        bool listed = b.act == play::Act::None;
        for (const KeyRowName& k : kKeyRows) listed |= k.act == b.act;
        if (!listed) add(play::act_info(b.act).name, play::input_name(b.in));
    }
    for (const auto& [label, keys] : rows) {
        row(m, label.c_str(), When::Info, nullptr);
        ImGui::AlignTextToFramePadding();
        ImGui::TextUnformatted(keys.c_str());
    }
    if (rows.empty()) {
        row(m, "No keys bound", When::Info, nullptr);
        ImGui::AlignTextToFramePadding();
        ImGui::TextUnformatted(tr("[bindings.all] in controls.ini"));
    }
    end_rows();
}

// The mouse is the game's own Joy-Con 2 mouse (hid jc2_mouse.h): the game runs its mouse controls with its own settings, and these rows
// set what the PC mouse sends it.
void mouse_section(UiHost& host, Menu& m) {
    section("Mouse");
    ImGui::TextWrapped("The mouse works as the game's Joy-Con 2 mouse: move it and the game switches to its mouse controls (camera in "
                       "bases and dungeons, map cursor in battle, selection in menus); press a key or use the controller and it switches back. "
                       "The game's own speeds are the (Mouse) rows of its System > Settings; System > Mouse Controls lists every button.");
    play::MouseSettings& ms = m.edit.mouse;
    if (begin_rows("mouse")) {
        constexpr const char* kNames[] = {"Automatic (the last device used)", "Always the mouse", "Off"};
        constexpr const char* kVals[] = {"auto", "on", "off"};
        const IniKey& k = *ini_key("input", "jc2_mouse");
        ini_combo_row(m, k, "Mouse controls", "Automatic: moving or clicking the mouse switches the game to its mouse controls, a key or "
                      "the controller switches back. Always: the game stays in its mouse controls. Off: the game never sees a mouse.",
                      kNames, kVals, 3);
        const std::string v = lower(ini_now(k));
        const hid::Jc2MouseMode want = v == "off" ? hid::Jc2MouseMode::Off : v == "on" ? hid::Jc2MouseMode::On : hid::Jc2MouseMode::Auto;
        if (want != hid::jc2_mouse_mode()) hid::jc2_mouse_set_mode(want);
        row(m, "Mouse speed", When::Live, "How far the game's cursor and camera move per mouse count, before the game's own speed settings.");
        if (slider(m, "##jc2s", &ms.jc2_speed, 0.25, 4.0, 0.05, "%.2fx", nullptr, ImGuiSliderFlags_Logarithmic)) apply(host, m);
        row(m, "Wake distance", When::Live, "How far the mouse must move (counts, within a quarter second) to switch to mouse controls.");
        if (slider(m, "##jc2w", &ms.jc2_wake, 0.0, 100.0, 1.0, "%.0f")) apply(host, m);
        row(m, "Capture the mouse", When::Live, "While the game is in its mouse controls the Windows pointer is hidden and held in the "
            "window (the game draws its own cursor). A key, the controller, Alt+Tab or F1 frees it.");
        if (check(m, "##jc2c", &ms.jc2_capture)) apply(host, m);
        const MouseInfo mi = mouse_info();
        const hid::Jc2Stats js = hid::jc2_mouse_stats();
        row(m, "Now", When::Info, nullptr);
        ImGui::AlignTextToFramePadding();
        if (!mi.valid) ImGui::TextDisabled("the game has not set up its mouse yet");
        else ImGui::Text("%s (mouse used %llu times)", mi.mode ? "mouse controls" : "buttons", static_cast<unsigned long long>(js.engages));
        end_rows();
    }
}

// ---- Haptics ----------------------------------------------------------------------------------------------------------------------------
// How the game's HD Rumble value reaches the pad. The backends live in gfx/haptic.cpp: the standard two-motor rumble always
// works, the DualSense gets real haptics, a Switch Pro Controller gets HD Rumble packets, an Xbox pad its four motors. The state line reports
// what is actually driving the pad, not what was asked for.
constexpr const char* kHapticNames[] = {"Automatic", "Off", "Standard rumble", "DualSense haptics"};
constexpr int kHapticCount = 4;

void haptic_group(UiHost& host, Menu& m) {
    play::PadSettings& p = m.edit.pad;
    const haptic::Status st = haptic::status();
    section("Haptics");
    if (begin_rows("haptic")) {
        row(m, "Vibration", When::Live,
            "Automatic: DualSense haptics (cable or Bluetooth), HD Rumble for a Switch Pro Controller, all four motors of "
            "an Xbox pad, the standard rumble for anything else. A mode change applies at once; if a backend cannot start, "
            "the standard rumble takes over.");
        int mode = std::clamp(p.haptic, 0, kHapticCount - 1);
        if (combo(m, "##hap", &mode, kHapticNames, kHapticCount)) {
            p.haptic = mode;
            apply(host, m);
        }
        row(m, "Vibration strength", When::Live,
            "For the haptic backends. Left at -1 the haptics follow the rumble strength above instead, so an untouched "
            "controls.ini keeps one strength knob; 0 silences them.");
        double strength = static_cast<double>(p.haptic_strength);
        if (slider(m, "##haps", &strength, -1.0, 100.0, 1.0, "%.0f%%")) {
            p.haptic_strength = static_cast<int>(std::lround(strength));
            apply(host, m);
        }
        row(m, "Test", When::Info, nullptr);
        const bool can_test = st.running && st.requested != haptic::Mode::Off;
        ImGui::BeginDisabled(!can_test);
        if (button(m, "Buzz once")) haptic::test_start();
        ImGui::EndDisabled();
        end_rows();
    }
    if (!st.dll) ImGui::TextDisabled("%s", tr("renhdrum.dll is not next to the game: the standard rumble only."));
    if (st.device[0] == 0) {
        ImGui::TextDisabled("%s", tr("No pad for the haptics yet."));
    } else {
        ImGui::TextDisabled(tr("%s: %s, %s"), st.device, tr(haptic::backend_name(st.backend)), tr(st.wired ? "on a cable" : "wireless"));
        if (st.detail[0]) ImGui::TextWrapped("%s", tr(st.detail));
        ImGui::TextDisabled(tr("Live: %s; %llu values from the game, %llu packets sent."), st.running ? tr("yes") : tr("no"),
                            static_cast<unsigned long long>(st.pushes), static_cast<unsigned long long>(st.packets));
    }
}

void controls_page(UiHost& host, Menu& m, const play::LiveStatus& live) {
    play::PadSettings& p = m.edit.pad;
    section("Controller");
    if (begin_rows("pad")) {
        row(m, "Face buttons", When::Live,
            "By label: the button printed A is the Switch A (Xbox layout). By position: the bottom button is the Switch B, as on a Switch pad.");
        int lay = p.label_abxy ? 0 : 1;
        constexpr const char* kLayout[] = {"By label (Xbox A = Switch A)", "By position (bottom = Switch B)"};
        if (combo(m, "##lay", &lay, kLayout, 2)) {
            p.label_abxy = lay == 0;
            apply(host, m);
        }
        row(m, "Button prompts", When::Live, "Which button icons the game shows ([prompts] style).");
        int ps = static_cast<int>(m.edit.prompts.style);
        if (combo(m, "##pr", &ps, kPromptNames, play::kPromptStyleCount)) {
            m.edit.prompts.style = static_cast<play::PromptStyle>(ps);
            apply(host, m);
        }
        row(m, "Left stick dead zone", When::Live, "How far the stick moves before it counts, as a share of its travel.");
        if (slider(m, "##dz", &p.stick_deadzone, 0.0, 0.5, 0.01, "%.2f")) apply(host, m);
        row(m, "Right stick dead zone", When::Live, "The same for the right stick (the camera).");
        if (slider(m, "##rdz", &p.right_deadzone, 0.0, 0.5, 0.01, "%.2f")) apply(host, m);
        row(m, "Left stick full tilt at", When::Live, "Lower it if the stick never reaches full speed (it tops out early).");
        if (slider(m, "##smax", &p.stick_max, 0.5, 1.0, 0.01, "%.2f")) apply(host, m);
        row(m, "Trigger threshold", When::Live, "Analog trigger travel that counts as ZL / ZR pressed.");
        if (slider(m, "##trig", &p.trigger_threshold, 0.05, 1.0, 0.05, "%.2f")) apply(host, m);
        row(m, "Rumble", When::Live, nullptr);
        if (check(m, "##rum", &p.rumble)) apply(host, m);
        row(m, "Rumble strength", When::Live, nullptr);
        if (slider(m, "##rums", &p.rumble_scale, 0.0, 2.0, 0.1, "%.1fx")) apply(host, m);
        end_rows();
    }
    if (live.pads == 0) ImGui::TextDisabled("%s", tr("No controller connected."));
    else ImGui::TextDisabled(tr("%d connected: %s (rumble %s)"), live.pads, live.pad_name.c_str(), live.pad_rumble ? "yes" : "no");
    haptic_group(host, m);
    mouse_section(host, m);

    section("Button mapping");
    ImGui::TextWrapped("%s", tr("What each controller button presses on the Switch pad. Default keeps the layout above."));
    if (begin_rows("map")) {
        for (int i = 0; i < play::kPadSrcCount; ++i) {
            uint64_t& v = p.remap[i];
            int sel = -1;
            for (int c = 0; c < kPadChoiceCount; ++c)
                if (pad_choice_value(c) == v) sel = c;
            ImGui::PushID(i);
            row(m, kPadSrcLabels[i], When::Live, "Left / right or A to change.");
            if (combo(m, "##map", &sel, kPadChoiceNames, kPadChoiceCount, "Custom (set in controls.ini)")) {
                v = pad_choice_value(sel);
                apply(host, m);
            }
            ImGui::PopID();
        }
        end_rows();
    }
    if (button(m, "Reset the mapping")) {
        for (uint64_t& v : p.remap) v = play::kPadDefault;
        apply(host, m);
    }
    keyboard_section(m);
    ImGui::Spacing();
    ImGui::TextDisabled("%s", tr("This menu: F1, or Minus and Plus together on the pad. The game gets no input while it is open."));
}

// ---- Game -------------------------------------------------------------------------------------------------------------------------------
// The languages the game ships (games/fefw/fefw_data.cpp), by the names the settings service reads (services/common.cpp). "" is Automatic.
// ImGui's font has no CJK glyphs, so the names are English.
constexpr const char* kLangNames[] = {"Automatic (Windows language)", "English (US)",   "English (UK)",           "Japanese",
                                      "French",                       "French (Canada)", "German",                 "Italian",
                                      "Spanish (Spain)",              "Spanish (Latin America)", "Korean",         "Chinese (Simplified)",
                                      "Chinese (Traditional)"};
constexpr const char* kLangVals[] = {"",       "AmericanEnglish", "BritishEnglish", "Japanese", "French", "CanadianFrench", "German",
                                     "Italian", "Spanish",        "LatinAmericanSpanish", "Korean", "SimplifiedChinese", "TraditionalChinese"};
constexpr int kLangCount = 13;

void game_page(UiHost& host, Menu& m) {
    section("Language");
    if (begin_rows("lang")) {
        row(m, "Game language", When::Restart,
            "The language of the game's text and voices. Automatic uses your Windows display language when the game has it, else English. "
            "It applies the next time the game starts. [tweaks] language in controls.ini; [system] language in fefw.ini wins over it.");
        std::string& lang = m.edit.fastboot.language;
        int sel = -1;
        for (int i = 0; i < kLangCount; ++i) {
            const std::string want = kLangVals[i];
            if (want.size() == lang.size() && std::equal(want.begin(), want.end(), lang.begin(), [](char a, char b) { return std::tolower(a) == std::tolower(b); }))
                sel = i;
        }
        if (combo(m, "##lang", &sel, kLangNames, kLangCount, lang.c_str())) {
            lang = kLangVals[sel];
            apply(host, m);
        }
        end_rows();
    }
    section("Movies");
    if (begin_rows("mov")) {
        constexpr const char* kNames[] = {"Play them (AV1 on the CPU)", "Skip them"};
        constexpr const char* kVals[] = {"host", "skip"};
        ini_combo_row(m, *ini_key("media", "movies"), "Movies", "The game's movies are decoded on the CPU (dav1d). Skip makes each one end at once, as a skip does.",
                      kNames, kVals, 2);
        end_rows();
    }
    section("Files");
    ImGui::TextDisabled(tr("Game settings: %s"), config_file_path().empty() ? "(none)" : config_file_path().c_str());
    ImGui::TextDisabled(tr("Controls and display: %s"), host.settings_path().c_str());
    ImGui::TextDisabled("%s", tr("Every key: runtime/docs/CONFIG.md"));
    section("About");
    play::draw_about_page(host.settings_path());
}

// ---- Debug ------------------------------------------------------------------------------------------------------------------------------
constexpr const char* kLevelNames[] = {"Errors", "Warnings", "Info (default)", "Debug", "Trace"};
constexpr const char* kLevelVals[] = {"error", "warn", "info", "debug", "trace"};

struct PhaseRow { int phase; const char* name; int depth; };
constexpr PhaseRow kPhases[] = {
    {kHpRenderReplay, "Render thread: replay (record and submit)", 0},
    {kHpRenderGpuWait, "waiting for a GPU slot", 1},
    {kHpRenderSubmit, "queue submit", 1},
    {kHpRenderUpload, "texture uploads", 1},
    {kHpRenderPipeline, "pipeline creation", 1},
    {kHpRenderLockWait, "waits on other threads", 1},
    {kHpRenderPresent, "Render thread: present", 0},
    {kHpRenderLimiter, "frame limiter wait", 1},
    {kHpRenderAcquire, "swapchain acquire", 1},
    {kHpRenderPresentEnd, "submit and vkQueuePresent", 1},
    {kHpRenderIdle, "Render thread: idle (waiting for the game)", 0},
    {kHpGameBackpressure, "Game: waiting for the render thread", 0},
    {kHpGameSyncWait, "Game: GPU sync waits", 0},
    {kHpGameSvc, "Game thread: system calls", 0},
    {kHpGameIpc, "service requests", 1},
    {kHpGameWait, "waits and sleeps", 1},
    {kHpFaults, "Write-watch faults (all threads)", 0},
    {kHpPoolImport, "Memory pool imports", 0},
};

void sample_timings(Menu& m, gfx::Renderer* r) {
    const double now = ImGui::GetTime();
    if (m.sample_at >= 0 && now - m.sample_at < 0.5) return;
    const uint64_t frames = r ? r->stats().frames : 0;
    const uint64_t df = frames - m.frames_prev;
    for (int i = 0; i < kHpCount; ++i) {
        const double t = hitch_phase_total_ms(i);
        if (m.sample_at >= 0 && df > 0) m.phase_ms[i] = (t - m.phase_prev[i]) / static_cast<double>(df);
        m.phase_prev[i] = t;
    }
    if (m.sample_at >= 0 && df > 0) m.frame_ms_avg = (now - m.sample_at) * 1000.0 / static_cast<double>(df);
    m.frames_prev = frames;
    m.sample_at = now;
}

// Debug > CPU: the processor (read once), the game's console cores on it, and the load twice a second: the process, every program, memory, and
// the busiest threads (percent of one hardware thread). nxrt/cpu_info.h.
std::string processor_list(uint64_t mask) {
    std::string t;
    for (int b = 0; b < 64; ++b)
        if (mask >> b & 1) t += (t.empty() ? "" : ", ") + std::to_string(b);
    return t;
}

void cpu_log(const Menu& m) {
    std::string lines = "CPU (F1 menu): " + cpu::summary_line();
    if (const char* b = cpu::game_build()) lines += std::string("; game code build ") + b;
    char buf[256];
    std::snprintf(buf, sizeof buf, "\n  load: process %.1f%% (%.2f threads), all programs %.1f%%; working set %.2f GB, committed %.2f GB, RAM free %.2f GB",
                  m.cpu_load.process_pct, m.cpu_load.process_threads, m.cpu_load.system_pct, m.cpu_load.working_set / 1073741824.0,
                  m.cpu_load.committed / 1073741824.0, m.cpu_load.ram_avail / 1073741824.0);
    lines += buf;
    for (const cpu::ThreadLoad& t : m.cpu_threads) {
        std::snprintf(buf, sizeof buf, "\n  %-24s tid %-6u %5.1f%% of a thread, ideal processor %d", t.name.c_str(), t.tid, t.pct, t.ideal);
        lines += buf;
    }
    NXLOG_I(kTag, "%s", lines.c_str());
}

void cpu_group(Menu& m) {
    section("CPU");
    const cpu::Static& c = cpu::info();
    const double now = ImGui::GetTime();
    if (m.cpu_at < 0 || now - m.cpu_at >= 0.5) {
        m.cpu_load = cpu::sample();
        m.cpu_threads = cpu::thread_loads(10);
        m.cpu_at = now;
    }
    ImGui::TextUnformatted(c.brand.empty() ? tr("Unknown processor") : c.brand.c_str());
    char split[96] = "";
    if (c.perf_cores || c.eff_cores)
        std::snprintf(split, sizeof split, tr(" (%d performance with %d threads, %d efficiency)"), c.perf_cores, c.perf_threads, c.eff_cores);
    ImGui::TextDisabled(tr("%d cores%s, %d threads; base clock %.2f GHz; L2 %.1f MB, L3 %.1f MB; %.1f GB of memory"), c.cores, split, c.logical,
                        c.base_mhz / 1000.0, c.l2_kb / 1024.0, c.l3_kb / 1024.0, static_cast<double>(c.ram_total) / 1073741824.0);
    const char* build = cpu::game_build();
    ImGui::TextDisabled(tr("Instruction sets: %s. Game code build: %s."), cpu::features_text(c).c_str(), build ? build : tr("not reported"));
    const kernel::SchedInfo si = kernel::sched_info();
    if (si.active) {
        uint64_t used = 0;
        for (int i = 0; i < si.cores && i < 16; ++i) used |= si.host_masks[i];
        ImGui::TextDisabled(tr("The game's %d console cores: hardware threads %s (%s)"), si.cores, processor_list(used).c_str(),
                            si.soft ? tr("preferred, may move") : tr("pinned"));
    } else {
        ImGui::TextDisabled("%s", tr("The game's console cores: not pinned (fewer hardware threads than console cores, or [sched] console_cores = 0)."));
    }
    const cpu::Load& l = m.cpu_load;
    if (l.valid) {
        ImGui::Text(tr("Load: the game %.0f%% of the CPU (%.1f threads busy), all programs %.0f%%"), l.process_pct, l.process_threads, l.system_pct);
        ImGui::Text(tr("Memory: the game %.2f GB in use, %.2f GB committed; %.1f of %.1f GB free"), l.working_set / 1073741824.0,
                    l.committed / 1073741824.0, l.ram_avail / 1073741824.0, static_cast<double>(c.ram_total) / 1073741824.0);
    } else {
        ImGui::TextDisabled("%s", tr("Load: measuring..."));
    }
    if (!m.cpu_threads.empty() &&
        ImGui::BeginTable("cputhreads", 3, ImGuiTableFlags_RowBg | ImGuiTableFlags_SizingStretchProp | ImGuiTableFlags_BordersInnerV)) {
        ImGui::TableSetupColumn(tr("Busiest threads"), ImGuiTableColumnFlags_WidthStretch, 0.56f);
        ImGui::TableSetupColumn(tr("% of one thread"), ImGuiTableColumnFlags_WidthStretch, 0.22f);
        ImGui::TableSetupColumn(tr("Ideal processor"), ImGuiTableColumnFlags_WidthStretch, 0.22f);
        ImGui::TableHeadersRow();
        for (const cpu::ThreadLoad& t : m.cpu_threads) {
            ImGui::TableNextRow();
            ImGui::TableSetColumnIndex(0);
            ImGui::TextUnformatted(t.name.c_str());
            ImGui::TableSetColumnIndex(1);
            if (t.pct >= 90.0) ImGui::TextColored(kOrange, "%.0f%%", t.pct);   // a thread that has a hardware thread to itself and still wants more
            else ImGui::Text("%.0f%%", t.pct);
            ImGui::TableSetColumnIndex(2);
            if (t.ideal >= 0) ImGui::Text("%d", t.ideal);
            else ImGui::TextDisabled("-");
        }
        ImGui::EndTable();
        ImGui::TextDisabled("%s", tr("A thread near 100% is busy all the time: it sets the frame rate whatever the GPU does."));
    }
    if (button(m, "Write the CPU details to the log")) {
        cpu_log(m);
        notify(m, "CPU details written to the log.");
    }
}

void debug_page(UiHost& host, Menu& m, gfx::Overlay& ov) {
    gfx::Renderer* r = ov.context().renderer;
    section("Frame times");
    {
        static float ft[300];
        const size_t n = frame_times_recent(ft, 300);
        float mx = 0, sum = 0;
        for (size_t i = 0; i < n; ++i) {
            mx = std::max(mx, ft[i]);
            sum += ft[i];
        }
        std::vector<float> sorted(ft, ft + n);
        std::sort(sorted.begin(), sorted.end());
        const float p99 = n ? sorted[std::min(n - 1, static_cast<size_t>(static_cast<double>(n) * 0.99))] : 0.f;
        const float avg = n ? sum / static_cast<float>(n) : 0.f;
        char overlay[128];
        std::snprintf(overlay, sizeof overlay, "avg %.2f ms (%.1f fps)  99%% %.1f ms  worst %.1f ms", avg, avg > 0 ? 1000.0 / avg : 0.0, p99, mx);
        const float top = std::max(34.f, std::ceil(mx / 8.33f) * 8.33f);
        ImGui::PlotLines("##ft", ft, static_cast<int>(n), 0, overlay, 0.f, top, ImVec2(-FLT_MIN, px(110)));
        ImGui::TextDisabled(tr("The last %d presented frames; the scale tops out at %.0f ms (16.7 ms = 60 fps, 33.3 ms = 30 fps)."), static_cast<int>(n),
                            static_cast<double>(top));
    }

    section("Where the frame goes (ms per presented frame, last half second)");
    sample_timings(m, r);
    if (ImGui::BeginTable("phases", 3, ImGuiTableFlags_RowBg | ImGuiTableFlags_SizingStretchProp | ImGuiTableFlags_BordersInnerV)) {
        ImGui::TableSetupColumn("Phase", ImGuiTableColumnFlags_WidthStretch, 0.62f);
        ImGui::TableSetupColumn("ms / frame", ImGuiTableColumnFlags_WidthStretch, 0.19f);
        ImGui::TableSetupColumn("share of the frame", ImGuiTableColumnFlags_WidthStretch, 0.19f);
        ImGui::TableHeadersRow();
        for (const PhaseRow& p : kPhases) {
            ImGui::TableNextRow();
            ImGui::TableSetColumnIndex(0);
            if (p.depth) ImGui::Indent(px(18));
            ImGui::TextUnformatted(p.name);
            if (p.depth) ImGui::Unindent(px(18));
            ImGui::TableSetColumnIndex(1);
            ImGui::Text("%.2f", m.phase_ms[p.phase]);
            ImGui::TableSetColumnIndex(2);
            if (m.frame_ms_avg > 0) ImGui::Text(tr("%.0f%%"), 100.0 * m.phase_ms[p.phase] / m.frame_ms_avg);
        }
        ImGui::EndTable();
    }
    if (r) {
        const gfx::RendererStats rs = r->stats();
        ImGui::Text(tr("Frame %.2f ms on average. GPU, the present pass (output, scaling, this menu): %.2f ms (avg %.2f)."), m.frame_ms_avg,
                    static_cast<double>(rs.gpu_ms_last), static_cast<double>(rs.gpu_ms_avg));
        const VkExtent2D e = r->extent();
        ImGui::TextDisabled(tr("GPU %s; swapchain %u x %u, %u images, present mode %s; %llu frames, %llu swapchains"), ov.context().device->gpu().name.c_str(),
                            e.width, e.height, r->image_count(), gfx::present_mode_name(r->present_mode()),
                            static_cast<unsigned long long>(rs.frames), static_cast<unsigned long long>(rs.swapchain_creates));
    }
    if (const extras::LowLatency* ll = extras::lowlat(); ll && ll->stats().enabled) {
        const extras::LowLatency::Stats st = ll->stats();
        ImGui::TextDisabled(tr("%s: input to present %.2f ms, input to GPU end %.2f ms, sleep %.2f ms"), ll->backend_name(), st.input_to_present_ms,
                            st.input_to_gpu_ms, st.sleep_ms_avg);
    }
    if (const int left = gfx::shader_backlog(); left > 0) ImGui::TextColored(kOrange, tr("Preparing shaders: %d left"), left);
    if (button(m, "Write the last 30 s of frame times to the log")) {
        NXLOG_I(kTag, "frame times (F1 menu):\n%s", frame_times_summary(30.0).c_str());
        notify(m, "Frame time summary written to the log.");
    }

    cpu_group(m);

    section("Game clock");
    const ClockInfo ck = clock_info();
    if (ck.valid)
        ImGui::Text(tr("%.3fx real time over %d s (1.000 is right); target %d fps, display rate %d, step %.2f frames"), ck.rate, ck.window_s, ck.target,
                    ck.display, static_cast<double>(ck.step));
    else
        ImGui::TextDisabled("%s", tr("Not running yet (title or loading)."));
    ImGui::TextDisabled(tr("Hooks at start: fps60 %s, fps120 %s, smooth step %s"), ck.fps60 ? "on" : "off", ck.fps120 ? "on" : "off", ck.smooth ? "on" : "off");

    section("Ultrawide");
    const WideInfo wi = wide_info();
    if (!wi.active) {
        ImGui::TextDisabled("%s", tr("Not installed: the output is 16:9 or narrower, or [fefw] ultrawide = off."));
    } else {
        ImGui::Text(tr("Output %d x %d (%.4fx 16:9). Times each hook changed something:"), wi.out_w, wi.out_h, static_cast<double>(wi.k));
        for (int n = 0; n < WideInfo::kCounters; ++n) {
            ImGui::TextDisabled(tr("%s %llu"), wi.names[n] ? wi.names[n] : "?", static_cast<unsigned long long>(wi.counts[n]));
            if (n % 5 != 4 && n + 1 < WideInfo::kCounters) ImGui::SameLine(0, 18);
        }
    }

    section("Logging");
    if (begin_rows("log")) {
        const IniKey& clk = *ini_key("fefw", "clock_log");
        row(m, "Game clock log", When::Live, "A log line with the game clock's speed every this many seconds ([fefw] clock_log); 0 is off.");
        if (!env_locked(clk)) {
            int period = ck.period_s;
            if (islider(m, "##clk", &period, 0, 60, 5, period ? "every %d s" : "off")) {
                set_clock_log(period);
                ini_put(m, clk, std::to_string(period));
            }
        }
        const IniKey& lvl = *ini_key("log", "level");
        row(m, "Log level", When::Live, "The lowest level written to the log ([log] level).");
        if (!env_locked(lvl)) {
            int idx = static_cast<int>(log_level());
            if (combo(m, "##lvl", &idx, kLevelNames, 5)) {
                log_set_level(static_cast<LogLevel>(idx));
                ini_put(m, lvl, kLevelVals[idx]);
            }
        }
        constexpr const char* kOnOff[] = {"Off", "On"};
        constexpr const char* kOnOffVals[] = {"0", "1"};
        ini_combo_row(m, *ini_key("gfx", "log_fps"), "Frame rate log", "One line a second with the presented frame rate and the present pass's GPU time ([gfx] log_fps).",
                      kOnOff, kOnOffVals, 2);
        ini_combo_row(m, *ini_key("play", "log_status"), "Play status log", "One line a second with play mode's input state ([play] log_status).", kOnOff,
                      kOnOffVals, 2);
        constexpr const char* kSum[] = {"Off", "Every 5 s", "Every 10 s (default)", "Every 30 s", "Every 60 s"};
        constexpr const char* kSumVals[] = {"0", "5", "10", "30", "60"};
        ini_combo_row(m, *ini_key("diag", "hitch_summary_s"), "Frame summary log", "Frames, mean and 1% low fps, worst frame ([diag] hitch_summary_s).", kSum,
                      kSumVals, 5);
        row(m, "Low latency timings", When::Restart, "One line of Reflex / Anti-Lag timings a second ([latency] log_timings in controls.ini).");
        bool lt = m.edit.extras.latency.log_timings;
        if (check(m, "##lt", &lt)) {
            m.edit.extras.latency.log_timings = lt;
            apply(host, m);
        }
        pending_note(lt != m.boot.extras.latency.log_timings, m.boot.extras.latency.log_timings ? "on" : "off");
        end_rows();
    }
    ImGui::TextDisabled(tr("Log: %llu errors, %llu warnings so far"), static_cast<unsigned long long>(log_count(LogLevel::Error)),
                        static_cast<unsigned long long>(log_count(LogLevel::Warn)));
}

// ---- what waits for a restart -----------------------------------------------------------------------------------------------------------
std::string pending_list(Menu& m) {
    std::string s;
    auto add = [&](const char* what) { s += (s.empty() ? "" : ", ") + std::string(what); };
    for (const IniKey& k : kIni)
        if (ini_pending(m, k)) add(k.label);
    const play::Settings& e = m.edit;
    const play::Settings& b = m.boot;
    if (e.extras.hdr.mode != b.extras.hdr.mode) add("HDR output");
    if (e.extras.hdr.format != b.extras.hdr.format) add("HDR signal");
    if (e.extras.latency.mode != b.extras.latency.mode) add("low latency");
    if (e.extras.latency.log_timings != b.extras.latency.log_timings) add("latency log");
    if (e.audio.device != b.audio.device) add("audio device");
    if (e.audio.channels != b.audio.channels) add("speakers");
    if (std::fabs(e.display.npc_distance - npc_distance_applied()) > 1e-6) add("NPC limit");
    return s;
}

}  // namespace

// ---- the window -------------------------------------------------------------------------------------------------------------------------
void settings_menu(play::UiHost& host, gfx::Overlay& ov) {
    static bool i18n_once = [] { nxrt::gfx::menu_i18n_init(); return true; }();
    (void)i18n_once;
    Menu& m = menu();
    if (!m.loaded) {
        m.boot = host.settings();
        for (const IniKey& k : kIni) ini_boot(m, k);
        m.loaded = true;
    }

    const int frame = ImGui::GetFrameCount();
    const bool opened = frame != m.last_frame + 1;
    m.last_frame = frame;
    if (opened) {   // the gamepad cursor starts on the first row of the page shown, so A and the d-pad work at once
        m.focus_first = true;
        m.focus_tab = m.tab;
    }
    if (!ImGui::IsAnyItemActive()) m.edit = host.settings();   // the working copy follows the game (the counter hotkey, Alt+Enter)
    const play::LiveStatus live = host.status();
    StyleScope style;

    // B or Escape go back; with nothing open or held they close the menu
    const bool back = ImGui::IsKeyPressed(ImGuiKey_GamepadFaceRight, false) || ImGui::IsKeyPressed(ImGuiKey_Escape, false);
    if (back && !m.busy_last && !opened) ov.request_hide();
    // L / R switch tabs (not while a widget is held: L / R then tweak its speed)
    if (!ImGui::IsAnyItemActive() && !ImGui::IsPopupOpen("", ImGuiPopupFlags_AnyPopup)) {
        int step = ImGui::IsKeyPressed(ImGuiKey_GamepadR1, false) ? 1 : ImGui::IsKeyPressed(ImGuiKey_GamepadL1, false) ? -1 : 0;
        if (step) {
            m.tab_request = (m.tab + step + kTabCount) % kTabCount;
            if (m.tab_request == kTabMods && !gfx::has_mod_panels()) m.tab_request = (m.tab_request + step + kTabCount) % kTabCount;
            m.focus_tab = m.tab_request;
            m.focus_first = true;
        }
    }

    const ImVec2 disp = ImGui::GetIO().DisplaySize;
    ImGui::GetBackgroundDrawList()->AddRectFilled(ImVec2(0, 0), disp, IM_COL32(0, 0, 0, 130));
    const float w = std::min(disp.x - px(32), std::max(px(760), std::min(px(1320), disp.x * 0.91f)));
    const float h = std::min(disp.y - px(32), std::max(px(500), std::min(px(920), disp.y * 0.90f)));
    ImGui::SetNextWindowPos(ImVec2(disp.x * 0.5f, disp.y * 0.5f), ImGuiCond_Always, ImVec2(0.5f, 0.5f));
    ImGui::SetNextWindowSize(ImVec2(w, h), ImGuiCond_Always);
    if (opened) ImGui::SetNextWindowFocus();
    const ImGuiWindowFlags wf = ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoSavedSettings;
    if (ImGui::Begin("##fefw_menu", nullptr, wf)) {
        draw_header(live);

        m.help.clear();
        const float help_h = ImGui::GetTextLineHeightWithSpacing() * 2.2f;
        const float bottom = help_h + ImGui::GetFrameHeightWithSpacing() + ImGui::GetStyle().ItemSpacing.y * 4 + px(8);
        if (ImGui::BeginTabBar("##tabs", ImGuiTabBarFlags_NoTabListScrollingButtons | ImGuiTabBarFlags_FittingPolicyShrink)) {
            const bool mods_tab = gfx::has_mod_panels();   // the Mods tab only when a code mod added a panel
            for (int t = 0; t < kTabCount; ++t) {
                if (t == kTabMods && !mods_tab) continue;
                char label[48];
                std::snprintf(label, sizeof label, "  %s  ###tab%d", tr(kTabNames[t]), t);
                if (!ImGui::BeginTabItem(label, nullptr, m.tab_request == t ? ImGuiTabItemFlags_SetSelected : 0)) continue;
                m.tab = t;
                ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(px(14), px(12)));
                ImGui::BeginChild("##page", ImVec2(0, -bottom), ImGuiChildFlags_NavFlattened | ImGuiChildFlags_Borders);
                ImGui::PushItemWidth(-FLT_MIN);
                switch (t) {
                    case kTabDisplay: display_page(host, m, live); break;
                    case kTabGraphics: graphics_page(host, m); break;
                    case kTabFrameRate: framerate_page(host, m, live, ov); break;
                    case kTabAudio: audio_page(host, m); break;
                    case kTabControls: controls_page(host, m, live); break;
                    case kTabGame: game_page(host, m); break;
                    case kTabMods:   // the code mods' panels (mod API 4): they apply at once, not with this menu's Save
                        ImGui::TextColored(kMuted, "%s", tr("Code mods' own settings. Changes apply at once."));
                        ImGui::Spacing();
                        ImGui::PushItemWidth(ImGui::GetContentRegionAvail().x * 0.55f);   // room for the sliders' labels
                        gfx::draw_mod_panels();
                        ImGui::PopItemWidth();
                        break;
                    case kTabDebug: debug_page(host, m, ov); break;
                }
                ImGui::PopItemWidth();
                ImGui::EndChild();
                ImGui::PopStyleVar();
                ImGui::EndTabItem();
            }
            m.tab_request = -1;
            ImGui::EndTabBar();
        }

        // bottom: the row's description, the notice or what waits for a restart, Close
        ImGui::Spacing();
        ImGui::Separator();
        ImGui::Spacing();
        ImGui::PushTextWrapPos(0.f);
        if (!m.help.empty()) ImGui::TextUnformatted(tr(m.help.c_str()));
        else ImGui::TextDisabled("%s", tr("Changes apply at once and are saved. Rows marked restart apply when the game starts again."));
        ImGui::PopTextWrapPos();
        ImGui::SetCursorPosY(ImGui::GetWindowHeight() - ImGui::GetStyle().WindowPadding.y - ImGui::GetFrameHeight());
        const std::string waits = pending_list(m);
        if (!m.notice.empty() && ImGui::GetTime() < m.notice_until) {
            ImGui::AlignTextToFramePadding();
            ImGui::TextColored(m.notice_err ? kRed : kGreen, "%s", m.notice.c_str());
        } else if (!waits.empty()) {
            ImGui::AlignTextToFramePadding();
            ImGui::TextColored(kOrange, tr("Restart the game to apply: %s"), waits.c_str());
        } else {
            ImGui::AlignTextToFramePadding();
            ImGui::TextDisabled("%s", tr("Saved to fefw.ini and controls.ini"));
        }
        const char* close = tr("Close");
        const float bw = ImGui::CalcTextSize(close).x + ImGui::GetStyle().FramePadding.x * 6;
        // Quit game (#72): the same exit as the window's close button (settings saved, caches flushed, exit code 0). Two clicks, so a
        // slip on the way to Close does not end the session.
        static double quit_armed_until = 0;
        const bool quit_armed = ImGui::GetTime() < quit_armed_until;
        const char* quit_ask = tr("Click again to quit");
        const char* quit_idle = tr("Quit game");
        const float qw = std::max(ImGui::CalcTextSize(quit_ask).x, ImGui::CalcTextSize(quit_idle).x) + ImGui::GetStyle().FramePadding.x * 6;
        ImGui::SameLine(ImGui::GetWindowWidth() - ImGui::GetStyle().WindowPadding.x - bw - ImGui::GetStyle().ItemSpacing.x - qw);
        const std::string quit_label = std::string(quit_armed ? quit_ask : quit_idle) + "###fefw_quit";
        if (ImGui::Button(quit_label.c_str(), ImVec2(qw, 0))) {
            if (quit_armed) play::exit_process(0, "pass", "quit from the F1 menu");
            quit_armed_until = ImGui::GetTime() + 4.0;
        }
        ImGui::SameLine(ImGui::GetWindowWidth() - ImGui::GetStyle().WindowPadding.x - bw);
        ImGui::PushStyleColor(ImGuiCol_Button, kAccentDark);
        ImGui::PushStyleColor(ImGuiCol_ButtonHovered, ImVec4(0.20f, 0.44f, 0.84f, 1.0f));
        ImGui::PushStyleColor(ImGuiCol_ButtonActive, ImVec4(0.12f, 0.28f, 0.60f, 1.0f));
        if (ImGui::Button(close, ImVec2(bw, 0))) ov.request_hide();
        ImGui::PopStyleColor(3);
    }
    ImGui::End();
    m.busy_last = ImGui::IsAnyItemActive() || ImGui::IsPopupOpen("", ImGuiPopupFlags_AnyPopup);
    static const bool debug_nav = config_get_bool("menu", "debug_nav", false);   // [menu] debug_nav: log the gamepad cursor (test aid)
    if (debug_nav) {
        ImGuiContext& g = *GImGui;
        static ImGuiID last_nav = 1;
        static bool last_pop = false;
        const bool pop = ImGui::IsPopupOpen("", ImGuiPopupFlags_AnyPopup);
        if (g.NavId != last_nav || pop != last_pop || opened)
            NXLOG_I(kTag, "nav: id %08x window %s cursor %d active %08x popup %d focus_first %d tab %d", g.NavId, g.NavWindow ? g.NavWindow->Name : "-",
                    g.NavCursorVisible ? 1 : 0, g.ActiveId, pop ? 1 : 0, m.focus_first ? 1 : 0, m.tab);
        last_nav = g.NavId;
        last_pop = pop;
    }
}

}  // namespace nxrt::games::fefw
