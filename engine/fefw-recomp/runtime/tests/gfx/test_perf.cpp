// SPDX-License-Identifier: GPL-3.0-or-later
// The adaptive performance controller: measurement, classification and the one
// live action it owns (the sampler mip-bias offset).
#include "test_framework.h"

#include <cmath>
#include <cstdio>

#include "nxrt/config.h"
#include "nxrt/gfx_tuning.h"
#include "nxrt/perf.h"

NXTEST_GROUP(gfx_perf)

namespace {

namespace perf = nxrt::perf;

void policy(const char* mode = "automatic", bool adaptive_mip = false, const char* scene = "auto") {
    nxrt::config_clear();
    nxrt::config_set("perf", "enabled", "1");
    nxrt::config_set("perf", "mode", mode);
    nxrt::config_set("perf", "target_fps", "60");
    nxrt::config_set("perf", "stability", "1");
    nxrt::config_set("perf", "scene_profiles", "1");
    nxrt::config_set("perf", "scene_override", scene);
    nxrt::config_set("perf", "adaptive_mip", adaptive_mip ? "1" : "0");
    nxrt::config_set("perf", "max_mip_bias", "0.75");
    perf::test::reset();
    perf::reload();
}

void feed(int frames, double cpu_ms, double gpu_ms, int vram = 50, int backlog = 0, double upload_ms = 0.0, bool menu = false) {
    double upload_total = 0.0;
    for (int i = 0; i < frames; ++i) {
        perf::Sample s;
        s.cpu_ms = cpu_ms;
        s.gpu_ms = gpu_ms;
        s.vram_percent = vram;
        s.shader_backlog = backlog;
        s.menu_open = menu;
        upload_total += upload_ms;
        s.upload_total_ms = upload_total;
        perf::frame(s);
    }
}

}  // namespace

NXTEST(words_round_trip) {
    CHECK_EQ(std::string(perf::mode_name(perf::Mode::Aggressive)), std::string("Aggressive"));
    CHECK(perf::mode_from_word("manual") == perf::Mode::Manual);
    CHECK(perf::mode_from_word("CONSERVATIVE") == perf::Mode::Conservative);
    CHECK(perf::mode_from_word("nonsense") == perf::Mode::Automatic);
    CHECK(perf::scene_from_word("crowded") == perf::Scene::Crowded);
    CHECK(perf::scene_from_word("") == perf::Scene::Exploration);
}

NXTEST(percentiles_and_lows) {
    policy("manual");
    // 297 steady frames and three 50 ms hitches: P50 stays 16.67, P99 reaches
    // the hitches, and the 1% low is 20 fps.
    double upload = 0;
    for (int i = 0; i < 300; ++i) {
        perf::Sample s;
        s.cpu_ms = (i == 50 || i == 150 || i == 250) ? 50.0 : 1000.0 / 60.0;
        s.gpu_ms = 15.0;
        s.upload_total_ms = upload;
        perf::frame(s);
    }
    const perf::Status st = perf::status();
    CHECK(st.loaded);
    CHECK_MSG(std::fabs(st.p50_ms - 1000.0 / 60.0) < 0.05, "P50 must be the steady frame time");
    CHECK_MSG(st.p99_ms > 40.0, "P99 must see the hitches");
    CHECK_MSG(std::fabs(st.low1_fps - 20.0) < 0.1, "the 1% low must be about 20 fps");
    CHECK_MSG(st.low01_fps > 0.0 && st.low01_fps <= 20.0, "0.1% low must not exceed the 1% low");
    CHECK(st.spikes >= 3);
}

NXTEST(classifies_gpu_cpu_vram_shader_streaming) {
    policy("manual");
    feed(120, 16.0, 25.0);
    CHECK(perf::status().bottleneck == perf::Bottleneck::Gpu);

    policy("manual");
    feed(120, 26.0, 8.0);
    CHECK(perf::status().bottleneck == perf::Bottleneck::Cpu);

    policy("manual");
    feed(120, 16.0, 18.0, 95);
    CHECK(perf::status().bottleneck == perf::Bottleneck::Vram);

    policy("manual");
    feed(120, 16.0, 18.0, 50, 24);
    CHECK(perf::status().bottleneck == perf::Bottleneck::Shader);

    policy("manual");
    feed(120, 16.0, 18.0, 50, 0, 6.0);
    CHECK(perf::status().bottleneck == perf::Bottleneck::Streaming);
}

NXTEST(scene_profiles_and_override) {
    policy("manual", false, "combat");
    feed(120, 16.0, 18.0);
    CHECK(perf::status().scene == perf::Scene::Combat);
    CHECK_MSG(perf::status().scene_manual, "an explicit override must be reported as manual");

    policy("manual", false, "auto");
    feed(120, 30.0, 12.0);
    CHECK(perf::status().scene == perf::Scene::Crowded);
}

NXTEST(adaptive_mip_rises_under_gpu_pressure_and_is_released) {
    policy("aggressive", true);
    feed(180, 16.0, 28.0);
    const perf::Status hot = perf::status();
    CHECK_MSG(hot.mip_bias > 0.1f, "sustained GPU pressure must move the mip offset");
    CHECK_MSG(hot.adjustments > 0, "the controller must report that it acted");
    CHECK_MSG(std::fabs(nxrt::gfx::tuning().lod_bias_auto.load()) - hot.mip_bias < 0.001f, "the sampler must receive the offset");

    // A cool window brings the offset back to zero. The user's own lod_bias is
    // never touched; only the controller-owned offset moves.
    feed(240, 16.0, 8.0);
    CHECK_MSG(std::fabs(perf::status().mip_bias) < 0.01f, "the offset must return to zero once pressure ends");
    CHECK_MSG(std::fabs(nxrt::gfx::tuning().lod_bias_auto.load()) < 0.01f, "the sampler offset must be released");
}

NXTEST(manual_mode_and_menu_never_act) {
    policy("manual", true);
    feed(180, 16.0, 28.0);
    CHECK_MSG(std::fabs(perf::status().mip_bias) < 0.001f, "Manual mode must not act");

    policy("aggressive", true);
    feed(180, 16.0, 28.0, 50, 0, 0.0, true);
    CHECK_MSG(std::fabs(perf::status().mip_bias) < 0.001f, "an open menu must pause adaptive changes");
    CHECK(perf::status().scene == perf::Scene::Menu);
}
