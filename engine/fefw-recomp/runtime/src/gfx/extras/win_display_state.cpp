// SPDX-License-Identifier: GPL-3.0-or-later
// Windows display state (win_display_state.cpp, Issue #78), see display_state.h. Capture reads each monitor through GDI
// (EnumDisplayMonitors, EnumDisplaySettingsExW) and its HDR state through the CCD database (DisplayConfigGetDeviceInfo:
// advanced colour and SDR white level). Restore writes a mode back with ChangeDisplaySettingsExW and HDR with
// DISPLAYCONFIG_DEVICE_INFO_SET_HDR_STATE (Windows 11 24H2 and later). Windows has no public setter for the SDR white level
// (DISPLAYCONFIG_SDR_WHITE_LEVEL is read-only), so it is captured and logged but never written.
#include <windows.h>
#include <wingdi.h>

#include <cstdio>
#include <string>
#include <vector>

#include "extras/display_state.h"
#include "nxrt/log.h"

#pragma comment(lib, "user32.lib")

namespace nxrt::extras {
namespace {

constexpr const char* kTag = "display";

// --- CCD helpers (the same lookup win_display.cpp uses, kept here so capture and set share one path) -------------------

bool find_target(const std::wstring& gdi_name, LUID* adapter, UINT32* target_id) {
    std::vector<DISPLAYCONFIG_PATH_INFO> paths;
    std::vector<DISPLAYCONFIG_MODE_INFO> modes;
    for (int attempt = 0; attempt < 3; ++attempt) {
        UINT32 path_count = 0, mode_count = 0;
        if (GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &path_count, &mode_count) != ERROR_SUCCESS) return false;
        paths.assign(path_count, DISPLAYCONFIG_PATH_INFO{});
        modes.assign(mode_count, DISPLAYCONFIG_MODE_INFO{});
        const LONG rc = QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &path_count, paths.data(), &mode_count, modes.data(), nullptr);
        if (rc == ERROR_INSUFFICIENT_BUFFER) continue;
        if (rc != ERROR_SUCCESS) return false;
        paths.resize(path_count);
        break;
    }
    if (paths.empty()) return false;
    for (const DISPLAYCONFIG_PATH_INFO& p : paths) {
        DISPLAYCONFIG_SOURCE_DEVICE_NAME src{};
        src.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        src.header.size = sizeof src;
        src.header.adapterId = p.sourceInfo.adapterId;
        src.header.id = p.sourceInfo.id;
        if (DisplayConfigGetDeviceInfo(&src.header) != ERROR_SUCCESS) continue;
        if (gdi_name == src.viewGdiDeviceName) {
            *adapter = p.targetInfo.adapterId;
            *target_id = p.targetInfo.id;
            return true;
        }
    }
    return false;
}

// Reads the monitor's HDR state. GET_ADVANCED_COLOR_INFO_2 separates HDR from wide-colour SDR; the older query reports
// both as "advanced colour", so a HDR10 DXGI colour space is not needed here (the mode is restored, not the format).
bool read_hdr(const std::string& device, bool* supported, bool* enabled, float* white_nits, std::string* why) {
    std::wstring wide(device.begin(), device.end());
    LUID adapter{};
    UINT32 target = 0;
    if (!find_target(wide, &adapter, &target)) {
        if (why) *why = "no active CCD path";
        return false;
    }

    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO_2 info2{};
    info2.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO_2;
    info2.header.size = sizeof info2;
    info2.header.adapterId = adapter;
    info2.header.id = target;
    LONG rc = DisplayConfigGetDeviceInfo(&info2.header);
    if (rc == ERROR_SUCCESS) {
        *supported = info2.highDynamicRangeSupported != 0;
        *enabled = info2.highDynamicRangeUserEnabled != 0;
    } else {
        DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO info{};
        info.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO;
        info.header.size = sizeof info;
        info.header.adapterId = adapter;
        info.header.id = target;
        rc = DisplayConfigGetDeviceInfo(&info.header);
        if (rc != ERROR_SUCCESS) {
            if (why) *why = "advanced colour query " + std::to_string(rc);
            return false;
        }
        *supported = info.advancedColorSupported != 0;
        *enabled = info.advancedColorEnabled != 0;
    }

    DISPLAYCONFIG_SDR_WHITE_LEVEL sw{};
    sw.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL;
    sw.header.size = sizeof sw;
    sw.header.adapterId = adapter;
    sw.header.id = target;
    if (DisplayConfigGetDeviceInfo(&sw.header) == ERROR_SUCCESS) *white_nits = static_cast<float>(sw.SDRWhiteLevel) / 1000.f * 80.f;
    return true;
}

// --- capture -----------------------------------------------------------------------------------------------------------

struct CaptureContext {
    DisplayState* out;
    int seen;
};

BOOL CALLBACK capture_monitor(HMONITOR hmon, HDC, LPRECT, LPARAM param) {
    CaptureContext* ctx = reinterpret_cast<CaptureContext*>(param);
    if (ctx->seen >= 16) return FALSE;
    MONITORINFOEXW mi{};
    mi.cbSize = sizeof mi;
    if (!GetMonitorInfoW(hmon, &mi)) return TRUE;

    MonitorState& m = ctx->out->monitors[ctx->seen];
    m = MonitorState{};
    for (const wchar_t* p = mi.szDevice; *p && m.device.size() + 1 < 64; ++p) m.device.push_back(*p < 128 ? static_cast<char>(*p) : '?');
    if (m.device.empty()) return TRUE;
    m.primary = (mi.dwFlags & MONITORINFOF_PRIMARY) != 0;

    DEVMODEW dm{};
    dm.dmSize = sizeof dm;
    if (EnumDisplaySettingsExW(mi.szDevice, ENUM_CURRENT_SETTINGS, &dm, 0)) {
        m.mode_known = true;
        m.mode.width = static_cast<int>(dm.dmPelsWidth);
        m.mode.height = static_cast<int>(dm.dmPelsHeight);
        m.mode.bits = static_cast<int>(dm.dmBitsPerPel);
        m.mode.hz = static_cast<int>(dm.dmDisplayFrequency);
        m.mode.x = dm.dmPosition.x;
        m.mode.y = dm.dmPosition.y;
#ifdef DM_DISPLAYORIENTATION
        if (dm.dmFields & DM_DISPLAYORIENTATION) m.mode.orientation = static_cast<int>(dm.dmDisplayOrientation) * 90;
#endif
    }

    std::string why;
    m.hdr_known = read_hdr(m.device, &m.hdr_supported, &m.hdr_enabled, &m.sdr_white_nits, &why);
    if (!m.hdr_known) NXLOG_D(kTag, "%s: HDR not readable: %s", m.device.c_str(), why.c_str());
    ++ctx->seen;
    return TRUE;
}

// --- the Windows backend -----------------------------------------------------------------------------------------------

class WinBackend final : public DisplayBackend {
public:
    bool current_mode(const std::string& device, DisplayMode* out, std::string* why) override {
        const std::wstring wide(device.begin(), device.end());
        DEVMODEW dm{};
        dm.dmSize = sizeof dm;
        if (!EnumDisplaySettingsExW(wide.c_str(), ENUM_CURRENT_SETTINGS, &dm, 0)) {
            if (why) *why = "EnumDisplaySettingsEx failed (monitor gone?)";
            return false;
        }
        *out = DisplayMode{};
        out->width = static_cast<int>(dm.dmPelsWidth);
        out->height = static_cast<int>(dm.dmPelsHeight);
        out->bits = static_cast<int>(dm.dmBitsPerPel);
        out->hz = static_cast<int>(dm.dmDisplayFrequency);
        out->x = dm.dmPosition.x;
        out->y = dm.dmPosition.y;
#ifdef DM_DISPLAYORIENTATION
        if (dm.dmFields & DM_DISPLAYORIENTATION) out->orientation = static_cast<int>(dm.dmDisplayOrientation) * 90;
#endif
        return true;
    }

    bool apply_mode(const std::string& device, const DisplayMode& want, std::string* why) override {
        const std::wstring wide(device.begin(), device.end());
        DEVMODEW dm{};
        dm.dmSize = sizeof dm;
        dm.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY | DM_POSITION;
        dm.dmPelsWidth = static_cast<DWORD>(want.width);
        dm.dmPelsHeight = static_cast<DWORD>(want.height);
        dm.dmDisplayFrequency = static_cast<DWORD>(want.hz);
        dm.dmPosition.x = want.x;
        dm.dmPosition.y = want.y;
        if (want.bits > 0) {
            dm.dmFields |= DM_BITSPERPEL;
            dm.dmBitsPerPel = static_cast<DWORD>(want.bits);
        }
#ifdef DM_DISPLAYORIENTATION
        dm.dmFields |= DM_DISPLAYORIENTATION;
        dm.dmDisplayOrientation = static_cast<DWORD>(want.orientation / 90);
#endif
        // CDS_UPDATEREGISTRY: the restored mode survives a reboot, as the user's own choice did.
        const LONG rc = ChangeDisplaySettingsExW(wide.c_str(), &dm, nullptr, CDS_UPDATEREGISTRY, nullptr);
        if (rc != DISP_CHANGE_SUCCESSFUL) {
            if (why) *why = "ChangeDisplaySettingsEx " + std::to_string(rc);
            return false;
        }
        return true;
    }

    bool current_hdr(const std::string& device, bool* supported, bool* enabled, std::string* why) override {
        float white = 0.f;
        return read_hdr(device, supported, enabled, &white, why);
    }

    bool apply_hdr(const std::string& device, bool enable, std::string* why) override {
        // Needs a Windows 11 24H2 SDK (the type is in SDK 10.0.26100 and later; this build uses 10.0.28000).
        const std::wstring wide(device.begin(), device.end());
        LUID adapter{};
        UINT32 target = 0;
        if (!find_target(wide, &adapter, &target)) {
            if (why) *why = "no active CCD path";
            return false;
        }
        DISPLAYCONFIG_SET_HDR_STATE set{};
        set.header.type = DISPLAYCONFIG_DEVICE_INFO_SET_HDR_STATE;
        set.header.size = sizeof set;
        set.header.adapterId = adapter;
        set.header.id = target;
        set.enableHdr = enable ? 1 : 0;
        const LONG rc = DisplayConfigSetDeviceInfo(&set.header);
        if (rc != ERROR_SUCCESS) {
            if (why) *why = "SET_HDR_STATE " + std::to_string(rc);
            return false;
        }
        return true;
    }
};

}  // namespace

DisplayBackend* win_display_backend() {
    static WinBackend backend;
    return &backend;
}

bool capture_display_state(DisplayState* out, std::string* err) {
    if (!out) return false;
    *out = DisplayState{};
    CaptureContext ctx{out, 0};
    if (!EnumDisplayMonitors(nullptr, nullptr, capture_monitor, reinterpret_cast<LPARAM>(&ctx)) || ctx.seen == 0) {
        if (err) *err = "EnumDisplayMonitors found no monitor";
        return false;
    }
    out->count = ctx.seen;
    FILETIME ft{};
    GetSystemTimeAsFileTime(&ft);
    ULARGE_INTEGER t{};
    t.LowPart = ft.dwLowDateTime;
    t.HighPart = ft.dwHighDateTime;
    out->captured_ms = static_cast<std::int64_t>((t.QuadPart / 10000ULL) - 11644473600000ULL);   // 1601 -> 1970, ms
    return true;
}

// --- session helpers --------------------------------------------------------------------------------------------------

void begin_display_session(const std::string& ini_path) {
    const std::string path = display_state_path(ini_path);
    if (path.empty()) return;

    // A snapshot left behind means the last session did not reach its end (crash, kill, power loss). Put the desktop back
    // before this session captures the (possibly still wrong) state, then drop the file either way.
    DisplayState stale;
    DisplayChanges changed;
    std::string err;
    if (load_display_state(&stale, &changed, path, &err)) {
        RestoreReport report = restore_display_state(stale, changed, *win_display_backend());
        NXLOG_W(kTag, "recovered a display snapshot from an earlier session: mode %d, HDR %d, unchanged %d, missing %d, failed %d%s%s",
                report.restored_mode, report.restored_hdr, report.unchanged, report.missing, report.failed, report.detail.empty() ? "" : ": ",
                report.detail.c_str());
        std::remove(path.c_str());
    } else if (std::FILE* f = std::fopen(path.c_str(), "rb")) {
        std::fclose(f);
        std::remove(path.c_str());   // an unreadable leftover is not applied; keeping it would retry it forever
        NXLOG_W(kTag, "dropped an unreadable display snapshot: %s", err.c_str());
    }

    DisplayState now;
    if (!capture_display_state(&now, &err)) {
        NXLOG_W(kTag, "no display snapshot: %s", err.c_str());
        return;
    }
    if (!save_display_state(now, DisplayChanges{}, path, &err)) NXLOG_W(kTag, "cannot write the display snapshot: %s", err.c_str());
    else NXLOG_I(kTag, "display snapshot: %d monitor(s), saved to %s", now.count, path.c_str());
}

void mark_display_session_exclusive(const std::string& ini_path) {
    const std::string path = display_state_path(ini_path);
    if (path.empty()) return;
    DisplayState state;
    DisplayChanges changed;
    std::string err;
    if (!load_display_state(&state, &changed, path, &err)) return;
    if (changed.mode) return;
    changed.mode = changed.hdr = true;
    if (!save_display_state(state, changed, path, &err)) NXLOG_W(kTag, "cannot mark the display snapshot: %s", err.c_str());
}

void end_display_session(const std::string& ini_path) {
    const std::string path = display_state_path(ini_path);
    if (path.empty()) return;
    DisplayState state;
    DisplayChanges changed;
    std::string err;
    if (!load_display_state(&state, &changed, path, &err)) return;   // nothing was captured (or it is already repaired)
    if (changed.any()) {
        RestoreReport report = restore_display_state(state, changed, *win_display_backend());
        NXLOG_I(kTag, "display restore: mode %d, HDR %d, unchanged %d, missing %d, failed %d%s%s", report.restored_mode, report.restored_hdr,
                report.unchanged, report.missing, report.failed, report.detail.empty() ? "" : ": ", report.detail.c_str());
    }
    std::remove(path.c_str());
}

}  // namespace nxrt::extras
