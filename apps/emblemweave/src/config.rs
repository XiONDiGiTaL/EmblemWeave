//! Persistent launcher settings.
//!
//! Stored as JSON at `%APPDATA%\EmblemWeave\config.json`. Every section has a
//! serde default, so a file written by an older build loads with the new
//! fields filled in instead of being discarded. Writes go to a temporary file
//! first and are renamed into place, so a crash mid-write cannot leave a
//! truncated config behind.

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
            // Two workers, below idle and in efficiency mode: a laptop stays a laptop. Players who want the
            // fastest first draw can raise this on Graphics.
            compile_threads: 2,
            compile_priority: "Idle".into(),
            compile_eco: true,
        }
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
        }
    }
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

/// Update checking and the launcher's own overlay.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Updates {
    /// Ask GitHub for a newer release when the launcher starts.
    pub check: bool,
    /// The tag the user turned down, so a dismissed update is not offered
    /// again on every start.
    pub skipped: String,
}

impl Default for Updates {
    fn default() -> Self {
        Updates {
            check: true,
            skipped: String::new(),
        }
    }
}

/// Discord Rich Presence while the game runs. The client id is the public
/// application id from Discord's Developer Portal; it is not a secret and no
/// token is stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Discord {
    pub enabled: bool,
    pub client_id: String,
}

impl Default for Discord {
    fn default() -> Self {
        Discord {
            enabled: false,
            client_id: String::new(),
        }
    }
}

/// Which of the launcher's own readouts are shown.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hud {
    /// The frame rate chip in the top bar.
    pub fps: bool,
    /// The frame-time graph on the Diagnostics page.
    pub graph: bool,
}

impl Default for Hud {
    fn default() -> Self {
        Hud {
            fps: false,
            graph: true,
        }
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub paths: Paths,
    pub system: System,
    pub runtime: Runtime,
    pub graphics: Graphics,
    pub audio: Audio,
    pub input: Input,
    pub theme: Scheme,
    pub updates: Updates,
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
    /// Built engine checkouts (as `engine.txt` records them) whose reuse offer was declined. The offer for one folder does not
    /// hide an offer for another, and a declined one stays declined across restarts.
    pub reuse_dismissed: Vec<String>,
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
                match serde_json::from_str(&text) {
                    Ok(cfg) => (cfg, None),
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
        assert_eq!(cfg.input.bindings.len(), default_bindings().len());
    }

    #[test]
    fn config_round_trips() {
        let mut cfg = Config::default();
        cfg.system.user_name = "XiØN".into();
        cfg.graphics.frame_limit = 120;
        let back: Config = serde_json::from_str(&serde_json::to_string(&cfg).unwrap()).unwrap();
        assert_eq!(back.system.user_name, "XiØN");
        assert_eq!(back.graphics.frame_limit, 120);
    }
}
