// SPDX-License-Identifier: GPL-3.0-or-later
// The ImGui overlay: toggle key through the input queue, which events it consumes, relative mouse released while it is shown and
// restored when hidden, extra panels, the configured toggle key.
#include <SDL3/SDL.h>

#include "gfx_test_util.h"
#include "nxrt/overlay.h"
NXTEST_GROUP(gfx_overlay)

using namespace gfxtest;

namespace {

struct Rig {
    std::unique_ptr<Device> dev;
    std::unique_ptr<Renderer> rend;
    FrameLimiter limiter;
    std::unique_ptr<Overlay> overlay;
    bool window = false;

    bool make(bool with_window) {
        window = with_window;
        if (window) {
            if (!platform::init()) return false;
            platform::WindowDesc wd;
            wd.hidden = true;
            if (!platform::create_window(wd)) return false;
        }
        DeviceDesc dd;
        dd.validation = 1;
        dd.headless = 1;
        dev = Device::create(dd);
        if (!dev) return false;
        RendererDesc rd;
        rd.headless_width = 800;
        rd.headless_height = 450;
        rend = Renderer::create(*dev, rd);
        if (!rend) return false;
        Overlay::Context oc;
        oc.device = dev.get();
        oc.renderer = rend.get();
        oc.limiter = &limiter;
        oc.has_window = window;
        overlay = Overlay::create(oc);
        if (!overlay) return false;
        overlay->install_on_renderer();
        return true;
    }

    // One frame: the overlay draws inside end_frame when visible.
    void frame() {
        FrameInfo fi;
        if (!rend->begin_frame(&fi)) return;
        VkClearColorValue clear = {{0.1f, 0.1f, 0.1f, 1.f}};
        VkImageSubresourceRange rng = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
        (void)clear; (void)rng;
        VkRenderingAttachmentInfo att = {VK_STRUCTURE_TYPE_RENDERING_ATTACHMENT_INFO};
        att.imageView = fi.view;
        att.imageLayout = VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
        att.loadOp = VK_ATTACHMENT_LOAD_OP_CLEAR;
        att.storeOp = VK_ATTACHMENT_STORE_OP_STORE;
        att.clearValue.color = clear;
        VkRenderingInfo ri = {VK_STRUCTURE_TYPE_RENDERING_INFO};
        ri.renderArea = {{0, 0}, fi.extent};
        ri.layerCount = 1;
        ri.colorAttachmentCount = 1;
        ri.pColorAttachments = &att;
        vkCmdBeginRendering(fi.cmd, &ri);
        vkCmdEndRendering(fi.cmd);
        rend->end_frame();
    }

    ~Rig() {
        overlay.reset();
        rend.reset();
        dev.reset();
        if (window) {
            platform::destroy_window();
            platform::shutdown();
        }
    }
};

input::Event key(int scancode, bool down, bool repeat = false) {
    input::Event e;
    e.type = down ? input::EventType::KeyDown : input::EventType::KeyUp;
    e.flags = repeat ? input::kFlagRepeat : 0;
    e.u.key.scancode = scancode;
    return e;
}

input::Event motion(float x, float y) {
    input::Event e;
    e.type = input::EventType::MouseMotion;
    e.u.motion.x = x;
    e.u.motion.y = y;
    return e;
}

}  // namespace

NXTEST(toggle_key_through_the_input_queue) {
    Rig rig;
    REQUIRE(rig.make(false));
    Overlay& o = *rig.overlay;
    CHECK_EQ(o.toggle_scancode(), static_cast<int>(SDL_SCANCODE_F1));
    CHECK(!o.visible());
    CHECK(!o.wants_input());
    CHECK(!o.handle_event(key(SDL_SCANCODE_A, true)));          // hidden: nothing consumed
    CHECK(!o.handle_event(motion(30, 30)));
    CHECK(o.handle_event(key(SDL_SCANCODE_F1, true)));          // the toggle key is consumed
    CHECK(o.visible());
    CHECK(o.handle_event(key(SDL_SCANCODE_F1, true, true)));    // auto-repeat: consumed, no second toggle
    CHECK(o.visible());
    CHECK(o.handle_event(key(SDL_SCANCODE_F1, false)));         // release: consumed too
    CHECK(o.visible());
    CHECK(o.handle_event(key(SDL_SCANCODE_F1, true)));
    CHECK(!o.visible());
    // the same through the real queue
    input::queue_clear();
    input::queue_push(key(SDL_SCANCODE_F1, true));
    input::Event e;
    while (input::queue_pop(&e)) o.handle_event(e);
    CHECK(o.visible());
    rig.frame();
    CHECK(o.frames_drawn() >= 1);
}

NXTEST(mouse_is_wanted_only_over_the_menu) {
    // ImGui decides hover while it builds a frame: the pointer position goes in with the event, the answer is there after the next
    // drawn frame (wants_mouse), and button events are consumed on that state.
    Rig rig;
    REQUIRE(rig.make(false));
    Overlay& o = *rig.overlay;
    o.set_visible(true);
    for (int i = 0; i < 3; ++i) rig.frame();    // ImGui learns where its window is (16,16, 420 wide)
    o.handle_event(motion(100, 60));
    rig.frame();
    std::printf("    pointer over the menu: wants_mouse %d\n", o.wants_mouse());
    CHECK(o.wants_mouse());
    input::Event click;
    click.type = input::EventType::MouseButtonDown;
    click.u.button = {1, 1, 100.f, 60.f};
    CHECK(o.handle_event(click));               // a click on the menu is the menu's
    click.type = input::EventType::MouseButtonUp;
    CHECK(o.handle_event(click));
    o.handle_event(motion(780, 430));           // bottom right corner: nothing there
    for (int i = 0; i < 4; ++i) rig.frame();    // ImGui trickles queued mouse events in, one change per frame
    std::printf("    pointer outside the menu: wants_mouse %d\n", o.wants_mouse());
    CHECK(!o.wants_mouse());
    click.type = input::EventType::MouseButtonDown;
    click.u.button = {1, 1, 780.f, 430.f};
    CHECK(!o.handle_event(click));              // the game gets it
    click.type = input::EventType::MouseButtonUp;
    CHECK(!o.handle_event(click));
    for (int i = 0; i < 3; ++i) rig.frame();
    input::Event w;
    w.type = input::EventType::MouseWheel;
    w.u.wheel.dy = 1.f;
    CHECK(!o.handle_event(w));
    o.set_visible(false);
    CHECK(!o.wants_mouse());
    CHECK(!o.handle_event(motion(100, 60)));    // hidden: nothing is consumed
}

NXTEST(relative_mouse_is_released_while_shown_and_restored_when_hidden) {
    Rig rig;
    REQUIRE(rig.make(true));
    Overlay& o = *rig.overlay;
    REQUIRE(platform::set_relative_mouse(true));
    CHECK(platform::relative_mouse());
    o.set_visible(true);
    CHECK(!platform::relative_mouse());      // the menu needs the pointer
    o.set_visible(false);
    CHECK(platform::relative_mouse());       // and the game gets raw aim back
    // a game that was not in relative mode stays out of it
    platform::set_relative_mouse(false);
    o.set_visible(true);
    o.set_visible(false);
    CHECK(!platform::relative_mouse());
}

NXTEST(extra_panels_run_only_while_the_overlay_is_visible) {
    Rig rig;
    REQUIRE(rig.make(false));
    Overlay& o = *rig.overlay;
    int runs = 0;
    o.add_panel("extra panel", [&] { ++runs; });
    rig.frame();
    rig.frame();
    CHECK_EQ(runs, 0);
    o.set_visible(true);
    rig.frame();
    rig.frame();
    CHECK_EQ(runs, 2);
    o.set_visible(false);
    rig.frame();
    CHECK_EQ(runs, 2);
    CHECK_EQ(rig.dev->validation_stats().errors, 0ull);
    CHECK_EQ(rig.dev->validation_stats().warnings, 0ull);
}

NXTEST(configured_toggle_key) {
    _putenv_s("NXRT_GFX_OVERLAY_KEY", "F9");
    Rig rig;
    const bool ok = rig.make(false);
    _putenv_s("NXRT_GFX_OVERLAY_KEY", "");
    REQUIRE(ok);
    Overlay& o = *rig.overlay;
    CHECK_EQ(o.toggle_scancode(), static_cast<int>(SDL_SCANCODE_F9));
    CHECK(!o.handle_event(key(SDL_SCANCODE_F1, true)));
    CHECK(!o.visible());
    CHECK(o.handle_event(key(SDL_SCANCODE_F9, true)));
    CHECK(o.visible());
    o.set_toggle_scancode(SDL_SCANCODE_GRAVE);
    CHECK(o.handle_event(key(SDL_SCANCODE_GRAVE, true)));
    CHECK(!o.visible());
}

NXTEST(menu_changes_are_applied_between_frames) {
    // The menu cannot recreate the swapchain in the middle of a frame; apply_pending() does it before the next one.
    Rig rig;
    REQUIRE(rig.make(false));
    Overlay& o = *rig.overlay;
    o.set_visible(true);
    rig.frame();
    const uint64_t creates = rig.rend->stats().swapchain_creates;
    o.apply_pending();   // nothing pending: nothing happens
    CHECK_EQ(rig.rend->stats().swapchain_creates, creates);
    CHECK_EQ(rig.dev->validation_stats().errors, 0ull);
}
