//! `nn::hid`: controllers.
//!
//! The host presents one controller: a Pro Controller (the full-key style) as
//! player 1. Its state comes from [`input`], which the host fills from the
//! first XInput gamepad on Windows and which a front end may also set. With no
//! gamepad attached the controller stays connected with nothing pressed, the
//! same as a console with an idle controller. Handheld and dual Joy-Con
//! report disconnected; motion sensors, vibration and the Joy-Con mouse report
//! no devices.
//!
//! Button bits, stick ranges and the state layout are the console's published
//! interface: `NpadFullKeyState` is a sampling number, a 64-bit button set,
//! two sticks of two 32-bit axes, and a 32-bit attribute set.

use crate::hle::{write_bytes, HostResult, HostServices};
use ew_lift::State;
use std::sync::Mutex;

/// Button bits of `nn::hid::NpadButton`.
pub mod button {
    pub const A: u64 = 1 << 0;
    pub const B: u64 = 1 << 1;
    pub const X: u64 = 1 << 2;
    pub const Y: u64 = 1 << 3;
    pub const STICK_L: u64 = 1 << 4;
    pub const STICK_R: u64 = 1 << 5;
    pub const L: u64 = 1 << 6;
    pub const R: u64 = 1 << 7;
    pub const ZL: u64 = 1 << 8;
    pub const ZR: u64 = 1 << 9;
    pub const PLUS: u64 = 1 << 10;
    pub const MINUS: u64 = 1 << 11;
    pub const LEFT: u64 = 1 << 12;
    pub const UP: u64 = 1 << 13;
    pub const RIGHT: u64 = 1 << 14;
    pub const DOWN: u64 = 1 << 15;
    pub const STICK_L_LEFT: u64 = 1 << 16;
    pub const STICK_L_UP: u64 = 1 << 17;
    pub const STICK_L_RIGHT: u64 = 1 << 18;
    pub const STICK_L_DOWN: u64 = 1 << 19;
    pub const STICK_R_LEFT: u64 = 1 << 20;
    pub const STICK_R_UP: u64 = 1 << 21;
    pub const STICK_R_RIGHT: u64 = 1 << 22;
    pub const STICK_R_DOWN: u64 = 1 << 23;
}

/// `NpadStyleTag` bits.
const STYLE_FULL_KEY: u32 = 1 << 0;
/// `NpadAttribute`: connected, wired.
const ATTR_CONNECTED: u32 = 1 << 0;
const ATTR_WIRED: u32 = 1 << 1;
/// `NpadId` values.
const NPAD_NO1: u32 = 0;

/// The state the host controller reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pad {
    pub buttons: u64,
    pub left: (i32, i32),
    pub right: (i32, i32),
}

static INPUT: Mutex<Option<Pad>> = Mutex::new(None);

/// A scripted press: these buttons are held from frame `start` for `frames`
/// presented frames. Headless runs use it to get past screens that wait for
/// a button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Press {
    pub buttons: u64,
    pub start: u64,
    pub frames: u64,
}

static SCRIPT: Mutex<Vec<Press>> = Mutex::new(Vec::new());

/// Replace the scripted presses.
pub fn set_script(presses: Vec<Press>) {
    *SCRIPT.lock().unwrap_or_else(|e| e.into_inner()) = presses;
}

/// The buttons a script holds at `frame`.
fn scripted(frame: u64) -> u64 {
    SCRIPT
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|p| frame >= p.start && frame < p.start + p.frames)
        .fold(0, |b, p| b | p.buttons)
}

/// Parse `BUTTON@FRAME` or `BUTTON@FRAME+FRAMES` (held for 10 frames unless
/// given); several buttons join with `+` before the `@`, as in `A+B@600`.
pub fn parse_press(text: &str) -> Result<Press, String> {
    let (names, when) = text.split_once('@').ok_or_else(|| format!("{text}: expected BUTTON@FRAME"))?;
    let (start, frames) = match when.split_once('+') {
        Some((s, f)) => (s, f.parse::<u64>().map_err(|_| format!("{text}: bad frame count"))?),
        None => (when, 10),
    };
    let start = start.parse::<u64>().map_err(|_| format!("{text}: bad start frame"))?;
    let mut buttons = 0;
    for name in names.split('+') {
        buttons |= match name.to_ascii_uppercase().as_str() {
            "A" => button::A,
            "B" => button::B,
            "X" => button::X,
            "Y" => button::Y,
            "L" => button::L,
            "R" => button::R,
            "ZL" => button::ZL,
            "ZR" => button::ZR,
            "PLUS" | "START" => button::PLUS,
            "MINUS" | "SELECT" => button::MINUS,
            "LEFT" => button::LEFT,
            "UP" => button::UP,
            "RIGHT" => button::RIGHT,
            "DOWN" => button::DOWN,
            other => return Err(format!("{text}: unknown button {other}")),
        };
    }
    Ok(Press { buttons, start, frames })
}

/// Set the controller state from a front end (a window's keyboard, say). Once
/// set it takes precedence over the gamepad until cleared with `None`.
pub fn set_input(pad: Option<Pad>) {
    *INPUT.lock().unwrap_or_else(|e| e.into_inner()) = pad;
}

/// The controller state now: the front end's, else the first XInput gamepad.
pub fn input() -> Pad {
    if let Some(p) = *INPUT.lock().unwrap_or_else(|e| e.into_inner()) {
        return p;
    }
    xinput::poll().unwrap_or_default()
}

/// Stick-direction bits for a stick past a third of its range.
fn stick_bits(x: i32, y: i32, left: u64, up: u64, right: u64, down: u64) -> u64 {
    const T: i32 = 0x7FFF / 3;
    let mut b = 0;
    if x < -T {
        b |= left;
    }
    if x > T {
        b |= right;
    }
    if y > T {
        b |= up;
    }
    if y < -T {
        b |= down;
    }
    b
}

#[cfg(windows)]
mod xinput {
    use super::{button, stick_bits, Pad};

    #[repr(C)]
    #[derive(Default)]
    struct Gamepad {
        buttons: u16,
        left_trigger: u8,
        right_trigger: u8,
        lx: i16,
        ly: i16,
        rx: i16,
        ry: i16,
    }

    #[repr(C)]
    #[derive(Default)]
    struct XState {
        packet: u32,
        pad: Gamepad,
    }

    #[link(name = "xinput")]
    extern "system" {
        fn XInputGetState(user: u32, state: *mut XState) -> u32;
    }

    /// XInput button bits.
    const DPAD_UP: u16 = 0x0001;
    const DPAD_DOWN: u16 = 0x0002;
    const DPAD_LEFT: u16 = 0x0004;
    const DPAD_RIGHT: u16 = 0x0008;
    const START: u16 = 0x0010;
    const BACK: u16 = 0x0020;
    const LEFT_THUMB: u16 = 0x0040;
    const RIGHT_THUMB: u16 = 0x0080;
    const LEFT_SHOULDER: u16 = 0x0100;
    const RIGHT_SHOULDER: u16 = 0x0200;
    const BTN_A: u16 = 0x1000;
    const BTN_B: u16 = 0x2000;
    const BTN_X: u16 = 0x4000;
    const BTN_Y: u16 = 0x8000;

    fn axis(v: i16) -> i32 {
        (v as i32).clamp(-0x7FFF, 0x7FFF)
    }

    /// Gamepad 0, mapped by position: the bottom face button is the
    /// console's B, the right one A, the left one Y and the top one X.
    pub fn poll() -> Option<Pad> {
        let mut s = XState::default();
        if unsafe { XInputGetState(0, &mut s) } != 0 {
            return None;
        }
        let g = &s.pad;
        let map = [
            (BTN_A, button::B),
            (BTN_B, button::A),
            (BTN_X, button::Y),
            (BTN_Y, button::X),
            (LEFT_SHOULDER, button::L),
            (RIGHT_SHOULDER, button::R),
            (START, button::PLUS),
            (BACK, button::MINUS),
            (LEFT_THUMB, button::STICK_L),
            (RIGHT_THUMB, button::STICK_R),
            (DPAD_UP, button::UP),
            (DPAD_DOWN, button::DOWN),
            (DPAD_LEFT, button::LEFT),
            (DPAD_RIGHT, button::RIGHT),
        ];
        let mut buttons = map.iter().filter(|(x, _)| g.buttons & x != 0).fold(0, |b, (_, n)| b | n);
        if g.left_trigger > 0x40 {
            buttons |= button::ZL;
        }
        if g.right_trigger > 0x40 {
            buttons |= button::ZR;
        }
        let left = (axis(g.lx), axis(g.ly));
        let right = (axis(g.rx), axis(g.ry));
        buttons |= stick_bits(left.0, left.1, button::STICK_L_LEFT, button::STICK_L_UP, button::STICK_L_RIGHT, button::STICK_L_DOWN);
        buttons |= stick_bits(right.0, right.1, button::STICK_R_LEFT, button::STICK_R_UP, button::STICK_R_RIGHT, button::STICK_R_DOWN);
        Some(Pad { buttons, left, right })
    }
}

#[cfg(not(windows))]
mod xinput {
    pub fn poll() -> Option<super::Pad> {
        None
    }
}

fn r32(state: &State, a: u64) -> Result<u32, HostResult> {
    let b = crate::hle::read_bytes(state, a, 4)?;
    Ok(u32::from_le_bytes(b.try_into().unwrap()))
}

fn ok(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `GetNpadStyleSet(const NpadIdType&)`: player 1 is a Pro Controller.
fn get_style_set(state: &mut State) -> HostResult {
    match r32(state, state.x[0]) {
        Ok(NPAD_NO1) => HostResult::Returned(STYLE_FULL_KEY as u64),
        Ok(_) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn sampling() -> u64 {
    crate::threads::now_ticks() / (crate::threads::TICK_FREQ / 200)
}

fn write_state(state: &mut State, out: u64, pad: Option<Pad>) -> HostResult {
    let mut b = [0u8; 40];
    b[0..8].copy_from_slice(&sampling().to_le_bytes());
    if let Some(p) = pad {
        b[8..16].copy_from_slice(&p.buttons.to_le_bytes());
        b[16..20].copy_from_slice(&p.left.0.to_le_bytes());
        b[20..24].copy_from_slice(&p.left.1.to_le_bytes());
        b[24..28].copy_from_slice(&p.right.0.to_le_bytes());
        b[28..32].copy_from_slice(&p.right.1.to_le_bytes());
        b[32..36].copy_from_slice(&(ATTR_CONNECTED | ATTR_WIRED).to_le_bytes());
    }
    match write_bytes(state, out, &b) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

/// `GetNpadState(NpadFullKeyState*, const NpadIdType&)`.
fn get_full_key_state(state: &mut State) -> HostResult {
    let (out, id) = (state.x[0], state.x[1]);
    let pad = match r32(state, id) {
        Ok(NPAD_NO1) => {
            let mut pad = input();
            pad.buttons |= scripted(crate::nvn_win::frames_presented(state));
            Some(pad)
        }
        Ok(_) => None,
        Err(e) => return e,
    };
    write_state(state, out, pad)
}

/// Handheld and dual Joy-Con states: not connected on the host.
fn get_disconnected_state(state: &mut State) -> HostResult {
    let out = state.x[0];
    write_state(state, out, None)
}

/// Handle getters that report how many handles they wrote: none.
fn no_handles(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `GetVibrationDeviceInfo(info*, handle)`: zeroed (no device).
fn zero_info(state: &mut State) -> HostResult {
    match write_bytes(state, state.x[0], &[0u8; 8]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

/// Write an empty 32-bit flag set to the out pointer.
fn zero_word(state: &mut State) -> HostResult {
    match write_bytes(state, state.x[0], &[0u8; 4]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

/// A zeroed state record: no buttons, no movement, not connected.
fn zero_state(state: &mut State) -> HostResult {
    match write_bytes(state, state.x[0], &[0u8; 64]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("_ZN2nn3hid14InitializeNpadEv", ok, "the host controller needs no setup"),
        ("_ZN2nn3hid22SetSupportedNpadIdTypeEPKjm", ok, "player 1 is always supported"),
        ("_ZN2nn3hid24SetSupportedNpadStyleSetENS_4util10BitFlagSetILi32ENS0_12NpadStyleTagEEE", ok, "the host controller is a Pro Controller"),
        ("_ZN2nn3hid18SetNpadJoyHoldTypeENS0_15NpadJoyHoldTypeE", ok, "no Joy-Con are attached"),
        ("_ZN2nn3hid15GetNpadStyleSetERKj", get_style_set, "player 1: Pro Controller; others: none"),
        ("_ZN2nn3hid12GetNpadStateEPNS0_16NpadFullKeyStateERKj", get_full_key_state, "player 1 from the host controller (XInput pad 0)"),
        ("_ZN2nn3hid12GetNpadStateEPNS0_17NpadHandheldStateERKj", get_disconnected_state, "handheld is not connected on a PC"),
        ("_ZN2nn3hid12GetNpadStateEPNS0_16NpadJoyDualStateERKj", get_disconnected_state, "no Joy-Con are attached"),
        ("_ZN2nn3hid23GetSixAxisSensorHandlesEPNS0_19SixAxisSensorHandleEiRKjNS_4util10BitFlagSetILi32ENS0_12NpadStyleTagEEE", no_handles, "no motion sensors"),
        ("_ZN2nn3hid18StartSixAxisSensorERKNS0_19SixAxisSensorHandleE", ok, "no motion sensors"),
        ("_ZN2nn3hid17StopSixAxisSensorERKNS0_19SixAxisSensorHandleE", ok, "no motion sensors"),
        ("_ZN2nn3hid25SetGyroscopeZeroDriftModeERKNS0_19SixAxisSensorHandleERKNS0_22GyroscopeZeroDriftModeE", ok, "no motion sensors"),
        ("_ZN2nn3hid22GetSixAxisSensorStatesEPNS0_18SixAxisSensorStateEiRKNS0_19SixAxisSensorHandleE", no_handles, "no motion samples"),
        ("_ZN2nn3hid25GetVibrationDeviceHandlesEPNS0_21VibrationDeviceHandleEiRKjNS_4util10BitFlagSetILi32ENS0_12NpadStyleTagEEE", no_handles, "no vibration devices"),
        ("_ZN2nn3hid25InitializeVibrationDeviceERKNS0_21VibrationDeviceHandleE", ok, "no vibration devices"),
        ("_ZN2nn3hid22GetVibrationDeviceInfoEPNS0_19VibrationDeviceInfoERKNS0_21VibrationDeviceHandleE", zero_info, "no vibration devices"),
        ("_ZN2nn3hid16InitializeNmouseEPvmii", ok, "no Joy-Con mouse"),
        ("_ZN2nn3hid21ShowControllerSupportERKNS0_20ControllerSupportArgE", ok, "the host controller is already connected"),
        ("_ZN2nn3hid16GetNmouseHandlesEPNS0_12NmouseHandleEij", no_handles, "no Joy-Con mouse"),
        ("_ZN2nn3hid32SetNmouseButtonConfigJoyLeftJcmsERKNS0_29NmouseButtonConfigJoyLeftJcmsE", ok, "no Joy-Con mouse to configure"),
        ("_ZN2nn3hid33SetNmouseButtonConfigJoyRightJcmsERKNS0_30NmouseButtonConfigJoyRightJcmsE", ok, "no Joy-Con mouse to configure"),
        ("_ZN2nn3hid24SetNmousePointerPositionENS0_12NmouseHandleEii", ok, "no Joy-Con mouse pointer"),
        ("_ZN2nn3hid16GetNmouseTypeSetEPNS_4util10BitFlagSetILi32ENS0_10NmouseTypeEEENS0_12NmouseHandleE", zero_word, "no mouse types: none attached"),
        ("_ZN2nn3hid14GetNmouseStateEPNS0_11NmouseStateENS0_12NmouseHandleE", zero_state, "an idle, disconnected mouse"),
    ];
    for (name, handler, claim) in table {
        s.register(name, *handler, claim);
    }
    // The vibration file player, mixer and node graph drive vibration
    // devices; with none attached, their objects have nothing to drive.
    for name in VIBRATION {
        s.register(name, ok, "no vibration devices to drive");
    }
}

const VIBRATION: &[&str] = &[
    "_ZN2nn3hid15VibrationPlayerC1Ev",
    "_ZN2nn3hid15VibrationPlayerD1Ev",
    "_ZN2nn3hid15VibrationPlayer18SetCurrentPositionEi",
    "_ZN2nn3hid15VibrationPlayer4PlayEv",
    "_ZN2nn3hid15VibrationPlayer4StopEv",
    "_ZN2nn3hid15VibrationPlayer7SetLoopEb",
    "_ZN2nn3hid15VibrationPlayer23SetVibrationFileAddressEPKvm",
    "_ZN2nn3hid23VibrationNodeConnectionC1Ev",
    "_ZN2nn3hid23VibrationNodeConnectionD1Ev",
    "_ZN2nn3hid23VibrationNodeConnection7ConnectEPNS0_13VibrationNodeES3_",
    "_ZN2nn3hid23VibrationNodeConnection10DisconnectEv",
    "_ZN2nn3hid13VibrationNode15SetModulationToEPKS1_RKNS0_19VibrationModulationE",
    "_ZN2nn3hid13VibrationNode6UpdateEv",
    "_ZN2nn3hid15VibrationWriterD1Ev",
    "_ZN2nn3hid14VibrationMixerC1Ev",
    "_ZN2nn3hid14VibrationMixerD1Ev",
    "_ZN2nn3hid15VibrationTargetC1Ev",
    "_ZN2nn3hid15VibrationTargetD1Ev",
    "_ZN2nn3hid15VibrationTarget24SetVibrationDeviceHandleERKNS0_21VibrationDeviceHandleE",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presses_parse() {
        assert_eq!(parse_press("A@600"), Ok(Press { buttons: button::A, start: 600, frames: 10 }));
        assert_eq!(parse_press("a+plus@5+3"), Ok(Press { buttons: button::A | button::PLUS, start: 5, frames: 3 }));
        assert!(parse_press("Q@1").is_err());
        assert!(parse_press("A").is_err());
    }

    #[test]
    fn player_one_reports_the_front_end_state() {
        set_input(Some(Pad { buttons: button::A | button::PLUS, left: (0x7FFF, 0), right: (0, 0) }));
        let mut s = State { mem: vec![0; 0x1000], ..State::default() };
        s.mem[0x100..0x104].copy_from_slice(&NPAD_NO1.to_le_bytes());
        s.x[0] = 0x200;
        s.x[1] = 0x100;
        assert_eq!(get_full_key_state(&mut s), HostResult::Returned(0));
        assert_eq!(u64::from_le_bytes(s.mem[0x208..0x210].try_into().unwrap()), button::A | button::PLUS);
        assert_eq!(i32::from_le_bytes(s.mem[0x210..0x214].try_into().unwrap()), 0x7FFF);
        assert_eq!(u32::from_le_bytes(s.mem[0x220..0x224].try_into().unwrap()) & ATTR_CONNECTED, ATTR_CONNECTED);
        set_input(None);
    }
}
