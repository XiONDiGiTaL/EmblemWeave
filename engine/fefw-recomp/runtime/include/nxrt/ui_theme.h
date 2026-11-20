// SPDX-License-Identifier: GPL-3.0-or-later
// The single source of truth for EmblemWeave's in-game interface.
//
// The values below are the XiØN identity used by xiondigital.net and by the
// launcher (apps/emblemweave/src/theme.rs): graphite surfaces, silver text and
// one blue accent. Every native overlay -- the F1 menu, diagnostics, the code
// mod panels, the on-screen keyboard, notifications and the audio panel --
// draws from these tokens instead of carrying its own palette.
#pragma once

#include <cstdint>

#include <imgui.h>

namespace nxrt::ui_theme {

inline ImVec4 rgb(std::uint32_t hex, float alpha = 1.0f) {
    return ImVec4(static_cast<float>((hex >> 16) & 0xFF) / 255.0f, static_cast<float>((hex >> 8) & 0xFF) / 255.0f,
                  static_cast<float>(hex & 0xFF) / 255.0f, alpha);
}
inline ImVec4 white(float alpha) { return ImVec4(1.0f, 1.0f, 1.0f, alpha); }
inline ImVec4 with_alpha(ImVec4 c, float alpha) { c.w = alpha; return c; }

// Brand palette, from the XiØN site and the launcher's Scheme::Xion.
inline const ImVec4 bg = rgb(0x050607);
inline const ImVec4 bg_deep = rgb(0x07080A);
inline const ImVec4 panel = rgb(0x0A0B0D);
inline const ImVec4 surface = rgb(0x0C0E11);
inline const ImVec4 surface_hi = rgb(0x101318);
inline const ImVec4 field = rgb(0x171B22);
inline const ImVec4 field_hover = rgb(0x202631);
inline const ImVec4 field_active = rgb(0x293446);
inline const ImVec4 line = white(0.07f);
inline const ImVec4 line_strong = white(0.14f);
inline const ImVec4 ink = rgb(0xECEDEF);
inline const ImVec4 body = rgb(0xBDC3CC);
inline const ImVec4 muted = rgb(0x8B9099);
inline const ImVec4 muted_dim = rgb(0x767D87);
inline const ImVec4 accent = rgb(0x3F80FF);
inline const ImVec4 accent_hover = rgb(0x4D8AFF);
inline const ImVec4 accent_hi = rgb(0x79A8FF);
inline const ImVec4 accent_deep = rgb(0x1A3D82);
inline const ImVec4 accent_soft = rgb(0x3F80FF, 0.12f);
inline const ImVec4 ok = rgb(0x4FC98C);
inline const ImVec4 warn = rgb(0xE6B04F);
inline const ImVec4 bad = rgb(0xE55F6A);
inline const ImVec4 glass = rgb(0x050607, 0.94f);
inline const ImVec4 shadow = rgb(0x000000, 0.45f);

// Geometry. The F1 menu adds its own larger radii on top, but every overlay
// starts from the same restrained surface language.
inline constexpr float radius_window = 14.0f;
inline constexpr float radius_panel = 10.0f;
inline constexpr float radius_control = 8.0f;
inline constexpr float space_xs = 4.0f;
inline constexpr float space_s = 8.0f;
inline constexpr float space_m = 14.0f;
inline constexpr float space_l = 22.0f;

// Applies the XiØN colours to an ImGui style. Sizes are left to the caller's
// UI scale; the overlay calls this before capturing its base style, and the F1
// menu calls it inside its StyleScope.
inline void apply(ImGuiStyle& s) {
    s.WindowRounding = radius_window;
    s.ChildRounding = radius_panel;
    s.FrameRounding = radius_control;
    s.PopupRounding = radius_panel;
    s.TabRounding = radius_control;
    s.GrabRounding = radius_control;
    s.ScrollbarRounding = radius_control;
    s.WindowBorderSize = 1.0f;
    s.ChildBorderSize = 1.0f;
    s.FrameBorderSize = 1.0f;
    s.PopupBorderSize = 1.0f;

    s.Colors[ImGuiCol_Text] = ink;
    s.Colors[ImGuiCol_TextDisabled] = muted;
    s.Colors[ImGuiCol_WindowBg] = glass;
    s.Colors[ImGuiCol_ChildBg] = surface;
    s.Colors[ImGuiCol_PopupBg] = rgb(0x0C0E11, 0.98f);
    s.Colors[ImGuiCol_Border] = line;
    s.Colors[ImGuiCol_BorderShadow] = ImVec4(0, 0, 0, 0);
    s.Colors[ImGuiCol_FrameBg] = field;
    s.Colors[ImGuiCol_FrameBgHovered] = field_hover;
    s.Colors[ImGuiCol_FrameBgActive] = field_active;
    s.Colors[ImGuiCol_TitleBg] = panel;
    s.Colors[ImGuiCol_TitleBgActive] = surface_hi;
    s.Colors[ImGuiCol_TitleBgCollapsed] = bg;
    s.Colors[ImGuiCol_MenuBarBg] = panel;
    s.Colors[ImGuiCol_ScrollbarBg] = bg_deep;
    s.Colors[ImGuiCol_ScrollbarGrab] = field_active;
    s.Colors[ImGuiCol_ScrollbarGrabHovered] = accent_deep;
    s.Colors[ImGuiCol_ScrollbarGrabActive] = accent;
    s.Colors[ImGuiCol_CheckMark] = accent_hi;
    s.Colors[ImGuiCol_SliderGrab] = accent;
    s.Colors[ImGuiCol_SliderGrabActive] = accent_hi;
    s.Colors[ImGuiCol_Button] = field;
    s.Colors[ImGuiCol_ButtonHovered] = field_hover;
    s.Colors[ImGuiCol_ButtonActive] = accent_deep;
    s.Colors[ImGuiCol_Header] = accent_soft;
    s.Colors[ImGuiCol_HeaderHovered] = rgb(0x3F80FF, 0.20f);
    s.Colors[ImGuiCol_HeaderActive] = rgb(0x3F80FF, 0.30f);
    s.Colors[ImGuiCol_Separator] = line;
    s.Colors[ImGuiCol_SeparatorHovered] = accent_hi;
    s.Colors[ImGuiCol_SeparatorActive] = accent;
    s.Colors[ImGuiCol_ResizeGrip] = rgb(0x3F80FF, 0.20f);
    s.Colors[ImGuiCol_ResizeGripHovered] = accent_hi;
    s.Colors[ImGuiCol_ResizeGripActive] = accent;
    s.Colors[ImGuiCol_Tab] = ImVec4(1, 1, 1, 0);
    s.Colors[ImGuiCol_TabHovered] = rgb(0x3F80FF, 0.20f);
    s.Colors[ImGuiCol_TabSelected] = rgb(0x3F80FF, 0.16f);
    s.Colors[ImGuiCol_TabSelectedOverline] = accent;
    s.Colors[ImGuiCol_TabDimmed] = ImVec4(1, 1, 1, 0);
    s.Colors[ImGuiCol_TabDimmedSelected] = rgb(0x3F80FF, 0.08f);
    s.Colors[ImGuiCol_TabDimmedSelectedOverline] = rgb(0x3F80FF, 0.35f);
    s.Colors[ImGuiCol_PlotLines] = accent_hi;
    s.Colors[ImGuiCol_PlotLinesHovered] = ink;
    s.Colors[ImGuiCol_PlotHistogram] = accent;
    s.Colors[ImGuiCol_PlotHistogramHovered] = accent_hi;
    s.Colors[ImGuiCol_TableHeaderBg] = surface_hi;
    s.Colors[ImGuiCol_TableBorderStrong] = line_strong;
    s.Colors[ImGuiCol_TableBorderLight] = line;
    s.Colors[ImGuiCol_TableRowBg] = ImVec4(0, 0, 0, 0);
    s.Colors[ImGuiCol_TableRowBgAlt] = white(0.025f);
    s.Colors[ImGuiCol_TextLink] = accent_hi;
    s.Colors[ImGuiCol_TextSelectedBg] = rgb(0x3F80FF, 0.45f);
    s.Colors[ImGuiCol_DragDropTarget] = accent_hi;
    s.Colors[ImGuiCol_NavCursor] = accent_hi;
    s.Colors[ImGuiCol_NavWindowingHighlight] = white(0.70f);
    s.Colors[ImGuiCol_NavWindowingDimBg] = white(0.20f);
    s.Colors[ImGuiCol_ModalWindowDimBg] = rgb(0x000000, 0.55f);
}

}  // namespace nxrt::ui_theme
