// SPDX-License-Identifier: GPL-3.0-or-later
// Display state restore and snapshot file (display_state.cpp), see display_state.h. Portable: the only OS calls go through
// DisplayBackend, so the tests drive every path of the algorithm with a fake desktop.
#include <cstdio>
#include <sstream>
#include <string>
#include <vector>

#include "extras/display_state.h"

namespace nxrt::extras {
namespace {

void note(RestoreReport* report, const std::string& text) {
    if (!report->detail.empty()) report->detail += "; ";
    report->detail += text;
}

std::string describe(const DisplayMode& m) {
    char text[96];
    std::snprintf(text, sizeof text, "%dx%d@%dHz %d-bit orient %d at %d,%d", m.width, m.height, m.hz, m.bits, m.orientation, m.x, m.y);
    return text;
}

}  // namespace

RestoreReport restore_display_state(const DisplayState& captured, const DisplayChanges& changed, DisplayBackend& backend) {
    RestoreReport report;
    if (!changed.any()) {
        report.unchanged = captured.count;
        return report;
    }

    for (int i = 0; i < captured.count; ++i) {
        const MonitorState& m = captured.monitors[i];
        if (m.device.empty()) continue;

        if (changed.mode && m.mode_known) {
            DisplayMode now;
            std::string why;
            if (!backend.current_mode(m.device, &now, &why)) {
                // Unplugged or disabled: there is nothing to put back, and the next monitor may still need its mode.
                ++report.missing;
                note(&report, m.device + ": mode " + (why.empty() ? "not readable" : why));
                continue;
            }
            if (now != m.mode) {
                std::string err;
                if (backend.apply_mode(m.device, m.mode, &err)) {
                    ++report.restored_mode;
                    note(&report, m.device + ": mode restored to " + describe(m.mode));
                } else {
                    ++report.failed;
                    note(&report, m.device + ": mode restore failed (" + (err.empty() ? "refused" : err) + ")");
                }
            } else {
                ++report.unchanged;
            }
        }

        if (changed.hdr && m.hdr_known) {
            bool supported = false, enabled = false;
            std::string why;
            if (!backend.current_hdr(m.device, &supported, &enabled, &why)) {
                ++report.missing;
                note(&report, m.device + ": HDR " + (why.empty() ? "not readable" : why));
                continue;
            }
            if (enabled != m.hdr_enabled) {
                std::string err;
                if (backend.apply_hdr(m.device, m.hdr_enabled, &err)) {
                    ++report.restored_hdr;
                    note(&report, m.device + ": HDR " + std::string(m.hdr_enabled ? "on" : "off") + " restored");
                } else {
                    ++report.failed;
                    note(&report, m.device + ": HDR restore failed (" + (err.empty() ? "refused" : err) + ")");
                }
            } else {
                ++report.unchanged;
            }
        }
    }
    return report;
}

// --- the snapshot file ------------------------------------------------------------------------------------------------

namespace {

constexpr const char* kHeader = "nxdisplay 1";

std::string state_line(const MonitorState& m) {
    char text[320];
    std::snprintf(text, sizeof text, "mon %s %d %d %d %d %d %d %d %d %d %d %d %d", m.device.c_str(), m.mode.width, m.mode.height, m.mode.bits,
                  m.mode.hz, m.mode.orientation, m.mode.x, m.mode.y, m.hdr_supported ? 1 : 0, m.hdr_enabled ? 1 : 0,
                  static_cast<int>(m.sdr_white_nits * 1000.f + 0.5f), m.primary ? 1 : 0, m.mode_known ? 1 : 0);
    std::string line(text);
    line += m.hdr_known ? " 1" : " 0";
    return line;
}

}  // namespace

std::string display_state_path(const std::string& ini_path) {
    if (ini_path.empty()) return {};
    return ini_path + ".display-state";
}

bool save_display_state(const DisplayState& state, const DisplayChanges& changed, const std::string& path, std::string* err) {
    if (path.empty()) {
        if (err) *err = "no ini path to put the display snapshot beside";
        return false;
    }
    // Write beside the file and rename over it: a crash mid-write leaves either the old snapshot or the new one, never half.
    const std::string temp = path + ".tmp";
    std::FILE* f = std::fopen(temp.c_str(), "wb");
    if (!f) {
        if (err) *err = "cannot open " + temp;
        return false;
    }
    std::string text = std::string(kHeader) + "\n";
    text += "time " + std::to_string(state.captured_ms) + "\n";
    text += std::string("changed ") + (changed.mode ? "1" : "0") + " " + (changed.hdr ? "1" : "0") + "\n";
    for (int i = 0; i < state.count && i < 16; ++i) {
        if (state.monitors[i].device.empty()) continue;
        text += state_line(state.monitors[i]) + "\n";
    }
    const bool wrote = std::fwrite(text.data(), 1, text.size(), f) == text.size();
    const bool closed = std::fclose(f) == 0;
    if (!wrote || !closed) {
        std::remove(temp.c_str());
        if (err) *err = "cannot write " + temp;
        return false;
    }
    std::remove(path.c_str());
    if (std::rename(temp.c_str(), path.c_str()) != 0) {
        std::remove(temp.c_str());
        if (err) *err = "cannot replace " + path;
        return false;
    }
    return true;
}

bool load_display_state(DisplayState* state, DisplayChanges* changed, const std::string& path, std::string* err) {
    if (!state) return false;
    *state = DisplayState{};
    *changed = DisplayChanges{};
    std::FILE* f = std::fopen(path.c_str(), "rb");
    if (!f) {
        if (err) *err = "no snapshot at " + path;
        return false;
    }
    std::string text;
    char buf[4096];
    size_t n = 0;
    while ((n = std::fread(buf, 1, sizeof buf, f)) > 0) text.append(buf, n);
    std::fclose(f);

    std::istringstream in(text);
    std::string line;
    if (!std::getline(in, line) || line != kHeader) {
        if (err) *err = "not a display snapshot: " + path;
        return false;
    }
    int monitors = 0;
    while (std::getline(in, line)) {
        std::istringstream ls(line);
        std::string tag;
        ls >> tag;
        if (tag == "time") {
            ls >> state->captured_ms;
        } else if (tag == "changed") {
            int mode = 0, hdr = 0;
            ls >> mode >> hdr;
            changed->mode = mode != 0;
            changed->hdr = hdr != 0;
        } else if (tag == "mon" && monitors < 16) {
            MonitorState m;
            int supported = 0, enabled = 0, white_milli = 0, primary = 0, mode_known = 0, hdr_known = 0;
            if (!(ls >> m.device >> m.mode.width >> m.mode.height >> m.mode.bits >> m.mode.hz >> m.mode.orientation >> m.mode.x >> m.mode.y >>
                  supported >> enabled >> white_milli >> primary >> mode_known >> hdr_known))
                continue;   // a truncated line is ignored; the rest of the file still describes the other monitors
            m.hdr_supported = supported != 0;
            m.hdr_enabled = enabled != 0;
            m.sdr_white_nits = static_cast<float>(white_milli) / 1000.f;
            m.primary = primary != 0;
            m.mode_known = mode_known != 0;
            m.hdr_known = hdr_known != 0;
            state->monitors[monitors++] = m;
        }
    }
    state->count = monitors;
    if (monitors == 0) {
        if (err) *err = "snapshot lists no monitor: " + path;
        return false;
    }
    return true;
}

}  // namespace nxrt::extras
