// SPDX-License-Identifier: GPL-3.0-or-later
// Adaptive performance controller (S35).
//
// The controller is deliberately narrow: it measures every presented frame,
// classifies the dominant bottleneck, selects a scene profile and explains its
// decisions. The only visual setting it changes live is the sampler mip bias
// (a texture-detail offset); every other optimization is a start-up knob owned
// by its subsystem and exposed in the launcher/F1 Performance pages.
//
// Why mip bias: the NVN back end rebuilds cached samplers from a key that
// includes the bias, so a change is live and safe. Render-target sizes,
// shadow maps, occlusion and scene traversal are fixed when the game chooses
// its render configuration; changing them mid-frame would be a renderer
// rewrite, not an optimization toggle.
#pragma once

#include <cstddef>
#include <cstdint>

namespace nxrt::perf {

enum class Mode : std::uint8_t { Automatic = 0, Conservative, Balanced, Aggressive, Manual };
enum class Bottleneck : std::uint8_t { None = 0, Cpu, Gpu, Vram, Streaming, Shader, Scheduling, Sync };
enum class Scene : std::uint8_t { Exploration = 0, Crowded, Combat, Cinematic, Menu, Loading };

const char* mode_name(Mode m);
const char* bottleneck_name(Bottleneck b);
const char* scene_name(Scene s);
Mode mode_from_word(const char* word);
Scene scene_from_word(const char* word);

struct Sample {
    double cpu_ms = 0.0;             // wall time between presented frames
    double gpu_ms = 0.0;             // present pass GPU time
    double upload_total_ms = 0.0;    // HitchPhase kHpRenderUpload, cumulative
    double replay_total_ms = 0.0;    // HitchPhase kHpRenderReplay, cumulative
    double present_total_ms = 0.0;   // HitchPhase kHpRenderPresent, cumulative
    int shader_backlog = 0;          // background shader jobs waiting/running
    int vram_percent = -1;           // process use of the video-memory budget
    bool menu_open = false;          // the F1 menu owns the screen this frame
};

struct Status {
    bool loaded = false;
    bool enabled = false;
    bool stability = false;
    bool scene_profiles = false;
    bool adaptive_mip = false;
    Mode mode = Mode::Automatic;
    Scene scene = Scene::Exploration;
    Scene scene_override = Scene::Exploration;
    bool scene_manual = false;
    Bottleneck bottleneck = Bottleneck::None;
    double avg_fps = 0.0, low1_fps = 0.0, low01_fps = 0.0;
    double p50_ms = 0.0, p95_ms = 0.0, p99_ms = 0.0, worst_ms = 0.0;
    double cpu_ms = 0.0, gpu_ms = 0.0, upload_ms = 0.0, replay_ms = 0.0;
    int vram_percent = -1;
    int shader_backlog = 0;
    int target_fps = 60;
    float mip_bias = 0.0f;           // the controller's live offset
    float max_mip_bias = 0.75f;
    std::uint64_t frames = 0;
    std::uint64_t adjustments = 0;
    std::uint64_t spikes = 0;
    char action[192] = {};
    char profile_reason[128] = {};
    char events[6][160] = {};
    int event_count = 0;
};

// Reads [perf] from nxrt's configuration (the launcher writes those keys to
// fefw.ini). Safe to call again after the F1 page writes a key: the controller
// releases its mip offset before adopting the new policy.
void reload();

// Presented-thread call, once per frame. It is cheap when the menu is closed:
// a 300-sample ring, a percentile refresh every 30 frames and a 250 ms-gated
// VRAM read by the caller. No locks are shared with the render thread.
void frame(const Sample& s);

// A copy for the launcher's overlay/F1 pages. Presenting thread.
Status status();

// Test hook: clears the measurement window and releases the mip offset.
namespace test {
void reset();
}

}  // namespace nxrt::perf
