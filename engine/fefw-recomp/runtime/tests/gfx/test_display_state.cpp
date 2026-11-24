// SPDX-License-Identifier: GPL-3.0-or-later
// Display state restore (display_state.h, Issue #78): the snapshot file, and the rule that only settings this run changed
// are put back. A fake backend stands in for the desktop, so every path (unchanged, restored, monitor gone, refused write,
// HDR) runs on a machine with no displays to change.
#include <cstdio>
#include <string>
#include <vector>

#include "test_framework.h"

#include "extras/display_state.h"

NXTEST_GROUP(display_state)

namespace {

namespace ds = nxrt::extras;

struct FakeMonitor {
    std::string device;
    ds::DisplayMode mode;
    bool hdr_supported = false;
    bool hdr_enabled = false;
    bool present = true;
    bool refuse_mode = false;
    bool refuse_hdr = false;
};

struct FakeBackend final : ds::DisplayBackend {
    std::vector<FakeMonitor> monitors;
    int mode_writes = 0;
    int hdr_writes = 0;

    FakeMonitor* find(const std::string& device) {
        for (FakeMonitor& m : monitors)
            if (m.device == device) return &m;
        return nullptr;
    }
    bool current_mode(const std::string& device, ds::DisplayMode* out, std::string* why) override {
        FakeMonitor* m = find(device);
        if (!m || !m->present) {
            if (why) *why = "not attached";
            return false;
        }
        *out = m->mode;
        return true;
    }
    bool apply_mode(const std::string& device, const ds::DisplayMode& want, std::string* why) override {
        FakeMonitor* m = find(device);
        if (!m || !m->present) {
            if (why) *why = "not attached";
            return false;
        }
        if (m->refuse_mode) {
            if (why) *why = "driver refused";
            return false;
        }
        ++mode_writes;
        m->mode = want;
        return true;
    }
    bool current_hdr(const std::string& device, bool* supported, bool* enabled, std::string* why) override {
        FakeMonitor* m = find(device);
        if (!m || !m->present) {
            if (why) *why = "not attached";
            return false;
        }
        *supported = m->hdr_supported;
        *enabled = m->hdr_enabled;
        return true;
    }
    bool apply_hdr(const std::string& device, bool enable, std::string* why) override {
        FakeMonitor* m = find(device);
        if (!m || !m->present) {
            if (why) *why = "not attached";
            return false;
        }
        if (m->refuse_hdr) {
            if (why) *why = "policy";
            return false;
        }
        ++hdr_writes;
        m->hdr_enabled = enable;
        return true;
    }
};

ds::DisplayMode mode(int w, int h, int hz) {
    ds::DisplayMode m;
    m.width = w;
    m.height = h;
    m.bits = 32;
    m.hz = hz;
    return m;
}

ds::DisplayState two_monitors() {
    ds::DisplayState s;
    s.count = 2;
    s.captured_ms = 1700000000000;
    s.monitors[0].device = "\\\\.\\DISPLAY1";
    s.monitors[0].mode = mode(2560, 1440, 144);
    s.monitors[0].mode_known = true;
    s.monitors[0].hdr_supported = true;
    s.monitors[0].hdr_enabled = true;
    s.monitors[0].hdr_known = true;
    s.monitors[0].sdr_white_nits = 203.f;
    s.monitors[0].primary = true;
    s.monitors[1].device = "\\\\.\\DISPLAY2";
    s.monitors[1].mode = mode(1920, 1080, 60);
    s.monitors[1].mode_known = true;
    s.monitors[1].hdr_known = true;
    return s;
}

FakeBackend backend_for(const ds::DisplayState& s) {
    FakeBackend b;
    for (int i = 0; i < s.count; ++i) {
        FakeMonitor m;
        m.device = s.monitors[i].device;
        m.mode = s.monitors[i].mode;
        m.hdr_supported = s.monitors[i].hdr_supported;
        m.hdr_enabled = s.monitors[i].hdr_enabled;
        b.monitors.push_back(m);
    }
    return b;
}

}  // namespace

NXTEST(a_session_that_changed_nothing_writes_nothing) {
    const ds::DisplayState captured = two_monitors();
    FakeBackend backend = backend_for(captured);
    backend.monitors[0].mode = mode(800, 600, 60);   // the user changed it during play; not ours to undo
    const ds::RestoreReport r = ds::restore_display_state(captured, ds::DisplayChanges{}, backend);
    CHECK_EQ(backend.mode_writes, 0);
    CHECK_EQ(backend.hdr_writes, 0);
    CHECK_EQ(r.restored_mode, 0);
    CHECK(r.ok());
}

NXTEST(a_changed_mode_is_put_back_and_reported) {
    const ds::DisplayState captured = two_monitors();
    FakeBackend backend = backend_for(captured);
    backend.monitors[0].mode = mode(1920, 1080, 120);   // what exclusive fullscreen picked
    ds::DisplayChanges changed;
    changed.mode = true;
    const ds::RestoreReport r = ds::restore_display_state(captured, changed, backend);
    CHECK_EQ(backend.mode_writes, 1);
    CHECK(backend.monitors[0].mode == captured.monitors[0].mode);
    CHECK(backend.monitors[1].mode == captured.monitors[1].mode);   // untouched monitor: no write
    CHECK_EQ(r.restored_mode, 1);
    CHECK_EQ(r.unchanged, 1);   // DISPLAY2 already matched
    CHECK(r.ok());
    CHECK(r.detail.find("DISPLAY1: mode restored") != std::string::npos);
}

NXTEST(hdr_is_restored_only_when_the_mode_change_took_it) {
    const ds::DisplayState captured = two_monitors();
    FakeBackend backend = backend_for(captured);
    backend.monitors[0].hdr_enabled = false;   // the mode switch turned HDR off
    ds::DisplayChanges changed;
    changed.mode = changed.hdr = true;
    const ds::RestoreReport r = ds::restore_display_state(captured, changed, backend);
    CHECK_EQ(backend.hdr_writes, 1);
    CHECK(backend.monitors[0].hdr_enabled);
    CHECK_EQ(r.restored_hdr, 1);
}

NXTEST(hdr_is_left_alone_when_this_run_did_not_change_it) {
    const ds::DisplayState captured = two_monitors();
    FakeBackend backend = backend_for(captured);
    backend.monitors[0].hdr_enabled = false;
    ds::DisplayChanges changed;
    changed.mode = true;   // mode only, so HDR is not ours
    const ds::RestoreReport r = ds::restore_display_state(captured, changed, backend);
    CHECK_EQ(backend.hdr_writes, 0);
    CHECK_EQ(r.restored_hdr, 0);
    CHECK(!backend.monitors[0].hdr_enabled);
}

NXTEST(a_monitor_that_is_gone_is_skipped_not_failed) {
    const ds::DisplayState captured = two_monitors();
    FakeBackend backend = backend_for(captured);
    backend.monitors[0].present = false;
    ds::DisplayChanges changed;
    changed.mode = true;
    const ds::RestoreReport r = ds::restore_display_state(captured, changed, backend);
    CHECK_EQ(r.missing, 1);
    CHECK_EQ(r.failed, 0);
    CHECK(r.ok());   // an unplugged monitor is not a restore failure
    CHECK_EQ(backend.mode_writes, 0);
}

NXTEST(a_refused_write_does_not_stop_the_other_monitor) {
    const ds::DisplayState captured = two_monitors();
    FakeBackend backend = backend_for(captured);
    backend.monitors[0].mode = mode(1280, 720, 60);
    backend.monitors[0].refuse_mode = true;
    backend.monitors[1].mode = mode(1024, 768, 60);
    ds::DisplayChanges changed;
    changed.mode = true;
    const ds::RestoreReport r = ds::restore_display_state(captured, changed, backend);
    CHECK_EQ(r.failed, 1);
    CHECK_EQ(r.restored_mode, 1);
    CHECK(!r.ok());
    CHECK(backend.monitors[1].mode == captured.monitors[1].mode);
    CHECK(r.detail.find("DISPLAY1: mode restore failed") != std::string::npos);
}

NXTEST(a_refused_hdr_write_is_reported) {
    const ds::DisplayState captured = two_monitors();
    FakeBackend backend = backend_for(captured);
    backend.monitors[0].hdr_enabled = false;
    backend.monitors[0].refuse_hdr = true;
    ds::DisplayChanges changed;
    changed.hdr = true;
    const ds::RestoreReport r = ds::restore_display_state(captured, changed, backend);
    CHECK_EQ(r.failed, 1);
    CHECK(!r.ok());
}

NXTEST(a_second_restore_writes_nothing) {
    const ds::DisplayState captured = two_monitors();
    FakeBackend backend = backend_for(captured);
    backend.monitors[0].mode = mode(800, 600, 60);
    backend.monitors[0].hdr_enabled = false;
    ds::DisplayChanges changed;
    changed.mode = changed.hdr = true;
    ds::restore_display_state(captured, changed, backend);
    const int modes = backend.mode_writes, hdrs = backend.hdr_writes;
    const ds::RestoreReport again = ds::restore_display_state(captured, changed, backend);
    CHECK_EQ(backend.mode_writes, modes);
    CHECK_EQ(backend.hdr_writes, hdrs);
    CHECK_EQ(again.restored_mode, 0);
    CHECK_EQ(again.restored_hdr, 0);
    CHECK_EQ(again.unchanged, 4);   // two monitors x two settings
}

NXTEST(a_snapshot_round_trips_through_the_file) {
    const ds::DisplayState captured = two_monitors();
    ds::DisplayChanges changed;
    changed.mode = changed.hdr = true;
    const std::string path = "display_state_test.snapshot";
    std::string err;
    REQUIRE(ds::save_display_state(captured, changed, path, &err));

    ds::DisplayState loaded;
    ds::DisplayChanges loaded_changes;
    REQUIRE(ds::load_display_state(&loaded, &loaded_changes, path, &err));
    CHECK_EQ(loaded.count, 2);
    CHECK_EQ(loaded.captured_ms, captured.captured_ms);
    CHECK(loaded_changes.mode && loaded_changes.hdr);
    CHECK_EQ(loaded.monitors[0].device, std::string("\\\\.\\DISPLAY1"));
    CHECK(loaded.monitors[0].mode == captured.monitors[0].mode);
    CHECK(loaded.monitors[0].hdr_enabled);
    CHECK(loaded.monitors[0].primary);
    CHECK_EQ(loaded.monitors[0].sdr_white_nits, 203.f);
    CHECK(loaded.monitors[1].hdr_known);
    CHECK(!loaded.monitors[1].hdr_enabled);
    std::remove(path.c_str());
}

NXTEST(a_truncated_monitor_line_does_not_hide_the_others) {
    const std::string path = "display_state_test_truncated.snapshot";
    std::FILE* f = std::fopen(path.c_str(), "wb");
    REQUIRE(f != nullptr);
    std::fputs("nxdisplay 1\ntime 12\nchanged 0 0\nmon \\\\.\\DISPLAY1 1 2\n"
               "mon \\\\.\\DISPLAY2 1920 1080 32 60 0 0 0 0 0 203000 0 1 1\n",
               f);
    std::fclose(f);
    ds::DisplayState state;
    ds::DisplayChanges changed;
    std::string err;
    REQUIRE(ds::load_display_state(&state, &changed, path, &err));
    CHECK_EQ(state.count, 1);   // the truncated line is dropped, the good one stays
    CHECK_EQ(state.monitors[0].device, std::string("\\\\.\\DISPLAY2"));
    std::remove(path.c_str());
}

NXTEST(a_file_that_is_not_a_snapshot_is_refused) {
    const std::string path = "display_state_test_bad.snapshot";
    std::FILE* f = std::fopen(path.c_str(), "wb");
    REQUIRE(f != nullptr);
    std::fputs("something else\n", f);
    std::fclose(f);
    ds::DisplayState state;
    ds::DisplayChanges changed;
    std::string err;
    CHECK(!ds::load_display_state(&state, &changed, path, &err));
    CHECK(!err.empty());
    std::remove(path.c_str());
}

NXTEST(a_missing_snapshot_is_reported_without_reading_anything) {
    ds::DisplayState state;
    ds::DisplayChanges changed;
    std::string err;
    CHECK(!ds::load_display_state(&state, &changed, "display_state_test_absent.snapshot", &err));
    CHECK(!err.empty());
}

NXTEST(the_snapshot_path_sits_beside_the_ini) {
    CHECK_EQ(ds::display_state_path("fefw.ini"), std::string("fefw.ini.display-state"));
    CHECK_EQ(ds::display_state_path(""), std::string(""));
}

// --- the real desktop ------------------------------------------------------------------------------------------------
// These touch the machine's display APIs, so they only run when NXRT_DISPLAY_LIVE_TEST=1. Both write only the value that is
// already there: the mode setter is asked for the current mode and the HDR setter for the current HDR state, so a passing run
// proves the calls are accepted without changing what anyone sees.

namespace {

bool live_enabled() {
    const char* v = std::getenv("NXRT_DISPLAY_LIVE_TEST");
    return v && *v && std::string(v) != "0";
}

}  // namespace

NXTEST(live_capture_reads_this_desktop) {
    if (!live_enabled()) {
        std::printf("display_state: live test skipped (NXRT_DISPLAY_LIVE_TEST is not set)\n");
        return;
    }
    ds::DisplayState state;
    std::string err;
    REQUIRE(ds::capture_display_state(&state, &err));
    CHECK(state.count >= 1);
    CHECK(state.captured_ms > 0);
    for (int i = 0; i < state.count; ++i) {
        const ds::MonitorState& m = state.monitors[i];
        std::printf("display_state: %s %dx%d@%dHz %d-bit primary=%d mode_known=%d hdr_supported=%d hdr_enabled=%d sdr_white=%.1f\n",
                    m.device.c_str(), m.mode.width, m.mode.height, m.mode.hz, m.mode.bits, m.primary ? 1 : 0, m.mode_known ? 1 : 0,
                    m.hdr_supported ? 1 : 0, m.hdr_enabled ? 1 : 0, static_cast<double>(m.sdr_white_nits));
        CHECK(!m.device.empty());
        CHECK(m.mode_known);
        CHECK(m.mode.width > 0 && m.mode.height > 0);
    }
}

NXTEST(live_setters_accept_the_current_values) {
    if (!live_enabled()) {
        std::printf("display_state: live test skipped (NXRT_DISPLAY_LIVE_TEST is not set)\n");
        return;
    }
    ds::DisplayState state;
    std::string err;
    REQUIRE(ds::capture_display_state(&state, &err));
    ds::DisplayBackend* backend = ds::win_display_backend();
    REQUIRE(backend != nullptr);
    CHECK_EQ(ds::restore_display_state(state, ds::DisplayChanges{}, *backend).restored_mode, 0);

    int checked = 0;
    for (int i = 0; i < state.count && checked < 1; ++i) {
        const ds::MonitorState& m = state.monitors[i];
        if (!m.mode_known) continue;
        ds::DisplayMode now;
        std::string why;
        REQUIRE(backend->current_mode(m.device, &now, &why));
        // Same mode, written back: proves ChangeDisplaySettingsExW accepts what we captured, with no visible change.
        const bool applied = backend->apply_mode(m.device, now, &why);
        std::printf("display_state: %s mode write of the current mode: %s%s%s\n", m.device.c_str(), applied ? "accepted" : "refused",
                    applied ? "" : ": ", applied ? "" : why.c_str());
        CHECK_MSG(applied, why);
        ++checked;
    }
    CHECK_EQ(checked, 1);
}

NXTEST(live_hdr_setter_accepts_the_current_state) {
    if (!live_enabled()) {
        std::printf("display_state: live test skipped (NXRT_DISPLAY_LIVE_TEST is not set)\n");
        return;
    }
    ds::DisplayState state;
    std::string err;
    REQUIRE(ds::capture_display_state(&state, &err));
    ds::DisplayBackend* backend = ds::win_display_backend();
    REQUIRE(backend != nullptr);
    int checked = 0;
    for (int i = 0; i < state.count && checked < 1; ++i) {
        const ds::MonitorState& m = state.monitors[i];
        if (!m.hdr_known || !m.hdr_supported) continue;
        bool supported = false, enabled = false;
        std::string why;
        REQUIRE(backend->current_hdr(m.device, &supported, &enabled, &why));
        if (!supported) continue;
        // The value it already has, so nothing changes on screen: this only proves the OS accepts the call.
        const bool applied = backend->apply_hdr(m.device, enabled, &why);
        std::printf("display_state: %s HDR write of the current state (%d): %s%s%s\n", m.device.c_str(), enabled ? 1 : 0,
                    applied ? "accepted" : "refused", applied ? "" : ": ", applied ? "" : why.c_str());
        CHECK_MSG(applied, why);
        ++checked;
    }
    std::printf("display_state: HDR write checked on %d monitor(s)\n", checked);
}
