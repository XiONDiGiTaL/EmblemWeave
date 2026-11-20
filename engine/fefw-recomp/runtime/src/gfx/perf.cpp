// SPDX-License-Identifier: GPL-3.0-or-later
// Adaptive performance controller. See include/nxrt/perf.h for the contract.
#include "nxrt/perf.h"

#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstdio>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <mutex>
#include <string>
#include <vector>

#include "nxrt/config.h"
#include "nxrt/gfx_tuning.h"
#include "nxrt/log.h"

namespace nxrt::perf {
namespace {

constexpr const char* kTag = "perf";
constexpr std::size_t kWindow = 300;   // five seconds at 60 fps
constexpr int kRefreshFrames = 30;

struct Point {
    double cpu = 0, gpu = 0, upload = 0, replay = 0, present = 0;
    int backlog = 0, vram = -1;
};

struct Policy {
    bool enabled = true;
    Mode mode = Mode::Automatic;
    bool stability = true;
    bool scene_profiles = true;
    Scene scene_override = Scene::Exploration;
    bool scene_manual = false;
    bool adaptive_mip = false;
    float max_mip_bias = 0.75f;
    int target_fps = 60;
};

struct State {
    std::mutex mu;
    bool loaded = false;
    Policy p;
    Status st;
    std::array<Point, kWindow> ring{};
    std::size_t head = 0, count = 0;
    int since_refresh = 0;
    int since_adjust = 0;
    double upload_total = -1, replay_total = -1, present_total = -1;
    double p50 = 0, p95 = 0, p99 = 0;
    double avg = 0, cpu_avg = 0, gpu_avg = 0, upload_avg = 0, replay_avg = 0, present_avg = 0;
    float bias = 0.0f;
    std::chrono::steady_clock::time_point last_log{};
    std::chrono::steady_clock::time_point last_status{};
    Bottleneck last_bottleneck = Bottleneck::None;
    Scene last_scene = Scene::Exploration;
    bool announced_policy = false;
};

State& state() {
    static State s;
    return s;
}

int target_ms(const Policy& p) { return p.target_fps > 0 ? p.target_fps : 60; }

double target_ms_value(const Policy& p) { return 1000.0 / static_cast<double>(target_ms(p)); }

void push_event(Status& st, const std::string& text) {
    if (text.empty()) return;
    for (int i = 5; i > 0; --i) std::memcpy(st.events[i], st.events[i - 1], sizeof st.events[i]);
    std::snprintf(st.events[0], sizeof st.events[0], "%s", text.c_str());
    st.event_count = std::min(st.event_count + 1, 6);
}

void refresh(State& s) {
    if (!s.count) return;
    std::vector<double> ft, cpu, gpu, upload, replay, present;
    ft.reserve(s.count);
    cpu.reserve(s.count);
    gpu.reserve(s.count);
    upload.reserve(s.count);
    replay.reserve(s.count);
    present.reserve(s.count);
    double sum = 0;
    for (std::size_t i = 0; i < s.count; ++i) {
        const Point& p = s.ring[(s.head + kWindow - s.count + i) % kWindow];
        ft.push_back(p.cpu);
        cpu.push_back(p.cpu);
        gpu.push_back(p.gpu);
        upload.push_back(p.upload);
        replay.push_back(p.replay);
        present.push_back(p.present);
        sum += p.cpu;
    }
    auto mean = [](const std::vector<double>& v) {
        double t = 0;
        for (double x : v) t += x;
        return v.empty() ? 0.0 : t / static_cast<double>(v.size());
    };
    std::sort(ft.begin(), ft.end());
    const auto at = [&](double q) {
        if (ft.empty()) return 0.0;
        const std::size_t i = std::min(ft.size() - 1, static_cast<std::size_t>(std::ceil(q * static_cast<double>(ft.size()))) - 1);
        return ft[i];
    };
    s.avg = sum / static_cast<double>(s.count);
    s.cpu_avg = mean(cpu);
    s.gpu_avg = mean(gpu);
    s.upload_avg = mean(upload);
    s.replay_avg = mean(replay);
    s.present_avg = mean(present);
    s.p50 = at(0.50);
    s.p95 = at(0.95);
    s.p99 = at(0.99);
    s.st.p50_ms = s.p50;
    s.st.p95_ms = s.p95;
    s.st.p99_ms = s.p99;
    s.st.worst_ms = ft.empty() ? 0.0 : ft.back();
    s.st.avg_fps = s.avg > 0 ? 1000.0 / s.avg : 0.0;
    s.st.low1_fps = s.p99 > 0 ? 1000.0 / s.p99 : 0.0;
    s.st.low01_fps = s.st.worst_ms > 0 ? 1000.0 / s.st.worst_ms : 0.0;
    s.st.cpu_ms = s.cpu_avg;
    s.st.gpu_ms = s.gpu_avg;
    s.st.upload_ms = s.upload_avg;
    s.st.replay_ms = s.replay_avg;
}

Bottleneck classify(const State& s) {
    const double target = target_ms_value(s.p);
    if (s.st.vram_percent >= 92) return Bottleneck::Vram;
    if (s.st.shader_backlog >= 16) return Bottleneck::Shader;
    if (s.upload_avg > std::max(2.0, target * 0.18)) return Bottleneck::Streaming;
    if (s.gpu_avg > target * 0.92 && s.gpu_avg >= s.cpu_avg * 0.95) return Bottleneck::Gpu;
    if (s.cpu_avg > target * 0.92 && s.cpu_avg > s.gpu_avg * 1.08) return Bottleneck::Cpu;
    if (s.p50 > 0 && s.p99 > s.p50 * 2.2 && s.cpu_avg < target * 0.75 && s.gpu_avg < target * 0.75) return Bottleneck::Sync;
    if (s.cpu_avg > s.gpu_avg * 1.18 && s.p50 > 0 && s.p99 > s.p50 * 1.55) return Bottleneck::Scheduling;
    return Bottleneck::None;
}

Scene classify_scene(const State& s) {
    if (s.p.scene_manual) return s.p.scene_override;
    if (!s.p.scene_profiles) return Scene::Exploration;
    const double target = target_ms_value(s.p);
    if (s.avg > 80.0 && s.gpu_avg < target * 1.4 && s.upload_avg < 2.0) return Scene::Loading;
    if (s.replay_avg > std::max(4.0, target * 0.28) || (s.cpu_avg > target * 0.90 && s.gpu_avg < target * 0.95)) return Scene::Crowded;
    if (s.p50 > 0 && s.p99 > s.p50 * 1.8 && s.gpu_avg > target * 0.75) return Scene::Combat;
    if (s.p50 > 0 && s.p99 < s.p50 * 1.25 && s.gpu_avg < target * 0.65 && s.cpu_avg < target * 0.60) return Scene::Cinematic;
    return Scene::Exploration;
}

const char* scene_why(const State& s, Scene scene) {
    if (s.p.scene_manual) return "manual override";
    if (!s.p.scene_profiles) return "scene profiles are off";
    switch (scene) {
        case Scene::Loading: return "long frames with idle CPU and GPU";
        case Scene::Crowded: return "CPU submission / replay pressure";
        case Scene::Combat: return "frame-time spread with a loaded GPU";
        case Scene::Cinematic: return "steady, GPU-light frame delivery";
        case Scene::Menu: return "the F1 menu is open";
        default: return "no stronger workload signature";
    }
}

float mode_factor(Mode m) {
    switch (m) {
        case Mode::Conservative: return 0.25f;
        case Mode::Aggressive: return 1.0f;
        case Mode::Balanced: return 0.55f;
        default: return 0.55f;
    }
}

float effective_factor(const State& s, float severity) {
    if (s.p.mode != Mode::Automatic) return mode_factor(s.p.mode);
    if (severity >= 0.72f) return 1.0f;
    if (severity >= 0.32f) return 0.55f;
    return 0.25f;
}

float scene_factor(Scene scene) {
    switch (scene) {
        case Scene::Combat: return 0.45f;   // preserve character/combat readability
        case Scene::Cinematic: return 0.0f; // never trade picture detail during a cutscene
        case Scene::Menu: return 0.0f;
        case Scene::Loading: return 0.0f;
        default: return 1.0f;
    }
}

void set_bias(State& s, float v) {
    v = std::clamp(v, -2.0f, 2.0f);
    if (std::fabs(v - s.bias) < 0.0005f) return;
    s.bias = v;
    gfx::tuning().lod_bias_auto.store(v, std::memory_order_relaxed);
}

void act(State& s) {
    s.since_adjust++;
    if (!s.p.enabled || !s.p.adaptive_mip || s.p.mode == Mode::Manual) {
        if (s.bias != 0.0f) {
            set_bias(s, 0.0f);
            s.st.adjustments++;
            std::snprintf(s.st.action, sizeof s.st.action, "Restored the user's texture detail (automatic bias off)");
            push_event(s.st, s.st.action);
        } else {
            std::snprintf(s.st.action, sizeof s.st.action, "Monitoring only; no automatic texture change");
        }
        return;
    }
    if (s.st.scene == Scene::Menu || s.st.scene == Scene::Loading) {
        std::snprintf(s.st.action, sizeof s.st.action, "%s: adaptive changes paused", scene_name(s.st.scene));
        return;
    }
    const bool pressure = s.st.bottleneck == Bottleneck::Gpu || s.st.bottleneck == Bottleneck::Vram;
    const double target = target_ms_value(s.p);
    float severity = 0.0f;
    if (s.st.bottleneck == Bottleneck::Vram && s.st.vram_percent > 0) {
        severity = std::clamp((static_cast<float>(s.st.vram_percent) - 88.0f) / 12.0f, 0.0f, 1.0f);
    } else if (s.st.bottleneck == Bottleneck::Gpu && target > 0) {
        severity = std::clamp(static_cast<float>((s.gpu_avg - target * 0.88) / (target * 0.55)), 0.0f, 1.0f);
    }
    float wanted = pressure ? std::clamp(s.p.max_mip_bias * severity * effective_factor(s, severity) * scene_factor(s.st.scene), 0.0f,
                                         s.p.max_mip_bias)
                            : 0.0f;
    // A long frame should not trigger another adjustment while it settles.
    const bool spiking = s.p.stability && s.p50 > 0 && s.p99 > s.p50 * 2.5;
    if (spiking && wanted > s.bias) {
        std::snprintf(s.st.action, sizeof s.st.action, "Frame-time spike: holding texture detail for stability");
        return;
    }
    if (s.since_adjust < 15) return;
    s.since_adjust = 0;
    const float before = s.bias;
    const float step = wanted > s.bias ? 0.05f : 0.10f;
    if (wanted > s.bias + 0.02f) set_bias(s, std::min(wanted, s.bias + step));
    else if (wanted < s.bias - 0.02f) set_bias(s, std::max(wanted, s.bias - step));
    if (std::fabs(s.bias - before) >= 0.005f) {
        s.st.adjustments++;
        std::snprintf(s.st.action, sizeof s.st.action, "%s: texture mip bias %+.2f (target %.1f ms)", bottleneck_name(s.st.bottleneck),
                      static_cast<double>(s.bias), target);
        push_event(s.st, s.st.action);
    } else if (wanted <= 0.0f) {
        std::snprintf(s.st.action, sizeof s.st.action, "No pressure; user texture detail restored");
    } else {
        std::snprintf(s.st.action, sizeof s.st.action, "%s: holding mip bias %+.2f", bottleneck_name(s.st.bottleneck),
                      static_cast<double>(s.bias));
    }
}

// A one-second snapshot next to fefw.ini for the launcher's Performance page.
// The presenting thread writes it; it is small and never read on this thread.
void write_status(State& s) {
    const auto now = std::chrono::steady_clock::now();
    if (now - s.last_status < std::chrono::seconds(1)) return;
    s.last_status = now;
    const std::string ini = config_file_path();
    if (ini.empty()) return;
    const std::filesystem::path path = std::filesystem::path(ini).parent_path() / "perf_status.json";
    std::ofstream f(path, std::ios::trunc);
    if (!f) return;
    const auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::system_clock::now().time_since_epoch()).count();
    f << "{\"updated_unix_ms\":" << ms << ",\"frames\":" << s.st.frames << ",\"enabled\":" << (s.p.enabled ? "true" : "false")
      << ",\"mode\":\"" << mode_name(s.p.mode) << "\",\"scene\":\"" << scene_name(s.st.scene) << "\""
      << ",\"bottleneck\":\"" << bottleneck_name(s.st.bottleneck) << "\""
      << ",\"avg_fps\":" << s.st.avg_fps << ",\"low1_fps\":" << s.st.low1_fps << ",\"low01_fps\":" << s.st.low01_fps
      << ",\"p50_ms\":" << s.st.p50_ms << ",\"p95_ms\":" << s.st.p95_ms << ",\"p99_ms\":" << s.st.p99_ms
      << ",\"cpu_ms\":" << s.st.cpu_ms << ",\"gpu_ms\":" << s.st.gpu_ms << ",\"upload_ms\":" << s.st.upload_ms
      << ",\"replay_ms\":" << s.st.replay_ms << ",\"vram_percent\":" << s.st.vram_percent << ",\"shader_backlog\":" << s.st.shader_backlog
      << ",\"target_fps\":" << s.st.target_fps << ",\"mip_bias\":" << s.bias << ",\"adjustments\":" << s.st.adjustments
      << ",\"spikes\":" << s.st.spikes << ",\"action\":\"" << s.st.action << "\",\"profile_reason\":\"" << s.st.profile_reason
      << "\"}\n";
}

void load(State& s) {
    s.p.enabled = config_get_bool("perf", "enabled", true);
    s.p.mode = mode_from_word(config_get_str("perf", "mode", "automatic").c_str());
    s.p.stability = config_get_bool("perf", "stability", true);
    s.p.scene_profiles = config_get_bool("perf", "scene_profiles", true);
    const std::string ov = config_get_str("perf", "scene_override", "auto");
    s.p.scene_manual = ov != "auto" && !ov.empty();
    s.p.scene_override = scene_from_word(ov.c_str());
    s.p.adaptive_mip = config_get_bool("perf", "adaptive_mip", false);
    s.p.max_mip_bias = static_cast<float>(std::clamp(config_get_double("perf", "max_mip_bias", 0.75), 0.0, 2.0));
    s.p.target_fps = static_cast<int>(std::clamp<int64_t>(config_get_int("perf", "target_fps", 60), 0, 360));
    s.loaded = true;   // frame() reads it: without this the policy was re-read every frame
    s.st.loaded = true;
    s.st.enabled = s.p.enabled;
    s.st.mode = s.p.mode;
    s.st.stability = s.p.stability;
    s.st.scene_profiles = s.p.scene_profiles;
    s.st.scene_manual = s.p.scene_manual;
    s.st.scene_override = s.p.scene_override;
    s.st.adaptive_mip = s.p.adaptive_mip;
    s.st.max_mip_bias = s.p.max_mip_bias;
    s.st.target_fps = s.p.target_fps;
    if (!s.announced_policy) {
        s.announced_policy = true;
        NXLOG_I(kTag, "adaptive controller: %s, mode %s, target %d fps, stability %s, scene profiles %s, mip bias %s (max %.2f)", 
                s.p.enabled ? "on" : "off", mode_name(s.p.mode), target_ms(s.p), s.p.stability ? "on" : "off",
                s.p.scene_profiles ? "on" : "off", s.p.adaptive_mip ? "on" : "off", static_cast<double>(s.p.max_mip_bias));
    }
}

}  // namespace

const char* mode_name(Mode m) {
    switch (m) {
        case Mode::Automatic: return "Automatic";
        case Mode::Conservative: return "Conservative";
        case Mode::Balanced: return "Balanced";
        case Mode::Aggressive: return "Aggressive";
        case Mode::Manual: return "Manual";
    }
    return "Automatic";
}

const char* bottleneck_name(Bottleneck b) {
    switch (b) {
        case Bottleneck::Cpu: return "CPU-bound";
        case Bottleneck::Gpu: return "GPU-bound";
        case Bottleneck::Vram: return "VRAM pressure";
        case Bottleneck::Streaming: return "Streaming";
        case Bottleneck::Shader: return "Shader compilation";
        case Bottleneck::Scheduling: return "CPU scheduling";
        case Bottleneck::Sync: return "Synchronization stalls";
        default: return "No dominant bottleneck";
    }
}

const char* scene_name(Scene s) {
    switch (s) {
        case Scene::Crowded: return "Crowded";
        case Scene::Combat: return "Combat";
        case Scene::Cinematic: return "Cinematic";
        case Scene::Menu: return "Menu";
        case Scene::Loading: return "Loading";
        default: return "Exploration";
    }
}

Mode mode_from_word(const char* word) {
    if (!word) return Mode::Automatic;
    if (!_stricmp(word, "conservative")) return Mode::Conservative;
    if (!_stricmp(word, "balanced")) return Mode::Balanced;
    if (!_stricmp(word, "aggressive")) return Mode::Aggressive;
    if (!_stricmp(word, "manual")) return Mode::Manual;
    return Mode::Automatic;
}

Scene scene_from_word(const char* word) {
    if (!word) return Scene::Exploration;
    if (!_stricmp(word, "crowded")) return Scene::Crowded;
    if (!_stricmp(word, "combat")) return Scene::Combat;
    if (!_stricmp(word, "cinematic")) return Scene::Cinematic;
    if (!_stricmp(word, "menu")) return Scene::Menu;
    if (!_stricmp(word, "loading")) return Scene::Loading;
    return Scene::Exploration;
}

void reload() {
    State& s = state();
    std::lock_guard<std::mutex> lk(s.mu);
    const bool was_loaded = s.loaded;
    s.loaded = false;
    load(s);
    if (was_loaded) {
        set_bias(s, 0.0f);
        s.since_adjust = 0;
        std::snprintf(s.st.action, sizeof s.st.action, "Policy reloaded: %s / %s", mode_name(s.p.mode),
                      s.p.adaptive_mip ? "adaptive texture detail" : "monitoring only");
        push_event(s.st, s.st.action);
    }
}

void frame(const Sample& sample) {
    State& s = state();
    std::lock_guard<std::mutex> lk(s.mu);
    if (!s.loaded) load(s);
    s.st.frames++;
    s.st.enabled = s.p.enabled;
    s.st.mode = s.p.mode;
    s.st.stability = s.p.stability;
    s.st.scene_profiles = s.p.scene_profiles;
    s.st.scene_manual = s.p.scene_manual;
    s.st.scene_override = s.p.scene_override;
    s.st.adaptive_mip = s.p.adaptive_mip;
    s.st.max_mip_bias = s.p.max_mip_bias;
    s.st.target_fps = s.p.target_fps;
    s.st.vram_percent = sample.vram_percent;
    s.st.shader_backlog = std::max(0, sample.shader_backlog);
    const double upload = s.upload_total < 0 ? 0.0 : std::max(0.0, sample.upload_total_ms - s.upload_total);
    const double replay = s.replay_total < 0 ? 0.0 : std::max(0.0, sample.replay_total_ms - s.replay_total);
    const double present = s.present_total < 0 ? 0.0 : std::max(0.0, sample.present_total_ms - s.present_total);
    s.upload_total = sample.upload_total_ms;
    s.replay_total = sample.replay_total_ms;
    s.present_total = sample.present_total_ms;

    if (sample.menu_open) {
        s.st.scene = Scene::Menu;
        s.st.bottleneck = classify(s);
        std::snprintf(s.st.action, sizeof s.st.action, "F1 menu open; adaptive changes paused");
        std::snprintf(s.st.profile_reason, sizeof s.st.profile_reason, "%s", scene_why(s, Scene::Menu));
        act(s);
        write_status(s);
        return;
    }

    if (sample.cpu_ms > 0.1 && sample.cpu_ms < 1000.0) {
        Point& p = s.ring[s.head];
        p.cpu = sample.cpu_ms;
        p.gpu = std::max(0.0, sample.gpu_ms);
        p.upload = upload;
        p.replay = replay;
        p.present = present;
        p.backlog = sample.shader_backlog;
        p.vram = sample.vram_percent;
        s.head = (s.head + 1) % kWindow;
        s.count = std::min(s.count + 1, kWindow);
        if (s.p50 > 0 && sample.cpu_ms > s.p50 * 2.0) s.st.spikes++;
        if (++s.since_refresh >= kRefreshFrames || s.count == 1) {
            s.since_refresh = 0;
            refresh(s);
        }
    }
    if (!s.count) {
        std::snprintf(s.st.action, sizeof s.st.action, "Collecting frame-time samples");
        return;
    }
    s.st.scene = classify_scene(s);
    s.st.bottleneck = classify(s);
    std::snprintf(s.st.profile_reason, sizeof s.st.profile_reason, "%s", scene_why(s, s.st.scene));
    if (s.st.bottleneck != s.last_bottleneck && s.st.frames > 30) {
        char line[160];
        std::snprintf(line, sizeof line, "Bottleneck: %s", bottleneck_name(s.st.bottleneck));
        push_event(s.st, line);
        s.last_bottleneck = s.st.bottleneck;
    }
    if (s.st.scene != s.last_scene && s.st.frames > 30) {
        char line[160];
        std::snprintf(line, sizeof line, "Profile: %s (%s)", scene_name(s.st.scene), scene_why(s, s.st.scene));
        push_event(s.st, line);
        s.last_scene = s.st.scene;
    }
    act(s);
    s.st.mip_bias = s.bias;
    if (!s.p.enabled) {
        std::snprintf(s.st.action, sizeof s.st.action, "Controller off; measurements only");
    } else if (s.p.mode == Mode::Manual) {
        std::snprintf(s.st.action, sizeof s.st.action, "Manual mode: user settings untouched");
    }
    const auto now = std::chrono::steady_clock::now();
    if (now - s.last_log > std::chrono::seconds(1)) {
        s.last_log = now;
        NXLOG_D(kTag, "%s, profile %s, %.1f fps, %.2f/%.2f ms CPU/GPU, VRAM %d%%, shaders %d, bias %.2f: %s",
                bottleneck_name(s.st.bottleneck), scene_name(s.st.scene), s.st.avg_fps, s.cpu_avg, s.gpu_avg, s.st.vram_percent,
                s.st.shader_backlog, static_cast<double>(s.bias), s.st.action);
    }
    write_status(s);
}

Status status() {
    State& s = state();
    std::lock_guard<std::mutex> lk(s.mu);
    Status out = s.st;
    out.mip_bias = s.bias;
    if (!s.loaded) {
        out.loaded = false;
        out.enabled = s.p.enabled;
    }
    return out;
}

namespace test {
void reset() {
    State& s = state();
    std::lock_guard<std::mutex> lk(s.mu);
    s.head = s.count = 0;
    s.since_refresh = s.since_adjust = 0;
    s.upload_total = s.replay_total = s.present_total = -1;
    s.p50 = s.p95 = s.p99 = s.avg = s.cpu_avg = s.gpu_avg = s.upload_avg = s.replay_avg = s.present_avg = 0;
    s.st = Status{};
    s.bias = 0;
    s.last_bottleneck = Bottleneck::None;
    s.last_scene = Scene::Exploration;
    s.loaded = false;
    s.announced_policy = false;
    gfx::tuning().lod_bias_auto.store(0.0f, std::memory_order_relaxed);
}
}  // namespace test

}  // namespace nxrt::perf
