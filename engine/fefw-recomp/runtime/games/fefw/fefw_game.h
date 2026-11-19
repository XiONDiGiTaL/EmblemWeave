// SPDX-License-Identifier: GPL-3.0-or-later
// The Fire Emblem: Fortune's Weave game module (docs/GAME_BOUNDARY.md): what the game-independent base needs to run this title, as one
// nxrt::game::Info. Libraries (games/fefw/CMakeLists.txt), each a superset of the one before:
//   nxrt_game_fefw_data   identity, file names (pure data)        the launcher           nxrt_register_game_fefw_data()
//   nxrt_game_fefw_core   + title facts (needs nxrt)              the tests              nxrt_register_game_fefw_core()
//   nxrt_game_fefw        + play-mode hooks (needs nxrt_gfx)      fefw.exe              nxrt_register_game_fefw()
// Play mode: the 60 fps option and the game clock log (fefw_play.cpp), the button glyphs (fefw_prompts.cpp); no timestep or probes yet. The game exe
// (game-build/fefw/fefw_main.cpp) calls nxrt_register_game_fefw() before nxrt::init().
#pragma once

#include <cstdint>
#include <string>

#include "nxrt/game.h"

void nxrt_register_game_fefw();
void nxrt_register_game_fefw_core();
void nxrt_register_game_fefw_data();

namespace nxrt::play {
class GameHost;
class UiHost;
struct Settings;
}  // namespace nxrt::play
namespace nxrt::gfx {
class Overlay;
}

namespace nxrt::games::fefw {
const game::Info& data_info();     // fefw_data.cpp
const game::Info& core_info();     // fefw_core.cpp
const game::Info& full_info();     // fefw_game.cpp

// fefw_play.cpp
void play_install(uint64_t base, play::GameHost* host, const play::Settings& s);
void play_remove();

// fefw_frame.cpp: per-frame amounts kept at their real speed at 60 and 120 fps ([fefw] frame_fixes, frame_fixes_skip)
void frame_install(uint64_t base);
void frame_step(float step, int target);   // once per frame (the time manager's step, frames of 1/59.94 s; the target frame rate)
void frame_set_enabled(bool on);           // live
struct FrameFixInfo {
    bool installed = false, on = false, quake = false;
    float scale = 1.0f;                    // the frame's share of a 1/30 s step (1 = the game's own values)
    int native = 0;                        // the target the game asked for last (30 gameplay, 60 menus; 0 none seen)
    static constexpr int kGroups = 9;
    uint64_t applied[kGroups] = {};        // values changed, per group (frame_group_name)
    uint64_t quake_held = 0;               // quake blocks held to 30 Hz
};
FrameFixInfo frame_info();
const char* frame_group_name(int g);
// The game's clock against real time over the last few seconds (sampled once a second) and the frame-rate options read at start.
struct ClockInfo {
    bool valid = false;
    double rate = 0;          // 1.000 = the right speed
    int window_s = 0;         // seconds the rate covers
    int target = 0, display = 0;
    float step = 0;           // the last step, frames of 1/59.94 s
    int period_s = 0;         // [fefw] clock_log in force (0 = no log lines)
    bool fps60 = false, fps120 = false, smooth = false;
};
ClockInfo clock_info();
void set_clock_log(int period_s);   // live



// fefw_menu.cpp: the F1 menu (GamePlay::settings_menu)
void settings_menu(play::UiHost& host, gfx::Overlay& overlay);

// fefw_res.cpp: [fefw] output / render_ratio / render / shadow_size / quality (the render configs, notes/resolution.md)
void res_install(uint64_t base);

// fefw_detail.cpp: [fefw] reflections / ambient_occlusion / crowd_detail / scenery_detail, the game's graphics settings files edited as
// they load. Install before the game's file system is set up.
void detail_install();
std::string detail_summary();      // "" when nothing is changed
// The render-target pool's allocator was given room past the pool ([fefw] rt_pool_overflow_mb): renders above half of a 2160p output fit.
bool res_rt_pool_extended();
// fefw_crowd.cpp: [display] npc_distance (controls file; [fefw] npc_distance in the ini wins) scales the hub crowd's NPC model limit,
// display distance and draw budget. Installed from play_install.
void crowd_install(uint64_t base, double npc_distance);
double npc_distance_applied();  // the hub NPC factor in force since the start (1 = the game's own)

// fefw_wide.cpp: outputs wider than 16:9 ([fefw] ultrawide; hooks after Kompas9's ultrawide patch). res_install calls wide_install with
// the output it gives the game; nothing is installed unless that output is wider than 16:9.
std::string wide_mode();                             // [fefw] ultrawide: "auto" or "off"
bool wide_aspect(int w, int h);                      // wider than 16:9 (by more than 1 %)
void wide_install(uint64_t base, int out_w, int out_h);
void wide_remove();
struct WideInfo {
    bool active = false;
    int out_w = 0, out_h = 0;
    float k = 1.0f;                                  // the output's width over a 16:9 picture's of the same height
    static constexpr int kCounters = 11;
    uint64_t counts[kCounters] = {};                 // times each hook did its work
    const char* names[kCounters] = {};
};
WideInfo wide_info();

// fefw_mouse.cpp: the game's own Joy-Con 2 mouse controls, read for the log, the tests and the F1 menu
struct MouseInfo {
    bool valid = false;
    bool mode = false;                 // the game is in its mouse controls
    bool manual = false;               // turned on with SL + SR (the manual switching setting)
    bool allowed = false, enabled = false;
    int32_t dx = 0, dy = 0;            // the game's movement this frame (pointer units)
    int32_t cursor_x = 0, cursor_y = 0;   // the game's pointer, 1920 x 1080
    float cfg[5] = {};                 // +0xc8..+0xd8: seconds to switch on, movement threshold (squared), two motion thresholds, seconds to switch off
    uint64_t frames = 0, frames_mouse = 0, entries = 0, exits = 0;
};
void mouse_install(uint64_t base);
void mouse_frame();                    // main thread, once per frame
MouseInfo mouse_info();

// fefw_prompts.cpp: the button glyphs of the game's text follow the controller in use ([prompts] style; notes/prompts.md)
void prompts_install(play::GameHost* host);
void prompts_remove();
void prompts_frame();      // main thread, once per frame (GamePlay::frame)
}  // namespace nxrt::games::fefw
