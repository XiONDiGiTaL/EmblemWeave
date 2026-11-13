//! The controller the Controls page listens to, and the pad settings it writes
//! into the engine's `controls.ini`.
//!
//! The game takes player 1 from the first XInput gamepad and reads its pad
//! settings from the `[pad]` section of `controls.ini` in its play folder
//! (`engine/fefw-recomp/runtime/docs/CONFIG.md`): `deadzone`, `right_deadzone`,
//! `stick_max` and `trigger_threshold`. The page shows the pad as it is and
//! writes those four keys where the game looks for them.

use std::path::Path;

/// XInput button bits, as the pad reports them. The face buttons are named by
/// position, which is how the game maps them: the bottom one is the console's
/// B, the right one its A, the left one Y and the top one X.
pub mod button {
    pub const DPAD_UP: u16 = 0x0001;
    pub const DPAD_DOWN: u16 = 0x0002;
    pub const DPAD_LEFT: u16 = 0x0004;
    pub const DPAD_RIGHT: u16 = 0x0008;
    pub const START: u16 = 0x0010;
    pub const BACK: u16 = 0x0020;
    pub const LEFT_SHOULDER: u16 = 0x0100;
    pub const RIGHT_SHOULDER: u16 = 0x0200;
    pub const SOUTH: u16 = 0x1000;
    pub const EAST: u16 = 0x2000;
    pub const WEST: u16 = 0x4000;
    pub const NORTH: u16 = 0x8000;
}

/// A pad as it is this frame. Sticks run -1..1 with up positive, triggers 0..1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Live {
    /// The XInput slot, 0 for the first pad.
    pub slot: u32,
    pub buttons: u16,
    pub left: (f32, f32),
    pub right: (f32, f32),
    pub lt: f32,
    pub rt: f32,
}

impl Live {
    pub fn held(&self, bit: u16) -> bool {
        bit != 0 && self.buttons & bit != 0
    }
}

/// The first connected pad, if any: the one the game will use.
#[cfg(windows)]
pub fn poll() -> Option<Live> {
    use windows_sys::Win32::UI::Input::XboxController::*;
    for slot in 0..4 {
        let mut st: XINPUT_STATE = unsafe { std::mem::zeroed() };
        if unsafe { XInputGetState(slot, &mut st) } != 0 {
            continue;
        }
        let g = st.Gamepad;
        let axis = |v: i16| (v as f32 / 32767.0).clamp(-1.0, 1.0);
        return Some(Live {
            slot,
            buttons: g.wButtons,
            left: (axis(g.sThumbLX), axis(g.sThumbLY)),
            right: (axis(g.sThumbRX), axis(g.sThumbRY)),
            lt: g.bLeftTrigger as f32 / 255.0,
            rt: g.bRightTrigger as f32 / 255.0,
        });
    }
    None
}

#[cfg(not(windows))]
pub fn poll() -> Option<Live> {
    None
}

/// The `[pad]` values the game reads, as the page shows them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tuning {
    pub left_deadzone: f32,
    pub right_deadzone: f32,
    pub stick_max: f32,
    pub trigger: f32,
}

impl Default for Tuning {
    /// The engine's own defaults (`play_types.h`).
    fn default() -> Self {
        Tuning { left_deadzone: 0.10, right_deadzone: 0.10, stick_max: 1.0, trigger: 0.35 }
    }
}

/// Read `key` in `[section]` of ini text, without its comment.
pub fn value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut in_section = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_section = l.trim_matches(['[', ']']).eq_ignore_ascii_case(section);
        } else if in_section {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.split([';', '#']).next().unwrap_or("").trim());
                }
            }
        }
    }
    None
}

impl Tuning {
    /// The settings in `text` (a `controls.ini`), with the engine's rules: a
    /// number outside its range is ignored and the default kept, and a file
    /// with `deadzone` but no `right_deadzone` gives the right stick the left
    /// stick's value. `seed` is used for the left stick when the file has no
    /// `deadzone` at all (a launcher preference saved before the game ever
    /// wrote the file).
    pub fn from_ini(text: &str, seed: f32) -> Tuning {
        let num = |key: &str| value(text, "pad", key).and_then(|v| v.parse::<f32>().ok());
        let mut t = Tuning { left_deadzone: seed.clamp(0.0, 0.5), ..Tuning::default() };
        if let Some(v) = num("deadzone").filter(|v| (0.0..=0.5).contains(v)) {
            t.left_deadzone = v;
        }
        // The engine gives the right stick the left stick's value when only
        // `deadzone` is in the file.
        t.right_deadzone = t.left_deadzone;
        if let Some(v) = num("right_deadzone").filter(|v| (0.0..=0.5).contains(v)) {
            t.right_deadzone = v;
        }
        if let Some(v) = num("stick_max").filter(|v| (0.5..=1.0).contains(v)) {
            t.stick_max = v;
        }
        if let Some(v) = num("trigger_threshold").filter(|v| (0.05..=1.0).contains(v)) {
            t.trigger = v;
        }
        t
    }

    /// The game's `controls.ini` at `path`, or the defaults when it is not
    /// there yet.
    pub fn load(path: &Path, seed: f32) -> Tuning {
        match std::fs::read_to_string(path) {
            Ok(text) => Tuning::from_ini(&text, seed),
            Err(_) => Tuning { left_deadzone: seed.clamp(0.0, 0.5), ..Tuning::default() },
        }
    }
}

/// Write one `[pad]` value into the game's `controls.ini`, keeping every other
/// line of the file as it was.
pub fn write(path: &Path, key: &str, value: f32) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    crate::ingame::ini_set(path, "pad", key, &format!("{value:.2}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_file_gives_the_engine_defaults() {
        assert_eq!(Tuning::from_ini("", 0.10), Tuning::default());
        assert_eq!(Tuning::from_ini("[pad]\n", 0.10), Tuning::default());
    }

    #[test]
    fn keys_are_read_and_clamped_like_the_engine() {
        let t = Tuning::from_ini("[pad]\ndeadzone = 0.25\nstick_max = 0.85\ntrigger_threshold = 0.50\n", 0.10);
        assert_eq!(t.left_deadzone, 0.25);
        assert_eq!(t.stick_max, 0.85);
        assert_eq!(t.trigger, 0.50);
        // Out of range and unreadable values keep the default.
        let bad = Tuning::from_ini("[pad]\ndeadzone = 0.7\nstick_max = x\ntrigger_threshold = 0\n", 0.10);
        assert_eq!(bad, Tuning::default());
    }

    #[test]
    fn the_right_stick_falls_back_to_the_left_dead_zone() {
        let t = Tuning::from_ini("[pad]\ndeadzone = 0.30\n", 0.10);
        assert_eq!((t.left_deadzone, t.right_deadzone), (0.30, 0.30));
        let t = Tuning::from_ini("[pad]\ndeadzone = 0.30\nright_deadzone = 0.05\n", 0.10);
        assert_eq!((t.left_deadzone, t.right_deadzone), (0.30, 0.05));
    }

    #[test]
    fn values_ignore_case_and_comments() {
        let text = "[Pad]\nDeadZone = 0.20 ; the left stick\n";
        assert_eq!(value(text, "pad", "deadzone"), Some("0.20"));
        assert_eq!(value(text, "pad", "missing"), None);
        assert_eq!(value("[mouse]\ndeadzone = 0.1\n", "pad", "deadzone"), None);
    }

    #[test]
    fn writing_lands_where_the_game_reads_it() {
        let path = std::env::temp_dir().join(format!("ew-pad-{}.ini", std::process::id()));
        std::fs::write(&path, "[display]\nwindow_mode = windowed\n\n[pad]\ndeadzone = 0.10\n").unwrap();
        write(&path, "stick_max", 0.90).unwrap();
        write(&path, "trigger_threshold", 0.50).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let t = Tuning::from_ini(&text, 0.10);
        assert_eq!((t.stick_max, t.trigger), (0.90, 0.50));
        // Everything else in the file survives.
        assert_eq!(value(&text, "display", "window_mode"), Some("windowed"));
        assert_eq!(value(&text, "pad", "deadzone"), Some("0.10"));
        // A file that does not exist yet is created with the section.
        let fresh = std::env::temp_dir().join(format!("ew-pad-new-{}.ini", std::process::id()));
        let _ = std::fs::remove_file(&fresh);
        write(&fresh, "deadzone", 0.20).unwrap();
        assert_eq!(value(&std::fs::read_to_string(&fresh).unwrap(), "pad", "deadzone"), Some("0.20"));
        let _ = std::fs::remove_file(&fresh);
        let _ = std::fs::remove_file(&path);
    }
}
