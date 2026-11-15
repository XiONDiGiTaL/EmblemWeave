// SPDX-License-Identifier: GPL-3.0-or-later
// Scene tests: present-mode switching, resizing, minimise, fullscreen toggle and leak checks.
// Reference image: tests/data/gfx/test_scene_640x360_f60.png, made by the screenshot test with NXRT_GFX_UPDATE_REF=1 on the RTX 3080.
#include <SDL3/SDL.h>

#include "gfx_test_util.h"
NXTEST_GROUP(gfx_present)

using namespace gfxtest;

NXTEST(present_mode_switching_and_resizing_with_a_window) {
    DWORD handles_start = handle_count();
    SceneRunDesc d = base_desc();
    d.headless = false;
    d.width = 640;
    d.height = 360;
    d.frames = 400;
    uint64_t switches = 0, resizes = 0;
    PresentMode last = PresentMode::Fifo;
    d.per_frame = [&](uint64_t frame, Renderer& r) {
        const PresentMode order[] = {PresentMode::Mailbox, PresentMode::Immediate, PresentMode::FifoRelaxed, PresentMode::Fifo};
        if (frame % 20 == 10) {
            const PresentMode m = order[(frame / 20) % 4];
            if (r.set_present_mode(m)) ++switches;
            last = m;
            CHECK(r.present_mode_supported(r.present_mode()));
        }
        if (frame % 13 == 5) {
            const int w = 480 + static_cast<int>((frame * 37) % 400);
            const int h = 270 + static_cast<int>((frame * 23) % 300);
            if (platform::set_window_size(w, h)) ++resizes;
        }
    };
    const SceneRunResult r = run_scene(d);
    print_result("modes+resize", r);
    std::printf("    %llu present-mode switches, %llu window resizes, %llu swapchains created, %llu out-of-date\n", static_cast<unsigned long long>(switches),
                static_cast<unsigned long long>(resizes), static_cast<unsigned long long>(r.renderer.swapchain_creates), static_cast<unsigned long long>(r.renderer.out_of_date));
    REQUIRE(r.ok);
    CHECK_EQ(r.frames, 400ull);
    CHECK_EQ(r.validation.errors, 0ull);
    CHECK(switches >= 15);
    CHECK(resizes >= 20);
    CHECK(r.renderer.swapchain_creates >= 20);
    CHECK_EQ(r.memory_end.allocations, 0ull);
    (void)last;
    (void)handles_start;
}

NXTEST(no_handle_or_memory_growth_over_hundreds_of_swapchain_recreations) {
    // One device, one window, 700 frames with a present-mode change or a resize every 3 frames (about 300 swapchain rebuilds,
    // the overlay on). After a warm-up the process handle count, the private bytes and the allocator's live allocations must not
    // grow. (Creating and destroying whole devices is not what this checks: the NVIDIA driver on this machine keeps 5 handles per
    // vkDestroyDevice, see docs/debug/006.md.)
    SceneRunDesc d = base_desc();
    d.headless = false;
    d.width = 640;
    d.height = 360;
    d.frames = 700;
    d.overlay = true;
    d.overlay_visible_from_frame = 5;
    DWORD h_start = 0, h_end = 0;
    double mb_start = 0, mb_end = 0;
    auto private_mb = [] {
        PROCESS_MEMORY_COUNTERS_EX pm = {sizeof pm};
        GetProcessMemoryInfo(GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&pm), sizeof pm);
        return static_cast<double>(pm.PrivateUsage) / (1024.0 * 1024.0);
    };
    d.per_frame = [&](uint64_t frame, Renderer& r) {
        if (frame == 100) { h_start = handle_count(); mb_start = private_mb(); }
        if (frame == 699) { h_end = handle_count(); mb_end = private_mb(); }
        const PresentMode order[] = {PresentMode::Mailbox, PresentMode::Immediate, PresentMode::FifoRelaxed, PresentMode::Fifo};
        if (frame % 6 == 0) r.set_present_mode(order[(frame / 6) % 4]);
        if (frame % 6 == 3) {
            const int w = 400 + static_cast<int>((frame * 41) % 500);
            const int h = 240 + static_cast<int>((frame * 29) % 300);
            platform::set_window_size(w, h);
        }
    };
    const SceneRunResult r = run_scene(d);
    print_result("recreation soak", r);
    REQUIRE(r.ok);
    std::printf("    %llu swapchains created; handles %lu -> %lu, private bytes %.2f -> %.2f MiB between frame 100 and 699\n",
                static_cast<unsigned long long>(r.renderer.swapchain_creates), static_cast<unsigned long>(h_start), static_cast<unsigned long>(h_end), mb_start, mb_end);
    CHECK_EQ(r.validation.errors, 0ull);
    CHECK(r.renderer.swapchain_creates >= 200);
    CHECK_EQ(r.memory_end.allocations, 0ull);
    CHECK(h_end <= h_start + 2);
    CHECK(mb_end - mb_start < 8.0);
}

NXTEST(headless_resize_and_present_mode_calls) {
    SceneRunDesc d = base_desc();
    d.frames = 120;
    d.width = 320;
    d.height = 180;
    d.per_frame = [](uint64_t frame, Renderer& r) {
        if (frame % 10 == 3) r.resize_headless(160 + static_cast<uint32_t>(frame % 7) * 40, 90 + static_cast<uint32_t>(frame % 5) * 30);
        if (frame % 15 == 4) r.set_present_mode(static_cast<PresentMode>(frame % 4));
    };
    const SceneRunResult r = run_scene(d);
    print_result("headless resize", r);
    REQUIRE(r.ok);
    CHECK_EQ(r.validation.errors, 0ull);
    CHECK(r.renderer.swapchain_creates > 10);
    CHECK_EQ(r.memory_end.allocations, 0ull);
}

NXTEST(minimise_and_restore) {
    SceneRunDesc d = base_desc();
    d.headless = false;
    d.frames = 90;
    bool minimised_seen = false;
    uint64_t calls = 0;   // per_frame runs on every loop pass, also on the passes where nothing could be drawn
    d.per_frame = [&](uint64_t, Renderer&) {
        ++calls;
        auto* w = static_cast<SDL_Window*>(platform::native_window());
        if (calls == 20) SDL_MinimizeWindow(w);
        if (calls == 40) SDL_RestoreWindow(w);
        if (calls > 20 && calls < 40 && platform::minimized()) minimised_seen = true;
    };
    const SceneRunResult r = run_scene(d);
    print_result("minimise", r);
    REQUIRE(r.ok);
    CHECK_EQ(r.validation.errors, 0ull);
    std::printf("    minimised state seen: %d, frames skipped while minimised: %llu\n", minimised_seen, static_cast<unsigned long long>(r.renderer.skipped));
    if (!minimised_seen) {
        nxtest::skip("SDL did not minimise the hidden window on this machine");
        return;
    }
    CHECK(r.renderer.skipped > 0);
    CHECK_EQ(r.frames, 90ull);
}

NXTEST(fullscreen_toggle_recreates_the_swapchain) {
    SceneRunDesc d = base_desc();
    d.headless = false;
    d.frames = 120;
    d.width = 640;
    d.height = 360;
    uint32_t windowed_w = 0, full_w = 0;
    bool toggled = false, fullscreen_seen = false, windowed_again = false, visible_seen = false;
    d.per_frame = [&](uint64_t frame, Renderer& r) {
        if (frame == 20) windowed_w = r.extent().width;
        if (frame == 30) toggled = platform::set_fullscreen(platform::FullscreenMode::Borderless);
        if (frame == 60) {
            full_w = r.extent().width;
            fullscreen_seen = platform::fullscreen_mode() == platform::FullscreenMode::Borderless;
            visible_seen = platform::visible();
        }
        if (frame == 70) platform::set_fullscreen(platform::FullscreenMode::Windowed);
        if (frame == 100) windowed_again = platform::fullscreen_mode() == platform::FullscreenMode::Windowed;
    };
    const SceneRunResult r = run_scene(d);
    print_result("fullscreen", r);
    REQUIRE(r.ok);
    CHECK_EQ(r.validation.errors, 0ull);
    std::printf("    windowed width %u, borderless width %u (toggle %s)\n", windowed_w, full_w, toggled ? "ok" : "refused");
    if (!toggled) {
        nxtest::skip("SDL refused fullscreen on this hidden window");
        return;
    }
    CHECK(fullscreen_seen);
    CHECK(windowed_again);
    if (!visible_seen) {
        // A hidden window reports fullscreen but keeps its size: there is nothing for the swapchain to follow. Set
        // NXRT_GFX_HIDDEN=0 (windows are then shown, and the monitor flashes) to check the size as well.
        std::printf("    window hidden: size follow-up not checked\n");
        return;
    }
    CHECK(full_w != windowed_w);
    CHECK(r.renderer.swapchain_creates >= 3);
}

