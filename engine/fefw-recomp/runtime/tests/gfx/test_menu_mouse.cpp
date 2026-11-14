// SPDX-License-Identifier: GPL-3.0-or-later
// Mouse pointer in menus (src/gfx/play/menu_mouse.h, menu_mouse_math.cpp): the picture rectangle, the window pixel to UI plane mapping, the
// transform helper and the pointer state (hover, click, cursor). Pure functions, no game, no window.
#include "test_framework.h"

#include <cmath>

#include "play/menu_mouse.h"

NXTEST_GROUP(menu_mouse)

namespace {

namespace mm = nxrt::play::mm;

bool close_to(double got, double want, double eps = 1e-4) { return std::fabs(got - want) < eps; }

mm::Geometry geom(int win_w, int win_h, int render_w, int render_h, double display_aspect = 0, bool keep_aspect = true) {
    mm::Geometry g;
    g.win_w = win_w;
    g.win_h = win_h;
    g.render_w = render_w;
    g.render_h = render_h;
    g.display_aspect = display_aspect;
    g.keep_aspect = keep_aspect;
    return g;
}

}  // namespace

// ---- picture rectangle ----------------------------------------------------------------------------------------------------------

NXTEST(menu_mouse_picture_rect) {
    mm::Rect r = mm::picture_rect(geom(1280, 720, 1280, 720));
    CHECK(close_to(r.x, 0) && close_to(r.y, 0) && close_to(r.w, 1280) && close_to(r.h, 720));

    r = mm::picture_rect(geom(1920, 1080, 2560, 1080));          // wider picture in a 16:9 window: bars top and bottom
    CHECK(close_to(r.x, 0) && close_to(r.y, 135) && close_to(r.w, 1920) && close_to(r.h, 810));

    r = mm::picture_rect(geom(2560, 1080, 1280, 720));           // 16:9 picture in an ultrawide window: bars left and right
    CHECK(close_to(r.x, 320) && close_to(r.y, 0) && close_to(r.w, 1920) && close_to(r.h, 1080));

    r = mm::picture_rect(geom(2560, 1080, 1280, 720, 0, false));  // stretched: always the whole window
    CHECK(close_to(r.x, 0) && close_to(r.y, 0) && close_to(r.w, 2560) && close_to(r.h, 1080));
}

// ---- window pixel to UI plane ---------------------------------------------------------------------------------------------------

NXTEST(menu_mouse_window_to_ui_16_9_centre_and_corners) {
    const mm::Geometry g = geom(1280, 720, 1280, 720);
    float nx = 9, ny = 9;
    bool inside = false;

    mm::window_to_ui(g, 640, 360, &nx, &ny, &inside);
    CHECK(close_to(nx, 0) && close_to(ny, 0) && inside);

    mm::window_to_ui(g, 0, 0, &nx, &ny, &inside);
    CHECK(close_to(nx, -1) && close_to(ny, 1) && inside);

    mm::window_to_ui(g, 1279.99f, 719.99f, &nx, &ny, &inside);   // a hair inside the bottom right corner
    CHECK(close_to(nx, 1) && close_to(ny, -1) && inside);

    mm::window_to_ui(g, -10, 360, &nx, &ny, &inside);            // left of the window
    CHECK(!inside);

    mm::window_to_ui(g, 1290, 360, &nx, &ny, &inside);           // right of the window
    CHECK(!inside);
}

NXTEST(menu_mouse_window_to_ui_ultrawide_keeps_16_9_centred) {
    // 2560x1080 window, render 2560x1080: the picture fills the window, the 16:9 UI region is the middle 1920 pixels (x 320..2240).
    const mm::Geometry g = geom(2560, 1080, 2560, 1080);
    float nx = 9, ny = 9;
    bool inside = false;

    mm::window_to_ui(g, 1280, 540, &nx, &ny, &inside);
    CHECK(close_to(nx, 0) && close_to(ny, 0) && inside);

    mm::window_to_ui(g, 320, 540, &nx, &ny, &inside);            // left edge of the 16:9 region
    CHECK(close_to(nx, -1));

    mm::window_to_ui(g, 2240, 540, &nx, &ny, &inside);           // right edge of the 16:9 region
    CHECK(close_to(nx, 1));

    mm::window_to_ui(g, 100, 540, &nx, &ny, &inside);            // beside the 16:9 region: off the UI, nx past -1
    CHECK(!inside);
    CHECK(nx < -1.0f);

    mm::window_to_ui(g, 1280, 0, &nx, &ny, &inside);             // the y axis is not affected by the width
    CHECK(close_to(ny, 1));
}

NXTEST(menu_mouse_window_to_ui_forced_display_aspect) {
    // 1920x1080 window and render with the 3D view forced to 21:9: the same centred 16:9 region as the ultrawide case, scaled to this window.
    const mm::Geometry g = geom(1920, 1080, 1920, 1080, 2.3704);
    float nx = 9, ny = 9;
    bool inside = false;

    mm::window_to_ui(g, 960, 540, &nx, &ny, &inside);
    CHECK(close_to(nx, 0) && close_to(ny, 0) && inside);

    mm::window_to_ui(g, 240, 540, &nx, &ny, &inside);            // left edge of the 16:9 region
    CHECK(close_to(nx, -1));

    mm::window_to_ui(g, 1680, 540, &nx, &ny, &inside);           // right edge of the 16:9 region
    CHECK(close_to(nx, 1));
}

NXTEST(menu_mouse_window_to_ui_narrower_than_16_9_keeps_the_full_width) {
    // 1600x1000 (16:10): the UI camera keeps its 16 units across the whole width and shows 10 units of height, so the pointer's x spans -1..1
    // across the window and its y spans -1.1111..1.1111 (16 / 14.4... = kUiAspect / aspect) across the height.
    const mm::Geometry g = geom(1600, 1000, 1600, 1000);
    float nx = 9, ny = 9;
    bool inside = false;
    mm::window_to_ui(g, 0, 500, &nx, &ny, &inside);
    CHECK(close_to(nx, -1) && close_to(ny, 0) && inside);
    mm::window_to_ui(g, 800, 0, &nx, &ny, &inside);
    CHECK(close_to(nx, 0) && close_to(ny, 16.0 / 9.0 / 1.6) && !inside);   // above the 16:9 content: off the UI
    mm::window_to_ui(g, 800, 45, &nx, &ny, &inside);                        // 4.5 % of the height down: y = 0.91 * 1.1111, still above the content
    CHECK(close_to(ny, 0.91 * (16.0 / 9.0) / 1.6) && !inside);
    mm::window_to_ui(g, 800, 70, &nx, &ny, &inside);                        // 7 % down: y = 0.86 * 1.1111, inside the +-1 range
    CHECK(inside);
}

// ---- transform ------------------------------------------------------------------------------------------------------------------

NXTEST(menu_mouse_transform_point) {
    float out[3] = {0, 0, 0};

    const float identity[12] = {1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0};
    mm::transform_point(identity, 3, 4, 5, out);
    CHECK(close_to(out[0], 3) && close_to(out[1], 4) && close_to(out[2], 5));

    const float translate[12] = {1, 0, 0, 10, 0, 1, 0, 20, 0, 0, 1, 30};
    mm::transform_point(translate, 1, 2, 3, out);
    CHECK(close_to(out[0], 11) && close_to(out[1], 22) && close_to(out[2], 33));

    const float scale[12] = {2, 0, 0, 0, 0, 3, 0, 0, 0, 0, 4, 0};
    mm::transform_point(scale, 1, 1, 1, out);
    CHECK(close_to(out[0], 2) && close_to(out[1], 3) && close_to(out[2], 4));
}

// ---- pointer --------------------------------------------------------------------------------------------------------------------

NXTEST(menu_mouse_motion_hovers_one_frame) {
    mm::Pointer p;
    const mm::Geometry g = geom(1280, 720, 1280, 720);
    p.on_motion(100, 100);
    mm::Snapshot s = p.begin_frame(true, g);
    CHECK(s.active && s.hover);
    s = p.begin_frame(true, g);
    CHECK(s.active && !s.hover);
}

NXTEST(menu_mouse_same_position_hovers_once) {
    mm::Pointer p;
    const mm::Geometry g = geom(1280, 720, 1280, 720);
    p.on_motion(100, 100);
    CHECK(p.begin_frame(true, g).hover);
    p.on_motion(100, 100);                                       // same position: no new movement
    CHECK(!p.begin_frame(true, g).hover);
}

NXTEST(menu_mouse_left_click_lasts_three_frames) {
    mm::Pointer p;
    const mm::Geometry g = geom(1280, 720, 1280, 720);
    p.on_button(1, true);
    CHECK(p.begin_frame(true, g).click);
    CHECK(p.begin_frame(true, g).click);
    CHECK(p.begin_frame(true, g).click);
    CHECK(!p.begin_frame(true, g).click);                        // ended by itself after three frames without click_used()
}

NXTEST(menu_mouse_click_ends_when_used) {
    mm::Pointer p;
    const mm::Geometry g = geom(1280, 720, 1280, 720);
    p.on_button(1, true);
    CHECK(p.begin_frame(true, g).click);
    CHECK(p.begin_frame(true, g).click);
    p.click_used();
    CHECK(!p.begin_frame(true, g).click);
    CHECK(!p.begin_frame(true, g).click);
}

NXTEST(menu_mouse_right_button_never_clicks) {
    mm::Pointer p;
    const mm::Geometry g = geom(1280, 720, 1280, 720);
    p.on_button(3, true);
    CHECK(!p.begin_frame(true, g).click);
}

NXTEST(menu_mouse_inactive_frame_drops_pending_input) {
    mm::Pointer p;
    const mm::Geometry g = geom(1280, 720, 1280, 720);
    p.on_motion(100, 100);
    p.on_button(1, true);
    mm::Snapshot s = p.begin_frame(false, g);
    CHECK(!s.active && !s.hover && !s.click);
    s = p.begin_frame(true, g);                                  // the hover and the click were dropped, not kept
    CHECK(s.active && !s.hover && !s.click);
}

NXTEST(menu_mouse_disabled_gives_inactive_snapshots) {
    mm::Pointer p;
    const mm::Geometry g = geom(1280, 720, 1280, 720);
    p.set_enabled(false);
    p.on_motion(100, 100);
    p.on_button(1, true);
    CHECK(!p.begin_frame(true, g).active);
    p.set_enabled(true);
    mm::Snapshot s = p.begin_frame(true, g);                     // pending input was dropped while disabled
    CHECK(s.active && !s.hover && !s.click);
}

NXTEST(menu_mouse_other_input_hides_cursor_until_mouse_moves) {
    mm::Pointer p;
    CHECK(p.cursor_visible(true));
    p.on_other_input();
    CHECK(!p.cursor_visible(true));
    p.on_motion(10, 10);
    CHECK(p.cursor_visible(true));
}

NXTEST(menu_mouse_cursor_never_visible_without_menu) {
    mm::Pointer p;
    CHECK(!p.cursor_visible(false));
    p.on_motion(10, 10);
    CHECK(!p.cursor_visible(false));
}
