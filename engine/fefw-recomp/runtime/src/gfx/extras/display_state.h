// SPDX-License-Identifier: GPL-3.0-or-later
// Display state capture and restoration (display_state.h, Issue #78). A play session records every monitor's desktop mode,
// Windows HDR (advanced colour) state and SDR white level before it changes anything, writes that snapshot beside fefw.ini,
// and puts back only the settings it changed: the mode exclusive fullscreen switched (and the HDR state that mode change
// took with it). The snapshot outlives a crash, so the next start repairs the desktop instead of leaving a mode behind.
//
// The restore algorithm and the snapshot file are portable and tested with a fake backend (tests/gfx/test_display_state.cpp).
// Capturing and applying are Windows-only (win_display_state.cpp); on other hosts capture() returns false and the session
// helpers do nothing.
#pragma once

#include <cstdint>
#include <string>

namespace nxrt::extras {

// The desktop mode fields EmblemWeave can change and put back (DEVMODE's dmPelsWidth .. dmDisplayOrientation).
struct DisplayMode {
    int width = 0;
    int height = 0;
    int bits = 0;
    int hz = 0;
    int orientation = 0;   // 0, 90, 180, 270 (DEVMODE dmDisplayOrientation in degrees)
    int x = 0;
    int y = 0;

    bool operator==(const DisplayMode& o) const {
        return width == o.width && height == o.height && bits == o.bits && hz == o.hz && orientation == o.orientation && x == o.x && y == o.y;
    }
    bool operator!=(const DisplayMode& o) const { return !(*this == o); }
};

// One monitor as it was before the session changed anything.
struct MonitorState {
    std::string device;          // the GDI device name, e.g. \\.\DISPLAY1
    DisplayMode mode;
    bool hdr_supported = false;  // Windows says this monitor supports HDR
    bool hdr_enabled = false;    // the user has HDR on for this monitor
    float sdr_white_nits = 0.f;  // Windows "SDR content brightness", 0 when the query failed
    bool primary = false;
    bool mode_known = false;     // EnumDisplaySettingsEx answered
    bool hdr_known = false;      // the CCD query answered
};

struct DisplayState {
    std::int64_t captured_ms = 0;   // unix time of the capture, for the log
    int count = 0;
    MonitorState monitors[16];      // more than any desktop this runs on; EnumDisplayMonitors is capped to this
};

// Which captured settings this run changed and may put back. A session that stayed windowed changes neither.
struct DisplayChanges {
    bool mode = false;   // exclusive fullscreen changed the desktop mode
    bool hdr = false;    // that mode change took the monitor's HDR state with it
    bool any() const { return mode || hdr; }
};

// The OS side of a restore: current values and the two setters. The fake in the tests implements this without a desktop.
struct DisplayBackend {
    virtual ~DisplayBackend() = default;
    // False, with a reason, when the monitor is no longer attached or the OS refuses the query.
    virtual bool current_mode(const std::string& device, DisplayMode* out, std::string* why) = 0;
    virtual bool apply_mode(const std::string& device, const DisplayMode& want, std::string* why) = 0;
    virtual bool current_hdr(const std::string& device, bool* supported, bool* enabled, std::string* why) = 0;
    virtual bool apply_hdr(const std::string& device, bool enable, std::string* why) = 0;
};

struct RestoreReport {
    int restored_mode = 0;
    int restored_hdr = 0;
    int unchanged = 0;   // the setting still matched the capture; nothing was written
    int missing = 0;     // the monitor is gone (unplugged or disabled); skipped
    int failed = 0;      // the OS refused a write
    std::string detail;

    bool ok() const { return failed == 0; }
};

// Puts back only what `changed` says this run changed. Every monitor is independent: a monitor that is gone is skipped,
// a refused write does not stop the others, and a value that already matches the capture is never written.
RestoreReport restore_display_state(const DisplayState& captured, const DisplayChanges& changed, DisplayBackend& backend);

// The snapshot file beside fefw.ini. A single line per monitor, so a half-written file is detected instead of applied.
bool save_display_state(const DisplayState& state, const DisplayChanges& changed, const std::string& path, std::string* err);
bool load_display_state(DisplayState* state, DisplayChanges* changed, const std::string& path, std::string* err);

// Windows: captures every attached monitor. False when nothing could be read (the caller keeps running without a snapshot).
bool capture_display_state(DisplayState* out, std::string* err = nullptr);

// The sidecar path for an ini: "<ini>.display-state", or "" when there is no ini.
std::string display_state_path(const std::string& ini_path);

// Session helpers used by play mode. begin recovers a snapshot left by a crash (and removes the file), then captures the
// current desktop and writes the new snapshot. mark_changed records that exclusive fullscreen switched the mode, so a
// crash after that point is repaired too. end restores what the snapshot says was changed, then removes the file.
// All three are no-ops when capture is unavailable, and never throw.
void begin_display_session(const std::string& ini_path);
void mark_display_session_exclusive(const std::string& ini_path);
void end_display_session(const std::string& ini_path);

// The real Windows backend and a factory for it. Off Windows the factory returns nullptr.
DisplayBackend* win_display_backend();

}  // namespace nxrt::extras
