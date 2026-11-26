//! Persistent launcher settings.
//!
//! Stored as JSON at `%APPDATA%\EmblemWeave\config.json`. Every section has a
//! serde default, so a file written by an older build loads with the new
//! fields filled in instead of being discarded. Writes go to a temporary file
//! first and are renamed into place, so a crash mid-write cannot leave a
//! truncated config behind.

use crate::i18n::Language;
use crate::theme::Scheme;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Where the user's own game data lives. Every path is optional: the setup
/// page detects what it can from `game_root` and the user may override any
/// individual location.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Paths {
    /// The folder the user picked: an extracted dump, an NSP extraction, or a
    /// folder holding `exefs` and `romfs`.
    pub game_root: String,
    pub exefs: String,
    pub romfs: String,
    /// The player's dumped add-on content: a folder with one `<title id>/romfs`
    /// (or `<title id>/<n>/romfs`) per pack. Empty: detect `dlc` beside the dump.
    pub dlc: String,
    pub nacp: String,
    /// Decompressed `main` segments (`text.bin`, `rodata.bin`, `data.bin`).
    pub decoded: String,
}

/// Values the runtime reports to the game as system settings. These are live:
/// the boot passes them to `nn::oe` and `nn::account`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct System {
    pub language: String,
    pub user_name: String,
}

impl Default for System {
    fn default() -> Self {
        System {
            language: "en-US".into(),
            user_name: "Player".into(),
        }
    }
}

/// Run parameters for the guest execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Runtime {
    /// Guest instructions before a boot run stops on its own.
    pub max_steps: u64,
    /// Guest heap arena, in MiB.
    pub heap_mib: u64,
}

impl Default for Runtime {
    fn default() -> Self {
        Runtime {
            max_steps: 50_000_000,
            heap_mib: 4096,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Graphics {
    pub resolution: String,
    pub display_mode: String,
    pub vsync: bool,
    pub frame_limit: u32,
    pub aspect: String,
    /// `[fefw] ultrawide`: the game-specific hooks that widen the 3D view and
    /// keep the menus in the centred 16:9 area. `auto` (on) is the runtime's
    /// default; off stretches the 16:9 picture to a wide output as before.
    pub ultrawide: bool,
    pub render_scale: f32,
    pub upscaling: String,
    /// How crisp the final picture is: the engine's `[upscale] sharpness`, in
    /// the four steps the game's own menu scene understands.
    pub sharpening: String,
    pub quality: String,
    pub shader_precompile: bool,
    /// The temporal upscalers' render scale preset: the game's `[upscale] quality`.
    pub upscale_quality: String,
    /// Percent of the output the scene is drawn at, `[upscale] scene_scale`: the game's own scene buffer,
    /// separate from the launcher's render scale.
    pub scene_scale: u32,
    /// `[dlss] frame_gen`: off, DLSS Frame Generation or FSR 3 Frame Generation.
    pub frame_gen: String,
    /// Load ReShade's Vulkan layer while the game runs. Off disables the layer
    /// for this game process only; it does not uninstall ReShade.
    pub re_shade: bool,
    /// `[dlss] upscaler`: what renders frames the game asks its console's DLSS to make.
    pub dlss_upscaler: String,
    /// `[display] astc_quality`: how the game's ASTC textures reach the GPU.
    pub astc_quality: String,
    /// `[gfx] gpu`: which Vulkan device the engine picks.
    pub gpu: String,
    /// `[gfx] present_guard`: whether the engine replaces the immediate present path that can reset Intel's Windows
    /// driver (#72). "Automatic" (its own rule: Intel GPUs that are not discrete), "Always on" or "Off".
    pub present_guard: String,
    /// `[media] movie_decoder`: the engine's AV1 decoder for cutscene movies. "dav1d (CPU)" is built in;
    /// "FFmpeg (libavcodec)" loads the shipped avcodec/avutil/swresample DLLs and can use D3D11VA.
    pub video_decoder: String,
    /// `[media] movie_hwaccel`: ask FFmpeg for D3D11VA instead of its software decoder. Ignored by dav1d.
    pub video_hwaccel: bool,
    /// Threads the engine may use to compile shader parts while playing; 0 leaves the engine's own
    /// `min(4, cpus / 6)`.
    pub compile_threads: u32,
    /// `[nvn] compile_priority` for those threads.
    pub compile_priority: String,
    /// `[nvn] compile_eco`: run them in Windows efficiency mode.
    pub compile_eco: bool,
}

impl Default for Graphics {
    fn default() -> Self {
        Graphics {
            resolution: "Native display".into(),
            display_mode: "Borderless window".into(),
            vsync: true,
            frame_limit: 60,
            aspect: "16:9".into(),
            ultrawide: true,
            render_scale: 1.0,
            upscaling: "Off".into(),
            sharpening: "Default".into(),
            quality: "Original".into(),
            shader_precompile: true,
            upscale_quality: "Native".into(),
            scene_scale: 100,
            frame_gen: "Off".into(),
            re_shade: true,
            dlss_upscaler: "Automatic".into(),
            astc_quality: "High".into(),
            gpu: "Automatic".into(),
            present_guard: "Automatic".into(),
            video_decoder: "dav1d (CPU)".into(),
            video_hwaccel: true,
            // Two workers, below idle and in efficiency mode: a laptop stays a laptop. Players who want the
            // fastest first draw can raise this on Graphics.
            compile_threads: 2,
            compile_priority: "Idle".into(),
            compile_eco: true,
        }
    }
}

/// The engine's optimization policy, written to `fefw.ini` before Play and
/// edited live from the F1 menu. Defaults are the working systems' own defaults:
/// adding this section to an older config changes nothing until a switch is
/// moved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Performance {
    /// `[perf] enabled`: measure frames and classify the bottleneck.
    pub controller: bool,
    /// Automatic, Conservative, Balanced, Aggressive or Manual.
    pub mode: String,
    /// The controller's frame-time budget, not a frame limiter.
    pub target_fps: u32,
    /// `[perf] stability`: hold reductions through a spike and record them.
    pub stability: bool,
    /// `[perf] scene_profiles`: classify the workload shape.
    pub scene_profiles: bool,
    /// Auto, Exploration, Crowded, Combat, Cinematic, Menu or Loading.
    pub scene_override: String,
    /// `[perf] adaptive_mip`: the only live visual action the controller owns.
    pub adaptive_mip: bool,
    pub max_mip_bias: f32,
    /// `[nvn] staging_ring_mb`: persistent upload ring size.
    pub staging_ring_mb: u32,
    /// `[nvn] pool_upload_ms`: render-thread upload budget per frame.
    pub pool_upload_ms: u32,
    /// `[nvn] tex_prep`: prepare compressed textures on workers.
    pub tex_prep: bool,
    /// `[nvn] tex_prep_threads`: 0 chooses a third of the hardware threads.
    pub tex_prep_threads: u32,
    /// `[nvn] tex_prep_mb`: memory finished/queued preparations may hold.
    pub tex_prep_mb: u32,
    /// `[gfx] residency`: Automatic, On or Off.
    pub residency: String,
    /// `[gfx] residency_percent`: VRAM budget share that starts demotion.
    pub residency_percent: u32,
    /// `[nvn] lean_draws`: existing lean-replay path.
    pub lean_draws: bool,
    /// `[nvn] prewarm`: build recorded pipelines on workers.
    pub prewarm: bool,
    /// `[nvn] gpl`: background shader-part compilation.
    pub gpl: bool,
    /// Write the XiØN palette into ReShade.ini's supported `[STYLE]` keys when
    /// Play starts. ReShade remains a third-party overlay; this never patches
    /// its binaries.
    pub reshade_theme: bool,
}

impl Default for Performance {
    fn default() -> Self {
        Performance {
            controller: true,
            mode: "Automatic".into(),
            target_fps: 60,
            stability: true,
            scene_profiles: true,
            scene_override: "Auto".into(),
            adaptive_mip: false,
            max_mip_bias: 0.75,
            staging_ring_mb: 256,
            pool_upload_ms: 6,
            tex_prep: true,
            tex_prep_threads: 0,
            tex_prep_mb: 1024,
            residency: "Automatic".into(),
            residency_percent: 80,
            lean_draws: true,
            prewarm: true,
            gpl: true,
            reshade_theme: false,
        }
    }
}

impl Performance {
    pub fn normalize(&mut self) {
        const MODES: [&str; 5] = ["Automatic", "Conservative", "Balanced", "Aggressive", "Manual"];
        const SCENES: [&str; 7] = ["Auto", "Exploration", "Crowded", "Combat", "Cinematic", "Menu", "Loading"];
        const RESIDENCY: [&str; 3] = ["Automatic", "On", "Off"];
        if !MODES.iter().any(|m| m.eq_ignore_ascii_case(&self.mode)) {
            self.mode = "Automatic".into();
        } else {
            let i = MODES.iter().position(|m| m.eq_ignore_ascii_case(&self.mode)).unwrap_or(0);
            self.mode = MODES[i].into();
        }
        if !SCENES.iter().any(|s| s.eq_ignore_ascii_case(&self.scene_override)) {
            self.scene_override = "Auto".into();
        } else {
            let i = SCENES.iter().position(|s| s.eq_ignore_ascii_case(&self.scene_override)).unwrap_or(0);
            self.scene_override = SCENES[i].into();
        }
        if !RESIDENCY.iter().any(|r| r.eq_ignore_ascii_case(&self.residency)) {
            self.residency = "Automatic".into();
        } else {
            let i = RESIDENCY.iter().position(|r| r.eq_ignore_ascii_case(&self.residency)).unwrap_or(0);
            self.residency = RESIDENCY[i].into();
        }
        self.target_fps = self.target_fps.clamp(24, 360);
        if !self.max_mip_bias.is_finite() {
            self.max_mip_bias = 0.75;
        }
        self.max_mip_bias = self.max_mip_bias.clamp(0.25, 2.0);
        self.staging_ring_mb = self.staging_ring_mb.min(1024);
        self.pool_upload_ms = self.pool_upload_ms.min(24);
        self.tex_prep_threads = self.tex_prep_threads.min(8);
        self.tex_prep_mb = self.tex_prep_mb.clamp(128, 4096);
        self.residency_percent = self.residency_percent.clamp(50, 100);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Audio {
    pub device: String,
    pub master: f32,
    pub music: f32,
    pub effects: f32,
    pub voice: f32,
    pub buffer_ms: u32,
    /// The engine's software EQ on the final mix (`[audio] eq`). The F1 menu can
    /// change it live; this is the value written before the next start.
    pub eq: bool,
    /// A preset name from [`EQ_PRESETS`], or `custom` when the bands were moved
    /// by hand in the game's F1 menu.
    pub eq_preset: String,
    /// Ten gains in dB, 31 Hz through 16 kHz, matching the engine's band table.
    pub eq_bands: Vec<f32>,
    /// The preset's extra preamp. `None` leaves the game's existing
    /// `eq_preamp` alone, so adding this field to old configs does not reset a
    /// preamp tuned in the F1 menu.
    pub eq_preamp: Option<f32>,
}

impl Default for Audio {
    fn default() -> Self {
        Audio {
            device: "System default".into(),
            master: 1.0,
            music: 0.85,
            effects: 1.0,
            voice: 1.0,
            buffer_ms: 40,
            eq: false,
            eq_preset: "flat".into(),
            eq_bands: vec![0.0; EQ_BAND_COUNT],
            eq_preamp: None,
        }
    }
}

impl Audio {
    /// Keep an older `config.json` (or a hand-edited one) from writing a key the
    /// engine would reject: always a known preset key or `custom`, always ten
    /// finite band gains, and a master volume inside 0..200%.
    pub fn normalize(&mut self) {
        self.master = if self.master.is_finite() { self.master.clamp(0.0, 2.0) } else { 1.0 };
        let key = self.eq_preset.trim().to_ascii_lowercase().replace(' ', "-").replace('_', "-");
        self.eq_preset = if key.is_empty() {
            "flat".into()
        } else if key == "custom" || eq_preset_label(&key) != "Custom" {
            key
        } else {
            "custom".into()
        };
        if self.eq_bands.len() != EQ_BAND_COUNT {
            self.eq_bands = eq_preset_bands(&self.eq_preset).to_vec();
        }
        for db in &mut self.eq_bands {
            *db = if db.is_finite() { db.clamp(-12.0, 12.0) } else { 0.0 };
        }
        if let Some(db) = self.eq_preamp {
            self.eq_preamp = Some(if db.is_finite() { db.clamp(-12.0, 12.0) } else { 0.0 });
        }
    }
}

/// The ten slider positions the engine's EQ exposes: 31, 62, 125, 250, 500,
/// 1k, 2k, 4k, 8k and 16k Hz. Kept in step with `nxrt/audio_eq.h`.
pub const EQ_BAND_COUNT: usize = 10;

pub type EqCurve = [f32; EQ_BAND_COUNT];

/// `(key, label, curve, preamp)`. Keys are what `[audio] eq_preset` stores; the
/// labels are what the Audio page shows. The curves and preamps match the
/// engine's presets.
pub const EQ_PRESETS: [(&str, &str, EqCurve, f32); 8] = [
    ("flat", "Flat", [0.0; EQ_BAND_COUNT], 0.0),
    ("bass", "Bass", [7.0, 6.0, 5.0, 3.0, 1.0, 0.0, -1.0, -2.0, -3.0, -4.0], 0.0),
    ("treble", "Treble", [-3.0, -2.0, -1.0, 0.0, 0.0, 1.0, 3.0, 5.0, 6.0, 7.0], 0.0),
    ("vocal", "Vocal", [-4.0, -2.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0], 0.0),
    ("warm", "Warm", [3.0, 2.0, 1.0, 1.0, 0.0, 0.0, -1.0, -1.0, -2.0, -3.0], 0.0),
    ("v-shape", "V-Shape", [6.0, 5.0, 3.0, 0.0, -2.0, -2.0, 0.0, 3.0, 5.0, 6.0], 0.0),
    ("night", "Night", [-6.0, -4.0, -2.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 1.0], 0.0),
    (
        "headphones-boost",
        "Headphones Boost",
        [9.5, 6.0, 5.0, -2.5, 1.0, 0.0, -1.0, 7.0, 5.0, 9.5],
        -12.0,
    ),
];

/// The curve for a preset key, or a flat one for `custom`/an unknown key.
pub fn eq_preset_bands(key: &str) -> EqCurve {
    let key = key.trim().to_ascii_lowercase().replace(' ', "-").replace('_', "-");
    EQ_PRESETS
        .iter()
        .find(|(name, _, _, _)| *name == key)
        .map(|(_, _, curve, _)| *curve)
        .unwrap_or([0.0; EQ_BAND_COUNT])
}


/// The page label for a preset key. An unknown key is shown as Custom, which is
/// what the F1 menu writes after a band is moved by hand.
pub fn eq_preset_label(key: &str) -> &'static str {
    let key = key.trim().to_ascii_lowercase().replace(' ', "-").replace('_', "-");
    EQ_PRESETS
        .iter()
        .find(|(name, _, _, _)| *name == key)
        .map(|(_, label, _, _)| *label)
        .unwrap_or("Custom")
}

/// One action the game understands and the host keys and buttons bound to it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Binding {
    pub action: String,
    pub key: String,
    pub pad: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Input {
    pub profile: String,
    pub deadzone: f32,
    pub vibration: f32,
    pub bindings: Vec<Binding>,
}

pub fn default_bindings() -> Vec<Binding> {
    let b = |action: &str, key: &str, pad: &str| Binding {
        action: action.into(),
        key: key.into(),
        pad: pad.into(),
    };
    vec![
        b("Confirm", "Enter", "A / Cross"),
        b("Cancel", "Escape", "B / Circle"),
        b("Menu", "Tab", "X / Square"),
        b("Unit details", "Q", "Y / Triangle"),
        b("Previous unit", "PageUp", "L / L1"),
        b("Next unit", "PageDown", "R / R1"),
        b("Danger zone", "Z", "ZL / L2"),
        b("Fast forward", "Space", "ZR / R2"),
        b("End turn", "E", "Plus / Options"),
        b("Map", "M", "Minus / View"),
        b("Cursor up", "W", "D-pad up / Left stick"),
        b("Cursor down", "S", "D-pad down / Left stick"),
        b("Cursor left", "A", "D-pad left / Left stick"),
        b("Cursor right", "D", "D-pad right / Left stick"),
        b("Camera", "Mouse drag", "Right stick"),
    ]
}

impl Default for Input {
    fn default() -> Self {
        Input {
            profile: "Default".into(),
            deadzone: 0.15,
            vibration: 0.8,
            bindings: default_bindings(),
        }
    }
}

/// Anonymised issue reports (src/issue_upload.rs): sent only when `send` is on and a server is named.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Reports {
    /// The player agreed, on About > Issue reports, after reading what is sent.
    pub send: bool,
    /// The report server (`https://…`). Empty: `EW_REPORT_URL`, else none, and nothing is sent.
    pub endpoint: String,
}

/// Update checking and the launcher's own overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Updates {
    /// Ask GitHub for a newer release when the launcher starts.
    pub check: bool,
    /// Ask GameBanana for newer uploads of installed mods when the Mods page
    /// opens. Off by default: the page's Check for updates button asks on demand.
    pub mods: bool,
    /// The tag the user turned down, so a dismissed update is not offered
    /// again on every start.
    pub skipped: String,
}

impl Default for Updates {
    fn default() -> Self {
        Updates {
            check: true,
            mods: false,
            skipped: String::new(),
        }
    }
}

/// Discord Rich Presence while the launcher or game runs. The application id is
/// built into the launcher; this only stores whether the player wants presence.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Discord {
    pub enabled: bool,
}

impl Default for Discord {
    fn default() -> Self {
        Discord {
            // On by default. Presence is a no-op when Discord is not running.
            enabled: true,
        }
    }
}

/// Which of the launcher's own readouts are shown.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hud {
    /// The frame-time graph on the Diagnostics page.
    pub graph: bool,
}

impl Default for Hud {
    fn default() -> Self {
        Hud { graph: true }
    }
}

/// Navigation presentation the player has chosen. Groups remember whether
/// they are open and the rail can stay compact, so the same layout comes back
/// at the next launch instead of resetting itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NavState {
    /// Icon-only rail instead of the labelled sidebar.
    pub rail: bool,
    /// Group keys that are folded away.
    pub collapsed: Vec<String>,
}

impl Default for NavState {
    fn default() -> Self {
        NavState {
            rail: false,
            collapsed: vec!["system".into(), "settings".into()],
        }
    }
}

/// How much of the launcher the player wants to see. Both modes operate on the
/// same configuration and installation; the mode only changes navigation and
/// the amount of detail a page puts in front of the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InterfaceMode {
    /// The default for a new installation: Play, setup and everyday settings.
    #[default]
    Standard,
    /// Every diagnostic, build and engine control, for players who want them.
    Advanced,
}

impl InterfaceMode {
    pub fn label(self) -> &'static str {
        match self {
            InterfaceMode::Standard => "Standard",
            InterfaceMode::Advanced => "Advanced",
        }
    }

    pub fn detail(self) -> &'static str {
        match self {
            InterfaceMode::Standard => "Play, setup, saves, mods and everyday graphics and audio settings.",
            InterfaceMode::Advanced => "Everything in Standard, plus builds, diagnostics, logs and engine options.",
        }
    }
}

/// A named set of mods to switch on together.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModProfile {
    pub name: String,
    /// Names of the mods that are on in this profile (without the `_` a
    /// switched-off mod carries).
    pub enabled: Vec<String>,
}

/// The Low Memory profile's state: what the user had before it was applied, and
/// what it set, so the previous settings can be restored without touching any
/// section the preset does not own.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GraphicsPreset {
    pub low_memory: bool,
    /// The recommendation was shown and the user said not now; it stays quiet
    /// until it is applied or the card is opened again.
    pub dismissed: bool,
    pub previous: Option<Graphics>,
    pub applied: Option<Graphics>,
}

/// The guided setup's durable position. `step` is the guide's own page, not a
/// claim about the machine: every screen re-checks the real game data, tools
/// and build before it offers to continue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Onboarding {
    pub completed: bool,
    /// Closed without finishing. It stays quiet until Overview's setup card or
    /// Game Setup asks for it again.
    pub dismissed: bool,
    pub step: u8,
    /// Set on the first launch after this field exists, so an older config with
    /// an already chosen game folder is not greeted with first-run setup.
    pub introduced: bool,
}

impl Default for Onboarding {
    fn default() -> Self {
        Onboarding {
            completed: false,
            dismissed: false,
            step: 0,
            introduced: false,
        }
    }
}

/// Thresholds for the Performance Lab's read-only pressure indicator. GPU
/// thresholds are percentage of the driver budget in use; host thresholds are
/// percentage of installed RAM still available. They are deliberately
/// configurable because the right values differ by GPU and workload; the
/// defaults are conservative starting points, not experimentally universal
/// limits. The Lab never changes these values by itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Lab {
    pub gpu_elevated: u32,
    pub gpu_high: u32,
    pub gpu_critical: u32,
    pub host_elevated: u32,
    pub host_high: u32,
    pub host_critical: u32,
}

impl Default for Lab {
    fn default() -> Self {
        Lab {
            gpu_elevated: 70,
            gpu_high: 85,
            gpu_critical: 95,
            host_elevated: 20,
            host_high: 10,
            host_critical: 5,
        }
    }
}

impl Lab {
    /// Keep the thresholds ordered and inside 1..=100, so a hand-edited
    /// config cannot make "critical" easier than "elevated".
    pub fn normalize(&mut self) {
        let mut gpu = [self.gpu_elevated, self.gpu_high, self.gpu_critical];
        gpu.sort_unstable();
        for v in &mut gpu {
            *v = (*v).clamp(1, 100);
        }
        if gpu[0] == gpu[1] || gpu[1] == gpu[2] {
            let d = Lab::default();
            gpu = [d.gpu_elevated, d.gpu_high, d.gpu_critical];
        }
        self.gpu_elevated = gpu[0];
        self.gpu_high = gpu[1];
        self.gpu_critical = gpu[2];

        let mut host = [self.host_elevated, self.host_high, self.host_critical];
        host.sort_unstable_by(|a, b| b.cmp(a));
        for v in &mut host {
            *v = (*v).clamp(1, 100);
        }
        if host[0] == host[1] || host[1] == host[2] {
            let d = Lab::default();
            host = [d.host_elevated, d.host_high, d.host_critical];
        }
        self.host_elevated = host[0];
        self.host_high = host[1];
        self.host_critical = host[2];
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub paths: Paths,
    pub system: System,
    pub runtime: Runtime,
    pub graphics: Graphics,
    pub performance: Performance,
    pub lab: Lab,
    pub audio: Audio,
    pub input: Input,
    pub theme: Scheme,
    /// The launcher's own language. The game's text and voice language is
    /// `system.language`, reported to the runtime through nn::oe.
    pub ui_language: Language,
    pub updates: Updates,
    /// Opt-in issue reports after a run that went wrong (src/issue_upload.rs). Off, and no server, by default.
    pub reports: Reports,
    pub discord: Discord,
    pub graphics_preset: GraphicsPreset,
    pub hud: Hud,
    pub nav: NavState,
    pub profiles: Vec<ModProfile>,
    /// The profile last applied, for the Mods page.
    pub profile: String,
    /// User-chosen labels for save slots, keyed by profile and then by the
    /// slot path inside that profile. These are display names only: the game
    /// file name and contents are never changed.
    pub slot_names: BTreeMap<String, BTreeMap<String, String>>,
    /// The protagonist portrait chosen for a save profile. `cai`, `dietrich`,
    /// `eshmel`, `leda` or `theodora`; absent means Automatic, which reads the
    /// newest readable slot. Display-only, like the slot labels above.
    pub profile_portraits: BTreeMap<String, String>,
    /// First-run guided setup progress.
    pub onboarding: Onboarding,
    /// How much of the launcher is shown. Standard is the default for a fresh
    /// install; an older config is migrated to Advanced in `load` so existing
    /// users keep the interface they already know.
    pub interface_mode: InterfaceMode,
    /// Hide EmblemWeave when the game starts; the taskbar restores it when the
    /// player comes back. Off by default, so a new install keeps the launcher
    /// visible unless the player asks for this.
    pub minimize_on_play: bool,
    /// Show the runtime's debug console window (fefw.exe). Off by default to
    /// avoid an unwanted console window; on displays the console for troubleshooting.
    /// Logging to fefw.log continues regardless of this setting.
    pub show_debug_console: bool,
    /// Built engine checkouts (as `engine.txt` records them) whose reuse offer was declined. The offer for one folder does not
    /// hide an offer for another, and a declined one stays declined across restarts.
    pub reuse_dismissed: Vec<String>,
}

/// Keep an existing user's full launcher after this field is introduced. A
/// config written before interface modes existed has no `interface_mode` key;
/// someone already using the full launcher should not suddenly lose pages. A
/// genuinely new install has no file and starts with `Config::default`.
fn apply_saved_mode(text: &str, cfg: &mut Config) {
    let has_mode = serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|value| value.get("interface_mode").cloned())
        .is_some();
    if !has_mode {
        cfg.interface_mode = InterfaceMode::Advanced;
    }
}

impl Config {
    pub fn dir() -> Option<PathBuf> {
        // %APPDATA%\EmblemWeave on Windows, $XDG_CONFIG_HOME/EmblemWeave (else ~/.config/EmblemWeave)
        // elsewhere: platform.rs is the one place that decides, so a build for another platform reads
        // and writes a folder it can actually reach.
        crate::platform::data_root()
    }

    pub fn path() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("config.json"))
    }

    /// Load the config. A missing file is a fresh install; a file that cannot
    /// be parsed is reported, kept on disk under a `.bad` name, and replaced
    /// by defaults so the launcher still starts.
    pub fn load() -> (Config, Option<String>) {
        let Some(path) = Self::path() else {
            return (
                Config::default(),
                Some("the launcher's data folder is not set; settings will not persist".into()),
            );
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                match serde_json::from_str::<Config>(&text) {
                                        Ok(mut cfg) => {
                        apply_saved_mode(&text, &mut cfg);
                        cfg.audio.normalize();
                        cfg.performance.normalize();
                        // The DLSS replacement's FidelityFX entry was called "FSR 3" before it named the provider it runs.
                        if cfg.graphics.dlss_upscaler == "FSR 3" {
                            cfg.graphics.dlss_upscaler = "FSR 4 / 3 (FidelityFX)".into();
                        }
                        (cfg, None)
                    }
                    Err(e) => {
                        let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                        (Config::default(), Some(format!("config.json could not be read ({e}); it was kept as config.json.bad")))
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
            Err(e) => (
                Config::default(),
                Some(format!("config.json could not be opened: {e}")),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or(crate::platform::NO_DATA_DIR)?;
        let dir = path.parent().ok_or("config path has no parent")?;
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("replacing {}: {e}", path.display()))
    }

    /// Writable directory for save data, kept apart from the user's dump.
    pub fn save_dir() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("save"))
    }

    /// Writable directory for decoded modules and lift output.
    pub fn work_dir() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("work"))
    }

    /// Apply the Low Memory graphics profile, keeping the settings it replaces.
    /// Returns the settings that actually changed, for the page to name. The
    /// first application snapshots every graphics setting; later applications
    /// keep the original snapshot, so one press restores the player's setup.
    pub fn apply_low_memory(&mut self) -> Vec<String> {
        if !self.graphics_preset.low_memory {
            self.graphics_preset.previous = Some(self.graphics.clone());
        }
        let next = crate::memory::low_memory(&self.graphics);
        let changes = crate::memory::changes(&self.graphics, &next);
        self.graphics = next.clone();
        self.graphics_preset.applied = Some(next);
        self.graphics_preset.low_memory = true;
        self.graphics_preset.dismissed = false;
        changes
    }

    /// Keep the recommendation quiet for this profile. It changes no setting.
    pub fn dismiss_low_memory(&mut self) {
        self.graphics_preset.dismissed = true;
    }

    /// Put back every graphics setting from before the first Low Memory apply.
    pub fn restore_low_memory(&mut self) -> bool {
        let Some(previous) = self.graphics_preset.previous.take() else {
            return false;
        };
        self.graphics = previous;
        self.graphics_preset.low_memory = false;
        self.graphics_preset.applied = None;
        true
    }

    /// Whether a setting changed after the profile was applied, which a restore
    /// would overwrite. The page asks before it does that.
    pub fn low_memory_drifted(&self) -> bool {
        self.graphics_preset.low_memory
            && self
                .graphics_preset
                .applied
                .as_ref()
                .is_some_and(|a| *a != self.graphics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_config_without_new_sections_still_loads() {
        let cfg: Config = serde_json::from_str(r#"{"theme":"emblem"}"#).unwrap();
        assert_eq!(cfg.theme, Scheme::Fe);
        assert_eq!(cfg.system.language, "en-US");
        assert_eq!(cfg.ui_language, Language::English);
        assert_eq!(cfg.input.bindings.len(), default_bindings().len());
    }

    #[test]
    fn audio_eq_normalizes_old_and_hand_edited_values() {
        let mut a = Audio::default();
        a.master = 5.0;
        a.eq_preset = " BASS ".into();
        a.eq_bands.clear();
        a.normalize();
        assert_eq!(a.master, 2.0);
        assert_eq!(a.eq_preset, "bass");
        assert_eq!(a.eq_bands, eq_preset_bands("bass").to_vec());

        a.eq_preset = "something_else".into();
        a.eq_bands = vec![99.0; EQ_BAND_COUNT];
        a.normalize();
        assert_eq!(a.eq_preset, "custom");
        assert!(a.eq_bands.iter().all(|db| *db == 12.0));
    }

    #[test]
    fn config_round_trips() {
        let mut cfg = Config::default();
        cfg.system.user_name = "XiØN".into();
        cfg.graphics.frame_limit = 120;
        cfg.interface_mode = InterfaceMode::Advanced;
        cfg.ui_language = Language::Japanese;
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.system.user_name, "XiØN");
        assert_eq!(back.graphics.frame_limit, 120);
        assert_eq!(back.interface_mode, InterfaceMode::Advanced);
        assert_eq!(back.ui_language, Language::Japanese);
    }

    #[test]
    fn a_new_config_starts_in_standard_mode() {
        assert_eq!(Config::default().interface_mode, InterfaceMode::Standard);
    }

    #[test]
    fn a_config_from_before_interface_modes_keeps_the_advanced_interface() {
        let mut old = Config::default();
        apply_saved_mode(r#"{"system":{},"graphics":{}}"#, &mut old);
        assert_eq!(old.interface_mode, InterfaceMode::Advanced);

        let mut current = Config::default();
        apply_saved_mode(r#"{"interface_mode":"standard"}"#, &mut current);
        assert_eq!(current.interface_mode, InterfaceMode::Standard);
    }

    #[test]
    fn show_debug_console_defaults_to_false() {
        let cfg = Config::default();
        assert!(!cfg.show_debug_console, "show_debug_console should default to false");
    }

    #[test]
    fn show_debug_console_round_trips() {
        let mut cfg = Config::default();
        cfg.show_debug_console = true;
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert!(back.show_debug_console);
    }

    #[test]
    fn an_old_config_without_show_debug_console_defaults_to_false() {
        // Config from before this field existed should default to false
        let cfg: Config = serde_json::from_str(r#"{"theme":"emblem"}"#).unwrap();
        assert!(!cfg.show_debug_console);
    }
}
