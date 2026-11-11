//! The applet environment and system services a title asks about itself:
//! `nn::oe`, `nn::account`, `nn::time`, `nn::aoc` and `nn::diag`.
//!
//! Answers come from the launcher's settings (`state.host.settings`) or from
//! the title's own `control.nacp`, never from invented constants:
//!
//! * `language` (default `en-US`): `nn::oe::GetDesiredLanguage`.
//! * `display_version`: read from `control.nacp` by the launcher.
//! * The PC is presented as a docked console in boost performance mode, which
//!   is the configuration that selects the title's full-resolution output.
//!
//! `nn::diag` aborts stop the run with the game's own reason.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use crate::threads::now_ticks;
use ew_lift::State;

const OPERATION_MODE_CONSOLE: u64 = 1;
const PERFORMANCE_MODE_BOOST: u64 = 1;
const FOCUS_IN_FOCUS: u64 = 1;

macro_rules! t {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

fn setting(state: &State, key: &str, default: &str) -> String {
    state.host.settings.get(key).cloned().unwrap_or_else(|| default.to_string())
}

fn ok(_s: &mut State) -> HostResult {
    HostResult::Returned(0)
}

// ---- nn::oe ------------------------------------------------------------------

fn operation_mode(_s: &mut State) -> HostResult {
    HostResult::Returned(OPERATION_MODE_CONSOLE)
}
fn performance_mode(_s: &mut State) -> HostResult {
    HostResult::Returned(PERFORMANCE_MODE_BOOST)
}
fn focus_state(_s: &mut State) -> HostResult {
    HostResult::Returned(FOCUS_IN_FOCUS)
}

/// `nn::settings::LanguageCode` is eight bytes returned in `x0`.
fn desired_language(s: &mut State) -> HostResult {
    let lang = setting(s, "language", "en-US");
    let mut b = [0u8; 8];
    let n = lang.len().min(7);
    b[..n].copy_from_slice(&lang.as_bytes()[..n]);
    HostResult::Returned(u64::from_le_bytes(b))
}

/// `GetDisplayVersion(DisplayVersion*)`: sixteen bytes from `control.nacp`.
fn display_version(s: &mut State) -> HostResult {
    let Some(v) = s.host.settings.get("display_version").cloned() else {
        return HostResult::Unavailable(
            "GetDisplayVersion: the launcher did not supply control.nacp, so the version is unknown".into(),
        );
    };
    let mut b = [0u8; 16];
    let n = v.len().min(15);
    b[..n].copy_from_slice(&v.as_bytes()[..n]);
    t!(write_bytes(s, s.x[0], &b));
    HostResult::Returned(0)
}

/// `PopNotificationMessage()` waits for the next applet message. The host only
/// produces one when the launcher asks the game to exit, so until then the
/// calling thread waits.
fn pop_notification(s: &mut State) -> HostResult {
    match s.host.settings.remove("pending_message") {
        Some(m) => HostResult::Returned(m.parse().unwrap_or(0)),
        None => HostResult::Block { resume: pop_notification, timed: false },
    }
}

fn set_inactivity_extended(s: &mut State) -> HostResult {
    s.host.settings.insert("inactivity_extended".into(), ((s.x[0] & 1) != 0).to_string());
    HostResult::Returned(0)
}
fn is_inactivity_extended(s: &mut State) -> HostResult {
    HostResult::Returned((setting(s, "inactivity_extended", "false") == "true") as u64)
}

/// `GetExpectedVolumeBalance(float* main, float* applet)`.
fn volume_balance(s: &mut State) -> HostResult {
    let main = setting(s, "volume_main", "1").parse::<f32>().unwrap_or(1.0);
    let applet = setting(s, "volume_applet", "1").parse::<f32>().unwrap_or(1.0);
    t!(write_bytes(s, s.x[0], &main.to_le_bytes()));
    t!(write_bytes(s, s.x[1], &applet.to_le_bytes()));
    HostResult::Returned(0)
}
fn set_volume_balance(s: &mut State) -> HostResult {
    let main = f32::from_bits(s.v[0][0] as u32);
    let applet = f32::from_bits(s.v[1][0] as u32);
    s.host.settings.insert("volume_main".into(), main.to_string());
    s.host.settings.insert("volume_applet".into(), applet.to_string());
    HostResult::Returned(0)
}

/// `GetProgramTotalActiveTime()`: TimeSpan in nanoseconds since launch.
fn active_time(_s: &mut State) -> HostResult {
    let ticks = now_ticks().saturating_sub(crate::threads::TICK_FREQ);
    HostResult::Returned(((ticks as u128 * 1_000_000_000) / crate::threads::TICK_FREQ as u128) as u64)
}

// ---- nn::account ----------------------------------------------------------------

/// The local profile's Uid: 128 bits derived from the configured user name, so
/// it is stable across runs and distinct per name.
fn local_uid(s: &State) -> [u8; 16] {
    let name = setting(s, "user_name", "Player");
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    let mut out = [0u8; 16];
    for (i, b) in name.bytes().chain(*b"EmblemWeave").enumerate() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01B3);
        out[i % 16] ^= (h >> 24) as u8;
    }
    out[0] |= 1; // a Uid of all zeroes means "no user"
    out
}

/// `TryOpenPreselectedUser(UserHandle*)`: the PC has exactly one profile.
fn open_preselected_user(s: &mut State) -> HostResult {
    let uid = local_uid(s);
    let mut handle = [0u8; 24];
    handle[..16].copy_from_slice(&uid);
    handle[16..].copy_from_slice(&0xE3_0000_0001u64.to_le_bytes());
    t!(write_bytes(s, s.x[0], &handle));
    HostResult::Returned(1)
}

fn get_user_id(s: &mut State) -> HostResult {
    let handle = t!(read_bytes(s, s.x[1], 16));
    t!(write_bytes(s, s.x[0], &handle));
    HostResult::Returned(0)
}

fn last_opened_user(s: &mut State) -> HostResult {
    let uid = local_uid(s);
    t!(write_bytes(s, s.x[0], &uid));
    HostResult::Returned(0)
}

// ---- nn::time -----------------------------------------------------------------

fn current_time(s: &mut State) -> HostResult {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    t!(write_bytes(s, s.x[0], &secs.to_le_bytes()));
    HostResult::Returned(0)
}

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// Days since 1970-01-01 of a civil date, the inverse of [`civil`].
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `ToCalendarTime(CalendarTime*, CalendarAdditionalInfo*, const PosixTime&)`
/// in UTC: the host does not carry the console's time zone database.
fn to_calendar(s: &mut State) -> HostResult {
    let raw = t!(read_bytes(s, s.x[2], 8));
    let secs = i64::from_le_bytes(raw.try_into().unwrap());
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil(days);
    let mut cal = Vec::with_capacity(8);
    cal.extend_from_slice(&(y as i16).to_le_bytes());
    cal.extend_from_slice(&[m as u8, d as u8, (rem / 3600) as u8, (rem % 3600 / 60) as u8, (rem % 60) as u8, 0]);
    t!(write_bytes(s, s.x[0], &cal));
    if s.x[1] != 0 {
        let dow = (days + 4).rem_euclid(7) as i32; // 1970-01-01 was a Thursday
        let doy = (days - days_from_civil(y, 1, 1)) as i32;
        let mut info = Vec::with_capacity(0x18);
        info.extend_from_slice(&dow.to_le_bytes());
        info.extend_from_slice(&doy.to_le_bytes());
        info.extend_from_slice(b"UTC\0\0\0\0\0");
        info.extend_from_slice(&[0u8; 4]); // isDaylightSavingTime + padding
        info.extend_from_slice(&0i32.to_le_bytes());
        t!(write_bytes(s, s.x[1], &info));
    }
    HostResult::Returned(0)
}

// ---- nn::aoc and nn::diag ---------------------------------------------------------

fn no_dlc(_s: &mut State) -> HostResult {
    HostResult::Returned(0)
}

fn abort(s: &mut State) -> HostResult {
    let detail = if s.x[0] != 0 {
        read_bytes(s, s.x[0], 4)
            .ok()
            .map(|b| {
                let v = u32::from_le_bytes(b.try_into().unwrap());
                format!(" with nn::Result 0x{v:x} (module {}, description {})", v & 0x1FF, (v >> 9) & 0x1FFF)
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    HostResult::Unavailable(format!("the game aborted{detail}; its last host calls are in the report"))
}

/// `nn::diag::detail::NotNullImpl(uintptr_t)`: whether the value is non-null.
/// Callers branch on bit 0 of the result, so it is a test, not an abort.
fn not_null(state: &mut State) -> HostResult {
    HostResult::Returned((state.x[0] != 0) as u64)
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("_ZN2nn4util11ReferSymbolEPKv", ok, "only keeps a symbol linked; it has no run-time effect"),
        ("_ZN2nn2oe10InitializeEv", ok, "the applet environment needs no setup on the host"),
        ("_ZN2nn2oe23GetCurrentGraphicsScaleEv", ok, "0, the default scale: the game takes its normal path at 0x1818434"),
        ("_ZN2nn2oe16GetOperationModeEv", operation_mode, "the PC presents as a docked console"),
        ("_ZN2nn2oe18GetPerformanceModeEv", performance_mode, "docked boost mode, the full-resolution configuration"),
        ("_ZN2nn2oe20GetCurrentFocusStateEv", focus_state, "the game window is in focus"),
        ("_ZN2nn2oe18GetDesiredLanguageEv", desired_language, "the launcher's language setting, default en-US"),
        ("_ZN2nn2oe17GetDisplayVersionEPNS0_14DisplayVersionE", display_version, "the version string from the title's control.nacp"),
        ("_ZN2nn2oe22PopNotificationMessageEv", pop_notification, "waits for an applet message; the launcher sends one only to request exit"),
        ("_ZN2nn2oe25GetProgramTotalActiveTimeEv", active_time, "time since launch on the host clock"),
        ("_ZN2nn2oe24GetExpectedVolumeBalanceEPfS1_", volume_balance, "launcher volume balance, default 1.0 and 1.0"),
        ("_ZN2nn2oe24SetExpectedVolumeBalanceEff", set_volume_balance, "recorded for later reads"),
        ("_ZN2nn2oe38SetUserInactivityDetectionTimeExtendedEb", set_inactivity_extended, "recorded; the PC has no auto-sleep to extend"),
        ("_ZN2nn2oe44SetUserInactivityDetectionTimeExtendedUnsafeEb", set_inactivity_extended, "recorded; the PC has no auto-sleep to extend"),
        ("_ZN2nn2oe37IsUserInactivityDetectionTimeExtendedEv", is_inactivity_extended, "reports the recorded flag"),
        ("_ZN2nn2oe20SetFocusHandlingModeENS0_17FocusHandlingModeE", ok, "the host never suspends the game on focus loss"),
        ("_ZN2nn2oe28SetResumeNotificationEnabledEb", ok, "the host never suspends the game, so no resume message is produced"),
        ("_ZN2nn2oe42SetOperationModeChangedNotificationEnabledEb", ok, "the operation mode never changes on the PC"),
        ("_ZN2nn2oe44SetPerformanceModeChangedNotificationEnabledEb", ok, "the performance mode never changes on the PC"),
        ("_ZN2nn2oe42SetGraphicsScaleChangedNotificationEnabledEb", ok, "the graphics scale never changes on the PC"),
        ("_ZN2nn2oe31EnterExitRequestHandlingSectionEv", ok, "exit requests are delivered as messages"),
        ("_ZN2nn2oe31LeaveExitRequestHandlingSectionEv", ok, "exit requests are delivered as messages"),
        ("_ZN2nn2oe15SetCpuBoostModeENS0_12CpuBoostModeE", ok, "the host CPU does not change clocks for the guest"),
        ("_ZN2nn7account10InitializeEv", ok, "one local profile"),
        ("_ZN2nn7account22TryOpenPreselectedUserEPNS0_10UserHandleE", open_preselected_user, "the PC's single local profile is always preselected"),
        ("_ZN2nn7account9GetUserIdEPNS0_3UidERKNS0_10UserHandleE", get_user_id, "the Uid carried in the handle"),
        ("_ZN2nn7account9CloseUserERKNS0_10UserHandleE", ok, "handles hold no host resources"),
        ("_ZN2nn7account17GetLastOpenedUserEPNS0_3UidE", last_opened_user, "the local profile's Uid"),
        ("_ZN2nn4time10InitializeEv", ok, "the host clock needs no setup"),
        ("_ZN2nn4time8FinalizeEv", ok, "the host clock needs no teardown"),
        ("_ZN2nn4time23StandardUserSystemClock14GetCurrentTimeEPNS0_9PosixTimeE", current_time, "host wall clock, POSIX seconds"),
        ("_ZN2nn4time14ToCalendarTimeEPNS0_12CalendarTimeEPNS0_22CalendarAdditionalInfoERKNS0_9PosixTimeE", to_calendar, "civil time in UTC; no console time-zone database"),
        ("_ZN2nn3aoc17CountAddOnContentEv", no_dlc, "no add-on content is installed"),
        ("_ZN2nn3aoc16ListAddOnContentEPiii", no_dlc, "no add-on content is installed"),
        ("_ZN2nn4diag6detail9AbortImplEPKNS_6ResultE", abort, "a game abort stops the run with its result code"),
        ("_ZN2nn4diag6detail9AbortImplEv", abort, "a game abort stops the run"),
        ("_ZN2nn4diag6detail11NotNullImplEm", not_null, "true when the value is not null; the caller tests the result (0x1567e0)"),
    ];
    for (name, handler, claim) in table {
        s.register(name, *handler, claim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_code_is_packed_little_endian() {
        let mut s = State::default();
        assert_eq!(desired_language(&mut s), HostResult::Returned(u64::from_le_bytes(*b"en-US\0\0\0")));
        s.host.settings.insert("language".into(), "ja".into());
        assert_eq!(desired_language(&mut s), HostResult::Returned(u64::from_le_bytes(*b"ja\0\0\0\0\0\0")));
    }

    #[test]
    fn calendar_conversion_matches_known_dates() {
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(19_723), (2024, 1, 1));
        assert_eq!(civil(20_401), (2025, 11, 9));
        let mut s = State::default();
        s.mem[0x100..0x108].copy_from_slice(&1_762_646_400i64.to_le_bytes()); // 2025-11-09 00:00:00
        s.x = [0; 31];
        s.x[0] = 0x200;
        s.x[1] = 0x300;
        s.x[2] = 0x100;
        assert_eq!(to_calendar(&mut s), HostResult::Returned(0));
        assert_eq!(&s.mem[0x200..0x207], &[0xE9, 0x07, 11, 9, 0, 0, 0]);
        assert_eq!(i32::from_le_bytes(s.mem[0x300..0x304].try_into().unwrap()), 0, "a Sunday");
        assert_eq!(i32::from_le_bytes(s.mem[0x304..0x308].try_into().unwrap()), 312);
    }

    #[test]
    fn the_local_uid_is_stable_and_nonzero() {
        let s = State::default();
        assert_eq!(local_uid(&s), local_uid(&s));
        assert_ne!(local_uid(&s), [0u8; 16]);
    }
}
