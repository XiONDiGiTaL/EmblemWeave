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

// The library's own constants (renhdrum.h v0.4.0, kept in third_party/renhdrum). The header is not included: these are the
// ABI-stable parts we call. renhdrum_version() must be on the same major.minor branch (0x0004xx) before anything is created;
// a patch release of that branch keeps the ABI and is accepted.
constexpr uint32_t kExpectedVersion = 0x000400;
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

// The settings struct the library fills in (renhdrum.h, RenHdcSettings). Only `enabled` and the three transport flags are set
// by us; the rest keeps what renhdrum_settings_default() writes.
struct RenSettings {
    bool enabled;
    int32_t usb_interval_ms;
    int32_t bluetooth_level;
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
    bool has_ds = false;       // DualSense DSP (wired audio, and the base of the wireless path)
    bool has_ds_bt = false;    // DualSense report 0x32
    bool has_xbox = false;     // Xbox four-motor converter
    HMODULE dll = nullptr;
    uint32_t (*version)() = nullptr;
    int32_t (*settings_default)(RenSettings*) = nullptr;
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
    RenXbox* (*xb_create)(bool) = nullptr;
    void (*xb_destroy)(RenXbox*) = nullptr;
    int32_t (*xb_push)(RenXbox*, int64_t, bool, float, float, float, float) = nullptr;
    int32_t (*xb_tick)(RenXbox*, int64_t, uint16_t*) = nullptr;
    int32_t (*xb_resend)(RenXbox*) = nullptr;
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
    a.has_ds = bind(a.dll, "renhdrum_dualsense_dsp_create", &a.dsp_create) &&
               bind(a.dll, "renhdrum_dualsense_dsp_destroy", &a.dsp_destroy) &&
               bind(a.dll, "renhdrum_dualsense_dsp_set_half_target", &a.dsp_set_half) &&
               bind(a.dll, "renhdrum_dualsense_dsp_fill_f32", &a.dsp_fill);
    a.has_ds_bt = a.has_ds && bind(a.dll, "renhdrum_dualsense_dsp_build_bt_report", &a.dsp_bt_report);
    a.has_xbox = bind(a.dll, "renhdrum_xbox_rumble_create", &a.xb_create) &&
                 bind(a.dll, "renhdrum_xbox_rumble_destroy", &a.xb_destroy) &&
                 bind(a.dll, "renhdrum_xbox_rumble_push_half_sample", &a.xb_push) &&
                 bind(a.dll, "renhdrum_xbox_rumble_tick", &a.xb_tick) &&
                 bind(a.dll, "renhdrum_xbox_rumble_resend", &a.xb_resend);
    // The game is a Switch 2 title: its values use the Switch 2 ranges, and the library picks its Switch 2 curves for every
    // controller from this one global switch.
    a.set_nx2(2);
    a.ok = true;
    NXLOG_I(kTag, "renhdrum %u.%u.%u loaded (DualSense %s, DualSense Bluetooth %s, Xbox %s)", (a.version() >> 16) & 0xFF,
            (a.version() >> 8) & 0xFF, a.version() & 0xFF, a.has_ds ? "yes" : "no", a.has_ds_bt ? "yes" : "no",
            a.has_xbox ? "yes" : "no");
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

std::mutex g_life_mu;                     // attach/detach, main thread only

std::mutex g_note_mu;                     // g_detail
char g_detail[192] = {};

struct PadState {
    uint32_t id = 0;
    SDL_Joystick* joy = nullptr;      // also guarded by g_host_mu, read by the worker when it sends
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

std::mutex g_audio_mu;                    // the DualSense stream
SDL_AudioStream* g_stream = nullptr;
SDL_AudioDeviceID g_dev = 0;

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
    RenDsDsp* dsp = api().dsp_create(100, false);
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
        g_dev = dev;
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
        g_dev = 0;
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

// One hand's value to whichever of the three backends exists (only one does at a time). g_eng_mu must be held. Every path
// that feeds a backend goes through here: the game's values, the silence watchdog and the master switch.
void push_locked(int64_t t, int side, float al, float fl, float ah, float fh) {
    if (g_engine) api().push_half(g_engine, t, side == 1, al, fl, ah, fh);
    else if (g_xbox) api().xb_push(g_xbox, t, side == 1, al, fl, ah, fh);
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

int worker(void*) {
    uint8_t pkt[10];
    uint16_t motors[4] = {};
    int64_t xb_retry_at = 0;   // the next time a refused Xbox send may be retried
    uint64_t last_report = 0;
    while (g_run.load(std::memory_order_relaxed)) {
        const int64_t t = now_us();
        const Backend be = g_backend.load(std::memory_order_relaxed);
        int ready = 0;
        int xb_ready = 0;
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
                // Some input paths refuse rumble until the pad's first button press: ask the converter to send the same
                // motors again rather than giving the pad up, but at most every 100 ms, so a pad that keeps refusing is not
                // asked a thousand times a second.
                xb_retry_at = t + kXboxRetryUs;
                std::lock_guard<std::mutex> lk(g_eng_mu);
                if (g_xbox) api().xb_resend(g_xbox);
            }
        }
        if (be == Backend::DualSenseUsb) {
            if (!audio_alive()) {
                // Either it has not opened yet, or the cable went away: both end with the pad on the standard rumble until the
                // endpoint is back (open_dualsense_audio() sets g_running true again when it is). The backend stays
                // DualSenseUsb, which is what keeps this branch, and so the rescan below, running.
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
        // The test pattern runs while the window is open and stops by itself. It has to be re-sent every millisecond:
        // the DSP and the HD Rumble packets both treat a single sample as a one-off.

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
    RenDsDsp* dsp = api().dsp_create(100, false);
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

// Called with g_life_mu held and nothing else: it waits for the worker, so no inner lock may be held here.
void detach() {
    const Backend was = g_backend.load(std::memory_order_relaxed);
    stop_worker();
    stop_bt_writer();
    close_dualsense_audio();
    RenXbox* xbox = nullptr;
    {
        std::lock_guard<std::mutex> lk(g_eng_mu);
        if (g_engine) {
            api().engine_destroy(g_engine);
            g_engine = nullptr;
        }
        xbox = g_xbox;
        g_xbox = nullptr;
        g_at[0] = g_at[1] = 0;
        g_muted[0] = g_muted[1] = false;
        g_pro_wired = false;
        g_last_pkt_live = false;
        g_last_send_us = 0;
    }
    if (was == Backend::ProController) {   // a neutral packet so the pad does not keep buzzing
        static const uint8_t kStop[10] = {0x10, 0, 0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40};
        send_packet(kStop);
    }
    if (xbox) {   // all four motors to zero, then the converter goes
        static const uint16_t kZero[4] = {0, 0, 0, 0};
        send_motors(kZero, 0, 0);
        api().xb_destroy(xbox);
    }
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
    const bool want_ds = mode == Mode::Auto || mode == Mode::DualSense;
    if (pad.dualsense && want_ds) {
        if (!api().has_ds) {
            note("this renhdrum.dll has no DualSense haptics: standard rumble in use");
        } else if (pad.bt) {
            if (!api().has_ds_bt) note("this renhdrum.dll has no DualSense Bluetooth haptics: standard rumble in use");
            else start_dualsense_bt(pad);   // sets the backend, or the reason it could not
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
        if (!api().has_xbox) {
            note("this renhdrum.dll has no Xbox rumble: standard rumble in use");
        } else if (!pad.rumble) {
            // A Microsoft device without rumble motors (vendor 0x045E is wider than the Xbox pads): nothing to claim.
            note("%s has no haptic backend here: standard rumble in use", pad.name);
        } else {
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
    if ((be == Backend::ProController || be == Backend::DualSenseUsb || be == Backend::DualSenseBt || be == Backend::Xbox) &&
        !g_thread) {
        g_run.store(true, std::memory_order_relaxed);
        bool started = false;
        {
            std::lock_guard<std::mutex> lk(g_host_mu);
            g_thread = SDL_CreateThread(worker, "nxrt_haptic", nullptr);
            started = g_thread != nullptr;
        }
        if (!started) {
            // Nothing would tick the engine or watch for silence: release everything the backend holds (the Bluetooth
            // writer, the DSP, the engines), which also clears g_running so play.cpp sends the standard rumble again.
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

}  // namespace

const char* backend_name(Backend b) {
    switch (b) {
        case Backend::Rumble: return "standard rumble";
        case Backend::ProController: return "HD Rumble packets";
        case Backend::DualSenseUsb: return "DualSense haptics (USB)";
        case Backend::DualSenseBt: return "DualSense haptics (Bluetooth)";
        case Backend::Xbox: return "Xbox four-motor rumble";
        default: return "none";
    }
}

void configure(Mode m, int strength_percent, double rumble_scale, bool rumble_on) {
    const int strength = std::clamp(strength_percent, -1, 100);
    const float scale = static_cast<float>(std::clamp(static_cast<float>(rumble_scale), 0.f, 2.f));
    std::lock_guard<std::mutex> lk(g_life_mu);
    const bool was_on = g_rumble_on.exchange(rumble_on, std::memory_order_relaxed);
    // Only a different mode or a different master switch changes what drives the pad. The strength and the scale are read by
    // push() on the next value, so moving the slider must not touch the audio stream.
    const bool changed = m != g_mode.load(std::memory_order_relaxed) || was_on != rumble_on;
    g_mode.store(m, std::memory_order_relaxed);
    g_strength.store(strength, std::memory_order_relaxed);
    g_scale.store(scale, std::memory_order_relaxed);
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
        g_pad.product = product;
        g_pad.dualsense = is_dualsense(name, vendor, product);
        g_pad.pro = is_pro(vendor, product);
        g_pad.xbox = xbox && !g_pad.dualsense && !g_pad.pro;
        g_pad.bt = conn == SDL_JOYSTICK_CONNECTION_WIRELESS;
        g_pad.wired = conn == SDL_JOYSTICK_CONNECTION_WIRED;
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
    if (be != Backend::ProController && be != Backend::DualSenseUsb && be != Backend::DualSenseBt && be != Backend::Xbox) return;
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
    if (!g_engine && !g_xbox && !g_dsp) return;
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
