//! What the rest of the launcher asks the operating system for, in one place.
//!
//! Every question the launcher needs an OS to answer goes through here, so the
//! Windows assumptions are visible in one file instead of scattered through
//! pages and job code (docs/PLATFORMS.md has the audit that led to it):
//!
//! * whether the game is running (a process list),
//! * whether the launcher may run dependency installers (winget),
//! * whether this platform can build the game at all (MSVC through
//!   `vcvars64.bat`, CMake, Ninja),
//! * the folders the launcher keeps its own data in,
//! * the name of the platform the interface shows in a diagnostic line.
//!
//! Nothing here is a promise that a non-Windows build plays the game: the
//! engine is Windows-only today, and docs/LINUX.md is the plan that has to be
//! built and tested before that changes. The point of this module is that a
//! build for another platform cannot accidentally run a winget line, write to
//! `%APPDATA%`, or think a game is running because it read a Windows process
//! list.

/// Whether this build targets Windows. The code below is `cfg`-selected, so
/// this is a compile-time fact, not a runtime probe.
pub const WINDOWS: bool = cfg!(windows);

/// The platform the interface names when it explains what it can do here.
pub fn name() -> &'static str {
    if WINDOWS {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "linux") {
        "Linux"
    } else {
        std::env::consts::OS
    }
}

/// Whether the game's process is running.
///
/// On Windows this reads the process list; elsewhere there is no list to read,
/// so the answer is "no", which is what a save editor needs before it writes.
pub fn game_running() -> bool {
    cfg!(windows) && crate::cheats::process::find(crate::cheats::GAME_EXE).is_some()
}

/// Whether the launcher can start a dependency installer itself.
///
/// The registry's install lines are `winget` commands (and one `py -m pip`
/// line) run from a `.cmd` script. Off Windows that is not a supported way to
/// install anything, and running it would fail with a shell error the user
/// cannot act on, so the button is refused with a sentence that says why.
pub fn install_supported() -> bool {
    WINDOWS
}

/// Why `install_supported` said no, for the message the interface shows.
pub fn install_unsupported_reason() -> String {
    format!(
        "installing build tools is a Windows step (winget and a .cmd script); on {} use your distribution's packages \
         (docs/LINUX.md lists the ones the engine needs)",
        name()
    )
}

/// Whether this platform can build the game.
///
/// The engine is compiled by MSVC's clang-cl through `vcvars64.bat` and links
/// the lifted libraries built the same way. That toolchain exists on Windows
/// only, so this is the single place that says so; `engine::build` asks here
/// before it starts a job that would fail 90 steps in.
pub fn build_supported() -> bool {
    WINDOWS
}

/// Why `build_supported` said no.
pub fn build_unsupported_reason() -> String {
    format!(
        "EmblemWeave builds the game with the Windows toolchain (Visual Studio's clang-cl through vcvars64.bat). \
         {} support is planned but has not been built and tested, so it is not advertised: docs/LINUX.md has the \
         implementation plan and the build requirements",
        name()
    )
}

/// The launcher's own data folder, the one that holds `play`, `work`, the logs
/// and the save folder.
///
/// Windows: `%APPDATA%\EmblemWeave`. Linux: `$XDG_CONFIG_HOME/EmblemWeave`, else
/// `$HOME/.config/EmblemWeave` (the same place a native build would keep it; the
/// name is the same so a future port can read a folder made by hand).
pub fn data_root_from(env: &dyn Fn(&str) -> Option<String>) -> Option<std::path::PathBuf> {
    if WINDOWS {
        return env("APPDATA").filter(|s| !s.is_empty()).map(|s| std::path::PathBuf::from(s).join("EmblemWeave"));
    }
    env("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(|s| std::path::PathBuf::from(s).join("EmblemWeave"))
        .or_else(|| env("HOME").filter(|s| !s.is_empty()).map(|s| std::path::PathBuf::from(s).join(".config").join("EmblemWeave")))
}

/// The file beside the executable that turns portable mode on.
pub const PORTABLE_MARKER: &str = "portable.txt";

/// Where a portable copy keeps its data: `data` next to `EmblemWeave.exe`, holding `play`, `work`, `save` and `logs`
/// just as `%APPDATA%\EmblemWeave` does. Portable mode is on when `portable.txt` sits beside the executable, so an
/// unzipped folder with that one empty file in it can be moved, copied or carried on a stick as one piece, and no
/// other copy on the PC sees its saves or its cache.
pub fn portable_root(exe_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    let marker = std::fs::read_dir(exe_dir)
        .ok()?
        .flatten()
        .any(|e| e.file_name().to_string_lossy().eq_ignore_ascii_case(PORTABLE_MARKER));
    marker.then(|| exe_dir.join("data"))
}

/// A fresh copy keeps everything in its own folder. Nothing else on the PC holds data for this launcher, so nothing moves and nothing
/// surprises; a zip unpacked onto a stick is portable without anyone creating a marker file, which is what issue #21 asked for. A copy
/// that sits beside an existing profile keeps using it, and the About page switches it with the button there.
///
/// Returns the folder the profile holds, when it was adopted.
pub fn adopt_portable_on_first_run() -> Option<std::path::PathBuf> {
    let exe_dir = exe_dir()?;
    if portable_root(&exe_dir).is_some() {
        return None;
    }
    let profile = data_root_from(&|k| std::env::var(k).ok())?;
    if profile.exists() {
        return None;
    }
    let data = exe_dir.join("data");
    std::fs::create_dir_all(&data).ok()?;
    std::fs::write(exe_dir.join(PORTABLE_MARKER), b"").ok()?;
    Some(data)
}

/// Switch this copy to keeping everything beside the program, the way the About page's button does: the marker goes in, and the saves
/// that the profile already holds are copied across so the player's progress comes with them. The game folder and the build output are
/// not copied: they are rebuilt in the new folder by the next build, and the old ones are left where they were.
pub fn set_portable(exe_dir: &std::path::Path, on: bool) -> Result<Option<std::path::PathBuf>, String> {
    let marker = exe_dir.join(PORTABLE_MARKER);
    if !on {
        match std::fs::remove_file(&marker) {
            Ok(()) => return Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("removing {}: {e}", marker.display())),
        }
    }
    let data = exe_dir.join("data");
    std::fs::create_dir_all(&data).map_err(|e| format!("creating {}: {e}", data.display()))?;
    std::fs::write(&marker, b"").map_err(|e| format!("writing {}: {e}", marker.display()))?;
    // The saves are the part a player cannot get back, so they follow the switch; a copy that cannot be made does not undo it. The
    // settings come too (theme, chosen folders, save profiles), and so do installed mods: a mod the player added is their own file, and
    // making them find and install it again after pressing a button is not "everything in this folder".
    let mut copied = None;
    if let Some(profile) = data_root_from(&|k| std::env::var(k).ok()) {
        let from = profile.join("save");
        let to = data.join("save");
        if from.is_dir() && !to.exists() && crate::saves::copy_tree(&from, &to).is_ok() {
            copied = Some(to);
        }
        let settings = profile.join("config.json");
        let target = data.join("config.json");
        if settings.is_file() && !target.exists() {
            let _ = std::fs::copy(&settings, &target);
        }
        let mods = profile.join("play").join("mods");
        let target = data.join("play").join("mods");
        if mods.is_dir() && !target.exists() {
            let _ = crate::saves::copy_tree(&mods, &target);
        }
    }
    Ok(copied)
}

/// The folder the launcher's copy was started from, for portable mode.
pub fn exe_dir() -> Option<std::path::PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()))
}

/// `data_root_from` against this process's environment, unless this copy is portable.
pub fn data_root() -> Option<std::path::PathBuf> {
    if let Some(dir) = exe_dir().and_then(|d| portable_root(&d)) {
        return Some(dir);
    }
    data_root_from(&|k| std::env::var(k).ok())
}

/// The sentence a job shows when `data_root` found nothing: it names the variable per platform instead
/// of asking a Linux user for `%APPDATA%`.
pub const NO_DATA_DIR: &str = if cfg!(windows) {
    "APPDATA is not set"
} else {
    "neither XDG_CONFIG_HOME nor HOME is set"
};

/// Start title-bar moves in the Windows hit-test pass, before egui sees the
/// press. `ViewportCommand::StartDrag` can only run after the frame that saw
/// the click, which leaves one frame where the pointer travels and the window
/// catches up: the slide the launcher had when grabbed.
pub fn install_native_drag() {
    #[cfg(windows)]
    native_drag::install();
}

#[cfg(windows)]
mod native_drag {
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, EnumWindows, GetClientRect, GetWindowLongPtrW, GetWindowPlacement, GetWindowRect, GetWindowTextW,
        GetWindowThreadProcessId, IsZoomed, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWLP_WNDPROC, HTCAPTION, SWP_NOACTIVATE, SWP_NOSIZE,
        SWP_NOZORDER, SW_RESTORE, WINDOWPLACEMENT, WM_NCHITTEST, WM_NCLBUTTONDOWN,
    };

    type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

    static ORIGINAL: AtomicIsize = AtomicIsize::new(0);
    static INSTALLED: AtomicBool = AtomicBool::new(false);

    /// The painted top bar and this hit test share the same insets; only the
    /// empty centre of the bar is caption, so the chips and window controls
    /// keep their own clicks.
    /// A maximized window has no caption left to move from: the shell restores
    /// it on the press and starts the move at once. This window is borderless,
    /// so the press goes through winit's procedure, which starts the move only
    /// for a restored window, and a grabbed maximized launcher sat still until
    /// it was restored by hand. Restore it here, under the pointer, so the same
    /// press moves the window from the place it was grabbed.
    unsafe fn restore_under_cursor(hwnd: HWND, lparam: LPARAM) {
        let cx = (lparam & 0xffff) as u16 as i16 as i32;
        let cy = ((lparam >> 16) & 0xffff) as u16 as i16 as i32;
        let mut maximized: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut maximized) == 0 {
            return;
        }
        let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
        placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
        let normal = if GetWindowPlacement(hwnd, &mut placement) != 0 { placement.rcNormalPosition } else { maximized };
        let mw = (maximized.right - maximized.left).max(1) as f32;
        let mh = (maximized.bottom - maximized.top).max(1) as f32;
        let nw = (normal.right - normal.left).max(1) as f32;
        let nh = (normal.bottom - normal.top).max(1) as f32;
        // The pointer keeps the same fraction of the window, so the caption does
        // not jump out from under it.
        let fx = ((cx - maximized.left) as f32 / mw).clamp(0.0, 1.0);
        let fy = ((cy - maximized.top) as f32 / mh).clamp(0.0, 1.0);
        let x = cx - (fx * nw).round() as i32;
        let y = cy - (fy * nh).round() as i32;
        ShowWindow(hwnd, SW_RESTORE);
        SetWindowPos(hwnd, std::ptr::null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
    }

    unsafe extern "system" fn hit_test(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if msg == WM_NCLBUTTONDOWN && wparam as u32 == HTCAPTION && IsZoomed(hwnd) != 0 {
            restore_under_cursor(hwnd, lparam);
        }
        if msg == WM_NCHITTEST {
            let mut point = POINT {
                x: (lparam & 0xffff) as u16 as i16 as i32,
                y: ((lparam >> 16) & 0xffff) as u16 as i16 as i32,
            };
            let mut client: RECT = std::mem::zeroed();
            if ScreenToClient(hwnd, &mut point) != 0 && GetClientRect(hwnd, &mut client) != 0 {
                let dpi = GetDpiForWindow(hwnd);
                let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
                let top = (crate::theme::TOPBAR_HEIGHT * scale).round() as i32;
                let left = (crate::theme::TOPBAR_DRAG_LEFT * scale).round() as i32;
                let right = client.right - (crate::theme::TOPBAR_DRAG_RIGHT * scale).round() as i32;
                if point.y >= 0 && point.y < top && point.x >= left && point.x < right {
                    return HTCAPTION as LRESULT;
                }
            }
        }

        let original = ORIGINAL.load(Ordering::Relaxed);
        if original != 0 {
            let original: WndProc = std::mem::transmute(original);
            return CallWindowProcW(Some(original), hwnd, msg, wparam, lparam);
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    unsafe extern "system" fn find_launcher(hwnd: HWND, found: LPARAM) -> BOOL {
        let mut process = 0;
        GetWindowThreadProcessId(hwnd, &mut process);
        if process == GetCurrentProcessId() {
            let mut title = [0u16; 64];
            let len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            if len > 0 && String::from_utf16_lossy(&title[..len as usize]) == "EmblemWeave" {
                *(found as *mut HWND) = hwnd;
                return 0;
            }
        }
        1
    }

    pub fn install() {
        if INSTALLED.load(Ordering::Relaxed) {
            return;
        }
        unsafe {
            let mut hwnd: HWND = std::ptr::null_mut();
            EnumWindows(Some(find_launcher), &mut hwnd as *mut HWND as LPARAM);
            if hwnd.is_null() {
                return;
            }
            let original = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
            if original == 0 {
                return;
            }
            ORIGINAL.store(original, Ordering::Relaxed);
            if SetWindowLongPtrW(hwnd, GWLP_WNDPROC, hit_test as *const () as usize as isize) == 0 {
                ORIGINAL.store(0, Ordering::Relaxed);
                return;
            }
            INSTALLED.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| pairs.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
    }

    #[test]
    fn the_data_folder_is_appdata_on_windows_and_xdg_elsewhere() {
        let win = data_root_from(&env_of(&[("APPDATA", "/home/x/.config")]));
        if WINDOWS {
            assert_eq!(win.unwrap(), std::path::PathBuf::from("/home/x/.config/EmblemWeave"));
        }
        let xdg = data_root_from(&env_of(&[("XDG_CONFIG_HOME", "/home/x/cfg"), ("HOME", "/home/x")]));
        let home = data_root_from(&env_of(&[("HOME", "/home/x")]));
        if !WINDOWS {
            assert_eq!(xdg.unwrap(), std::path::PathBuf::from("/home/x/cfg/EmblemWeave"));
            assert_eq!(home.unwrap(), std::path::PathBuf::from("/home/x/.config/EmblemWeave"));
        }
        assert!(data_root_from(&env_of(&[])).is_none());
    }

    #[test]
    fn a_portable_marker_beside_the_exe_moves_every_folder_next_to_it() {
        let dir = std::env::temp_dir().join(format!("emblemweave-portable-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(portable_root(&dir), None, "no marker, no portable mode");
        std::fs::write(dir.join("Portable.TXT"), "").unwrap();
        // The name Windows would match, whatever case the file was made in.
        assert_eq!(portable_root(&dir), Some(dir.join("data")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_button_writes_the_marker_and_creates_the_data_folder() {
        let dir = std::env::temp_dir().join(format!("emblemweave-portable-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(portable_root(&dir), None);
        set_portable(&dir, true).unwrap();
        assert!(dir.join(PORTABLE_MARKER).is_file());
        assert!(dir.join("data").is_dir());
        assert_eq!(portable_root(&dir), Some(dir.join("data")));
        // Off removes the marker; the data folder stays, because it holds what the copy has been using.
        set_portable(&dir, false).unwrap();
        assert!(!dir.join(PORTABLE_MARKER).is_file());
        assert!(dir.join("data").is_dir());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_install_and_build_paths_match_the_platform() {
        assert_eq!(install_supported(), WINDOWS);
        assert_eq!(build_supported(), WINDOWS);
        if !WINDOWS {
            assert!(install_unsupported_reason().contains(name()));
            assert!(build_unsupported_reason().contains("docs/LINUX.md"));
        }
    }

}
