// SPDX-License-Identifier: GPL-3.0-or-later
// Display extras (S14): HDR output, upscaling, frame generation and low-latency modes.
//
// Everything here is off by default except HDR "auto" (HDR is used when the display and Windows offer it). The settings are read once at
// start-up from the play-mode settings file (controls.ini, sections [hdr] [upscale] [latency]) with mpr.ini / environment overrides, and
// the live part can be changed from the F1 page. The pieces:
//
//   HDR       the game's scene pipeline is switched to float targets and a tone curve whose shoulder reaches the display's peak; the
//             present step encodes the result as HDR10 (ST 2084) or scRGB. See src/gfx/extras/hdr_*.
//   Upscale   a spatial pass, AMD FSR 1 or NVIDIA Image Scaling, in the present step (any internal resolution below the window size) or on
//             the HDR scene before the game's tone map. Temporal: FSR 3.1 (FidelityFX API, Vulkan DLLs loaded at run time) and DLSS / DLAA
//             (NVIDIA Streamline, loaded at run time) on the HDR scene before the tone map, with motion vectors made from depth and the
//             camera matrices and the game's own projection jitter (src/gfx/extras/temporal.h, docs/S14.md). XeSS through libxess.dll (temporal_xess.cpp).
//   Latency   NVIDIA Reflex (VK_NV_low_latency2) and AMD Anti-Lag 2 (VK_AMD_anti_lag) markers and sleeps around input sampling.
#pragma once
#include <cctype>
#include <cstdint>
#include <string>

namespace nxrt::extras {

// ---- settings --------------------------------------------------------------------------------------------------------------------
// Auto: HDR when the display offers it. On: the HDR scene pipeline even without an HDR display. Native: as On, and the levels follow what the game
// declares for an HDR TV on Switch 2 (nn::vi layer content luminance ranges, NVN2 window colour space and HDR metadata); see docs/S14.md.
enum class HdrMode : uint8_t { Off = 0, Auto, On, Native };
enum class HdrFormat : uint8_t { Auto = 0, Hdr10, ScRgb };     // swapchain flavour (Auto: HDR10 when offered, else scRGB)

struct HdrSettings {
    HdrMode mode = HdrMode::Auto;
    HdrFormat format = HdrFormat::Auto;
    double peak_nits = 0;            // 0 = the display's reported peak (1000 when it reports none), else 200..10000
    double paper_white_nits = 0;     // 0 = Windows' SDR content brightness when HDR is on there (203 otherwise), else 80..500
    double highlight_boost = 1.0;    // 0 = the game's SDR look up to its own white, 1 = highlights reach the peak (0 .. 2)
    double ui_nits = 0;              // 0 = the UI's white is paper white (as before), else 40..500: the UI's own white; the 3D scene keeps paper white
    double sdr_gamma = 2.2;          // HDR output: the power the game's SDR range is shown with, like a TV (0 = the sRGB curve, else 1.8..2.6)
    // scene grading before the tone map (GT tone map only: Fire Emblem Fortune's Weave); neutral = 0, 1, 1, 1, 1 and 0
    double grade_exposure = 0;       // stops (-2 .. 2)
    double grade_contrast = 1;       // power of luminance around mid grey (0.5 .. 1.5)
    double grade_shadows = 1;        // below mid grey: > 1 lifts, < 1 deepens (0.5 .. 1.5)
    double grade_highlights = 1;     // above mid grey: > 1 brighter, < 1 dimmer (0.5 .. 1.5)
    double grade_saturation = 1;     // scene saturation (0 .. 2)
    double hue_preserve = 0;         // 0 = the game's per-channel tone curve, 1 = the curve on luminance, scene hues kept (0 .. 1; GT tone map, HDR)
    double highlight_saturation = 0; // 0 = the game's highlights (they whiten), 1 = highlights keep the scene's colour (0 .. 1)
    bool compare = false;            // split screen for testing: the left half as the game draws it, the right half with grading, highlight saturation and sharpen
    double sharpen = 1.0;            // the game's sharpening strength, SDR and HDR (0 = none, 1 = the game's, up to 3); Fire Emblem Fortune's Weave
    double motion_blur = 1.0;        // the game's motion blur strength (0 = none, 1 = the game's, up to 2); Fire Emblem Fortune's Weave
    double depth_of_field = 1.0;     // the game's depth-of-field blur strength (0 = none, 1 = the game's, up to 2); Fire Emblem Fortune's Weave
    double vignette = 1.0;           // the game's vignette strength where a scene has one (0 = none, 1 = the game's, up to 2); Fire Emblem Fortune's Weave
    double bloom = 1.0;              // the game's bloom strength, SDR and HDR (0 = none, 1 = the game's, up to 4); Fire Emblem Fortune's Weave
    bool wide_gamut = false;         // keep the game's BT.2020 scene colours outside BT.709 (the tone map clips them); restart to apply
    bool dither = false;             // add +-1 code value of fine noise to the output to hide steps in smooth gradients (off by default)
    double gamut_expand = 0;         // HDR output: 0 = the game's own BT.709 colours (as on the console), up to 1 = saturated colours toward DCI-P3
};

// Nis: NVIDIA Image Scaling v1.0.3 (spatial, any GPU). Fsr3, Dlss, Dlaa, Xess: temporal (Dlaa = DLSS at the input size).
enum class UpscaleMode : uint8_t { Off = 0, Fsr1, Fsr3, Dlss, Xess, Nis, Dlaa };
// Temporal quality preset: the input scale per side (native 1, quality 1/1.5, balanced 1/1.7, performance 1/2, ultra performance 1/3); Custom
// keeps scene_scale. The scale only takes effect where the game renders the scene below its output (scene_scale); otherwise the temporal
// upscalers run at the input size (native anti-aliasing).
enum class UpscaleQuality : uint8_t { Native = 0, Quality, Balanced, Performance, UltraPerformance, Custom };
struct UpscaleSettings {
    UpscaleMode mode = UpscaleMode::Off;
    UpscaleQuality quality = UpscaleQuality::Quality;
    double sharpness = 0.8;          // 0 .. 1 (FSR 1 RCAS or the NIS sharpening; 1 = sharpest)
    int scene_scale = 100;           // 25 .. 100: percent of the output size the 3D scene is rendered at (the game's own dynamic-resolution path; HUD and post stay native)
    std::string fsr_dll;             // FidelityFX API DLL for FSR 3 (amd_fidelityfx_loader_vk.dll or amd_fidelityfx_vk.dll; empty: next to the exe)
    std::string dlss_dll;            // folder (or sl.interposer.dll path) of the Streamline + DLSS DLLs (empty: next to the exe)
    std::string xess_dll;            // libxess.dll the player supplies (empty: not used)
    bool frame_generation = false;   // FSR 3 frame interpolation (needs the FSR 3 mode)
};

enum class LatencyMode : uint8_t { Off = 0, Auto, On, Boost };   // Auto: whichever of Reflex / Anti-Lag 2 the GPU has; Boost: Reflex boost
struct LatencySettings {
    LatencyMode mode = LatencyMode::Off;
    int frames_ahead = 1;            // with the mode on: simulation of a frame waits until at most this many earlier frames are still unpresented (0 .. 3; -1 = no wait)
    bool log_timings = false;        // log one line of timings per second
};

// Picture controls applied in the present step (the output pass), on the display-encoded picture: black level, contrast, saturation, then gamma.
// All neutral (1, 1, 0, 1): the present step is left exactly as it is (the pass is skipped).
struct PictureSettings {
    double contrast = 1.0;     // 0.5 .. 1.5, around mid grey (0.5 of the encoded signal)
    double saturation = 1.0;   // 0 .. 2 (0 = grey)
    double black = 0.0;        // -0.5 .. 0.25: below 0 pulls the blacks down (the signal at -black becomes black), above 0 lifts them; white stays
    double gamma = 1.0;        // 0.5 .. 2: mid-tone brightness (signal^(1/gamma)); above 1 brightens
    bool neutral() const { return contrast == 1.0 && saturation == 1.0 && black == 0.0 && gamma == 1.0; }
};
// The named presets ([picture] preset): "neutral" and "vivid". False for any other name (*out unchanged).
inline bool picture_preset(const std::string& name, PictureSettings* out) {   // inline: the launcher's settings code uses it without nxrt_gfx
    if (name == "neutral" || name == "off" || name.empty()) {
        *out = PictureSettings{};
        return true;
    }
    if (name == "vivid") {   // a modest push against the game's hazy look
        *out = PictureSettings{};
        out->contrast = 1.10;
        out->saturation = 1.15;
        out->black = -0.03;
        return true;
    }
    return false;
}
PictureSettings picture_clamped(PictureSettings p);

struct Settings {
    HdrSettings hdr;
    UpscaleSettings upscale;
    LatencySettings latency;
    PictureSettings picture;
};

// Reads [hdr] [upscale] [latency] from the nxrt configuration (mpr.ini, environment NXRT_HDR_MODE ...). Never fails.
Settings settings_from_config();
// The settings in force (set once by play mode / the boot code from the file; the live fields change from the F1 page).
Settings settings();
void set_settings(const Settings& s);
// Lock-free reads of the live upscale fields (the render thread asks every frame).
UpscaleMode upscale_mode();
float upscale_sharpness();
// The DLSS model ([dlss] preset): 0 = default, else Streamline's DLSSPreset number (J 10, K 11, L 12, M 13). Read from the config at the first call;
// the F1 menu changes it live (set_dlss_preset) and the DLSS backend sets its options again when it differs from the one in use.
int dlss_preset();
void set_dlss_preset(int preset);
inline int dlss_preset_from_word(std::string w) {   // "default", "j" .. "m" ("preset_m" too); -1 when unknown
    for (char& c : w) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    if (w.rfind("preset", 0) == 0) w.erase(0, 6);
    while (!w.empty() && (w[0] == ' ' || w[0] == '_')) w.erase(0, 1);
    if (w.empty() || w == "default") return 0;
    if (w.size() == 1 && w[0] >= 'j' && w[0] <= 'm') return w[0] - 'a' + 1;
    return -1;
}
inline const char* dlss_preset_word(int preset) {
    switch (preset) {
        case 10: return "j";
        case 11: return "k";
        case 12: return "l";
        case 13: return "m";
        default: return "default";
    }
}
int upscale_scene_scale();          // percent, 100 = off (a temporal mode with a quality preset other than Custom: the preset's percent)
UpscaleQuality upscale_quality();
bool upscale_temporal();            // the mode is FSR 3, DLSS, DLAA or XeSS
int quality_percent(UpscaleQuality q);   // the preset's input scale per side in percent (Custom: -1)
// The game's own projection jitter as its TAA code last stored it (NDC units, the previous frame's; pkp_taa.cpp), for the log only.
void temporal_note_game_jitter(float x_ndc, float y_ndc);
// The picture controls in force for the present step (lock-free; the F1 page's sliders change them live before Apply and save).
struct PictureLive {
    float contrast = 1, saturation = 1, black = 0, gamma = 1;
    bool active = false;             // false: all neutral, the present step skips the adjustment
};
PictureLive picture_live();
void set_picture_live(const PictureSettings& p);   // only the live copy; set_settings() also sets it

// ---- HDR runtime state ---------------------------------------------------------------------------------------------------------
enum class OutputKind : uint8_t { Sdr8 = 0, Hdr10, ScRgb };
const char* output_kind_name(OutputKind k);

struct HdrState {
    bool scene_hdr = false;          // the game runs the HDR scene pipeline: float window / post targets, patched tone-map shader and curve
    OutputKind output = OutputKind::Sdr8;   // what the swapchain carries
    float paper_white_nits = 203;
    float peak_nits = 1000;
    float peak_ratio = 1;            // asymptote of the extended shoulder, in paper-white units (1 = the game's own SDR curve)
    float ui_nits = 0;               // [hdr] ui_nits: 0 = off (the UI's white is paper white); else the UI's white (hdr_ui_split)
    bool display_hdr = false;        // Windows reports HDR on for the monitor (informational)
    float display_peak_nits = 0;
    float display_sdr_white_nits = 80;
    // What the game declares about its own output (0 / -1: not declared). Used by HdrMode::Native.
    float game_standard_nits = 0;    // nn::vi::SetLayerNextHdrStandardContentLuminanceRangeMetadata: the SDR-graded (paper white) range's maximum
    float game_extended_nits = 0;    // nn::vi::SetLayerNextHdrExtendedContentLuminanceRangeMetadata: the content's highlight maximum
    int game_color_space = -1;       // nvnWindowBuilderSetColorSpace value (NVN2 enum, raw)
    float game_max_cll = 0, game_max_fall = 0;   // nvnWindow(Builder)SetHdrContentMetadata
    float game_display_max_nits = 0; // nvnWindow(Builder)SetHdrDisplayMetadata maximum luminance
    uint32_t metadata_serial = 0;    // bumped whenever the swapchain's HDR metadata should be sent again
};
// Readable from any thread (a copy of a snapshot behind a lock-free sequence).
HdrState hdr_state();
void hdr_set_state(const HdrState& s);
inline bool hdr_scene_active() { return hdr_state().scene_hdr; }

// Recomputes paper white, peak and the peak ratio from settings() and the display numbers kept in the state (after a live settings change).
// The swapchain's output kind and scene_hdr never change here (they are decided when the renderer is created).
void hdr_refresh_levels();

// The game's declarations (HdrMode::Native). Thread-safe; each call refreshes the levels and the swapchain metadata, and logs the native decision once.
void hdr_note_layer_luminance(bool extended, unsigned min_nits, unsigned max_nits);
// color_space: the NVN2 value (-1 = not set); display: 10 floats (red, green, blue, white x/y, max, min luminance) or null; content: max CLL, max FALL or null.
void hdr_note_window_hdr(int color_space, const float* display, const float* content);
bool hdr_mode_native();

// Resolves the numbers the settings leave at 0 ("auto") against what the display reports.
void hdr_resolve_levels(const HdrSettings& s, bool display_hdr, float display_peak_nits, float display_sdr_white_nits, float* paper, float* peak);
// UI brightness ([hdr] ui_nits). The output pass maps 1.0 of the window texture to ui_nits, and the scene's last pass before the UI (the
// first draw after the HDR tone-map draw into a target of the window's kind, or the tone-map draw itself when it writes there) multiplies
// its colour by hdr_scene_gain, so the 3D scene keeps paper white.
// hdr_ui_split: ui_nits is set, the HDR scene runs and the swapchain carries HDR (the setting does nothing on an SDR swapchain).
bool hdr_ui_split(const HdrState& hs);
float hdr_scene_gain(const HdrState& hs);   // paper_white_nits / ui_nits; 1 when hdr_ui_split is false
// The replay's verdict for the frame being presented (render thread, once per game present): true when its scene got the gain, or when it
// drew no 3D scene at all (a 2D screen, which is all UI). hdr_output_white_nits is ui_nits only for such a frame and paper white otherwise,
// so a frame whose scene missed the gain is never brightened. The output pass and the engine overlay both read it.
void hdr_set_ui_white_frame(bool on);
float hdr_output_white_nits(const HdrState& hs);

}  // namespace nxrt::extras
