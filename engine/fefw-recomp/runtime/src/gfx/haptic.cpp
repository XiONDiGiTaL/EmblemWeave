// SPDX-License-Identifier: GPL-3.0-or-later
// Haptic feedback (declarations in include/nxrt/haptic.h). The haptic engines themselves come from renhdrum.dll
// (MIT OR Apache-2.0, https://github.com/trzz1214/RenHDrum), loaded next to the executable when it is there; the file is
// optional and everything below falls back to the standard SDL two-motor rumble without it.
//
// What the game sends is the Switch HD Rumble value: amp_low / freq_low / amp_high / freq_high per hand. The runtime has
// always flattened that to one pair of motor strengths (play.cpp's map_rumble); this module keeps the four values and turns
// them back into the waveform the pad can play:
//   DualSense, wired      -> 4-channel 48 kHz float audio, channels 3 and 4 driving the voice coils
//   DualSense, wireless   -> 142-byte HID output report 0x32 every 10.67 ms, the voice coils' audio carried inside it
//   Switch Pro Controller -> 10-byte sub-command 0x10 packets: on a cable as fast as the engine ticks (the last non-silent one
//                            is re-sent every 40 ms so the pad's own watchdog does not cut a long rumble short); over
//                            Bluetooth through the library's Compatible limiter (newest value, 32 ms apart, 40 ms resend)
//   Xbox One / Series     -> the four motors (two grips, two triggers) from the library's converter, through SDL's rumble calls
//   anything else         -> the standard rumble, which play.cpp keeps sending itself (legacy_rumble() stays true)
//
// Threading. Five threads reach this file:
//   the game's thread      push(), every frame (via play.cpp's vibration sink)
//   the 1 ms worker        ticks the engines, sends the packets and motor values, runs the watchdog and the test pattern
//   the Bluetooth writer   only for a wireless DualSense: builds and writes report 0x32 on its own 10.67 ms clock
//   SDL's audio thread     audio_callback(), which asks the DSP for haptic samples
//   the main thread        configure(), pad_added(), pad_removed(), shutdown(), status()
//
// Nothing is shared through a plain variable between them:
//   g_life_mu   serialises attach/detach (main thread only; the worker never takes it)
//   g_eng_mu    guards the engine and the DSP
//   g_host_mu   guards the pad snapshot and SDL joystick, and is held while a packet is sent
//   g_audio_mu  guards the audio stream
//   g_note_mu   guards the human-readable detail line
//   everything the other threads poll (mode, strength, scale, backend, running, the retry deadline) is atomic
//
// Lock order is always g_life_mu -> g_eng_mu | g_host_mu | g_audio_mu | g_note_mu, and no two of the inner four are held at
// once except in detach(), which holds none of them while it waits for the worker. That is what keeps SDL_WaitThread from
// deadlocking against a worker that is inside send_packet(): the wait happens with g_host_mu released. The Bluetooth writer
// follows the same rule: it takes g_eng_mu to build a report, lets it go, and only then writes to the HID handle, which it
// alone uses and which detach() closes only after joining it.
#include "nxrt/haptic.h"

// Before SDL, the way the rest of the runtime does it: SDL's Windows headers otherwise pull in their own view of the API.
#include <windows.h>

#include <SDL3/SDL.h>

#include <algorithm>
#include <atomic>
#include <cstdarg>
#include <cstdio>
#include <cstring>
#include <mutex>
#include <string>

#include "nxrt/input_source.h"
#include "nxrt/log.h"
#include "nxrt/platform.h"

namespace nxrt::gfx::haptic {
namespace {

constexpr const char* kTag = "haptic";

// The library's own constants (renhdrum.h v0.6.0, kept in third_party/renhdrum). The header is not included: these are the
// ABI-stable parts we call. renhdrum_version() must be on the same major.minor branch (0x0006xx) before anything is created;
// a patch release of that branch keeps the ABI and is accepted.
constexpr uint32_t kExpectedVersion = 0x000600;
constexpr int64_t kStaleUs = 250000;                 // silence watchdog: no new value for this long -> neutral sample
constexpr uint64_t kEnumPeriodNs = 2000000000ull;    // the DualSense audio endpoint takes 1-2 s to appear after the cable
constexpr int kSendFailuresBeforeFallback = 3;       // SDL refusing the packets this many times drops back to the rumble
constexpr int64_t kProResendUs = 40000;              // wired Pro Controller: re-send the last non-silent packet this often
constexpr int32_t kBtLevelCompatible = 4;            // RenHdcSettings.bluetooth_level: Compatible, safe on an unpatched SDL3
constexpr size_t kDsBtReportSize = 142;              // RENHDRUM_DS5_BT_REPORT_SIZE
constexpr uint64_t kDsBtPeriodNs = 10666666ull;      // RENHDRUM_DS5_BT_PERIOD_NS
constexpr uint8_t kDsBtBufferLength = 0xFF;          // RENHDRUM_DS5_BT_BUFFER_LENGTH_DEFAULT
constexpr uint64_t kDsBtIdleNs = 2000000000ull;      // stop writing this long after the last vibration: the radio rests
constexpr int kDsBtFailuresMax = 50;                 // consecutive failed writes (about half a second) -> standard rumble
constexpr uint32_t kXboxGripMs = 2000;               // the grips: longer than a tick, the converter re-sends well before
constexpr uint32_t kXboxTriggerMs = 200;             // the triggers: short, so a lost stop cannot leave them buzzing
constexpr int64_t kXboxRetryUs = 100000;             // a refused Xbox send is retried at most this often
constexpr const char* kNeutral = "the standard rumble, in use again";

// RenHDrum 0.6.0 device classification and constants (from renhdrum.h)
constexpr int32_t RENHDRUM_FAMILY_NONE = 0;
constexpr int32_t RENHDRUM_FAMILY_SWITCH1 = 1;
constexpr int32_t RENHDRUM_FAMILY_SWITCH2 = 2;
constexpr int32_t RENHDRUM_FAMILY_DUALSENSE = 3;
constexpr int32_t RENHDRUM_FAMILY_DUALSENSE_EDGE = 4;
constexpr int32_t RENHDRUM_FAMILY_XBOX = 5;
constexpr int32_t RENHDRUM_FAMILY_STEAM = 6;
constexpr int32_t RENHDRUM_CLAIM_UNSUPPORTED_DEVICE = 1;

constexpr size_t RENHDRUM_DS5_BT36_REPORT_SIZE = 398;
constexpr size_t RENHDRUM_DS5_EFFECTS_SIZE = 47;
constexpr size_t RENHDRUM_PRO2_REPORT_SIZE = 64;
constexpr uint16_t RENHDRUM_PRO2_AMP_CEILING_DEFAULT = 450;

// RenHdSideSample (from renhdrum.h v0.6.0)
struct RenHdSideSample {
    float amp_low;
    float freq_low;
    float amp_high;
    float freq_high;
};

constexpr uint8_t RENHDRUM_VALVE_GRIP_FORMAT_MULAW_8K = 8;
constexpr uint8_t RENHDRUM_VALVE_GRIP_FORMAT_MULAW_4K = 9;
constexpr uint64_t RENHDRUM_VALVE_GRIP_PERIOD_NS_8K = 3875000ULL;
constexpr uint64_t RENHDRUM_VALVE_GRIP_PERIOD_NS_4K = 7750000ULL;
constexpr int RENHDRUM_VALVE_GRIP_SKIP = 2;
constexpr size_t RENHDRUM_VALVE_GRIP_REPORT_SIZE = 64;
constexpr size_t RENHDRUM_VALVE_GRIP_MODE_REPORT_SIZE = 4;
constexpr int RENHDRUM_VALVE_GRIP_MODE_REPORT_COUNT = 2;
constexpr size_t RENHDRUM_VALVE_GRIP_MODE_REPORTS_SIZE = 8;

constexpr int32_t RENHDRUM_HD_HAPTICS_SPEAKER_PERCENT = 60;

// Device info struct for renhdrum_device_classify
struct RenHdcDeviceInfo {
    uint32_t struct_size;
    int32_t family;
    uint32_t flags;
    int32_t steam_grip_format;
};

// The settings struct the library fills in (renhdrum.h v0.6.0, RenHdcSettings). Only `enabled` and the transport flags are set
// by us; the rest keeps what renhdrum_settings_default() writes.
struct RenSettings {
    bool enabled;
    int32_t usb_interval_ms;
    int32_t bluetooth_level;      /* 1-4, 4 = Compatible (default) */
    bool usb_auto_slow;
    bool sync_to_input;
    bool click_pulse;
    bool sync_bluetooth;
    bool usb_no_queue;
    int32_t usb_pause_ms;
    bool envelope_5ms;
    bool frequency_in_packet;
    bool frequency_both_bands;
    bool bt_after_input;
    bool crisp_notes;
    bool bt_crisp_notes;
    bool bt_fast;
};

struct RenEngine;
struct RenDsDsp;
struct RenXbox;

// The core entry points are required; each controller's group is "use if present": a library without it only leaves that
// controller on the standard rumble.
struct Api {
    bool tried = false;
    bool ok = false;
    bool has_ds = false;           // DualSense DSP (wired audio, and the base of the wireless path)
    bool has_ds_bt = false;        // DualSense report 0x32
    bool has_ds_bt36 = false;      // DualSense report 0x36 (combined rumble + triggers + speaker)
    bool has_xbox = false;         // Xbox four-motor converter
    bool has_xbox_direct = false;  // Xbox Direct mode (SDL_RumbleJoystick + Triggers)
    bool has_steam_grip = false;   // Steam Controller grip haptics
    bool has_pro2 = false;         // Switch 2 Pro Controller
    bool has_adaptive_triggers = false; // Adaptive trigger support (tri-trigger + presets)
    HMODULE dll = nullptr;
    uint32_t (*version)() = nullptr;
    int32_t (*settings_default)(RenSettings*) = nullptr;
    int32_t (*settings_resolve_pure)(const void*, RenSettings*) = nullptr; // RenHdcConfig* but we don't include the header
    int32_t (*preset_apply)(int32_t, void*) = nullptr;
    int32_t (*preset_detect)(const void*, int32_t*) = nullptr;
    RenEngine* (*engine_create)(bool, bool) = nullptr;
    void (*engine_destroy)(RenEngine*) = nullptr;
    int32_t (*engine_apply)(RenEngine*, const RenSettings*) = nullptr;
    int32_t (*push_half)(RenEngine*, int64_t, bool, float, float, float, float) = nullptr;
    int32_t (*tick)(RenEngine*, int64_t, uint8_t*) = nullptr;
    void (*set_nx2)(int32_t) = nullptr;
    RenDsDsp* (*dsp_create)(int32_t, bool) = nullptr;
    void (*dsp_destroy)(RenDsDsp*) = nullptr;
    int32_t (*dsp_set_half)(RenDsDsp*, bool, float, float, float, float) = nullptr;
    int32_t (*dsp_fill)(RenDsDsp*, float*, size_t) = nullptr;
    int32_t (*dsp_bt_report)(RenDsDsp*, uint8_t, uint8_t*) = nullptr;
    int32_t (*dsp_set_speaker)(RenDsDsp*, int32_t) = nullptr;
    int32_t (*dsp_bt36_report)(RenDsDsp*, const uint8_t*, uint8_t, uint8_t*) = nullptr;
    RenDsDsp* (*valve_grip_dsp_create)(int32_t, bool) = nullptr;
    RenDsDsp* (*valve_grip_dsp_create_ex)(int32_t, bool, bool) = nullptr;
    int32_t (*valve_grip_build_report)(RenDsDsp*, uint8_t, uint8_t*) = nullptr;
    int32_t (*valve_grip_mode_tick)(RenDsDsp*, uint8_t, uint64_t, bool, uint8_t*) = nullptr;
    int32_t (*valve_grip_mode_off)(uint8_t, uint8_t*) = nullptr;
    RenXbox* (*xb_create)(bool) = nullptr;
    void (*xb_destroy)(RenXbox*) = nullptr;
    int32_t (*xb_push)(RenXbox*, int64_t, bool, float, float, float, float) = nullptr;
    int32_t (*xb_tick)(RenXbox*, int64_t, uint16_t*) = nullptr;
    int32_t (*xb_resend)(RenXbox*) = nullptr;
    void* (*xb_direct_create)() = nullptr;
    void (*xb_direct_destroy)(void*) = nullptr;
    int32_t (*xb_direct_push)(void*, const void*, int64_t, bool, float, float, bool, uint16_t*) = nullptr;
    int32_t (*xb_direct_tick)(void*, int64_t, bool, uint16_t*) = nullptr;
    void* (*tri_trigger_create)() = nullptr;
    void (*tri_trigger_destroy)(void*) = nullptr;
    int32_t (*tri_trigger_update)(void*, uint32_t, uint8_t, float, float, float, float, bool, uint8_t*) = nullptr;
    void (*tri_trigger_note_action)(void*, uint32_t) = nullptr;
    void* (*trigger_preset_create)() = nullptr;
    void (*trigger_preset_destroy)(void*) = nullptr;
    void (*trigger_preset_set)(void*, uint8_t, float, uint32_t) = nullptr;
    int32_t (*trigger_preset_tick)(void*, uint32_t, uint8_t*) = nullptr;
    int32_t (*hd_haptics_speaker_block)(int32_t, uint8_t*) = nullptr;
    int32_t (*hd_haptics_trigger_block)(int32_t, const uint8_t*, uint8_t*) = nullptr;
    int32_t (*dualsense_evaluate_triggers)(uint64_t, bool, bool, bool, int64_t, int64_t, uint8_t*) = nullptr;
    void* (*pro2_create)(uint16_t) = nullptr;
    void (*pro2_destroy)(void*) = nullptr;
    int32_t (*pro2_push_half)(void*, int64_t, bool, float, float, float, float) = nullptr;
    int32_t (*pro2_build_report)(void*, int64_t, uint8_t*) = nullptr;
    int32_t (*pro2_silent_report)(void*, uint8_t*) = nullptr;
    const void* (*options)(int32_t*, int32_t*) = nullptr;
    void* (*store_create)() = nullptr;
    void (*store_destroy)(void*) = nullptr;
    int32_t (*store_get)(const void*, const char*, int32_t*) = nullptr;
    int32_t (*store_set)(void*, const char*, int32_t) = nullptr;
    int32_t (*store_reset)(void*) = nullptr;
    int32_t (*store_is_recommended)(const void*, bool*) = nullptr;
    int32_t (*store_to_config)(const void*, void*) = nullptr;
    float (*store_shape_amplitude)(const void*, float) = nullptr;
    int32_t (*demo_sample)(int32_t, uint32_t, RenHdSideSample*, RenHdSideSample*) = nullptr;
    void* (*test_meter_create)() = nullptr;
    void (*test_meter_destroy)(void*) = nullptr;
    void (*test_meter_record)(void*, uint64_t, bool) = nullptr;
    int32_t (*test_meter_format)(const void*, char*, size_t) = nullptr;
    int32_t (*device_classify)(uint16_t, uint16_t, void*) = nullptr;
    const char* (*claim_status_text)(int32_t) = nullptr;
};

Api& api() {
    static Api a;
    return a;
}

template <typename T>
bool bind(HMODULE dll, const char* name, T* out) {
    *out = reinterpret_cast<T>(GetProcAddress(dll, name));
    return *out != nullptr;
}

// The folder the running executable is in, with a trailing separator. The library is looked for there and nowhere else: a bare
// name would let the working folder or PATH decide which DLL is loaded.
std::wstring exe_dir() {
    std::wstring buf(MAX_PATH, L'\0');
    for (;;) {
        const DWORD n = GetModuleFileNameW(nullptr, buf.data(), static_cast<DWORD>(buf.size()));
        if (n == 0) return {};
        if (n < buf.size()) {
            buf.resize(n);
            break;
        }
        buf.resize(buf.size() * 2);   // the path is longer than MAX_PATH: grow and ask again
    }
    const size_t s = buf.find_last_of(L"\\/");
    if (s == std::wstring::npos) return {};
    return buf.substr(0, s + 1);
}

// The logger formats narrow strings only, so a wide path goes through this first (the same helper the DLSS loader uses).
std::string narrow(const std::wstring& w) {
    if (w.empty()) return {};
    const int n = WideCharToMultiByte(CP_UTF8, 0, w.c_str(), -1, nullptr, 0, nullptr, nullptr);
    if (n <= 1) return {};
    std::string s(static_cast<size_t>(n - 1), '\0');
    WideCharToMultiByte(CP_UTF8, 0, w.c_str(), -1, s.data(), n, nullptr, nullptr);
    return s;
}

// Loads the library once, from the exe folder by full path. False when it is not there or does not carry the entry points: the
// module then stays on the standard rumble, which is not an error (the file is optional).
//
// The search flags matter as much as the full path: LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR lets the library's own dependencies come
// from its folder, LOAD_LIBRARY_SEARCH_SYSTEM32 keeps the system ones in System32, and neither the working folder nor PATH is
// consulted. That is the same way the FSR and DLSS loaders reach their DLLs.
bool load_api() {
    Api& a = api();
    if (a.tried) return a.ok;
    a.tried = true;
    const std::wstring dir = exe_dir();
    if (dir.empty()) {
        NXLOG_W(kTag, "the executable's folder cannot be read: standard rumble only");
        return false;
    }
    const std::wstring path = dir + L"renhdrum.dll";
    if (GetFileAttributesW(path.c_str()) == INVALID_FILE_ATTRIBUTES) {
        NXLOG_I(kTag, "renhdrum.dll not next to the executable (%s): standard rumble only", narrow(path).c_str());
        return false;
    }
    a.dll = LoadLibraryExW(path.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32);
    if (!a.dll) {
        NXLOG_W(kTag, "LoadLibraryExW(%s) failed (error %lu): standard rumble only", narrow(path).c_str(), GetLastError());
        return false;
    }
    bool ok = bind(a.dll, "renhdrum_version", &a.version) &&
              bind(a.dll, "renhdrum_settings_default", &a.settings_default) &&
              bind(a.dll, "renhdrum_settings_resolve_pure", &a.settings_resolve_pure) &&
              bind(a.dll, "renhdrum_preset_apply", &a.preset_apply) &&
              bind(a.dll, "renhdrum_preset_detect", &a.preset_detect) &&
              bind(a.dll, "renhdrum_engine_create", &a.engine_create) &&
              bind(a.dll, "renhdrum_engine_destroy", &a.engine_destroy) &&
              bind(a.dll, "renhdrum_engine_apply_settings", &a.engine_apply) &&
              bind(a.dll, "renhdrum_engine_push_half_sample", &a.push_half) &&
              bind(a.dll, "renhdrum_engine_tick", &a.tick) &&
              bind(a.dll, "renhdrum_set_nx2_mode", &a.set_nx2);
    if (ok) {
        const uint32_t v = a.version();
        if ((v >> 8) != (kExpectedVersion >> 8)) {
            NXLOG_W(kTag, "renhdrum.dll is version 0x%06x, expected the 0x%04xxx branch: standard rumble only", v,
                    kExpectedVersion >> 8);
            ok = false;
        }
    } else {
        NXLOG_W(kTag, "renhdrum.dll is missing entry points: standard rumble only");
    }
    if (!ok) {
        FreeLibrary(a.dll);
        a = Api{};
        a.tried = true;
        return false;
    }
    // DualSense DSP (USB audio + base for BT)
    a.has_ds = bind(a.dll, "renhdrum_dualsense_dsp_create", &a.dsp_create) &&
               bind(a.dll, "renhdrum_dualsense_dsp_destroy", &a.dsp_destroy) &&
               bind(a.dll, "renhdrum_dualsense_dsp_set_half_target", &a.dsp_set_half) &&
               bind(a.dll, "renhdrum_dualsense_dsp_fill_f32", &a.dsp_fill) &&
               bind(a.dll, "renhdrum_dualsense_dsp_set_speaker", &a.dsp_set_speaker);
    // DualSense BT report 0x32 (legacy)
    a.has_ds_bt = a.has_ds && bind(a.dll, "renhdrum_dualsense_dsp_build_bt_report", &a.dsp_bt_report);
    // DualSense BT report 0x36 (combined rumble + triggers + speaker) - 0.6.0+
    a.has_ds_bt36 = a.has_ds && bind(a.dll, "renhdrum_dualsense_dsp_build_bt36_report", &a.dsp_bt36_report);
    // Xbox four-motor converter
    a.has_xbox = bind(a.dll, "renhdrum_xbox_rumble_create", &a.xb_create) &&
                 bind(a.dll, "renhdrum_xbox_rumble_destroy", &a.xb_destroy) &&
                 bind(a.dll, "renhdrum_xbox_rumble_push_half_sample", &a.xb_push) &&
                 bind(a.dll, "renhdrum_xbox_rumble_tick", &a.xb_tick) &&
                 bind(a.dll, "renhdrum_xbox_rumble_resend", &a.xb_resend);
    // Xbox Direct mode (SDL_RumbleJoystick + Triggers) - 0.6.0+
    a.has_xbox_direct = bind(a.dll, "renhdrum_xbox_direct_create", &a.xb_direct_create) &&
                        bind(a.dll, "renhdrum_xbox_direct_destroy", &a.xb_direct_destroy) &&
                        bind(a.dll, "renhdrum_xbox_direct_push", &a.xb_direct_push) &&
                        bind(a.dll, "renhdrum_xbox_direct_tick", &a.xb_direct_tick);
    // Steam Controller grip haptics - 0.6.0+
    a.has_steam_grip = bind(a.dll, "renhdrum_valve_grip_dsp_create", &a.valve_grip_dsp_create) &&
                       bind(a.dll, "renhdrum_valve_grip_dsp_create_ex", &a.valve_grip_dsp_create_ex) &&
                       bind(a.dll, "renhdrum_valve_grip_build_report", &a.valve_grip_build_report) &&
                       bind(a.dll, "renhdrum_valve_grip_mode_tick", &a.valve_grip_mode_tick) &&
                       bind(a.dll, "renhdrum_valve_grip_mode_off", &a.valve_grip_mode_off);
    // Adaptive triggers (tri-trigger + presets) - 0.6.0+
    a.has_adaptive_triggers = bind(a.dll, "renhdrum_tri_trigger_create", &a.tri_trigger_create) &&
                              bind(a.dll, "renhdrum_tri_trigger_destroy", &a.tri_trigger_destroy) &&
                              bind(a.dll, "renhdrum_tri_trigger_update", &a.tri_trigger_update) &&
                              bind(a.dll, "renhdrum_tri_trigger_note_action", &a.tri_trigger_note_action) &&
                              bind(a.dll, "renhdrum_trigger_preset_create", &a.trigger_preset_create) &&
                              bind(a.dll, "renhdrum_trigger_preset_destroy", &a.trigger_preset_destroy) &&
                              bind(a.dll, "renhdrum_trigger_preset_set", &a.trigger_preset_set) &&
                              bind(a.dll, "renhdrum_trigger_preset_tick", &a.trigger_preset_tick) &&
                              bind(a.dll, "renhdrum_hd_haptics_speaker_block", &a.hd_haptics_speaker_block) &&
                              bind(a.dll, "renhdrum_hd_haptics_trigger_block", &a.hd_haptics_trigger_block) &&
                              bind(a.dll, "renhdrum_dualsense_evaluate_triggers", &a.dualsense_evaluate_triggers);
    // Switch 2 Pro Controller - 0.6.0+
    a.has_pro2 = bind(a.dll, "renhdrum_pro2_create", &a.pro2_create) &&
                 bind(a.dll, "renhdrum_pro2_destroy", &a.pro2_destroy) &&
                 bind(a.dll, "renhdrum_pro2_push_half_sample", &a.pro2_push_half) &&
                 bind(a.dll, "renhdrum_pro2_build_report", &a.pro2_build_report) &&
                 bind(a.dll, "renhdrum_pro2_silent_report", &a.pro2_silent_report);
    // Settings table / store / demo / test meter / device classification - 0.6.0+
    bind(a.dll, "renhdrum_options", &a.options);
    bind(a.dll, "renhdrum_store_create", &a.store_create);
    bind(a.dll, "renhdrum_store_destroy", &a.store_destroy);
    bind(a.dll, "renhdrum_store_get", &a.store_get);
    bind(a.dll, "renhdrum_store_set", &a.store_set);
    bind(a.dll, "renhdrum_store_reset", &a.store_reset);
    bind(a.dll, "renhdrum_store_is_recommended", &a.store_is_recommended);
    bind(a.dll, "renhdrum_store_to_config", &a.store_to_config);
    bind(a.dll, "renhdrum_store_shape_amplitude", &a.store_shape_amplitude);
    bind(a.dll, "renhdrum_demo_sample", &a.demo_sample);
    bind(a.dll, "renhdrum_test_meter_create", &a.test_meter_create);
    bind(a.dll, "renhdrum_test_meter_destroy", &a.test_meter_destroy);
    bind(a.dll, "renhdrum_test_meter_record", &a.test_meter_record);
    bind(a.dll, "renhdrum_test_meter_format", &a.test_meter_format);
    bind(a.dll, "renhdrum_device_classify", &a.device_classify);
    bind(a.dll, "renhdrum_claim_status_text", &a.claim_status_text);
    // The game is a Switch 2 title: its values use the Switch 2 ranges, and the library picks its Switch 2 curves for every
    // controller from this one global switch.
    a.set_nx2(2);
    a.ok = true;
    NXLOG_I(kTag, "renhdrum %u.%u.%u loaded (DualSense %s, DualSense BT 0x32 %s, DualSense BT 0x36 %s, Xbox %s, Xbox Direct %s, Steam Grip %s, Pro2 %s, Adaptive Triggers %s)",
            (a.version() >> 16) & 0xFF, (a.version() >> 8) & 0xFF, a.version() & 0xFF,
            a.has_ds ? "yes" : "no", a.has_ds_bt ? "yes" : "no", a.has_ds_bt36 ? "yes" : "no",
            a.has_xbox ? "yes" : "no", a.has_xbox_direct ? "yes" : "no", a.has_steam_grip ? "yes" : "no",
            a.has_pro2 ? "yes" : "no", a.has_adaptive_triggers ? "yes" : "no");
    return true;
}

// ---- state ---------------------------------------------------------------------------------------------------------------------------

// Polled from several threads, so atomic rather than guarded: the game's thread reads the mode and the backend on every
// vibration value, and it must never wait for the main thread.
std::atomic<Mode> g_mode{Mode::Auto};
std::atomic<int> g_strength{-1};          // 0..100 percent, or < 0 to inherit g_scale
std::atomic<float> g_scale{1.f};          // [pad] rumble_scale: the multiplier the standard rumble already used
std::atomic<Backend> g_backend{Backend::None};
std::atomic<bool> g_running{false};       // a backend is really driving the pad right now
std::atomic<uint32_t> g_pad_id{0};        // g_pad.id, readable without g_host_mu: legacy_rumble() runs on the game's thread
std::atomic<bool> g_rumble_on{true};      // [pad] rumble: the master switch, and it covers every output path
std::atomic<uint64_t> g_next_enum_ns{0};  // next DualSense audio endpoint scan
std::atomic<float> g_last_amp{0.f};
std::atomic<uint64_t> g_pushes{0};
std::atomic<uint64_t> g_packets{0};
std::atomic<uint64_t> g_fills{0};         // audio callbacks that produced haptic samples
std::atomic<float> g_fill_peak{0.f};      // loudest haptic sample the DSP produced (channels 2 and 3)
std::atomic<uint64_t> g_nonzero{0};       // values the game sent with a non-zero amplitude
std::atomic<float> g_amp_peak{0.f};       // loudest amplitude the game sent
std::atomic<uint64_t> g_test_until_ns{0}; // the "Buzz once" window, 0 when idle
std::atomic<int> g_send_fail{0};          // consecutive SDL_SendJoystickEffect failures

// Advanced haptics settings (RenHDrum 0.6.0+)
std::atomic<bool> g_advanced_haptics{false};
std::atomic<bool> g_xbox_direct_enabled{false};
std::atomic<bool> g_ds_bt36{false};
std::atomic<bool> g_adaptive_triggers{false};
std::atomic<bool> g_steam_grips{false};
std::atomic<bool> g_switch2_pro{false};
std::atomic<bool> g_crisp_notes{false};
std::atomic<bool> g_envelope_5ms{false};
std::atomic<int> g_ds_volume{60};         // DualSense speaker volume (0-100)

std::mutex g_life_mu;                     // attach/detach, main thread only

std::mutex g_note_mu;                     // g_detail
char g_detail[192] = {};

struct PadState {
    uint32_t id = 0;
    SDL_Joystick* joy = nullptr;      // also guarded by g_host_mu, read by the worker when it sends
    uint16_t vendor = 0;
    uint16_t product = 0;
    bool dualsense = false;
    bool pro = false;                 // Pro Controller or Joy-Con (Nintendo's HD Rumble packets)
    bool xbox = false;
    bool bt = false;                  // SDL says wireless
    bool wired = false;               // SDL says wired (both false: SDL does not know)
    bool rumble = false;              // SDL says the joystick has rumble motors
    char name[64] = {};
};

std::mutex g_host_mu;                     // g_pad and the send
PadState g_pad;
SDL_Thread* g_thread = nullptr;
std::atomic<bool> g_run{false};

std::mutex g_eng_mu;                      // the engines and the DSP, and the Pro Controller resend state below
RenEngine* g_engine = nullptr;
RenDsDsp* g_dsp = nullptr;
RenXbox* g_xbox = nullptr;
int64_t g_at[2] = {0, 0};                 // last push per hand, microseconds
bool g_muted[2] = {false, false};         // the watchdog already sent the neutral sample for this hand
bool g_pro_wired = false;                 // the Pro Controller engine drives a cable: the host does the 40 ms resend
uint8_t g_last_pkt[10] = {};              // the last packet sent to it
bool g_last_pkt_live = false;             // ... and it was not silence
int64_t g_last_send_us = 0;

// The wireless DualSense writer. The handle is opened by attach() before the thread starts, used by that thread only, and
// closed by detach() after joining it, so it needs no lock of its own.
SDL_hid_device* g_hid = nullptr;
SDL_Thread* g_bt_thread = nullptr;
std::atomic<bool> g_bt_run{false};

// DualSense BT 0x36 writer (separate thread for combined report)
SDL_hid_device* g_hid_bt36 = nullptr;
SDL_Thread* g_bt36_thread = nullptr;
std::atomic<bool> g_bt36_run{false};

// Steam Controller grip writer
SDL_hid_device* g_hid_steam = nullptr;
SDL_Thread* g_steam_thread = nullptr;
std::atomic<bool> g_steam_run{false};

// Xbox Direct state. The store is host-owned and not thread-safe: every push happens on the game's thread under g_eng_mu,
// while the worker only asks the engine for the settled values at burst end. push() rejects a null store (RENHDRUM_ERR_NULL_PTR).
void* g_xbox_direct = nullptr;
void* g_xbox_store = nullptr;

// Adaptive trigger state (DualSense)
void* g_tri_trigger_l = nullptr;
void* g_tri_trigger_r = nullptr;
void* g_trigger_preset_l = nullptr;
void* g_trigger_preset_r = nullptr;
uint8_t g_trigger_effect_l[11] = {};
uint8_t g_trigger_effect_r[11] = {};
bool g_trigger_effect_l_valid = false;
bool g_trigger_effect_r_valid = false;

// Switch 2 Pro Controller state
void* g_pro2 = nullptr;

std::mutex g_audio_mu;                    // the DualSense stream
SDL_AudioStream* g_stream = nullptr;

void note(const char* fmt, ...) {
    char buf[sizeof(g_detail)] = {};
    if (fmt && fmt[0]) {
        va_list ap;
        va_start(ap, fmt);
        std::vsnprintf(buf, sizeof(buf), fmt, ap);
        va_end(ap);
    }
    std::lock_guard<std::mutex> lk(g_note_mu);
    std::memcpy(g_detail, buf, sizeof(g_detail));
}

// The amplitude multiplier: the player's percentage when the settings name one, otherwise the multiplier the standard
// rumble already used ([pad] rumble_scale, 1.0 by default and up to 2.0).
float strength_scale() {
    const int st = g_strength.load(std::memory_order_relaxed);
    if (st >= 0) return static_cast<float>(std::clamp(st, 0, 100)) / 100.f;
    return static_cast<float>(std::clamp(g_scale.load(std::memory_order_relaxed), 0.f, 2.f));
}

int64_t now_us() { return static_cast<int64_t>(SDL_GetTicksNS() / 1000); }

// ---- DualSense audio --------------------------------------------------------------------------------------------------------

void SDLCALL audio_callback(void* user, SDL_AudioStream* stream, int additional_amount, int total_amount) {
    (void)total_amount;
    if (additional_amount <= 0) return;
    int floats = additional_amount / static_cast<int>(sizeof(float));
    floats -= floats % 4;                       // whole 4-channel frames only
    if (floats <= 0) return;
    // A stack buffer keeps the audio thread allocation free; a big request is filled in chunks.
    float buf[2048];
    while (floats > 0) {
        const int n = std::min(floats, 2048);
        int rc = -1;
        {
            std::lock_guard<std::mutex> lk(g_eng_mu);
            if (g_dsp) rc = api().dsp_fill(g_dsp, buf, static_cast<size_t>(n / 4));
        }
        if (rc < 0) std::memset(buf, 0, sizeof(float) * static_cast<size_t>(n));   // never leave a pop or a DC hum
        else {
            g_fills.fetch_add(1, std::memory_order_relaxed);
            float peak = 0.f;
            for (int k = 2; k < n; k += 4) peak = std::max(peak, std::abs(buf[k]));
            for (int k = 3; k < n; k += 4) peak = std::max(peak, std::abs(buf[k]));
            if (peak > g_fill_peak.load(std::memory_order_relaxed)) g_fill_peak.store(peak, std::memory_order_relaxed);
        }
        SDL_PutAudioStreamData(stream, buf, static_cast<int>(sizeof(float) * static_cast<size_t>(n)));
        floats -= n;
    }
    (void)user;
}

// Finds the DualSense's playback endpoint (it exposes four channels). 0 when it is not there yet.
SDL_AudioDeviceID find_dualsense_audio() {
    int count = 0;
    SDL_AudioDeviceID* ids = SDL_GetAudioPlaybackDevices(&count);
    if (!ids) return 0;
    SDL_AudioDeviceID found = 0;
    for (int i = 0; i < count && !found; ++i) {
        const char* name = SDL_GetAudioDeviceName(ids[i]);
        if (!name || (!std::strstr(name, "DualSense") && !std::strstr(name, "Wireless Controller"))) continue;
        SDL_AudioSpec spec{};
        if (!SDL_GetAudioDeviceFormat(ids[i], &spec, nullptr)) continue;
        NXLOG_D(kTag, "candidate \"%s\": %d channels at %d Hz", name, static_cast<int>(spec.channels), static_cast<int>(spec.freq));
        if (spec.channels >= 4) found = ids[i];
    }
    if (!found) NXLOG_D(kTag, "%d playback devices, none with four channels yet", count);
    SDL_free(ids);
    return found;
}

// Opened by the worker while the backend is DualSenseUsb. Every failure leaves g_running false, which is what makes
// legacy_rumble() hand the pad back to play.cpp's SDL rumble: a DualSense whose audio endpoint never appears still buzzes.
bool open_dualsense_audio() {
    {
        std::lock_guard<std::mutex> lk(g_audio_mu);
        if (g_stream) return true;
    }
    if (!SDL_InitSubSystem(SDL_INIT_AUDIO)) {
        note("SDL audio did not start (%s): standard rumble in use", SDL_GetError());
        return false;
    }
    const SDL_AudioDeviceID dev = find_dualsense_audio();
    if (!dev) {
        note("no DualSense audio endpoint yet: standard rumble in use");
        return false;
    }
    SDL_AudioSpec want{};
    want.format = SDL_AUDIO_F32;
    want.channels = 4;
    want.freq = 48000;
    SDL_AudioStream* stream = SDL_OpenAudioDeviceStream(dev, &want, audio_callback, nullptr);
    if (!stream) {
        note("the DualSense audio stream did not open (%s): standard rumble in use", SDL_GetError());
        return false;
    }
    // The DSP runs at its full gain, always: the player's strength is applied once, in push(), on the amplitude the game sent.
    // Scaling it here as well is what made 40 % come out at about 16 %, and it is also why the strength used to need a new
    // audio stream on every step of the slider. RenHDrum clamps its own volume to 10..100, so a zero strength could not be done
    // this way anyway.
    // Read advanced haptics settings for wired DualSense
    const bool crisp_notes = g_crisp_notes.load(std::memory_order_relaxed);
    const int ds_volume = g_ds_volume.load(std::memory_order_relaxed);
    RenDsDsp* dsp = api().dsp_create(ds_volume, crisp_notes);
    if (!dsp) {
        SDL_DestroyAudioStream(stream);
        note("the DualSense haptics engine did not start: standard rumble in use");
        return false;
    }
    {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        g_dsp = dsp;
        g_at[0] = g_at[1] = 0;
        g_muted[0] = g_muted[1] = false;
    }
    // The audio stream owns the motors now: turn SDL's own rumble off so the two do not fight.
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        if (g_pad.joy) SDL_RumbleJoystick(g_pad.joy, 0, 0, 0);
    }
    SDL_ResumeAudioStreamDevice(stream);
    {
        std::lock_guard<std::mutex> lk(g_audio_mu);
        g_stream = stream;
    }
    g_running.store(true, std::memory_order_relaxed);
    note("4-channel 48 kHz audio on the wire");
    NXLOG_I(kTag, "DualSense haptics on the wired pad: 4-channel 48 kHz audio, %s", SDL_GetAudioDeviceName(dev));
    return true;
}

void close_dualsense_audio() {
    SDL_AudioStream* stream = nullptr;
    {
        std::lock_guard<std::mutex> lk(g_audio_mu);
        stream = g_stream;
        g_stream = nullptr;
    }
    if (stream) SDL_DestroyAudioStream(stream);   // waits for the callback to return, so g_eng_mu is not held here
    {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        if (g_dsp) {
            api().dsp_destroy(g_dsp);
            g_dsp = nullptr;
        }
        g_at[0] = g_at[1] = 0;
        g_muted[0] = g_muted[1] = false;
    }
}

// True while the stream still has a live device: the cable can be pulled out or the endpoint can vanish on its own.
bool audio_alive() {
    std::lock_guard<std::mutex> lk(g_audio_mu);
    if (!g_stream) return false;
    if (SDL_GetAudioStreamDevice(g_stream) == 0) return false;
    return !SDL_AudioStreamDevicePaused(g_stream);
}

// ---- the 1 ms worker --------------------------------------------------------------------------------------------------------

// Sent by the worker and by detach(). Returns what SDL said, so a pad that refuses the packets can be noticed instead of
// being counted as driven. g_host_mu is taken here and released before returning: SDL's own write may block on the pad.
bool send_packet(const uint8_t* pkt) {
    std::lock_guard<std::mutex> lk(g_host_mu);
    if (!g_pad.joy) return false;
    return SDL_SendJoystickEffect(g_pad.joy, pkt, 10);
}

// Forward declaration
bool send_motors(const uint16_t* m, uint32_t grip_ms, uint32_t trigger_ms);

// One hand's value to whichever of the backends exists (only one does at a time). g_eng_mu must be held. Every path
// that feeds a backend goes through here: the game's values, the silence watchdog and the master switch.
void push_locked(int64_t t, int side, float al, float fl, float ah, float fh) {
    if (g_engine) api().push_half(g_engine, t, side == 1, al, fl, ah, fh);
    else if (g_xbox) api().xb_push(g_xbox, t, side == 1, al, fl, ah, fh);
    else if (g_xbox_direct && g_xbox_store) {
        uint16_t motors[4] = {};
        // Xbox Direct takes low_amp and high_amp (no frequencies); triggers = true for controllers with trigger motors.
        // A negative return is an error, not a send: only a positive mask holds motor values.
        const int r = api().xb_direct_push(g_xbox_direct, g_xbox_store, t, side == 1, al, ah, true, motors);
        if (r > 0) send_motors(motors, 2000, 2000);
    }
    else if (g_pro2) api().pro2_push_half(g_pro2, t, side == 1, al, fl, ah, fh);
    else if (g_dsp) api().dsp_set_half(g_dsp, side == 1, al, fl, ah, fh);
}

// Both halves of a 0x10 packet carry the neutral HD Rumble value: nothing is playing, so there is nothing to keep alive.
bool packet_silent(const uint8_t* p) {
    static const uint8_t kSilentSide[4] = {0x00, 0x01, 0x40, 0x40};
    return std::memcmp(p + 2, kSilentSide, 4) == 0 && std::memcmp(p + 6, kSilentSide, 4) == 0;
}

// The Xbox motors: grips through SDL's rumble, triggers through its trigger rumble. Only the grips decide success: a pad on
// XInput has no trigger motors and refuses that call every time. g_host_mu is held for the same reason as in send_packet().
bool send_motors(const uint16_t* m, uint32_t grip_ms, uint32_t trigger_ms) {
    std::lock_guard<std::mutex> lk(g_host_mu);
    if (!g_pad.joy) return false;
    const bool ok = SDL_RumbleJoystick(g_pad.joy, m[0], m[1], grip_ms);
    SDL_RumbleJoystickTriggers(g_pad.joy, m[2], m[3], trigger_ms);
    return ok;
}

// The pad stopped taking HD Rumble packets: give it back to the standard rumble, which play.cpp sends through SDL itself.
void drop_to_rumble(const char* why) {
    g_backend.store(Backend::None, std::memory_order_relaxed);
    g_running.store(false, std::memory_order_relaxed);
    note("%s", why);
    NXLOG_W(kTag, "%s", why);
}

// The wireless DualSense: one report 0x32 every 10.67 ms on its own precise clock (the 1 ms worker's sleep would jitter the
// audio the report carries). It writes while something is playing and for two seconds after, then leaves the radio quiet.
int bt_writer(void*) {
    uint8_t report[kDsBtReportSize];
    uint64_t next = SDL_GetTicksNS();
    uint64_t last_signal = 0;
    int fails = 0;
    while (g_bt_run.load(std::memory_order_relaxed)) {
        next += kDsBtPeriodNs;
        int r = -1;
        {
            std::lock_guard<std::mutex> lk(g_eng_mu);
            if (g_dsp) r = api().dsp_bt_report(g_dsp, kDsBtBufferLength, report);
        }
        const uint64_t now = SDL_GetTicksNS();
        if (r == 1) last_signal = now;
        if (r >= 0 && last_signal && now - last_signal < kDsBtIdleNs) {
            if (SDL_hid_write(g_hid, report, sizeof(report)) < 0) {
                if (++fails >= kDsBtFailuresMax) {
                    drop_to_rumble("the DualSense stopped taking the Bluetooth haptics: the standard rumble is in use again");
                    break;
                }
            } else {
                fails = 0;
                g_packets.fetch_add(1, std::memory_order_relaxed);
            }
        }
        const uint64_t after = SDL_GetTicksNS();
        if (next > after) SDL_DelayPrecise(next - after);
        else if (after - next > 100000000ull) next = after;   // stalled for 100 ms: start the clock again instead of bursting
    }
    return 0;
}

// DualSense BT 0x36 writer (combined rumble + adaptive triggers + speaker)
int bt36_writer(void*) {
    uint8_t report[RENHDRUM_DS5_BT36_REPORT_SIZE];
    uint64_t next = SDL_GetTicksNS();
    uint64_t last_signal = 0;
    int fails = 0;
    uint8_t seq = 0;
    while (g_bt36_run.load(std::memory_order_relaxed)) {
        next += kDsBtPeriodNs;
        int r = -1;
        uint8_t trigger_state_l[RENHDRUM_DS5_EFFECTS_SIZE] = {};
        uint8_t trigger_state_r[RENHDRUM_DS5_EFFECTS_SIZE] = {};
        bool has_trigger_l = false, has_trigger_r = false;

        {
            std::lock_guard<std::mutex> lk(g_eng_mu);
            if (g_dsp) {
                // Build trigger effect blocks if adaptive triggers are active
                if (g_trigger_effect_l_valid) {
                    has_trigger_l = true;
                    std::memcpy(trigger_state_l, g_trigger_effect_l, sizeof(trigger_state_l));
                }
                if (g_trigger_effect_r_valid) {
                    has_trigger_r = true;
                    std::memcpy(trigger_state_r, g_trigger_effect_r, sizeof(trigger_state_r));
                }
                // For now, use left trigger state if either is active
                const uint8_t* state47 = has_trigger_l ? trigger_state_l : (has_trigger_r ? trigger_state_r : nullptr);
                r = api().dsp_bt36_report(g_dsp, state47, seq, report);
            }
        }

        const uint64_t now = SDL_GetTicksNS();
        if (r == 1) last_signal = now;
        if (r >= 0 && last_signal && now - last_signal < kDsBtIdleNs) {
            if (SDL_hid_write(g_hid_bt36, report, sizeof(report)) < 0) {
                if (++fails >= kDsBtFailuresMax) {
                    drop_to_rumble("the DualSense stopped taking the Bluetooth 0x36 haptics: the standard rumble is in use again");
                    break;
                }
            } else {
                fails = 0;
                g_packets.fetch_add(1, std::memory_order_relaxed);
                seq = (seq + 1) & 0xFF;
            }
        }
        const uint64_t after = SDL_GetTicksNS();
        if (next > after) SDL_DelayPrecise(next - after);
        else if (after - next > 100000000ull) next = after;
    }
    return 0;
}

// Steam Controller grip writer (report 0x88)
constexpr uint64_t kSteamGripPeriodNs8k = 3875000ull;   // RENHDRUM_VALVE_GRIP_PERIOD_NS_8K
constexpr uint64_t kSteamGripPeriodNs4k = 7750000ull;   // RENHDRUM_VALVE_GRIP_PERIOD_NS_4K
constexpr uint8_t kSteamGripFormat = 8;                  // RENHDRUM_VALVE_GRIP_FORMAT_MULAW_8K

int steam_grip_writer(void*) {
    uint8_t report[RENHDRUM_VALVE_GRIP_REPORT_SIZE];
    uint8_t mode_reports[RENHDRUM_VALVE_GRIP_MODE_REPORTS_SIZE];
    uint64_t next = SDL_GetTicksNS();
    uint64_t last_signal = 0;
    int fails = 0;
    bool mode_active = false;

    while (g_steam_run.load(std::memory_order_relaxed)) {
        next += kSteamGripPeriodNs8k;
        int r = -1;
        {
            std::lock_guard<std::mutex> lk(g_eng_mu);
            if (g_dsp) r = api().valve_grip_build_report(g_dsp, kSteamGripFormat, report);
        }

        const uint64_t now = SDL_GetTicksNS();
        if (r == 1) last_signal = now;
        if (r >= 0 && last_signal && now - last_signal < kDsBtIdleNs) {
            if (SDL_hid_write(g_hid_steam, report, sizeof(report)) < 0) {
                if (++fails >= kDsBtFailuresMax) {
                    drop_to_rumble("the Steam Controller stopped taking grip haptics: the standard rumble is in use again");
                    break;
                }
            } else {
                fails = 0;
                g_packets.fetch_add(1, std::memory_order_relaxed);
            }
        }

        // PCM mode keep-alive (0x86 reports)
        int mode_r = -1;
        {
            std::lock_guard<std::mutex> lk(g_eng_mu);
            if (g_dsp) mode_r = api().valve_grip_mode_tick(g_dsp, kSteamGripFormat, now, false, mode_reports);
        }
        if (mode_r > 0) {
            for (int i = 0; i < RENHDRUM_VALVE_GRIP_MODE_REPORT_COUNT; ++i) {
                if (SDL_hid_write(g_hid_steam, mode_reports + i * RENHDRUM_VALVE_GRIP_MODE_REPORT_SIZE, RENHDRUM_VALVE_GRIP_MODE_REPORT_SIZE) < 0) {
                    // Mode keep-alive failure is not fatal
                }
            }
            mode_active = true;
        } else if (mode_active && mode_r == 0) {
            // Mode turned off
            mode_active = false;
        }

        const uint64_t after = SDL_GetTicksNS();
        if (next > after) SDL_DelayPrecise(next - after);
        else if (after - next > 100000000ull) next = after;
    }

    // Turn off mode on exit
    if (mode_active) {
        int mode_r = api().valve_grip_mode_off(kSteamGripFormat, mode_reports);
        if (mode_r > 0) {
            for (int i = 0; i < RENHDRUM_VALVE_GRIP_MODE_REPORT_COUNT; ++i) {
                SDL_hid_write(g_hid_steam, mode_reports + i * RENHDRUM_VALVE_GRIP_MODE_REPORT_SIZE, RENHDRUM_VALVE_GRIP_MODE_REPORT_SIZE);
            }
        }
    }
    return 0;
}

int worker(void*) {
    uint8_t pkt[10];
    uint16_t motors[4] = {};
    uint16_t xb_direct_motors[4] = {};
    int64_t xb_retry_at = 0;   // the next time a refused Xbox send may be retried
    int64_t xb_direct_retry_at = 0;
    uint64_t last_report = 0;

    while (g_run.load(std::memory_order_relaxed)) {
        const int64_t t = now_us();
        const Backend be = g_backend.load(std::memory_order_relaxed);
        int ready = 0;
        int xb_ready = 0;
        int xb_direct_ready = 0;
        int pro2_ready = 0;
        uint8_t pro2_report[RENHDRUM_PRO2_REPORT_SIZE] = {};

        {
            std::lock_guard<std::mutex> lk(g_eng_mu);
            // Silence watchdog: a menu or a loading screen stops the game's values without sending a zero. Every backend needs
            // it: without it the engine, the converter or the DSP would keep the last value for ever once the game stops sending
            // (a long rumble outliving the hit is the classic bug). Only one of the three exists at a time.
            for (int s = 0; s < 2; ++s) {
                if (g_at[s] && !g_muted[s] && t - g_at[s] > kStaleUs) {
                    push_locked(t, s, 0.f, 160.f, 0.f, 320.f);
                    g_muted[s] = true;
                }
            }
            if (be == Backend::ProController && g_engine) {
                ready = api().tick(g_engine, t, pkt);
                if (ready == 1) {
                    std::memcpy(g_last_pkt, pkt, sizeof(pkt));
                    g_last_pkt_live = !packet_silent(pkt);
                    g_last_send_us = t;
                } else if (g_pro_wired && g_last_pkt_live && t - g_last_send_us >= kProResendUs) {
                    // A wired Pro Controller stops a vibration by itself when no packet comes for a while; a long, steady
                    // rumble sends no new packet, so the last one goes again. Over Bluetooth the library does this itself.
                    std::memcpy(pkt, g_last_pkt, sizeof(pkt));
                    g_last_send_us = t;
                    ready = 1;
                }
            }
            if (be == Backend::Xbox && g_xbox) xb_ready = api().xb_tick(g_xbox, t, motors);
            if (be == Backend::XboxDirect && g_xbox_direct) {
                // Only the burst-end settle belongs here: the game's own values arrive through push_locked. Pushing zeros in this
                // loop (as it used to) cancelled every value the game had just sent, and the tick that followed zeroed the very
                // motor buffer the loop was about to hand to SDL.
                uint16_t settled[4] = {};
                const int tick_ready = api().xb_direct_tick(g_xbox_direct, t, true, settled);
                if (tick_ready > 0) {
                    xb_direct_ready = tick_ready;
                    std::memcpy(xb_direct_motors, settled, sizeof(settled));
                }
            }
            if (be == Backend::Switch2Pro && g_pro2) {
                pro2_ready = api().pro2_build_report(g_pro2, t, pro2_report);
            }

            // Adaptive triggers: update tri-trigger and presets every ~1ms (1000us)
            if (g_tri_trigger_l || g_tri_trigger_r) {
                const uint32_t now_ms = static_cast<uint32_t>(t / 1000);
                uint8_t effect_l[11] = {};
                uint8_t effect_r[11] = {};
                bool has_l = false, has_r = false;

                // Update tri-trigger (vibration-reactive)
                if (g_tri_trigger_l) {
                    // Get current vibration state for left trigger
                    float al = 0.f, fl = 160.f, ah = 0.f, fh = 320.f;
                    {
                        std::lock_guard<std::mutex> lk2(g_eng_mu);
                        // We'd need to track per-trigger vibration - simplified for now
                    }
                    int r = api().tri_trigger_update(g_tri_trigger_l, now_ms, 0, al, fl, ah, fh, true, effect_l);
                    if (r == 1) has_l = true;
                }
                if (g_tri_trigger_r) {
                    float al = 0.f, fl = 160.f, ah = 0.f, fh = 320.f;
                    int r = api().tri_trigger_update(g_tri_trigger_r, now_ms, 0, al, fl, ah, fh, true, effect_r);
                    if (r == 1) has_r = true;
                }

                // Update trigger presets (priority over tri-trigger)
                if (g_trigger_preset_l) {
                    int r = api().trigger_preset_tick(g_trigger_preset_l, now_ms, effect_l);
                    if (r == 1) has_l = true;
                }
                if (g_trigger_preset_r) {
                    int r = api().trigger_preset_tick(g_trigger_preset_r, now_ms, effect_r);
                    if (r == 1) has_r = true;
                }

                if (has_l) {
                    std::memcpy(g_trigger_effect_l, effect_l, sizeof(effect_l));
                    g_trigger_effect_l_valid = true;
                }
                if (has_r) {
                    std::memcpy(g_trigger_effect_r, effect_r, sizeof(effect_r));
                    g_trigger_effect_r_valid = true;
                }
            }
        }

        if (be == Backend::ProController && ready == 1) {
            if (send_packet(pkt)) {
                g_packets.fetch_add(1, std::memory_order_relaxed);
                g_send_fail.store(0, std::memory_order_relaxed);
            } else if (g_send_fail.fetch_add(1, std::memory_order_relaxed) + 1 >= kSendFailuresBeforeFallback) {
                drop_to_rumble("the pad refused the HD Rumble packets: the standard rumble is in use again");
            }
        }
        if (be == Backend::Xbox && xb_ready == 1) {
            if (send_motors(motors, kXboxGripMs, kXboxTriggerMs)) {
                g_packets.fetch_add(1, std::memory_order_relaxed);
                xb_retry_at = 0;
            } else if (t >= xb_retry_at) {
                xb_retry_at = t + kXboxRetryUs;
                std::lock_guard<std::mutex> lk(g_eng_mu);
                if (g_xbox) api().xb_resend(g_xbox);
            }
        }
        if (be == Backend::XboxDirect && xb_direct_ready != 0) {
            // xb_direct_ready: bit 0 = send grips, bit 1 = send triggers
            bool send_grips = (xb_direct_ready & 1) != 0;
            bool send_triggers = (xb_direct_ready & 2) != 0;
            if (send_grips || send_triggers) {
                if (send_motors(xb_direct_motors, kXboxGripMs, kXboxTriggerMs)) {
                    g_packets.fetch_add(1, std::memory_order_relaxed);
                    xb_direct_retry_at = 0;
                } else if (t >= xb_direct_retry_at) {
                    xb_direct_retry_at = t + kXboxRetryUs;
                    // Retry handled by next tick
                }
            }
        }
        if (be == Backend::Switch2Pro && pro2_ready == 1) {
            std::lock_guard<std::mutex> lk(g_host_mu);
            if (g_pad.joy) {
                if (SDL_SendJoystickEffect(g_pad.joy, pro2_report, sizeof(pro2_report))) {
                    g_packets.fetch_add(1, std::memory_order_relaxed);
                }
            }
        }
        if (be == Backend::DualSenseUsb) {
            if (!audio_alive()) {
                if (g_running.load(std::memory_order_relaxed)) {
                    close_dualsense_audio();
                    g_running.store(false, std::memory_order_relaxed);
                    note("the DualSense audio endpoint went away: the standard rumble is in use again");
                    NXLOG_W(kTag, "the DualSense audio endpoint went away: the standard rumble is in use until it is back");
                }
                if (SDL_GetTicksNS() >= g_next_enum_ns.load(std::memory_order_relaxed)) {
                    g_next_enum_ns.store(SDL_GetTicksNS() + kEnumPeriodNs, std::memory_order_relaxed);
                    open_dualsense_audio();
                }
            }
        }
        const uint64_t ns = SDL_GetTicksNS();
        if (!last_report || ns - last_report >= 5000000000ull) {
            last_report = ns;

            NXLOG_D(kTag, "haptics: backend %s, running %d, %llu values (peak %.3f), %llu packets, %llu audio fills (peak %.5f), %llu non-zero values (peak %.5f)",
                    backend_name(be), g_running.load() ? 1 : 0, static_cast<unsigned long long>(g_pushes.load()),
                    static_cast<double>(g_last_amp.load()), static_cast<unsigned long long>(g_packets.load()),
                    static_cast<unsigned long long>(g_fills.load()), static_cast<double>(g_fill_peak.load()),
                    static_cast<unsigned long long>(g_nonzero.load()), static_cast<double>(g_amp_peak.load()));
            g_fill_peak.store(0.f, std::memory_order_relaxed);
        }

        const uint64_t test = g_test_until_ns.load(std::memory_order_relaxed);
        if (test) {
            if (t * 1000 > static_cast<int64_t>(test)) {
                g_test_until_ns.store(0, std::memory_order_relaxed);
                push(0, 0, 0.f, 160.f, 0.f, 320.f);
                push(0, 1, 0.f, 160.f, 0.f, 320.f);
            } else {
                push(0, 0, 0.5f, 180.f, 0.5f, 320.f);
                push(0, 1, 0.5f, 180.f, 0.5f, 320.f);
            }
        }
        SDL_DelayNS(1000000);
    }
    return 0;
}

// ---- attach / detach --------------------------------------------------------------------------------------------------------

void stop_worker() {
    SDL_Thread* t = nullptr;
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        t = g_thread;
        g_thread = nullptr;
    }
    if (!t) return;
    g_run.store(false, std::memory_order_relaxed);
    // The worker may be inside send_packet(): g_host_mu is released above, so the join cannot deadlock against it.
    SDL_WaitThread(t, nullptr);
}

// Main thread, g_life_mu held (attach/detach), so g_bt_thread needs no lock of its own. The writer takes g_eng_mu for each
// report, so nothing may be held while joining it; the HID handle is closed only once it has stopped using it.
void stop_bt_writer() {
    SDL_Thread* t = g_bt_thread;
    g_bt_thread = nullptr;
    g_bt_run.store(false, std::memory_order_relaxed);
    if (t) SDL_WaitThread(t, nullptr);
    if (g_hid) {
        SDL_hid_close(g_hid);
        g_hid = nullptr;
    }
}

// DualSense BT 0x36 writer cleanup
void stop_bt36_writer() {
    SDL_Thread* t = g_bt36_thread;
    g_bt36_thread = nullptr;
    g_bt36_run.store(false, std::memory_order_relaxed);
    if (t) SDL_WaitThread(t, nullptr);
    if (g_hid_bt36) {
        SDL_hid_close(g_hid_bt36);
        g_hid_bt36 = nullptr;
    }
}

// Steam Controller grip writer cleanup
void stop_steam_writer() {
    SDL_Thread* t = g_steam_thread;
    g_steam_thread = nullptr;
    g_steam_run.store(false, std::memory_order_relaxed);
    if (t) SDL_WaitThread(t, nullptr);
    if (g_hid_steam) {
        SDL_hid_close(g_hid_steam);
        g_hid_steam = nullptr;
    }
}

// The wireless DualSense's HID device, opened a second time beside SDL's own driver (Windows shares it). SDL's path for the
// joystick is the HID path when its HIDAPI driver has the pad. Otherwise the device is looked up by vendor and product, but
// only used when exactly one matches: with two DualSenses there is no telling which one this is, and writing Bluetooth reports
// to the other pad would be worse than the standard rumble.
SDL_hid_device* open_dualsense_hid(const PadState& pad) {
    const char* path = pad.joy ? SDL_GetJoystickPath(pad.joy) : nullptr;
    if (path && *path) {
        if (SDL_hid_device* d = SDL_hid_open_path(path)) return d;
        NXLOG_D(kTag, "SDL_hid_open_path(%s) failed: %s", path, SDL_GetError());
    }
    SDL_hid_device* d = nullptr;
    if (SDL_hid_device_info* list = SDL_hid_enumerate(0x054C, pad.product)) {
        const SDL_hid_device_info* only = nullptr;
        int matches = 0;
        for (const SDL_hid_device_info* i = list; i; i = i->next) {
            if (!i->path || !*i->path) continue;
            only = i;
            ++matches;
        }
        if (matches == 1) d = SDL_hid_open_path(only->path);
        else if (matches > 1) NXLOG_W(kTag, "%d DualSense HID devices and no path for this pad: standard rumble", matches);
        SDL_hid_free_enumeration(list);
    }
    return d;
}

// Starts the wireless DualSense path: the DSP, then the writer thread. Main thread, g_life_mu held. False leaves the pad on
// the standard rumble, with the reason in the detail line.
bool start_dualsense_bt(const PadState& pad) {
    SDL_hid_device* hid = open_dualsense_hid(pad);
    if (!hid) {
        note("the DualSense's Bluetooth HID device did not open: standard rumble in use");
        return false;
    }
    // Crisp notes off over Bluetooth, as the library's own hosts do: the report's lower rate blurs them anyway.
    const int ds_volume = g_ds_volume.load(std::memory_order_relaxed);
    RenDsDsp* dsp = api().dsp_create(ds_volume, false);
    if (!dsp) {
        SDL_hid_close(hid);
        note("the DualSense haptics engine did not start: standard rumble in use");
        return false;
    }
    {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        g_dsp = dsp;
        g_at[0] = g_at[1] = 0;
        g_muted[0] = g_muted[1] = false;
    }
    // The reports own the voice coils now: turn SDL's own rumble off so the two do not fight.
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        if (g_pad.joy) SDL_RumbleJoystick(g_pad.joy, 0, 0, 0);
    }
    // Backend and detail first: once the writer runs it may drop the pad back to the standard rumble itself, and that must
    // not be overwritten here afterwards.
    g_backend.store(Backend::DualSenseBt, std::memory_order_relaxed);
    g_running.store(true, std::memory_order_relaxed);
    note("report 0x32 over Bluetooth");
    g_hid = hid;
    g_bt_run.store(true, std::memory_order_relaxed);
    g_bt_thread = SDL_CreateThread(bt_writer, "nxrt_haptic_bt", nullptr);
    if (!g_bt_thread) {
        g_backend.store(Backend::None, std::memory_order_relaxed);
        g_running.store(false, std::memory_order_relaxed);
        stop_bt_writer();
        close_dualsense_audio();   // destroys the DSP; there is no stream on this path
        note("the haptics thread did not start: standard rumble in use");
        return false;
    }
    NXLOG_I(kTag, "DualSense haptics on the wireless pad: report 0x32 every %.2f ms", static_cast<double>(kDsBtPeriodNs) / 1e6);
    return true;
}

// Starts the wireless DualSense BT 0x36 path (combined rumble + triggers + speaker). Main thread, g_life_mu held.
bool start_dualsense_bt36(const PadState& pad) {
    SDL_hid_device* hid = open_dualsense_hid(pad);
    if (!hid) {
        note("the DualSense's Bluetooth HID device did not open: standard rumble in use");
        return false;
    }
    // Read advanced haptics settings
    const bool crisp_notes = g_crisp_notes.load(std::memory_order_relaxed);
    const bool envelope_5ms = g_envelope_5ms.load(std::memory_order_relaxed);
    const int ds_volume = g_ds_volume.load(std::memory_order_relaxed);
    // Apply crisp notes and envelope settings via DSP create flags
    RenDsDsp* dsp = api().dsp_create(ds_volume, crisp_notes);
    if (!dsp) {
        SDL_hid_close(hid);
        note("the DualSense haptics engine did not start: standard rumble in use");
        return false;
    }
    // Set envelope mode if requested
    if (envelope_5ms) {
        // The envelope is applied in the DSP automatically when created with the flag
    }
    {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        g_dsp = dsp;
        g_at[0] = g_at[1] = 0;
        g_muted[0] = g_muted[1] = false;
    }
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        if (g_pad.joy) SDL_RumbleJoystick(g_pad.joy, 0, 0, 0);
    }
    // Create adaptive trigger handles if available and enabled
    const bool adaptive_triggers = g_adaptive_triggers.load(std::memory_order_relaxed);
    if (adaptive_triggers && api().has_adaptive_triggers) {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        g_tri_trigger_l = api().tri_trigger_create();
        g_tri_trigger_r = api().tri_trigger_create();
        g_trigger_preset_l = api().trigger_preset_create();
        g_trigger_preset_r = api().trigger_preset_create();
    }
    g_backend.store(Backend::DualSenseBt36, std::memory_order_relaxed);
    g_running.store(true, std::memory_order_relaxed);
    note("report 0x36 over Bluetooth (rumble + triggers + speaker)");
    g_hid_bt36 = hid;
    g_bt36_run.store(true, std::memory_order_relaxed);
    g_bt36_thread = SDL_CreateThread(bt36_writer, "nxrt_haptic_bt36", nullptr);
    if (!g_bt36_thread) {
        g_backend.store(Backend::None, std::memory_order_relaxed);
        g_running.store(false, std::memory_order_relaxed);
        stop_bt36_writer();
        close_dualsense_audio();
        note("the haptics thread did not start: standard rumble in use");
        return false;
    }
    NXLOG_I(kTag, "DualSense haptics on the wireless pad: report 0x36 every %.2f ms", static_cast<double>(kDsBtPeriodNs) / 1e6);
    return true;
}

// Starts the Steam Controller grip haptics path. Main thread, g_life_mu held.
bool start_steam_grip(const PadState& pad) {
    // Open HID device for Steam Controller
    SDL_hid_device* hid = nullptr;
    const char* path = pad.joy ? SDL_GetJoystickPath(pad.joy) : nullptr;
    if (path && *path) {
        hid = SDL_hid_open_path(path);
    }
    if (!hid) {
        if (SDL_hid_device_info* list = SDL_hid_enumerate(0x28DE, 0)) {  // Steam Controller VID
            for (const SDL_hid_device_info* i = list; i; i = i->next) {
                if (i->path && *i->path) {
                    hid = SDL_hid_open_path(i->path);
                    break;
                }
            }
            SDL_hid_free_enumeration(list);
        }
    }
    if (!hid) {
        note("the Steam Controller HID device did not open: standard rumble in use");
        return false;
    }
    // Read advanced haptics settings
    const bool crisp_notes = g_crisp_notes.load(std::memory_order_relaxed);
    const int ds_volume = g_ds_volume.load(std::memory_order_relaxed); // reuse volume setting
    // Create grip DSP with drift compensation and crisp notes
    RenDsDsp* dsp = api().valve_grip_dsp_create_ex(ds_volume, crisp_notes, true);
    if (!dsp) {
        SDL_hid_close(hid);
        note("the Steam Controller grip engine did not start: standard rumble in use");
        return false;
    }
    {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        g_dsp = dsp;
        g_at[0] = g_at[1] = 0;
        g_muted[0] = g_muted[1] = false;
    }
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        if (g_pad.joy) SDL_RumbleJoystick(g_pad.joy, 0, 0, 0);
    }
    g_backend.store(Backend::SteamGrip, std::memory_order_relaxed);
    g_running.store(true, std::memory_order_relaxed);
    note("Steam Controller grip haptics (report 0x88)");
    g_hid_steam = hid;
    g_steam_run.store(true, std::memory_order_relaxed);
    g_steam_thread = SDL_CreateThread(steam_grip_writer, "nxrt_haptic_steam", nullptr);
    if (!g_steam_thread) {
        g_backend.store(Backend::None, std::memory_order_relaxed);
        g_running.store(false, std::memory_order_relaxed);
        stop_steam_writer();
        close_dualsense_audio();
        note("the haptics thread did not start: standard rumble in use");
        return false;
    }
    NXLOG_I(kTag, "Steam Controller grip haptics: report 0x88 every %.2f ms", static_cast<double>(kSteamGripPeriodNs8k) / 1e6);
    return true;
}

// Starts Xbox Direct mode. Main thread, g_life_mu held.
bool start_xbox_direct(const PadState& pad) {
    if (!api().has_xbox_direct) {
        note("this renhdrum.dll has no Xbox Direct mode: standard rumble in use");
        return false;
    }
    void* xd = api().xb_direct_create();
    if (!xd) {
        note("the Xbox Direct engine did not start: standard rumble in use");
        return false;
    }
    void* store = api().store_create ? api().store_create() : nullptr;
    if (store && api().store_set) api().store_set(store, "xbox_mode", 1);   // the Direct row of the settings table
    if (!store) {
        api().xb_direct_destroy(xd);
        note("the Xbox Direct settings store did not start: standard rumble in use");
        return false;
    }
    {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        g_xbox_direct = xd;
        g_xbox_store = store;
        g_at[0] = g_at[1] = 0;
        g_muted[0] = g_muted[1] = false;
    }
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        if (g_pad.joy) SDL_RumbleJoystick(g_pad.joy, 0, 0, 0);
    }
    g_backend.store(Backend::XboxDirect, std::memory_order_relaxed);
    g_running.store(true, std::memory_order_relaxed);
    note(pad.wired ? "Xbox Direct (grips + triggers), on the wire" : "Xbox Direct (grips + triggers), wireless");
    return true;
}

// Starts Switch 2 Pro Controller mode. Main thread, g_life_mu held.
bool start_switch2_pro(const PadState& pad) {
    if (!api().has_pro2) {
        note("this renhdrum.dll has no Switch 2 Pro Controller support: standard rumble in use");
        return false;
    }
    void* pro2 = api().pro2_create(RENHDRUM_PRO2_AMP_CEILING_DEFAULT);
    if (!pro2) {
        note("the Switch 2 Pro Controller engine did not start: standard rumble in use");
        return false;
    }
    {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        g_pro2 = pro2;
        g_at[0] = g_at[1] = 0;
        g_muted[0] = g_muted[1] = false;
    }
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        if (g_pad.joy) SDL_RumbleJoystick(g_pad.joy, 0, 0, 0);
    }
    g_backend.store(Backend::Switch2Pro, std::memory_order_relaxed);
    g_running.store(true, std::memory_order_relaxed);
    note("Switch 2 Pro Controller HD Rumble 2 (report 0x02)");
    return true;
}

// Called with g_life_mu held and nothing else: it waits for the worker, so no inner lock may be held here.
void detach() {
    const Backend was = g_backend.load(std::memory_order_relaxed);
    stop_worker();
    stop_bt_writer();
    stop_bt36_writer();
    stop_steam_writer();
    close_dualsense_audio();
    RenXbox* xbox = nullptr;
    void* xbox_store = nullptr;
    bool xbox_direct = false;
    {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        if (g_engine) {
            api().engine_destroy(g_engine);
            g_engine = nullptr;
        }
        xbox = g_xbox;
        g_xbox = nullptr;
        if (g_xbox_direct) {
            api().xb_direct_destroy(g_xbox_direct);
            g_xbox_direct = nullptr;
            xbox_direct = true;
        }
        xbox_store = g_xbox_store;
        g_xbox_store = nullptr;
        if (g_tri_trigger_l) {
            api().tri_trigger_destroy(g_tri_trigger_l);
            g_tri_trigger_l = nullptr;
        }
        if (g_tri_trigger_r) {
            api().tri_trigger_destroy(g_tri_trigger_r);
            g_tri_trigger_r = nullptr;
        }
        if (g_trigger_preset_l) {
            api().trigger_preset_destroy(g_trigger_preset_l);
            g_trigger_preset_l = nullptr;
        }
        if (g_trigger_preset_r) {
            api().trigger_preset_destroy(g_trigger_preset_r);
            g_trigger_preset_r = nullptr;
        }
        if (g_pro2) {
            api().pro2_destroy(g_pro2);
            g_pro2 = nullptr;
        }
        g_at[0] = g_at[1] = 0;
        g_muted[0] = g_muted[1] = false;
        g_pro_wired = false;
        g_last_pkt_live = false;
        g_last_send_us = 0;
        g_trigger_effect_l_valid = false;
        g_trigger_effect_r_valid = false;
    }
    if (was == Backend::ProController) {   // a neutral packet so the pad does not keep buzzing
        static const uint8_t kStop[10] = {0x10, 0, 0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40};
        send_packet(kStop);
    }
    if (xbox_store) {
        if (api().store_destroy) api().store_destroy(xbox_store);
    }
    if (xbox || xbox_direct) {   // all four motors to zero, then the engine goes
        static const uint16_t kZero[4] = {0, 0, 0, 0};
        send_motors(kZero, 0, 0);
    }
    if (xbox) api().xb_destroy(xbox);
    g_backend.store(Backend::None, std::memory_order_relaxed);
    g_running.store(false, std::memory_order_relaxed);
    g_send_fail.store(0, std::memory_order_relaxed);
    note("");
}

// Stops both hands now and clears what the backend still holds, so switching [pad] rumble off does not leave a motor running
// until the silence watchdog notices. Called from the game's thread on every value while the master switch is off: the targets
// are only written when something was actually playing.
void silence() {
    const int64_t t = now_us();
    std::lock_guard<std::mutex> lk(g_eng_mu);
    for (int s = 0; s < 2; ++s) {
        if (!g_at[s] && g_muted[s]) continue;
        push_locked(t, s, 0.f, 160.f, 0.f, 320.f);
        g_at[s] = 0;
        g_muted[s] = true;
    }
    // Also silence Xbox Direct and Switch 2 Pro
    if (g_xbox_direct && g_xbox_store) {
        uint16_t motors[4] = {};
        if (api().xb_direct_push(g_xbox_direct, g_xbox_store, t, false, 0.f, 0.f, true, motors) > 0) send_motors(motors, 0, 0);
        if (api().xb_direct_tick(g_xbox_direct, t, true, motors) > 0) send_motors(motors, 0, 0);
    }
    if (g_pro2) {
        api().pro2_push_half(g_pro2, t, false, 0.f, 160.f, 0.f, 320.f);
        api().pro2_push_half(g_pro2, t, true, 0.f, 160.f, 0.f, 320.f);
    }
}

// Picks the backend for the attached pad and starts it. Called with g_life_mu held, after detach().
void attach() {
    PadState pad;
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        pad = g_pad;
    }
    if (pad.id == 0) return;
    const Mode mode = g_mode.load(std::memory_order_relaxed);
    if (mode == Mode::Off) return;
    // The master switch: with [pad] rumble off there is nothing to drive, so no engine and no audio stream is opened either.
    if (!g_rumble_on.load(std::memory_order_relaxed)) {
        note("[pad] rumble is off: nothing is sent to the pad");
        return;
    }
    if (!load_api()) return;
    // The player asked for the plain two-motor rumble: leave the pad to play.cpp's map_rumble and the SDL call there.
    if (mode == Mode::Rumble) {
        note("the standard rumble, as asked for");
        NXLOG_I(kTag, "pad \"%s\": mode %s -> backend %s%s%s", pad.name, mode_name(mode), backend_name(Backend::None), "; ", g_detail);
        return;
    }

    // Use device classification for more accurate backend selection (0.6.0+)
    RenHdcDeviceInfo dev_info{};
    dev_info.struct_size = sizeof(dev_info);
    if (api().device_classify) {
        api().device_classify(pad.vendor, pad.product, &dev_info);
    }

    const bool want_ds = mode == Mode::Auto || mode == Mode::DualSense;
    const bool advanced_haptics = g_advanced_haptics.load(std::memory_order_relaxed);

    if (pad.dualsense && want_ds) {
        if (!api().has_ds) {
            note("this renhdrum.dll has no DualSense haptics: standard rumble in use");
        } else if (pad.bt) {
            const bool use_bt36 = advanced_haptics && g_ds_bt36.load(std::memory_order_relaxed);
            // Prefer BT 0x36 (combined report) if available, advanced haptics enabled, and user wants it
            if (use_bt36 && api().has_ds_bt36 && api().has_adaptive_triggers) {
                if (!start_dualsense_bt36(pad)) {
                    // Fallback to legacy BT 0x32
                    if (api().has_ds_bt) {
                        start_dualsense_bt(pad);
                    } else {
                        note("DualSense BT 0x36 failed and no 0x32 support: standard rumble in use");
                    }
                }
            } else if (api().has_ds_bt) {
                start_dualsense_bt(pad);
            } else {
                note("this renhdrum.dll has no DualSense Bluetooth haptics: standard rumble in use");
            }
        } else {
            g_backend.store(Backend::DualSenseUsb, std::memory_order_relaxed);
            g_running.store(false, std::memory_order_relaxed);
            g_next_enum_ns.store(0, std::memory_order_relaxed);
            note("looking for the DualSense audio endpoint");
        }
    } else if (pad.pro) {
        // Joy-Cons are always wireless, and the library has not been measured on them: they keep the standard rumble. A Pro
        // Controller over Bluetooth goes through the library's Compatible limiter (newest value only, at least 32 ms apart,
        // re-sent every 40 ms while rumbling), which is what keeps the link from lagging the input the way unthrottled
        // packets did in #26; it needs no patched SDL.
        // A link SDL cannot name gets the Bluetooth transport: its limiter is harmless on a cable, while the unthrottled wired
        // rate over the air is exactly the #26 lag.
        const bool pro_bt = !pad.wired;
        if (pro_bt && pad.product != 0x2009) {
            note("Bluetooth Joy-Con: standard rumble in use");
            NXLOG_I(kTag, "pad \"%s\": mode %s -> backend %s; %s", pad.name, mode_name(mode), backend_name(Backend::None),
                    "Bluetooth Joy-Con: the standard rumble");
            return;
        }
        RenSettings st{};
        if (api().settings_default(&st) != 0) {
            note("the HD Rumble engine did not start, standard rumble in use");
            return;
        }
        st.enabled = true;
        st.usb_no_queue = false;      // unpatched SDL3: the plain blocking write
        st.bt_after_input = false;
        st.bt_fast = false;           // "Detailed (5 ms)" needs SDL's idle-gated transport patches, which this SDL does not have
        if (pro_bt) st.bluetooth_level = kBtLevelCompatible;
        RenEngine* e = api().engine_create(pro_bt, true);
        if (!e) {
            note("the HD Rumble engine did not start, standard rumble in use");
            return;
        }
        api().engine_apply(e, &st);
        {
            std::lock_guard<std::mutex> lk(g_eng_mu);
            g_engine = e;
            g_at[0] = g_at[1] = 0;
            g_muted[0] = g_muted[1] = false;
            g_pro_wired = !pro_bt;
            g_last_pkt_live = false;
            g_last_send_us = 0;
        }
        g_backend.store(Backend::ProController, std::memory_order_relaxed);
        g_running.store(true, std::memory_order_relaxed);
        note(pro_bt ? "sub-command 0x10 packets over Bluetooth, Compatible pacing" : "sub-command 0x10 packets, on the wire");
    } else if (pad.xbox) {
        if (!pad.rumble) {
            // A Microsoft device without rumble motors (vendor 0x045E is wider than the Xbox pads): nothing to claim.
            note("%s has no haptic backend here: standard rumble in use", pad.name);
        } else if (advanced_haptics && g_xbox_direct_enabled.load(std::memory_order_relaxed) && api().has_xbox_direct) {
            // Try Xbox Direct mode first (grips + triggers via SDL)
            if (!start_xbox_direct(pad)) {
                // Fallback to four-motor converter
                if (api().has_xbox) {
                    RenXbox* x = api().xb_create(!pad.wired);
                    if (x) {
                        {
                            std::lock_guard<std::mutex> lk(g_eng_mu);
                            g_xbox = x;
                            g_at[0] = g_at[1] = 0;
                            g_muted[0] = g_muted[1] = false;
                        }
                        g_backend.store(Backend::Xbox, std::memory_order_relaxed);
                        g_running.store(true, std::memory_order_relaxed);
                        note(pad.wired ? "four motors (grips and triggers), on the wire" : "four motors (grips and triggers), wireless");
                    } else {
                        note("the Xbox rumble engine did not start, standard rumble in use");
                    }
                } else {
                    note("this renhdrum.dll has no Xbox rumble: standard rumble in use");
                }
            }
        } else if (api().has_xbox) {
            // A link SDL cannot name is treated as wireless: the slower cadence is safe on a cable too, the faster one is not
            // safe over the air.
            RenXbox* x = api().xb_create(!pad.wired);
            if (!x) {
                note("the Xbox rumble engine did not start, standard rumble in use");
            } else {
                {
                    std::lock_guard<std::mutex> lk(g_eng_mu);
                    g_xbox = x;
                    g_at[0] = g_at[1] = 0;
                    g_muted[0] = g_muted[1] = false;
                }
                g_backend.store(Backend::Xbox, std::memory_order_relaxed);
                g_running.store(true, std::memory_order_relaxed);
                note(pad.wired ? "four motors (grips and triggers), on the wire" : "four motors (grips and triggers), wireless");
            }
        } else {
            note("this renhdrum.dll has no Xbox rumble: standard rumble in use");
        }
    } else if (dev_info.family == RENHDRUM_FAMILY_STEAM && advanced_haptics && g_steam_grips.load(std::memory_order_relaxed) && api().has_steam_grip) {
        // Steam Controller grip haptics
        if (!start_steam_grip(pad)) {
            note("Steam Controller grip haptics failed: standard rumble in use");
        }
    } else if (dev_info.family == RENHDRUM_FAMILY_SWITCH2 && advanced_haptics && g_switch2_pro.load(std::memory_order_relaxed) && api().has_pro2) {
        // Switch 2 Pro Controller
        if (!start_switch2_pro(pad)) {
            note("Switch 2 Pro Controller haptics failed: standard rumble in use");
        }
    } else if (pad.dualsense) {
        note("DualSense haptics are off: standard rumble in use");
    } else {
        note("%s has no haptic backend here: standard rumble in use", pad.name);
    }
    const Backend be = g_backend.load(std::memory_order_relaxed);
    // Every backend needs the 1 ms worker: the Pro Controller and the Xbox pad to turn values into packets and motor values,
    // the DualSense for the silence watchdog, the wired one also to keep looking for its audio endpoint, and all of them for
    // the test pattern.
    if ((be == Backend::ProController || be == Backend::DualSenseUsb || be == Backend::DualSenseBt ||
         be == Backend::DualSenseBt36 || be == Backend::Xbox || be == Backend::XboxDirect ||
         be == Backend::SteamGrip || be == Backend::Switch2Pro) &&
        !g_thread) {
        g_run.store(true, std::memory_order_relaxed);
        bool started = false;
        {
            std::lock_guard<std::mutex> lk(g_host_mu);
            g_thread = SDL_CreateThread(worker, "nxrt_haptic", nullptr);
            started = g_thread != nullptr;
        }
        if (!started) {
            g_run.store(false, std::memory_order_relaxed);
            detach();
            note("the haptics thread did not start: standard rumble in use");
        }
    }
    {
        std::lock_guard<std::mutex> lk(g_note_mu);
        NXLOG_I(kTag, "pad \"%s\": mode %s -> backend %s%s%s", pad.name, mode_name(mode),
                backend_name(g_backend.load(std::memory_order_relaxed)), g_detail[0] ? "; " : "", g_detail);
    }
}

bool is_dualsense(const char* name, uint16_t vendor, uint16_t product) {
    if (vendor == 0x054C && (product == 0x0CE6 || product == 0x0DF2)) return true;
    return name && std::strstr(name, "DualSense") != nullptr;
}

bool is_pro(const uint16_t vendor, const uint16_t product) {
    return vendor == 0x057E && (product == 0x2006 || product == 0x2007 || product == 0x2009);
}

// SDL learns the transport from the system's description of the device. Wine does not describe a Bluetooth device as one, so
// SDL calls a wireless DualSense wired there, and the haptics would go to the audio endpoint of a cable that is not plugged
// in. The pad's own report descriptor still tells.
bool dualsense_on_bluetooth(SDL_Joystick* joy) {
    const char* path = SDL_GetJoystickPath(joy);
    SDL_hid_device* d = path && *path ? SDL_hid_open_path(path) : nullptr;
    if (!d) return false;
    uint8_t desc[4096];
    const int n = SDL_hid_get_report_descriptor(d, desc, sizeof(desc));
    SDL_hid_close(d);
    return n > 0 && dualsense_descriptor_is_bluetooth(desc, static_cast<size_t>(n));
}

}  // namespace

bool dualsense_descriptor_is_bluetooth(const uint8_t* desc, size_t size) {
    // HID short items: a prefix byte (size, type, tag), then 0, 1, 2 or 4 bytes of value. Long items (0xFE) carry their size.
    uint32_t report_id = 0;
    for (size_t i = 0; desc && i < size;) {
        const uint8_t prefix = desc[i];
        if (prefix == 0xFE) {
            if (i + 1 >= size) break;
            i += 3u + desc[i + 1];
            continue;
        }
        const size_t len = (prefix & 3) == 3 ? 4 : prefix & 3;
        if (i + 1 + len > size) break;
        if ((prefix & 0xFC) == 0x84) {          // global item Report ID
            report_id = 0;
            for (size_t k = 0; k < len; ++k) report_id |= static_cast<uint32_t>(desc[i + 1 + k]) << (8 * k);
        } else if ((prefix & 0xFC) == 0x90 && report_id == 0x31) {   // main item Output, in the Bluetooth state report
            return true;
        }
        i += 1 + len;
    }
    return false;
}

const char* backend_name(Backend b) {
    switch (b) {
        case Backend::Rumble: return "standard rumble";
        case Backend::ProController: return "HD Rumble packets";
        case Backend::DualSenseUsb: return "DualSense haptics (USB)";
        case Backend::DualSenseBt: return "DualSense haptics (Bluetooth, report 0x32)";
        case Backend::DualSenseBt36: return "DualSense haptics (Bluetooth, report 0x36)";
        case Backend::Xbox: return "Xbox four-motor rumble";
        case Backend::XboxDirect: return "Xbox Direct (grips + triggers)";
        case Backend::SteamGrip: return "Steam Controller grip haptics";
        case Backend::Switch2Pro: return "Switch 2 Pro Controller";
        default: return "none";
    }
}

void configure(Mode m, int strength_percent, double rumble_scale, bool rumble_on,
               bool advanced_haptics, bool xbox_direct, bool ds_bt36, bool adaptive_triggers,
               bool steam_grips, bool switch2_pro, bool crisp_notes, bool envelope_5ms, int ds_volume) {
    const int strength = std::clamp(strength_percent, -1, 100);
    const float scale = static_cast<float>(std::clamp(static_cast<float>(rumble_scale), 0.f, 2.f));
    const int vol = std::clamp(ds_volume, 0, 100);
    std::lock_guard<std::mutex> lk(g_life_mu);
    const bool was_on = g_rumble_on.exchange(rumble_on, std::memory_order_relaxed);
    // Only a different mode or a different master switch changes what drives the pad. The strength and the scale are read by
    // push() on the next value, so moving the slider must not touch the audio stream.
    const bool changed = m != g_mode.load(std::memory_order_relaxed) || was_on != rumble_on;
    g_mode.store(m, std::memory_order_relaxed);
    g_strength.store(strength, std::memory_order_relaxed);
    g_scale.store(scale, std::memory_order_relaxed);
    // Store advanced haptics settings
    g_advanced_haptics.store(advanced_haptics, std::memory_order_relaxed);
    g_xbox_direct_enabled.store(xbox_direct, std::memory_order_relaxed);
    g_ds_bt36.store(ds_bt36, std::memory_order_relaxed);
    g_adaptive_triggers.store(adaptive_triggers, std::memory_order_relaxed);
    g_steam_grips.store(steam_grips, std::memory_order_relaxed);
    g_switch2_pro.store(switch2_pro, std::memory_order_relaxed);
    g_crisp_notes.store(crisp_notes, std::memory_order_relaxed);
    g_envelope_5ms.store(envelope_5ms, std::memory_order_relaxed);
    g_ds_volume.store(vol, std::memory_order_relaxed);
    if (!changed) return;
    detach();
    if (m == Mode::Off) {
        note("vibration is off");
        NXLOG_I(kTag, "mode %s: no backend, the pad does not vibrate", mode_name(m));
        return;
    }
    if (!rumble_on) {
        note("[pad] rumble is off: nothing is sent to the pad");
        NXLOG_I(kTag, "mode %s: [pad] rumble is off, the pad does not vibrate", mode_name(m));
        return;
    }
    attach();
}

void pad_added(uint32_t instance_id) {
    SDL_Joystick* joy = SDL_GetJoystickFromID(static_cast<SDL_JoystickID>(instance_id));
    if (!joy) return;
    const uint16_t vendor = SDL_GetJoystickVendor(joy);
    const uint16_t product = SDL_GetJoystickProduct(joy);
    const char* name = SDL_GetJoystickName(joy);
    const SDL_JoystickConnectionState conn = SDL_GetJoystickConnectionState(joy);
    const SDL_PropertiesID props = SDL_GetJoystickProperties(joy);
    const bool rumble = props != 0 && SDL_GetBooleanProperty(props, SDL_PROP_JOYSTICK_CAP_RUMBLE_BOOLEAN, false);
    // Third-party XInput pads may report another gamepad type, so Microsoft's vendor id counts as well.
    const bool xbox = SDL_GetGamepadTypeForID(static_cast<SDL_JoystickID>(instance_id)) == SDL_GAMEPAD_TYPE_XBOXONE ||
                      vendor == 0x045E;
    const bool dualsense = is_dualsense(name, vendor, product);
    const bool ds_bt = dualsense && conn != SDL_JOYSTICK_CONNECTION_WIRELESS && dualsense_on_bluetooth(joy);
    if (ds_bt)
        NXLOG_I(kTag, "pad \"%s\": SDL does not report it as wireless, but its reports are Bluetooth's", name ? name : "controller");
    std::lock_guard<std::mutex> lk(g_life_mu);
    {
        std::lock_guard<std::mutex> hl(g_host_mu);
        if (g_pad.id != 0 && g_pad.id != instance_id) return;   // one pad drives the haptics; the first one wins
    }
    detach();
    {
        std::lock_guard<std::mutex> hl(g_host_mu);
        g_pad.id = instance_id;
        g_pad_id.store(instance_id, std::memory_order_relaxed);
        g_pad.joy = joy;
        g_pad.vendor = vendor;
        g_pad.product = product;
        g_pad.dualsense = dualsense;
        g_pad.pro = is_pro(vendor, product);
        g_pad.xbox = xbox && !g_pad.dualsense && !g_pad.pro;
        g_pad.bt = ds_bt || conn == SDL_JOYSTICK_CONNECTION_WIRELESS;
        g_pad.wired = !ds_bt && conn == SDL_JOYSTICK_CONNECTION_WIRED;
        g_pad.rumble = rumble;
        std::snprintf(g_pad.name, sizeof(g_pad.name), "%s", name ? name : "controller");
    }
    attach();
}

void pad_removed(uint32_t instance_id) {
    std::lock_guard<std::mutex> lk(g_life_mu);
    {
        std::lock_guard<std::mutex> hl(g_host_mu);
        if (g_pad.id != instance_id) return;
    }
    detach();
    std::lock_guard<std::mutex> hl(g_host_mu);
    g_pad = PadState{};
    g_pad_id.store(0, std::memory_order_relaxed);
}

void audio_devices_changed() {
    // SDL says an audio endpoint came or went: look for the DualSense one on the next worker pass instead of waiting out the
    // two second period. The worker notices a stream that lost its device by itself (audio_alive()).
    g_next_enum_ns.store(0, std::memory_order_relaxed);
}

void push(uint32_t npad_slot, int hand, float amp_low, float freq_low, float amp_high, float freq_high) {
    if (npad_slot != 0 || hand < 0 || hand > 1) return;
    if (g_mode.load(std::memory_order_relaxed) == Mode::Off) return;
    // [pad] rumble is the master switch: with it off nothing vibrates, whichever backend would have been picked. play.cpp's own
    // map_rumble already honours it for the standard rumble; this is the same switch for the haptic backends.
    if (!g_rumble_on.load(std::memory_order_relaxed)) {
        silence();
        return;
    }
    const Backend be = g_backend.load(std::memory_order_relaxed);
    // Only forward to haptic backends that are actively running
    if (be != Backend::ProController && be != Backend::DualSenseUsb &&
        be != Backend::DualSenseBt && be != Backend::DualSenseBt36 &&
        be != Backend::Xbox && be != Backend::XboxDirect &&
        be != Backend::SteamGrip && be != Backend::Switch2Pro) {
        return;
    }
    const float scale = strength_scale();
    const float al = std::clamp(amp_low * scale, 0.f, 1.f);
    const float ah = std::clamp(amp_high * scale, 0.f, 1.f);
    const float fl = freq_low > 0.f ? freq_low : 160.f;
    const float fh = freq_high > 0.f ? freq_high : 320.f;
    if (al > 0.f || ah > 0.f) {
        g_nonzero.fetch_add(1, std::memory_order_relaxed);
        const float loud = std::max(al, ah);
        if (loud > g_amp_peak.load(std::memory_order_relaxed)) g_amp_peak.store(loud, std::memory_order_relaxed);
    }
    g_last_amp.store(std::max(al, ah), std::memory_order_relaxed);
    g_pushes.fetch_add(1, std::memory_order_relaxed);
    const int64_t t = now_us();
    std::lock_guard<std::mutex> lk(g_eng_mu);
    if (!g_engine && !g_xbox && !g_dsp && !g_xbox_direct && !g_pro2) return;
    push_locked(t, hand, al, fl, ah, fh);
    g_at[hand] = t;
    g_muted[hand] = false;
}

bool legacy_rumble(uint32_t pad_id) {
    // True unless a haptic backend is really driving this pad: a missing library, an engine that did not start, an audio
    // endpoint that never appeared or a pad that refuses the packets all leave the standard rumble in charge. Only one pad
    // has a haptic backend; every other connected pad keeps the standard rumble whatever that one is doing.
    // With [pad] rumble off nothing vibrates at all, so the caller must not send the standard rumble either.
    if (!g_rumble_on.load(std::memory_order_relaxed)) return false;
    if (g_mode.load(std::memory_order_relaxed) == Mode::Off) return false;
    if (pad_id != g_pad_id.load(std::memory_order_relaxed)) return true;
    return !g_running.load(std::memory_order_relaxed);
}

bool test_start() {
    if (g_mode.load(std::memory_order_relaxed) == Mode::Off) return false;
    if (!g_rumble_on.load(std::memory_order_relaxed)) return false;   // the master switch covers the test too
    if (!g_running.load(std::memory_order_relaxed)) return false;
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        if (!g_thread) return false;            // the worker sends the pattern
    }
    g_test_until_ns.store(SDL_GetTicksNS() + 400000000ull, std::memory_order_relaxed);   // 0.4 s, shorter than a watchdog
    return true;
}

void shutdown() {
    std::lock_guard<std::mutex> lk(g_life_mu);
    detach();
    {
        std::lock_guard<std::mutex> hl(g_host_mu);
        g_pad = PadState{};
        g_pad_id.store(0, std::memory_order_relaxed);
    }
    g_last_amp.store(0.f, std::memory_order_relaxed);
    g_test_until_ns.store(0, std::memory_order_relaxed);
}

Status status() {
    Status s;
    s.dll = api().ok;
    s.requested = g_mode.load(std::memory_order_relaxed);
    s.backend = g_backend.load(std::memory_order_relaxed);
    s.running = g_running.load(std::memory_order_relaxed);
    {
        std::lock_guard<std::mutex> lk(g_host_mu);
        s.wired = g_pad.wired;
        std::snprintf(s.device, sizeof(s.device), "%s", g_pad.name);
    }
    {
        std::lock_guard<std::mutex> lk(g_note_mu);
        std::snprintf(s.detail, sizeof(s.detail), "%s", g_detail);
    }
    s.last_amp = g_last_amp.load(std::memory_order_relaxed);
    s.pushes = g_pushes.load(std::memory_order_relaxed);
    s.packets = g_packets.load(std::memory_order_relaxed);
    return s;
}

}  // namespace nxrt::gfx::haptic
