#include "test_framework.h"

// SPDX-License-Identifier: GPL-3.0-or-later
// Platform layer and input queue (src/gfx/platform.cpp, src/gfx/input.cpp). Runs with NXRT_GFX_HIDDEN=1: windows are hidden.

#include <SDL3/SDL.h>

#include <algorithm>
#include <cstring>
#include <vector>

#include "nxrt/input.h"
#include "nxrt/platform.h"

NXTEST_GROUP(gfx_platform)

namespace {

using nxrt::input::Event;
using nxrt::input::EventType;
using nxrt::input::MouseDelta;
namespace platform = nxrt::platform;
namespace input = nxrt::input;

// Takes every queued input event, oldest first.
std::vector<Event> drain_all() {
    std::vector<Event> all;
    Event buf[64];
    for (;;) {
        const size_t n = input::queue_drain(buf, 64);
        if (n == 0) break;
        all.insert(all.end(), buf, buf + n);
    }
    return all;
}

// Pumps SDL until pred() holds or timeout_ms passes. Returns pred()'s last value.
template <class Pred>
bool pump_until(Pred pred, uint64_t timeout_ms) {
    const Uint64 deadline = SDL_GetTicks() + timeout_ms;
    for (;;) {
        platform::pump_events(false);
        if (pred()) return true;
        if (SDL_GetTicks() >= deadline) return false;
        platform::pump_events(true, 20);
    }
}

// SDL and a hidden 640x360 window for the length of one case. The destructor tears everything down, also when a
// REQUIRE returns early.
struct Session {
    bool ok = false;
    bool window = false;
    explicit Session(bool with_window = true) {
        ok = platform::init();
        if (ok && with_window) {
            platform::WindowDesc d;
            d.width = 640;
            d.height = 360;
            d.hidden = true;
            window = platform::create_window(d);
        }
    }
    ~Session() {
        platform::destroy_window();
        platform::shutdown();
    }
};

}  // namespace

// ---- (1) queue ------------------------------------------------------------------------------------------------------

NXTEST(queue_push_drain_order) {
    input::queue_clear();
    for (int i = 0; i < 10; ++i) {
        Event e;
        e.type = EventType::KeyDown;
        e.u.key.scancode = i;
        input::queue_push(e);
    }
    CHECK_EQ(input::queue_size(), static_cast<size_t>(10));
    CHECK_EQ(input::queue_pushed(), static_cast<uint64_t>(10));
    CHECK_EQ(input::queue_dropped(), static_cast<uint64_t>(0));

    Event out[4];
    CHECK_EQ(input::queue_drain(out, 4), static_cast<size_t>(4));
    bool ordered = true;
    for (int i = 0; i < 4; ++i) ordered = ordered && out[i].u.key.scancode == i;
    CHECK(ordered);

    Event one;
    CHECK(input::queue_pop(&one));
    CHECK_EQ(one.u.key.scancode, 4);
    CHECK_EQ(input::queue_size(), static_cast<size_t>(5));

    Event rest[8];
    CHECK_EQ(input::queue_drain(rest, 8), static_cast<size_t>(5));
    CHECK_EQ(rest[0].u.key.scancode, 5);
    CHECK_EQ(rest[4].u.key.scancode, 9);
    CHECK_EQ(input::queue_drain(rest, 8), static_cast<size_t>(0));
    CHECK(!input::queue_pop(&one));

    input::queue_clear();
}

NXTEST(queue_overflow_drops_oldest) {
    input::queue_clear();
    const size_t cap = input::kQueueCapacity;
    const size_t extra = 10;
    for (size_t i = 0; i < cap + extra; ++i) {
        Event e;
        e.type = EventType::KeyDown;
        e.u.key.scancode = static_cast<int32_t>(i);
        input::queue_push(e);
    }
    CHECK_EQ(input::queue_size(), cap);
    CHECK_EQ(input::queue_dropped(), static_cast<uint64_t>(extra));
    CHECK_EQ(input::queue_pushed(), static_cast<uint64_t>(cap + extra));

    // The oldest survivor is the event pushed right after the dropped ones.
    Event first;
    CHECK(input::queue_pop(&first));
    CHECK_EQ(first.u.key.scancode, static_cast<int32_t>(extra));

    input::queue_clear();
    CHECK_EQ(input::queue_size(), static_cast<size_t>(0));
    CHECK_EQ(input::queue_dropped(), static_cast<uint64_t>(0));
    CHECK_EQ(input::queue_pushed(), static_cast<uint64_t>(0));
}

// ---- (2) raw mouse accumulator --------------------------------------------------------------------------------------

NXTEST(mouse_delta_take_and_peek) {
    (void)input::mouse_delta_take();  // start from zero
    input::mouse_delta_add(1.5, -2.0);
    input::mouse_delta_add(0.25, 4.0);

    const MouseDelta p = input::mouse_delta_peek();
    CHECK_EQ(p.events, static_cast<uint64_t>(2));
    CHECK_EQ(p.dx, 1.75);
    CHECK_EQ(p.dy, 2.0);

    const MouseDelta t = input::mouse_delta_take();
    CHECK_EQ(t.events, static_cast<uint64_t>(2));
    CHECK_EQ(t.dx, 1.75);
    CHECK_EQ(t.dy, 2.0);

    const MouseDelta z = input::mouse_delta_take();
    CHECK_EQ(z.events, static_cast<uint64_t>(0));
    CHECK_EQ(z.dx, 0.0);
    CHECK_EQ(z.dy, 0.0);
    CHECK_EQ(input::mouse_delta_peek().events, static_cast<uint64_t>(0));
}

// ---- (3) window and resize ------------------------------------------------------------------------------------------

NXTEST(window_pixel_size_and_resize_event) {
    Session s;
    REQUIRE(s.ok);
    REQUIRE(s.window);
    REQUIRE(platform::has_window());

    int pw = 0, ph = 0;
    platform::pixel_size(&pw, &ph);
    int sw = 0, sh = 0;
    REQUIRE(SDL_GetWindowSizeInPixels(static_cast<SDL_Window*>(platform::native_window()), &sw, &sh));
    CHECK_EQ(pw, sw);
    CHECK_EQ(ph, sh);
    CHECK(pw >= 640);
    CHECK(ph >= 360);
    std::printf("    window pixels at 640x360: %dx%d\n", pw, ph);

    input::queue_clear();
    REQUIRE(platform::set_window_size(800, 450));

    // Collect events while pumping: the resize event may arrive a little after the pixel size already changed.
    std::vector<Event> seen;
    int nw = 0, nh = 0;
    auto resized_seen = [&]() {
        for (const Event& e : seen) {
            if (e.type == EventType::WindowResized && e.u.window.w == nw && e.u.window.h == nh) return true;
        }
        return false;
    };
    const bool done = pump_until(
        [&]() {
            const std::vector<Event> batch = drain_all();
            seen.insert(seen.end(), batch.begin(), batch.end());
            platform::pixel_size(&nw, &nh);
            return nw != pw && nw > 0 && resized_seen();
        },
        2000);
    std::printf("    window pixels after 800x450: %dx%d\n", nw, nh);
    CHECK(done);
    CHECK(nw > pw);
    CHECK(resized_seen());

    platform::set_window_size(640, 360);
}

// ---- (4) borderless fullscreen --------------------------------------------------------------------------------------

NXTEST(borderless_fullscreen_toggle) {
    Session s;
    REQUIRE(s.ok);
    REQUIRE(s.window);

    CHECK(platform::fullscreen_mode() == platform::FullscreenMode::Windowed);
    CHECK(platform::set_fullscreen(platform::FullscreenMode::Borderless));
    CHECK(platform::fullscreen_mode() == platform::FullscreenMode::Borderless);
    CHECK(platform::set_fullscreen(platform::FullscreenMode::Windowed));
    CHECK(platform::fullscreen_mode() == platform::FullscreenMode::Windowed);
}

// ---- (5) relative mouse and synthetic motion ------------------------------------------------------------------------

NXTEST(relative_mouse_round_trip_and_motion) {
    Session s;
    REQUIRE(s.ok);
    REQUIRE(s.window);

    CHECK(!platform::relative_mouse());
    CHECK(platform::set_relative_mouse(true));
    CHECK(platform::relative_mouse());
    CHECK(platform::relative_mouse_is_raw());
    CHECK(platform::set_relative_mouse(false));
    CHECK(!platform::relative_mouse());

    // Relative mode on, then a synthetic motion event pushed into SDL's queue.
    REQUIRE(platform::set_relative_mouse(true));
    platform::pump_events(false);
    input::queue_clear();
    (void)input::mouse_delta_take();

    SDL_Event ev{};
    ev.type = SDL_EVENT_MOUSE_MOTION;
    ev.motion.timestamp = SDL_GetTicksNS();
    ev.motion.windowID = SDL_GetWindowID(static_cast<SDL_Window*>(platform::native_window()));
    ev.motion.x = 10.f;
    ev.motion.y = 20.f;
    ev.motion.xrel = 3.5f;
    ev.motion.yrel = -2.25f;
    REQUIRE(SDL_PushEvent(&ev));
    platform::pump_events(false);

    const std::vector<Event> evs = drain_all();
    bool found = false;
    for (const Event& e : evs) {
        if (e.type == EventType::MouseMotion && e.u.motion.dx == 3.5f && e.u.motion.dy == -2.25f &&
            (e.flags & nxrt::input::kFlagRelative) != 0) {
            found = true;
        }
    }
    CHECK(found);

    const MouseDelta d = input::mouse_delta_take();
    CHECK(d.events >= 1);
    CHECK(d.dx >= 3.5 - 1e-6);
    CHECK(d.dy <= -2.25 + 1e-6);

    CHECK(platform::set_relative_mouse(false));
    CHECK(!platform::relative_mouse());
}

// ---- (6) gamepad hot-plug with a virtual pad ------------------------------------------------------------------------

NXTEST(gamepad_hotplug_virtual_pad) {
    Session s;
    REQUIRE(s.ok);

    const size_t before = input::gamepad_count();
    input::queue_clear();

    SDL_VirtualJoystickDesc desc;
    SDL_INIT_INTERFACE(&desc);
    desc.type = static_cast<Uint16>(SDL_JOYSTICK_TYPE_GAMEPAD);
    desc.naxes = static_cast<Uint16>(SDL_GAMEPAD_AXIS_COUNT);
    desc.nbuttons = static_cast<Uint16>(SDL_GAMEPAD_BUTTON_COUNT);
    desc.button_mask = (1u << SDL_GAMEPAD_BUTTON_COUNT) - 1u;
    desc.axis_mask = (1u << SDL_GAMEPAD_AXIS_COUNT) - 1u;
    desc.name = "nxrt virtual pad";

    const SDL_JoystickID id = SDL_AttachVirtualJoystick(&desc);
    REQUIRE(id != 0);

    bool added = false;
    std::vector<Event> seen;
    const bool got_add = pump_until(
        [&]() {
            const std::vector<Event> batch = drain_all();
            seen.insert(seen.end(), batch.begin(), batch.end());
            added = std::any_of(seen.begin(), seen.end(), [id](const Event& e) {
                return e.type == EventType::GamepadAdded && e.u.gamepad.id == id;
            });
            return added;
        },
        3000);
    CHECK(got_add);
    CHECK_EQ(input::gamepad_count(), before + 1);

    input::GamepadInfo infos[8];
    const size_t n = input::gamepad_list(infos, 8);
    bool named = false;
    for (size_t i = 0; i < n; ++i) {
        if (infos[i].id == id && std::strcmp(infos[i].name, "nxrt virtual pad") == 0) named = true;
    }
    CHECK(named);

    REQUIRE(SDL_DetachVirtualJoystick(id));
    seen.clear();
    bool removed = false;
    const bool got_remove = pump_until(
        [&]() {
            const std::vector<Event> batch = drain_all();
            seen.insert(seen.end(), batch.begin(), batch.end());
            removed = std::any_of(seen.begin(), seen.end(), [id](const Event& e) {
                return e.type == EventType::GamepadRemoved && e.u.gamepad.id == id;
            });
            return removed;
        },
        3000);
    CHECK(got_remove);
    CHECK_EQ(input::gamepad_count(), before);
}

// ---- (7) Vulkan instance extensions ---------------------------------------------------------------------------------

NXTEST(vulkan_instance_extensions_list) {
    Session s(false);
    REQUIRE(s.ok);

    uint32_t n = 0;
    const char* const* ext = platform::vulkan_instance_extensions(&n);
    REQUIRE(ext != nullptr);
    CHECK(n >= 2);
    bool surface = false;
    for (uint32_t i = 0; i < n; ++i) {
        if (std::strcmp(ext[i], "VK_KHR_surface") == 0) surface = true;
    }
    CHECK(surface);
}

// ---- (8) repeated init / window / shutdown --------------------------------------------------------------------------

NXTEST(repeated_init_window_shutdown) {
    for (int i = 0; i < 5; ++i) {
        REQUIRE(platform::init());
        platform::WindowDesc d;
        d.width = 640;
        d.height = 360;
        d.hidden = true;
        REQUIRE(platform::create_window(d));
        int w = 0, h = 0;
        platform::pixel_size(&w, &h);
        CHECK(w > 0);
        CHECK(h > 0);
        platform::destroy_window();
        platform::shutdown();
        CHECK(!platform::initialised());
    }
}

// ---- (9) gamepad poll / sensors / rumble on an id that is not open -------------------------------------------------

NXTEST(gamepad_calls_on_unknown_id) {
    Session s(false);
    REQUIRE(s.ok);

    const uint32_t bogus = 0xFFFFFFF0u;

    platform::GamepadRaw raw;
    CHECK(!platform::gamepad_poll(bogus, &raw));
    CHECK_EQ(raw.buttons, 0u);
    bool axes_zero = true;
    for (int16_t a : raw.axes) axes_zero = axes_zero && a == 0;
    CHECK(axes_zero);
    CHECK_EQ(static_cast<int>(raw.face_label[0]), 0);
    CHECK_EQ(static_cast<int>(raw.face_label[1]), 1);
    CHECK_EQ(static_cast<int>(raw.face_label[2]), 2);
    CHECK_EQ(static_cast<int>(raw.face_label[3]), 3);

    float accel[3] = {7.f, 7.f, 7.f};
    float gyro[3] = {7.f, 7.f, 7.f};
    CHECK(!platform::gamepad_sensors(bogus, accel, gyro));
    CHECK(accel[0] == 7.f && accel[1] == 7.f && accel[2] == 7.f);
    CHECK(gyro[0] == 7.f && gyro[1] == 7.f && gyro[2] == 7.f);

    CHECK(!platform::gamepad_rumble(bogus, 0, 0, 0));
    CHECK(!platform::gamepad_rumble(bogus, 65535, 65535, 100));
}
