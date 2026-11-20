// SPDX-License-Identifier: GPL-3.0-or-later
// Shared audio controls for the F1 menus: the 100% -> 200% safety gate and the glass "Audio Enhancements" panel (master gain, EQ
// presets, ten band sliders). Both the base settings page and fefw's own menu draw these, and both write the same AudioSettings.
#pragma once
#include "imgui.h"
#include "nxrt/audio_eq.h"
#include "play_types.h"

namespace nxrt::play::audio_ui {

// The result of a panel interaction. Live means the DSP should hear it now; Commit also means the caller should save the settings.
enum class Edit { None, Live, Commit };

struct State {
    bool eq_open = false;
    bool eq_focus = false;             // bring the panel to the front the first frame it opens
    bool eq_placed = false;
    ImVec2 last_display = ImVec2(0.0f, 0.0f);
    bool boost_warning = false;        // the modal is waiting for "I know the risks" / "Cancel"
    double boost_pending = 100.0;      // the percentage the player tried to set
    bool boost_approved = false;       // this session already ran above 100%: lowering and raising again is allowed without another modal
    bool edited_live = false;          // a slider changed and its mouse button has not been released yet
};

// AudioSettings -> the engine's process-wide DSP settings.
audio::EqSettings to_engine(const AudioSettings& a);
// Pushes the current settings into the running engine (master volume, EQ, headroom). The caller keeps the file untouched until save.
void preview_live(const AudioSettings& a);

// Applies a new master percentage through the safety gate. Returns true when the value changed; false means the warning intercepted
// it, and boost_warning() must be drawn this frame.
bool apply_volume(AudioSettings& a, State& s, double percent);
// Draws the modal while State::boost_warning is set. Returns true when accepting or cancelling changed the volume.
bool boost_warning(AudioSettings& a, State& s);

// The entry point in the Audio page. Returns true when the panel was opened.
bool enhancements_button(AudioSettings& a, State& s);
// Draws the floating glass panel while open. Returns Live while a slider moves and Commit when it is released or a preset/toggle is
// clicked. The caller previews the settings and, on Commit, saves them.
Edit enhancements_window(AudioSettings& a, State& s);

// Title-case names for the preset buttons, in the same order as audio::eq_preset_db.
const char* preset_label(int preset);
void apply_preset(AudioSettings& a, int preset);

}  // namespace nxrt::play::audio_ui
