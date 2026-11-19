// SPDX-License-Identifier: GPL-3.0-or-later
// A game registration for the base play-mode tests (test_play_config.cpp, test_play_engine.cpp, test_play_ui.cpp).
//
// The test executable registers the repository's test game (CMakeLists.txt NXRT_TEST_GAME), whose play-mode words, factory controls.ini
// and settings page are its own. The base play tests check the play code itself, so they run with this fixed registration instead: the
// action and context words, factory bindings and revisions the tests were written against (a first-person game with a primary and a
// secondary weapon, an alternate body, four selectable beams and four visors). Nothing else is registered (no play hooks, so the base
// settings page draws; no prompts).
//
// Usage: `PlayTestGame game;` at the top of a test. It registers the test game and puts the previous registration back when it goes out
// of scope.
#pragma once

#include "nxrt/game.h"
#include "play/play_types.h"

namespace nxrt::play::test {

// Revision 2 added X in [bindings.map]; revision 3 changed the factory [aim] lock_mode from classic to free.
constexpr int kTestDefaultsRev = 3;

inline const ActInfo* test_acts() {
    static const ActInfo kActs[] = {
        {Act::None, "none", ""},
        {Act::MoveForward, "move_forward", "LS_UP"},
        {Act::MoveBack, "move_back", "LS_DOWN"},
        {Act::MoveLeft, "move_left", "LS_LEFT"},
        {Act::MoveRight, "move_right", "LS_RIGHT"},
        {Act::LookUp, "look_up", "RS_UP"},
        {Act::LookDown, "look_down", "RS_DOWN"},
        {Act::LookLeft, "look_left", "RS_LEFT"},
        {Act::LookRight, "look_right", "RS_RIGHT"},
        {Act::Primary, "fire", "ZR"},
        {Act::Target, "lock_on", "ZL"},
        {Act::Secondary, "missile", "R"},
        {Act::Jump, "jump", "B"},
        {Act::AltForm, "morph", "Y"},
        {Act::LookHold, "look_hold", "L"},
        {Act::Map, "map", "PLUS"},
        {Act::Pause, "pause", "MINUS"},
        {Act::Confirm, "confirm", "A"},
        {Act::Back, "back", "B"},
        {Act::DpadUp, "dpad_up", "UP"},
        {Act::DpadDown, "dpad_down", "DOWN"},
        {Act::DpadLeft, "dpad_left", "LEFT"},
        {Act::DpadRight, "dpad_right", "RIGHT"},
        {Act::SetA1, "beam_1", "X+UP"},
        {Act::SetA2, "beam_2", "X+RIGHT"},
        {Act::SetA3, "beam_3", "X+DOWN"},
        {Act::SetA4, "beam_4", "X+LEFT"},
        {Act::SetB1, "visor_1", "L+UP"},
        {Act::SetB2, "visor_2", "L+RIGHT"},
        {Act::SetB3, "visor_3", "L+DOWN"},
        {Act::SetB4, "visor_4", "L+LEFT"},
        {Act::SetANext, "beam_next", ""},
        {Act::SetAPrev, "beam_prev", ""},
        {Act::BtnA, "btn_a", "A"},
        {Act::BtnB, "btn_b", "B"},
        {Act::BtnX, "btn_x", "X"},
        {Act::BtnY, "btn_y", "Y"},
        {Act::BtnL, "btn_l", "L"},
        {Act::BtnR, "btn_r", "R"},
        {Act::BtnZL, "btn_zl", "ZL"},
        {Act::BtnZR, "btn_zr", "ZR"},
        {Act::BtnPlus, "btn_plus", "PLUS"},
        {Act::BtnMinus, "btn_minus", "MINUS"},
        {Act::BtnLStick, "btn_lstick", "LSTICK"},
        {Act::BtnRStick, "btn_rstick", "RSTICK"},
        {Act::MouseMode, "mouse_mode", ""},
    };
    static_assert(sizeof(kActs) / sizeof(kActs[0]) == kActCount, "one row per Act");
    return kActs;
}

inline const GameVocabulary& test_vocabulary() {
    static const char* const kCtx[] = {"all", "gameplay", "morph", "map", "menu", "scan"};
    static_assert(sizeof(kCtx) / sizeof(kCtx[0]) == kCtxCount, "one name per Ctx");
    static const GameVocabulary v = [] {
        GameVocabulary g;
        g.acts = test_acts();
        g.ctx_names = kCtx;
        g.key_alt_yaw = "morph_yaw";
        g.key_alt_pitch = "morph_pitch";
        g.key_wheel_cycles = "wheel_beams";
        g.key_alt_orbit = "ball_orbit";
        g.key_alt_camera = "ball_camera";
        g.key_alt_sens = "ball_sensitivity";
        return g;
    }();
    return v;
}

// The factory controls.ini. Values that are not written take the built-in defaults (play_types.h).
inline const char* test_settings_text() {
    return "[meta]\n"
           "defaults_rev = 3\n"
           "\n"
           "[aim]\n"
           "lock_mode = free\n"
           "\n"
           "[display]\n"
           "window_mode = windowed\n"
           "render_scale = 100\n"
           "render_width = 0\n"
           "render_height = 0\n"
           "aspect = 0\n"
           "fov_offset = 0\n"
           "# dynamic_resolution: applies at the next start\n"
           "dynamic_resolution = 0\n"
           "# anisotropy: applies at the next start\n"
           "anisotropy = 0\n"
           "lod_bias = 0\n"
           "bloom = 1\n"
           "chromatic_aberration = 1\n"
           "# fast_gpu_memory: applies at the next start\n"
           "fast_gpu_memory = 1\n"
           "# fewer_gpu_waits: applies at the next start\n"
           "fewer_gpu_waits = 1\n"
           "\n"
           "[prompts]\n"
           "style = auto\n"
           "\n"
           "[bindings.all]\n"
           "W = move_forward\n"
           "S = move_back\n"
           "A = move_left\n"
           "D = move_right\n"
           "Mouse1 = fire\n"
           "Mouse3 = lock_on\n"
           "F = missile\n"
           "Space = jump\n"
           "Left Ctrl = morph\n"
           "Tab = map\n"
           "WheelUp = beam_prev\n"
           "WheelDown = beam_next\n"
           "1 = beam_1\n"
           "2 = beam_2\n"
           "3 = beam_3\n"
           "4 = beam_4\n"
           "\n"
           "[bindings.gameplay]\n"
           "\n"
           "[bindings.morph]\n"
           "\n"
           "[bindings.map]\n"
           "Return = confirm\n"
           "X = btn_x\n"
           "\n"
           "[bindings.menu]\n"
           "Mouse1 = confirm\n"
           "Return = confirm\n"
           "Escape = back\n"
           "Q = btn_l\n"
           "E = btn_r\n"
           "\n"
           "[bindings.scan]\n";
}

inline const GameDefaults& test_defaults() {
    static const GameDefaultAddition kAdds[] = {{2, Ctx::Map, "X", Act::BtnX}};
    static const GameDefaultChange kChanges[] = {{3, "aim", "lock_mode", "classic", "free"}};
    static const GameDefaults d = [] {
        GameDefaults g;
        g.settings_text = test_settings_text();
        g.defaults_rev = kTestDefaultsRev;
        g.additions = kAdds;
        g.addition_count = 1;
        g.vocabulary = &test_vocabulary();
        g.changes = kChanges;
        g.change_count = 1;
        return g;
    }();
    return d;
}

class PlayTestGame {
public:
    PlayTestGame() : before_(::nxrt::game::current()) {
        info_.id = "playtest";
        info_.play_defaults = &test_defaults();
        ::nxrt::game::set_current(&info_);
    }
    ~PlayTestGame() { ::nxrt::game::set_current(before_); }
    PlayTestGame(const PlayTestGame&) = delete;
    PlayTestGame& operator=(const PlayTestGame&) = delete;

private:
    const ::nxrt::game::Info* before_;
    ::nxrt::game::Info info_;
};

}  // namespace nxrt::play::test
