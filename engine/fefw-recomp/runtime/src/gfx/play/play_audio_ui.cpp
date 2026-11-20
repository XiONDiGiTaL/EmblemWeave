// SPDX-License-Identifier: GPL-3.0-or-later
// The F1 audio controls shared by play_ui.cpp and fefw_menu.cpp. The panel deliberately draws its own glass surface and controls:
// ImGui's stock widgets are used everywhere else, but this is the one place the launcher's XiØN glass-and-liquid language appears in
// the game. Colours are the launcher's: near-black graphite, #3F80FF, #79A8FF and silver ink.
#include "play_audio_ui.h"

#include <algorithm>
#include <cmath>
#include <cstdio>

#include "imgui.h"
#include "imgui_internal.h"   // ImRect, used by the custom slider tracks
#include "nxrt/audio_sink.h"
#include "nxrt/ui_theme.h"

namespace nxrt::play::audio_ui {
namespace {

// The audio panel uses the shared XiØN tokens, not a second palette.
const ImVec4& kAccent = ui_theme::accent;
const ImVec4& kAccentHi = ui_theme::accent_hi;
const ImVec4& kInk = ui_theme::ink;
const ImVec4& kMuted = ui_theme::muted;
const ImVec4& kDanger = ui_theme::bad;
const ImVec4& kGlass = ui_theme::glass;

float px(float v) { return v * ImGui::GetFontSize() / 16.0f; }

ImU32 rgba(const ImVec4& c, float alpha = 1.0f) {
    ImVec4 v = c;
    v.w *= alpha;
    return ImGui::ColorConvertFloat4ToU32(v);
}

bool pressed(ImGuiKey a, ImGuiKey b) { return ImGui::IsKeyPressed(a, true) || ImGui::IsKeyPressed(b, true); }

ImVec2 center_text(const char* text, const ImVec2& min, const ImVec2& max) {
    const ImVec2 size = ImGui::CalcTextSize(text);
    return ImVec2(min.x + (max.x - min.x - size.x) * 0.5f, min.y + (max.y - min.y - size.y) * 0.5f);
}

bool liquid_button(const char* id, const char* text, bool active, const ImVec2& size, bool danger = false) {
    ImGui::PushID(id);
    const bool clicked = ImGui::InvisibleButton("##liquid", size);
    const bool hovered = ImGui::IsItemHovered();
    const bool held = ImGui::IsItemActive();
    const ImVec2 min = ImGui::GetItemRectMin(), max = ImGui::GetItemRectMax();
    const float rounding = size.y * 0.5f;
    const ImVec4 tone = danger ? kDanger : kAccent;
    ImDrawList* dl = ImGui::GetWindowDrawList();
    const float lift = (hovered ? 1.0f : 0.0f) + (held ? 1.0f : 0.0f);

    // The halo and the same soft vertical gradient the launcher's buttons use.
    dl->AddRectFilled(ImVec2(min.x - px(3), min.y - px(3)), ImVec2(max.x + px(3), max.y + px(3)), rgba(tone, 0.06f + 0.05f * lift),
                      rounding + px(3));
    const ImVec4 top = active ? ImVec4(tone.x, tone.y, tone.z, 0.96f) : ui_theme::with_alpha(ui_theme::field_hover, 0.96f);
    const ImVec4 bottom = active ? ImVec4(tone.x * 0.72f, tone.y * 0.76f, tone.z * 0.95f, 0.96f) : ui_theme::with_alpha(ui_theme::field, 0.96f);
    dl->AddRectFilledMultiColor(min, max, rgba(top), rgba(top), rgba(bottom), rgba(bottom));
    dl->AddRectFilled(ImVec2(min.x + px(2), min.y + px(2)), ImVec2(max.x - px(2), min.y + size.y * 0.46f), rgba(ImVec4(1, 1, 1, 1), 0.055f),
                      rounding);
    dl->AddRect(min, max, rgba(active ? kAccentHi : tone, active ? 0.85f : 0.26f), rounding, 0, px(1));
    if (hovered || ImGui::IsItemFocused()) dl->AddRect(ImVec2(min.x - px(2), min.y - px(2)), ImVec2(max.x + px(2), max.y + px(2)),
                                                       rgba(kAccentHi, 0.42f + 0.25f * lift), rounding + px(2), 0, px(1));
    const ImVec2 at = center_text(text, min, max);
    dl->AddText(at, rgba(active ? ImVec4(1, 1, 1, 1) : kInk), text);
    ImGui::PopID();
    return clicked;
}

bool toggle(const char* id, bool* value) {
    ImGui::PushID(id);
    const ImVec2 size(px(54), px(27));
    const bool clicked = ImGui::InvisibleButton("##toggle", size);
    if (clicked && value) *value = !*value;
    const ImVec2 min = ImGui::GetItemRectMin(), max = ImGui::GetItemRectMax();
    const float rounding = size.y * 0.5f;
    const float t = value && *value ? 1.0f : 0.0f;
    ImDrawList* dl = ImGui::GetWindowDrawList();
    const ImVec4 track = t > 0.5f ? kAccent : ui_theme::field_active;
    dl->AddRectFilled(min, max, rgba(track), rounding);
    if (t > 0.5f) dl->AddRectFilled(ImVec2(min.x - px(2), min.y - px(2)), ImVec2(max.x + px(2), max.y + px(2)), rgba(kAccent, 0.12f),
                                    rounding + px(2));
    dl->AddRect(min, max, rgba(kAccentHi, 0.35f), rounding, 0, px(1));
    // The knob's centre travels between the track's two rounded ends, not from one edge to the other, so it never runs off the track.
    const float x = min.x + size.y * 0.5f + (size.x - size.y) * t;
    const ImVec2 knob(x, min.y + size.y * 0.5f);
    dl->AddCircleFilled(knob, px(10), IM_COL32(248, 250, 255, 255));
    dl->AddCircleFilled(ImVec2(knob.x - px(2.5f), knob.y - px(3)), px(3), rgba(ImVec4(1, 1, 1, 1), 0.75f));
    if (ImGui::IsItemFocused()) dl->AddRect(ImVec2(min.x - px(2), min.y - px(2)), ImVec2(max.x + px(2), max.y + px(2)),
                                            rgba(kAccentHi, 0.55f), rounding + px(2), 0, px(1));
    ImGui::PopID();
    return clicked;
}

// A horizontal glass slider with a defined value column. Drag, click, arrow keys and the d-pad all work.
bool glow_slider(const char* id, float* value, float lo, float hi, const char* fmt, float width) {
    if (!value) return false;
    ImGui::PushID(id);
    const float height = px(32);
    const float value_width = px(52);
    const ImVec2 size(std::max(px(120), width), height);
    ImGui::InvisibleButton("##glow", size);
    const ImVec2 min = ImGui::GetItemRectMin(), max = ImGui::GetItemRectMax();
    const float track_right = max.x - value_width;
    const ImRect track(ImVec2(min.x + px(2), min.y + height * 0.5f - px(4)),
                       ImVec2(track_right, min.y + height * 0.5f + px(4)));
    float t = (std::clamp(*value, lo, hi) - lo) / (hi - lo);
    bool changed = false;
    if (ImGui::IsItemActive() && ImGui::IsMouseDown(ImGuiMouseButton_Left)) {
        const float nt = std::clamp((ImGui::GetIO().MousePos.x - track.Min.x) / std::max(px(1), track.GetWidth()), 0.0f, 1.0f);
        *value = lo + (hi - lo) * nt;
        t = nt;
        changed = true;
    } else if (ImGui::IsItemFocused()) {
        int dir = pressed(ImGuiKey_GamepadDpadLeft, ImGuiKey_LeftArrow) ? -1 : pressed(ImGuiKey_GamepadDpadRight, ImGuiKey_RightArrow) ? 1 : 0;
        if (dir) {
            *value = std::clamp(*value + dir * 0.5f, lo, hi);
            changed = true;
        }
    }
    if (ImGui::IsItemHovered() && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left)) {
        *value = 0.0f;
        changed = true;
    }

    ImDrawList* dl = ImGui::GetWindowDrawList();
    dl->AddRectFilled(track.Min, track.Max, IM_COL32(15, 19, 26, 255), px(4));
    dl->AddRect(track.Min, track.Max, rgba(kAccent, 0.28f), px(4), 0, px(1));
    const ImVec2 fill_max(track.Min.x + track.GetWidth() * t, track.Max.y);
    if (fill_max.x > track.Min.x + 1) {
        dl->AddRectFilled(ImVec2(track.Min.x, track.Min.y - px(2)), ImVec2(fill_max.x, track.Max.y + px(2)), rgba(kAccent, 0.16f), px(6));
        dl->AddRectFilledMultiColor(track.Min, fill_max, rgba(kAccentHi), rgba(kAccentHi), rgba(kAccent), rgba(kAccent));
    }
    const float hx = track.Min.x + track.GetWidth() * t;
    dl->AddCircleFilled(ImVec2(hx, track.GetCenter().y), px(8), rgba(kAccent, 0.18f));
    dl->AddCircleFilled(ImVec2(hx, track.GetCenter().y), px(5), IM_COL32(245, 248, 255, 255));
    dl->AddCircle(ImVec2(hx, track.GetCenter().y), px(6), rgba(kAccentHi, 0.9f), 0, px(1));
    char text[48];
    std::snprintf(text, sizeof text, fmt, static_cast<double>(*value));
    const ImVec2 text_size = ImGui::CalcTextSize(text);
    dl->AddText(ImVec2(max.x - text_size.x, min.y + (height - text_size.y) * 0.5f), rgba(kInk), text);
    ImGui::PopID();
    return changed;
}

// One vertical band fader. The track is centred on 0 dB, so a preset's shape is readable at a glance.
bool band_slider(const char* id, float* value, float height) {
    if (!value) return false;
    ImGui::PushID(id);
    const float width = px(34);
    ImGui::InvisibleButton("##band", ImVec2(width, height));
    const ImVec2 min = ImGui::GetItemRectMin(), max = ImGui::GetItemRectMax();
    bool changed = false;
    const float lo = -12.0f, hi = 12.0f;
    auto set_from_y = [&](float y) {
        const float t = std::clamp(1.0f - (y - min.y) / std::max(1.0f, height), 0.0f, 1.0f);
        *value = std::round((lo + (hi - lo) * t) * 2.0f) * 0.5f;
        changed = true;
    };
    if (ImGui::IsItemActive() && ImGui::IsMouseDown(ImGuiMouseButton_Left)) set_from_y(ImGui::GetIO().MousePos.y);
    else if (ImGui::IsItemFocused()) {
        if (pressed(ImGuiKey_GamepadDpadUp, ImGuiKey_UpArrow)) {
            *value = std::clamp(*value + 0.5f, lo, hi);
            changed = true;
        } else if (pressed(ImGuiKey_GamepadDpadDown, ImGuiKey_DownArrow)) {
            *value = std::clamp(*value - 0.5f, lo, hi);
            changed = true;
        }
    }
    if (ImGui::IsItemHovered() && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left)) {
        *value = 0.0f;
        changed = true;
    }

    ImDrawList* dl = ImGui::GetWindowDrawList();
    const float cx = (min.x + max.x) * 0.5f;
    const ImVec2 track_min(cx - px(4), min.y + px(4)), track_max(cx + px(4), max.y - px(4));
    dl->AddRectFilled(track_min, track_max, IM_COL32(14, 18, 24, 255), px(4));
    dl->AddRect(track_min, track_max, rgba(kAccent, 0.25f), px(4), 0, px(1));
    const float mid = (track_min.y + track_max.y) * 0.5f;
    const float t = (std::clamp(*value, lo, hi) - lo) / (hi - lo);
    const float y = track_max.y - (track_max.y - track_min.y) * t;
    const float top = std::min(mid, y), bottom = std::max(mid, y);
    if (bottom - top > 1.0f) {
        dl->AddRectFilled(ImVec2(track_min.x - px(2), top), ImVec2(track_max.x + px(2), bottom), rgba(kAccent, 0.20f), px(4));
        dl->AddRectFilled(ImVec2(track_min.x, top), ImVec2(track_max.x, bottom), rgba(kAccent), px(3));
    }
    dl->AddLine(ImVec2(track_min.x - px(5), mid), ImVec2(track_max.x + px(5), mid), rgba(kMuted, 0.45f), px(1));
    dl->AddCircleFilled(ImVec2(cx, y), px(11), rgba(kAccent, 0.16f));
    dl->AddCircleFilled(ImVec2(cx, y), px(6), IM_COL32(242, 246, 255, 255));
    dl->AddCircle(ImVec2(cx, y), px(6.5f), rgba(kAccentHi, 0.9f), 0, px(1));
    if (ImGui::IsItemFocused()) dl->AddRect(ImVec2(min.x, min.y), ImVec2(max.x, max.y), rgba(kAccentHi, 0.45f), px(6), 0, px(1));
    ImGui::PopID();
    return changed;
}

void glass_background() {
    const ImVec2 pos = ImGui::GetWindowPos(), size = ImGui::GetWindowSize();
    ImDrawList* dl = ImGui::GetWindowDrawList();
    const float rounding = px(18);
    dl->AddRectFilled(pos, ImVec2(pos.x + size.x, pos.y + size.y), rgba(kGlass), rounding);
    dl->AddRectFilledMultiColor(pos, ImVec2(pos.x + size.x, pos.y + size.y * 0.42f), rgba(ui_theme::accent_deep, 0.18f),
                                rgba(ui_theme::accent_deep, 0.18f), rgba(kGlass, 0.0f), rgba(kGlass, 0.0f));
    dl->AddRect(pos, ImVec2(pos.x + size.x, pos.y + size.y), rgba(kAccent, 0.30f), rounding, 0, px(1));
    dl->AddRect(ImVec2(pos.x - px(1), pos.y - px(1)), ImVec2(pos.x + size.x + px(1), pos.y + size.y + px(1)), rgba(kAccentHi, 0.08f),
                rounding + px(1), 0, px(1));
    dl->AddLine(ImVec2(pos.x + px(22), pos.y + px(1)), ImVec2(pos.x + size.x - px(22), pos.y + px(1)), rgba(kAccentHi, 0.38f), px(1));
}

void section_label(const char* text) {
    ImGui::Spacing();
    ImGui::TextColored(kAccent, "%s", text);
    ImGui::Separator();
    ImGui::Spacing();
}

}  // namespace

audio::EqSettings to_engine(const AudioSettings& a) {
    audio::EqSettings s;
    s.enabled = a.eq_enabled;
    s.preamp_db = static_cast<float>(a.eq_preamp);
    for (int i = 0; i < audio::kEqBandCount; i++) s.bands_db[i] = static_cast<float>(a.eq_bands[i]);
    return s;
}

void preview_live(const AudioSettings& a) {
    audio::set_master_volume(static_cast<float>(a.volume));
    audio::set_eq_settings(to_engine(a));
}

const char* preset_label(int preset) {
    static const char* const kLabels[audio::kEqPresetCount] = {"Flat", "Bass", "Treble", "Vocal", "Warm", "V-Shape", "Night", "Headphones Boost"};
    if (preset < 0 || preset >= audio::kEqPresetCount) return "Custom";
    return kLabels[preset];
}

void apply_preset(AudioSettings& a, int preset) {
    const float* db = audio::eq_preset_db(preset);
    if (!db) return;
    a.eq_preset = audio::eq_preset_name(preset);
    a.eq_preamp = audio::eq_preset_preamp_db(preset);
    for (int i = 0; i < audio::kEqBandCount; i++) a.eq_bands[i] = db[i];
}

bool apply_volume(AudioSettings& a, State& s, double percent) {
    if (!std::isfinite(percent)) return false;
    const double pct = std::clamp(percent, 0.0, 200.0);
    // Once this session has run above 100%, lowering the level and raising it
    // again must not re-arm the safety modal: that read as a hard cap at 100%.
    // The modal is still shown the first time a 100% session is boosted.
    if (a.volume > 1.0001) s.boost_approved = true;
    if (pct > 100.0001 && a.volume <= 1.0001 && !s.boost_approved) {
        s.boost_warning = true;
        s.boost_pending = pct;
        return false;
    }
    const double volume = pct / 100.0;
    if (std::fabs(volume - a.volume) < 1e-9) return false;
    a.volume = volume;
    return true;
}

bool boost_warning(AudioSettings& a, State& s) {
    if (!s.boost_warning) return false;
    static const char* const kPopup = "Enable boost above 100%?###audio_boost_warning";
    if (!ImGui::IsPopupOpen(kPopup)) ImGui::OpenPopup(kPopup);
    ImGui::SetNextWindowSize(ImVec2(std::min(px(560), ImGui::GetIO().DisplaySize.x - px(40)), 0));
    ImGui::PushStyleColor(ImGuiCol_PopupBg, ui_theme::with_alpha(ui_theme::panel, 0.98f));
    ImGui::PushStyleColor(ImGuiCol_Border, rgba(kDanger, 0.45f));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, px(16));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(px(24), px(20)));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, px(1));
    bool open = true;
    bool changed = false;
    if (ImGui::BeginPopupModal(kPopup, &open, ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_AlwaysAutoResize)) {
        ImGui::TextColored(kDanger, "AUDIO AT RISK");
        ImGui::Spacing();
        ImGui::PushTextWrapPos(ImGui::GetCursorPos().x + px(500));
        ImGui::TextUnformatted("You are about to raise the master volume above 100%, up to 200% (about +6 dB).");
        ImGui::Spacing();
        ImGui::TextUnformatted("Sustained loud output can damage speakers, headphones or your hearing. The engine's limiter stops digital "
                              "clipping, but it cannot make a loud signal safe for your ears or your equipment. Start low and turn it up "
                              "only as far as you need.");
        ImGui::PopTextWrapPos();
        ImGui::Spacing();
        ImGui::TextDisabled("This choice applies to the current settings and is saved with them.");
        ImGui::Spacing();
        ImGui::Separator();
        ImGui::Spacing();
        if (liquid_button("cancel", "Cancel", false, ImVec2(px(130), px(36)))) {
            a.volume = std::min(a.volume, 1.0);
            s.boost_warning = false;
            changed = true;
            ImGui::CloseCurrentPopup();
        }
        ImGui::SameLine();
        if (liquid_button("risk", "I know the risks", true, ImVec2(px(180), px(36)), true)) {
            a.volume = std::clamp(s.boost_pending / 100.0, 1.0, 2.0);
            s.boost_warning = false;
            s.boost_approved = true;
            changed = true;
            ImGui::CloseCurrentPopup();
        }
        ImGui::EndPopup();
    } else if (!open) {
        a.volume = std::min(a.volume, 1.0);
        s.boost_warning = false;
        changed = true;
    }
    ImGui::PopStyleVar(3);
    ImGui::PopStyleColor(2);
    return changed;
}

bool enhancements_button(AudioSettings& a, State& s) {
    const float width = std::max(px(220), ImGui::GetContentRegionAvail().x);
    const bool clicked = liquid_button("audio-enhancements", "Audio Enhancements", false, ImVec2(width, px(42)));
    if (clicked) {
        s.eq_open = true;
        s.eq_focus = true;
    }
    const int preset = audio::eq_preset_from_name(a.eq_preset.c_str());
    ImGui::TextDisabled("Equalizer %s  ·  %s", a.eq_enabled ? "on" : "off", preset_label(preset));
    return clicked;
}

Edit enhancements_window(AudioSettings& a, State& s) {
    if (!s.eq_open) return Edit::None;

    const ImGuiIO& io = ImGui::GetIO();
    if (!s.eq_placed || io.DisplaySize.x != s.last_display.x || io.DisplaySize.y != s.last_display.y) {
        s.last_display = io.DisplaySize;
        s.eq_placed = true;
    }
    const ImVec2 size(std::min(io.DisplaySize.x - px(44), px(840)), std::min(io.DisplaySize.y - px(44), px(640)));
    ImGui::SetNextWindowPos(ImVec2(io.DisplaySize.x * 0.5f, io.DisplaySize.y * 0.5f), ImGuiCond_Always, ImVec2(0.5f, 0.5f));
    ImGui::SetNextWindowSize(size, ImGuiCond_Always);
    if (s.eq_focus) {
        ImGui::SetNextWindowFocus();
        s.eq_focus = false;
    }
    ImGui::PushStyleColor(ImGuiCol_WindowBg, ui_theme::white(0.0f));
    ImGui::PushStyleColor(ImGuiCol_Border, ui_theme::white(0.0f));
    ImGui::PushStyleColor(ImGuiCol_ChildBg, ui_theme::white(0.0f));
    ImGui::PushStyleColor(ImGuiCol_Text, kInk);
    ImGui::PushStyleColor(ImGuiCol_TextDisabled, kMuted);
    ImGui::PushStyleColor(ImGuiCol_Separator, rgba(kAccent, 0.18f));
    ImGui::PushStyleColor(ImGuiCol_FrameBg, ui_theme::field);
    ImGui::PushStyleColor(ImGuiCol_FrameBgHovered, ui_theme::field_hover);
    ImGui::PushStyleColor(ImGuiCol_FrameBgActive, ui_theme::field_active);
    ImGui::PushStyleColor(ImGuiCol_SliderGrab, kAccent);
    ImGui::PushStyleColor(ImGuiCol_SliderGrabActive, kAccentHi);
    ImGui::PushStyleColor(ImGuiCol_ScrollbarBg, ui_theme::white(0.0f));
    ImGui::PushStyleColor(ImGuiCol_ScrollbarGrab, rgba(kAccent, 0.45f));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, px(18));
    ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(px(24), px(18)));
    ImGui::PushStyleVar(ImGuiStyleVar_ChildRounding, px(12));
    ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(px(10), px(10)));
    ImGui::PushStyleVar(ImGuiStyleVar_FrameRounding, px(8));

    Edit result = Edit::None;
    if (ImGui::Begin("##audio_enhancements", nullptr,
                     ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoResize | ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoCollapse |
                         ImGuiWindowFlags_NoSavedSettings)) {
        glass_background();
        ImGui::TextColored(kAccentHi, "AUDIO ENHANCEMENTS");
        ImGui::SameLine();
        const float title_width = ImGui::GetContentRegionAvail().x;
        ImGui::SetCursorPosX(ImGui::GetCursorPosX() + title_width - px(30));
        if (liquid_button("close-panel", "X", false, ImVec2(px(30), px(30)))) {
            s.eq_open = false;
            if (s.edited_live) result = Edit::Commit;
        }
        ImGui::TextDisabled("Final-mix EQ and master gain  ·  XiØN audio");
        ImGui::Spacing();

        bool live = false;
        bool commit = false;
        if (ImGui::BeginChild("##audio_eq_body", ImVec2(0, 0), false, ImGuiWindowFlags_None)) {
            section_label("MASTER");
            ImGui::TextUnformatted("Master volume");
            ImGui::SameLine();
            ImGui::TextDisabled("0-200%%");
            float pct = static_cast<float>(a.volume * 100.0);
            if (glow_slider("master", &pct, 0.0f, 200.0f, "%.0f%%", ImGui::GetContentRegionAvail().x - px(8))) {
                if (apply_volume(a, s, pct)) live = true;
            }
            ImGui::TextDisabled("Above 100%% is a limited boost. The warning is shown once per session.");

            section_label("EQUALIZER");
            ImGui::TextUnformatted("Enable equalizer");
            ImGui::SameLine();
            if (toggle("eq", &a.eq_enabled)) {
                s.edited_live = false;
                commit = true;
            }
            ImGui::SameLine();
            ImGui::TextDisabled(a.eq_enabled ? "on the final mix" : "bypassed");
            ImGui::Spacing();

            ImGui::BeginDisabled(!a.eq_enabled);
            ImGui::TextDisabled("PRESETS");
            if (ImGui::BeginTable("##eq_presets", 4, ImGuiTableFlags_SizingStretchSame)) {
                for (int p = 0; p < audio::kEqPresetCount; p++) {
                    ImGui::TableNextColumn();
                    const int active = audio::eq_preset_from_name(a.eq_preset.c_str());
                    char id[24];
                    std::snprintf(id, sizeof id, "preset%d", p);
                    if (liquid_button(id, preset_label(p), p == active, ImVec2(ImGui::GetContentRegionAvail().x, px(32)))) {
                        apply_preset(a, p);
                        s.edited_live = false;
                        commit = true;
                    }
                }
                ImGui::EndTable();
            }
            const int current = audio::eq_preset_from_name(a.eq_preset.c_str());
            ImGui::TextDisabled("Curve: %s%s", current == audio::kEqPresetCustom ? "Custom" : preset_label(current),
                                current == audio::kEqPresetCustom ? " (move a band to make one)" : "");
            ImGui::Spacing();

            ImGui::TextDisabled("BANDS  ·  ±12 dB  ·  double-click a band to reset it");
            if (ImGui::BeginTable("##eq_bands", audio::kEqBandCount, ImGuiTableFlags_SizingStretchSame)) {
                for (int b = 0; b < audio::kEqBandCount; b++) ImGui::TableSetupColumn("##band", ImGuiTableColumnFlags_WidthStretch);
                ImGui::TableNextRow();
                for (int b = 0; b < audio::kEqBandCount; b++) {
                    ImGui::TableNextColumn();
                    float db = static_cast<float>(a.eq_bands[b]);
                    ImGui::Text("%+.1f", db);
                    char id[24];
                    std::snprintf(id, sizeof id, "band%d", b);
                    if (band_slider(id, &db, px(148))) {
                        a.eq_bands[b] = db;
                        a.eq_preset = "custom";
                        live = true;
                    }
                    ImGui::TextDisabled("%s", audio::eq_band_label(b));
                }
                ImGui::EndTable();
            }
            ImGui::Spacing();
            ImGui::TextUnformatted("Preamp");
            ImGui::SameLine();
            ImGui::TextDisabled("headroom is applied automatically");
            float preamp = static_cast<float>(a.eq_preamp);
            if (glow_slider("preamp", &preamp, -12.0f, 12.0f, "%+.1f dB", ImGui::GetContentRegionAvail().x - px(8))) {
                a.eq_preamp = preamp;
                live = true;
            }
            ImGui::EndDisabled();
            ImGui::Spacing();
            ImGui::PushTextWrapPos(ImGui::GetCursorPos().x + ImGui::GetContentRegionAvail().x);
            ImGui::TextDisabled("The EQ runs on the final 16-bit mix, after the game's voices and before the master boost. A peak limiter "
                                "holds the result below clipping, so boosts cannot create digital distortion.");
            ImGui::PopTextWrapPos();
        }
        ImGui::EndChild();

        if (s.edited_live && !ImGui::IsMouseDown(ImGuiMouseButton_Left)) {
            s.edited_live = false;
            commit = true;
        }
        if (live) s.edited_live = true;
        if (commit) result = Edit::Commit;
        else if (live) result = Edit::Live;
    }
    ImGui::End();
    ImGui::PopStyleVar(5);
    ImGui::PopStyleColor(13);
    return result;
}

}  // namespace nxrt::play::audio_ui
