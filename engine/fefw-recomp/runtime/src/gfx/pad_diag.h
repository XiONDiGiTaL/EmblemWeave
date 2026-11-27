// SPDX-License-Identifier: GPL-3.0-or-later
// What the log says about controllers beyond the pads that were opened, for "no controller connected" reports (issue 176).
// SDL drops a device it is told to ignore before it ever lists it, so SDL's list is printed next to what Windows (or Wine) offers:
// a pad in the second only was filtered or not taken by SDL, a pad in neither never reached the process. Main (SDL) thread only.
#pragma once

namespace nxrt::platform::pad_diag {

// The environment variables that decide which controllers SDL accepts (Steam and Proton set them). Once, after SDL_Init.
void log_environment();
// SDL's joysticks, then the system's HID game controllers and XInput slots. `when` names the moment in the log.
void log_snapshot(const char* when);
// The XInput slots the system reports a controller in (bit n: slot n); -1 without XInput.
int xinput_slots();

}  // namespace nxrt::platform::pad_diag
