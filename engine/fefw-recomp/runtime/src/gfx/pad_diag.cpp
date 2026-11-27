// SPDX-License-Identifier: GPL-3.0-or-later
// Controller diagnostics for the log (declarations in pad_diag.h).
#include "pad_diag.h"

#include <SDL3/SDL.h>
#include <windows.h>
#include <xinput.h>

#include <cstdio>
#include <string>
#include <vector>

#include "nxrt/log.h"

namespace nxrt::platform::pad_diag {
namespace {

constexpr const char* kTag = "pad";
constexpr size_t kValueShown = 160;   // Steam's ignore list runs to thousands of characters

const char* env(const char* name) { return SDL_GetEnvironmentVariable(SDL_GetEnvironment(), name); }

// Whether an SDL VID/PID list variable ("0x054c/0x0ce6,...") names this device.
bool listed(const char* variable, unsigned vid, unsigned pid) {
    const char* v = env(variable);
    if (!v) return false;
    char want[16];
    std::snprintf(want, sizeof want, "0x%04x/0x%04x", vid, pid);
    return SDL_strcasestr(v, want) != nullptr;
}

// The hardware id of a device path ("\\?\HID#VID_054C&PID_0CE6&IG_00#..." -> "VID_054C&PID_0CE6&IG_00"). What follows it can hold
// the controller's serial number, which does not belong in a log that gets posted.
std::string hardware_id(const char* path) {
    const std::string s = path ? path : "";
    const size_t a = s.find('#');
    if (a == std::string::npos) return "?";
    const size_t b = s.find('#', a + 1);
    return s.substr(a + 1, b == std::string::npos ? std::string::npos : b - a - 1);
}

void log_sdl_joysticks() {
    int n = 0;
    SDL_JoystickID* ids = SDL_GetJoysticks(&n);
    NXLOG_I(kTag, "SDL lists %d joystick(s)", ids ? n : 0);
    for (int i = 0; ids && i < n; ++i) {
        const SDL_JoystickID id = ids[i];
        const char* name = SDL_GetJoystickNameForID(id);
        NXLOG_I(kTag, "  SDL %u: \"%s\" %04x:%04x gamepad=%d", static_cast<unsigned>(id), name ? name : "unknown",
                SDL_GetJoystickVendorForID(id), SDL_GetJoystickProductForID(id), SDL_IsGamepad(id) ? 1 : 0);
    }
    SDL_free(ids);
}

// HID game controllers (generic desktop joystick or gamepad), and everything of Valve's: a Steam Deck's pad has other usages too.
void log_hid_devices() {
    UINT n = 0;
    if (GetRawInputDeviceList(nullptr, &n, sizeof(RAWINPUTDEVICELIST)) == static_cast<UINT>(-1)) {
        NXLOG_I(kTag, "the system's HID list is not available (error %lu)", GetLastError());
        return;
    }
    std::vector<RAWINPUTDEVICELIST> list(n);
    const UINT got = n ? GetRawInputDeviceList(list.data(), &n, sizeof(RAWINPUTDEVICELIST)) : 0;
    if (got == static_cast<UINT>(-1)) {
        NXLOG_I(kTag, "the system's HID list changed while being read");
        return;
    }
    int shown = 0;
    for (UINT i = 0; i < got; ++i) {
        if (list[i].dwType != RIM_TYPEHID) continue;
        RID_DEVICE_INFO info{};
        info.cbSize = sizeof info;
        UINT size = sizeof info;
        if (GetRawInputDeviceInfoA(list[i].hDevice, RIDI_DEVICEINFO, &info, &size) == static_cast<UINT>(-1)) continue;
        const unsigned vid = info.hid.dwVendorId, pid = info.hid.dwProductId;
        const bool controller = info.hid.usUsagePage == 1 && (info.hid.usUsage == 4 || info.hid.usUsage == 5);
        if (!controller && vid != 0x28DE) continue;
        char path[512] = {};
        UINT chars = sizeof path - 1;
        if (GetRawInputDeviceInfoA(list[i].hDevice, RIDI_DEVICENAME, path, &chars) == static_cast<UINT>(-1)) path[0] = '\0';
        NXLOG_I(kTag, "  system HID: %04x:%04x usage %u/%u %s ignore-list=%d except-list=%d", vid, pid,
                static_cast<unsigned>(info.hid.usUsagePage), static_cast<unsigned>(info.hid.usUsage), hardware_id(path).c_str(),
                listed("SDL_GAMECONTROLLER_IGNORE_DEVICES", vid, pid) ? 1 : 0,
                listed("SDL_GAMECONTROLLER_IGNORE_DEVICES_EXCEPT", vid, pid) ? 1 : 0);
        ++shown;
    }
    NXLOG_I(kTag, "the system offers %d HID game controller or Valve device(s)", shown);
}

void log_xinput_slots() {
    const int mask = xinput_slots();
    if (mask < 0) {
        NXLOG_I(kTag, "XInput is not available");
        return;
    }
    char slots[XUSER_MAX_COUNT + 1] = {};
    for (DWORD i = 0; i < XUSER_MAX_COUNT; ++i) slots[i] = (mask >> i & 1) ? '1' : '0';
    NXLOG_I(kTag, "XInput slots 0-3 connected: %s", slots);
}

}  // namespace

int xinput_slots() {
    // loaded here: nothing else in the runtime links XInput
    static const HMODULE dll = LoadLibraryA("xinput1_4.dll");
    using GetCapabilities = DWORD(WINAPI*)(DWORD, DWORD, XINPUT_CAPABILITIES*);
    const auto get = dll ? reinterpret_cast<GetCapabilities>(reinterpret_cast<void*>(GetProcAddress(dll, "XInputGetCapabilities"))) : nullptr;
    if (!get) return -1;
    int mask = 0;
    for (DWORD i = 0; i < XUSER_MAX_COUNT; ++i) {
        XINPUT_CAPABILITIES caps{};
        if (get(i, 0, &caps) == ERROR_SUCCESS) mask |= 1 << i;
    }
    return mask;
}

void log_environment() {
    static constexpr const char* kNames[] = {
        "SDL_GAMECONTROLLER_IGNORE_DEVICES", "SDL_GAMECONTROLLER_IGNORE_DEVICES_EXCEPT", "SDL_GAMECONTROLLER_ALLOW_STEAM_VIRTUAL_GAMEPAD",
        "SDL_HIDAPI_IGNORE_DEVICES", "SDL_JOYSTICK_HIDAPI", "SDL_JOYSTICK_HIDAPI_STEAMDECK", "SteamVirtualGamepadInfo",
        "SteamDeck", "SteamGamepadUI", "PROTON_ENABLE_HIDRAW", "PROTON_DISABLE_HIDRAW",
        "PROTON_PREFER_SDL", "PROTON_USE_SDL"};
    for (const char* name : kNames) {
        const char* v = env(name);
        if (!v) continue;
        const size_t len = SDL_strlen(v);
        if (len <= kValueShown) NXLOG_I(kTag, "env %s=%s", name, v);
        else NXLOG_I(kTag, "env %s=%.*s... (%zu characters)", name, static_cast<int>(kValueShown), v, len);
    }
}

void log_snapshot(const char* when) {
    NXLOG_I(kTag, "controllers %s:", when);
    log_sdl_joysticks();
    log_hid_devices();
    log_xinput_slots();
}

}  // namespace nxrt::platform::pad_diag
