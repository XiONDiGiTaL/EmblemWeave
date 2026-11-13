//! Persistent launcher settings.
//!
//! Stored as JSON at `%APPDATA%\EmblemWeave\config.json`. Every section has a
//! serde default, so a file written by an older build loads with the new
//! fields filled in instead of being discarded. Writes go to a temporary file
//! first and are renamed into place, so a crash mid-write cannot leave a
//! truncated config behind.

use crate::theme::Scheme;
use serde::{Deserialize, Serialize};
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
        System { language: "en-US".into(), user_name: "Player".into() }
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
        Runtime { max_steps: 50_000_000, heap_mib: 4096 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Graphics {
    pub resolution: String,
    pub display_mode: String,
    pub vsync: bool,
    pub frame_limit: u32,
    pub aspect: String,
    pub render_scale: f32,
    pub upscaling: String,
    pub quality: String,
    pub shader_precompile: bool,
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
            quality: "Original".into(),
            shader_precompile: true,
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
        Audio { device: "System default".into(), master: 1.0, music: 0.85, effects: 1.0, voice: 1.0, buffer_ms: 40 }
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
    let b = |action: &str, key: &str, pad: &str| Binding { action: action.into(), key: key.into(), pad: pad.into() };
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
        Input { profile: "Default".into(), deadzone: 0.15, vibration: 0.8, bindings: default_bindings() }
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
        Updates { check: true, skipped: String::new() }
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
        Hud { fps: false, graph: true }
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
    pub hud: Hud,
    pub profiles: Vec<ModProfile>,
    /// The profile last applied, for the Mods page.
    pub profile: String,
}

impl Config {
    pub fn dir() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|base| PathBuf::from(base).join("EmblemWeave"))
    }

    pub fn path() -> Option<PathBuf> {
        Self::dir().map(|d| d.join("config.json"))
    }

    /// Load the config. A missing file is a fresh install; a file that cannot
    /// be parsed is reported, kept on disk under a `.bad` name, and replaced
    /// by defaults so the launcher still starts.
    pub fn load() -> (Config, Option<String>) {
        let Some(path) = Self::path() else {
            return (Config::default(), Some("APPDATA is not set; settings will not persist".into()));
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(cfg) => (cfg, None),
                Err(e) => {
                    let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                    (Config::default(), Some(format!("config.json could not be read ({e}); it was kept as config.json.bad")))
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), None),
            Err(e) => (Config::default(), Some(format!("config.json could not be opened: {e}"))),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("APPDATA is not set")?;
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
