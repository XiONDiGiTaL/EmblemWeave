// SPDX-License-Identifier: GPL-3.0-or-later
// The game boundary (docs/GAME_BOUNDARY.md): everything one title supplies to the game-independent base.
//
// The runtime, the launcher and the test tooling know no game. A game module (runtime/games/<id>/, a static library the game exe links)
// fills one Info and registers it with set_current() before anything reads it: before nxrt::init() in the game exe, at the top of the
// launcher's WinMain and of the test mains. Every part is optional. A null table or an empty string turns off what needs it (no hooks, no
// probes, neutral title facts), so a new title can start with the identity alone and add tables as its hooks are found.
//
// What each table is for, and where the Metroid Prime Remastered one lives:
//   title_defaults   services::TitleProfile defaults (main.npdm, control.nacp); ini keys still override them    games/mpr/mpr_core.cpp
//   timestep         the sites of the real-time frame step (nxrt/timestep.h, Sites)                           games/mpr/mpr_core.cpp, mpr_sites.h
//   probes           test-tooling game-state probes and script steps (nxrt/tt.h, GameProbes)                  games/mpr/mpr_tt_probe.cpp
//   play_defaults    factory controls.ini text and the bindings added by later revisions (play_types.h)       games/mpr/mpr_defaults.cpp
//   play             play-mode hooks: aim, camera, lock-on, display, menu pointer, F1 entries (play_game.h)     games/mpr/mpr_play.cpp
//   prompts          the game's button icon textures (play/prompt_resolve.h, PromptGame)                      games/mpr/mpr_prompts.cpp
// The registry is header-only, so programs that do not link the runtime library (the launcher) use it too.
#pragma once
#include <cstdint>

namespace nxrt {
namespace services { struct TitleProfile; }
namespace timestep { struct Sites; }
namespace tt { struct GameProbes; }
namespace play { struct GamePlay; struct GameDefaults; struct PromptGame; }
}  // namespace nxrt

namespace nxrt::game {

// The console the game was built for. Switch 2 games run on the same Horizon model with a few platform facts that differ
// (system tick 31.25 MHz instead of 19.2 MHz, the NVN2 graphics API).
enum class Platform : uint8_t { Switch1, Switch2 };

struct Info {
    // ---- identity ----
    const char* id = "";                    // short id, also the stem of the game's file names: "mpr"
    const char* title = "";                 // the game's own name: "Metroid Prime Remastered"
    const char* product = "";               // the PC port's name in window titles and dialogs: "Metroid Prime Remastered PC"
    const char* short_title = "";           // a short name for file headers (the launcher's ini comment): "Metroid Prime" (empty: product)
    uint64_t program_id = 0;                // the Switch title id
    const char* version = "";               // the game version the hooks were found in: "1.0.0"
    uint64_t main_base = 0x7100000000ull;   // where main is mapped (the lifter's base for it): hook offsets are relative to this
    uint32_t languages = 0;                 // control.nacp SupportedLanguage: bit n = services::Language n (0: American English only)
    Platform platform = Platform::Switch1;  // the console the game targets
    int nvn_api_minor = 30;                 // Switch 2: the NVN2 driver is part of the game's sdk, version 231.<this> (Pokopia 30, SDK 22.2 40);
                                            // nvnDeviceGetInteger answers that driver's constants (gfx/nvn/api_device.cpp)
    bool movie_fail_at_open = false;        // Switch 2 without a host video decoder (core/switch2.cpp): false = nn::movie::Initialize fails and
                                            // the game leaves its movies out; true = the game's player aborts on that, so opening each movie
                                            // fails instead and the game moves on as after a skip
    bool movie_video_decoder = false;       // Switch 2: the game's player takes movie::VideoDecoder from its movie module; the host one decodes
                                            // AV1 into the player's CPU frames (media/movie_vdec.cpp, [media] movies = host; skip = the above)
    bool vi_hdr_status_sdr = false;         // Switch 2: nn::vi::QueryDisplayHdrStatus answers an SDR display on the host (core/switch2.cpp)
    int present_interval_hz = 0;            // the game paces itself by the console's vsync and steps its logic by a fixed amount per frame:
                                            // the frame limiter holds it to this rate / the NVN window's present interval (60: interval 2 is
                                            // 30 fps), as the console's display does ([nvn] present_interval_hz, gfx/nvn/nvn_present.cpp). 0 = off

    // ---- files and branding ----
    const char* exe_name = "";              // the game exe: "mpr.exe"
    const char* setup_exe_name = "";        // the setup tool: "mpr_setup.exe"
    const char* launcher_ini = "";          // the launcher's own settings file: "mpr_launcher.ini"
    const char* game_ini = "";              // the settings file the setup tool writes and the game exe reads: "mpr.ini"
    const char* shader_pack = "";           // the shader pack file next to the setup tool: "shader_cache.mprshc"
    const char* save_folder = "";           // save data folder under %APPDATA%: "MPRRecomp" (empty: "nxrt")
    const char* jc2_mouse_default = "";     // [input] jc2_mouse when the ini does not set it: "off" (empty: "auto")

    // ---- tables (null = the feature is off) ----
    void (*title_defaults)(services::TitleProfile& p) = nullptr;
    const timestep::Sites* timestep = nullptr;
    const tt::GameProbes* probes = nullptr;
    const play::GameDefaults* play_defaults = nullptr;
    const play::GamePlay* play = nullptr;
    const play::PromptGame* prompts = nullptr;
};

// The registered game, or nullptr. One slot per program (the function-local static is shared by every translation unit that inlines it).
inline const Info*& current_slot() {
    static const Info* slot = nullptr;
    return slot;
}
inline void set_current(const Info* g) { current_slot() = g; }
inline const Info* current() { return current_slot(); }
// The registered game, or an empty Info (every table null) when none is.
inline const Info& info() {
    static const Info none;
    const Info* g = current();
    return g ? *g : none;
}

}  // namespace nxrt::game
