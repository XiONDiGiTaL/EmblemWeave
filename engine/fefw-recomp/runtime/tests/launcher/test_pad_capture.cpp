// SPDX-License-Identifier: GPL-3.0-or-later
// "Find a button" on the Controller tab (apps/launcher/pad_capture.cpp): which row each controller button belongs to, and the poller
// that reports the first press. The poller part runs on an SDL virtual gamepad; it skips when SDL cannot create one here.
#include "test_framework.h"

#include <SDL3/SDL.h>

#include <chrono>
#include <thread>

#include "pad_capture.h"

NXTEST_GROUP(launcher_pad_capture)

namespace {

using mprl::PadCapture;
using mprl::pad_source_for_gamepad_button;
using mprl::pad_source_for_trigger;

constexpr int kRows = 16;   // the Controller tab's rows (nxrt::play::kPadSrcCount)

void set_axis(SDL_Joystick* joy, SDL_GamepadAxis a, int v) { SDL_SetJoystickVirtualAxis(joy, static_cast<int>(a), static_cast<Sint16>(v)); }

}  // namespace

// The mapping of the SDL buttons and triggers onto the 16 rows.

NXTEST(every_row_is_reached_by_exactly_one_button) {
    int seen[kRows] = {};
    for (int b = 0; b < SDL_GAMEPAD_BUTTON_COUNT; ++b) {
        const int row = pad_source_for_gamepad_button(b);
        if (row < 0) continue;
        CHECK(row < kRows);
        if (row < kRows) seen[row]++;
    }
    seen[pad_source_for_trigger(true)]++;
    seen[pad_source_for_trigger(false)]++;
    for (int r = 0; r < kRows; ++r) CHECK_EQ(seen[r], 1);
}

NXTEST(rows_follow_the_controller_tab_order) {
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_SOUTH), 0);        // A / Cross
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_EAST), 1);         // B / Circle
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_WEST), 2);         // X / Square
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_NORTH), 3);        // Y / Triangle
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_LEFT_SHOULDER), 4);   // LB / L1
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_RIGHT_SHOULDER), 5);  // RB / R1
    CHECK_EQ(pad_source_for_trigger(true), 6);                                   // LT / L2
    CHECK_EQ(pad_source_for_trigger(false), 7);                                  // RT / R2
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_START), 8);        // Start / Menu
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_BACK), 9);         // Back / View
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_LEFT_STICK), 10);  // L stick click
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_RIGHT_STICK), 11); // R stick click
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_DPAD_UP), 12);
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_DPAD_DOWN), 13);
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_DPAD_LEFT), 14);
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_DPAD_RIGHT), 15);
}

NXTEST(buttons_without_a_row_give_minus_one) {
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_GUIDE), -1);
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_MISC1), -1);
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_RIGHT_PADDLE1), -1);
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_LEFT_PADDLE1), -1);
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_TOUCHPAD), -1);
    CHECK_EQ(pad_source_for_gamepad_button(SDL_GAMEPAD_BUTTON_COUNT), -1);
    CHECK_EQ(pad_source_for_gamepad_button(-1), -1);
}

// The poller on a virtual gamepad. The virtual pad's button indices follow SDL's button enum for the buttons used here (South 0 ...
// Start 6), and its axes follow the SDL_GamepadAxis enum (the triggers are axes 4 and 5).

NXTEST(poll_reports_the_first_press_on_a_virtual_gamepad) {
    PadCapture cap;
    if (!cap.open()) {
        nxtest::skip("the SDL gamepad subsystem did not start here");
        return;
    }
    SDL_VirtualJoystickDesc d;
    SDL_INIT_INTERFACE(&d);
    d.type = SDL_JOYSTICK_TYPE_GAMEPAD;
    d.naxes = SDL_GAMEPAD_AXIS_COUNT;
    d.nbuttons = SDL_GAMEPAD_BUTTON_COUNT;
    d.vendor_id = 0x045E;   // an Xbox 360 pad
    d.product_id = 0x028E;
    d.name = "nxrt virtual capture pad";
    const SDL_JoystickID id = SDL_AttachVirtualJoystick(&d);
    if (!id) {
        cap.close();
        nxtest::skip("SDL cannot create a virtual gamepad here");
        return;
    }
    SDL_Joystick* joy = SDL_OpenJoystick(id);
    if (!joy) {
        SDL_DetachVirtualJoystick(id);
        cap.close();
        nxtest::skip("SDL cannot open the virtual gamepad here");
        return;
    }
    // A virtual trigger's rest position is the bottom of its range (SDL reports 0). Set it before the first update, so the trigger does not
    // start halfway and read as pulled.
    set_axis(joy, SDL_GAMEPAD_AXIS_LEFT_TRIGGER, -32768);
    set_axis(joy, SDL_GAMEPAD_AXIS_RIGHT_TRIGGER, -32768);

    // SDL announces the pad on a pump; the poller opens it then.
    for (int i = 0; i < 100 && !cap.has_pad(); ++i) {
        SDL_UpdateJoysticks();
        cap.poll();
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    if (!cap.has_pad()) {
        SDL_CloseJoystick(joy);
        SDL_DetachVirtualJoystick(id);
        cap.close();
        nxtest::skip("SDL did not list the virtual gamepad as a game controller here");
        return;
    }

    auto next_press = [&]() {
        int row = -1;
        for (int i = 0; i < 10 && row < 0; ++i) {
            SDL_UpdateJoysticks();
            row = cap.poll();
        }
        return row;
    };

    // Face button A (South) is the first button of the virtual pad: row 0.
    SDL_SetJoystickVirtualButton(joy, SDL_GAMEPAD_BUTTON_SOUTH, true);
    CHECK_EQ(next_press(), 0);
    SDL_SetJoystickVirtualButton(joy, SDL_GAMEPAD_BUTTON_SOUTH, false);
    SDL_UpdateJoysticks();
    CHECK_EQ(cap.poll(), -1);   // a release is not a press

    // Start: row 8.
    SDL_SetJoystickVirtualButton(joy, SDL_GAMEPAD_BUTTON_START, true);
    CHECK_EQ(next_press(), 8);
    SDL_SetJoystickVirtualButton(joy, SDL_GAMEPAD_BUTTON_START, false);
    SDL_UpdateJoysticks();
    cap.poll();

    // Right trigger pulled fully: row 7.
    set_axis(joy, SDL_GAMEPAD_AXIS_RIGHT_TRIGGER, 32767);
    CHECK_EQ(next_press(), 7);
    set_axis(joy, SDL_GAMEPAD_AXIS_RIGHT_TRIGGER, -32768);
    SDL_UpdateJoysticks();
    cap.poll();

    // Left trigger pulled fully: row 6.
    set_axis(joy, SDL_GAMEPAD_AXIS_LEFT_TRIGGER, 32767);
    CHECK_EQ(next_press(), 6);
    set_axis(joy, SDL_GAMEPAD_AXIS_LEFT_TRIGGER, -32768);
    SDL_UpdateJoysticks();
    cap.poll();

    SDL_CloseJoystick(joy);
    SDL_DetachVirtualJoystick(id);
    cap.close();
    CHECK(!cap.has_pad());
}
