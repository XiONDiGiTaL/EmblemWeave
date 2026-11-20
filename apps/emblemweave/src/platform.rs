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

/// Tell the native caption hit test where the top bar's empty centre is. The
/// shell reports the bounds each frame; fixed insets would cover the status
/// chips or the right-hand actions once their labels change.
pub fn set_caption_bounds(left: f32, right: f32) {
    #[cfg(windows)]
    native_drag::set_caption_bounds(left, right);
}

/// The logo, title and page label are not controls; they are part of the
/// caption. Reporting that strip as well means a drag that starts on the title
/// moves the window instead of doing nothing.
pub fn set_brand_caption_bounds(left: f32, right: f32) {
    #[cfg(windows)]
    native_drag::set_brand_caption_bounds(left, right);
}

#[cfg(windows)]
mod native_drag {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicIsize, AtomicU32, Ordering};

    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
    use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
    use windows_sys::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        DragDetect, GetAsyncKeyState, ReleaseCapture, SetCapture, VK_ESCAPE, VK_LBUTTON,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, EnumWindows, GetClassNameW, GetClientRect, GetCursorPos, GetSystemMetrics, GetWindowLongPtrW,
        GetWindowPlacement, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, IsZoomed, RegisterWindowMessageW,
        SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, GWLP_WNDPROC, GWL_STYLE, HTBOTTOM, HTBOTTOMLEFT,
        HTBOTTOMRIGHT, HTCAPTION, HTLEFT, HTRIGHT, HTTOP, HTTOPLEFT, HTTOPRIGHT, SM_CXDRAG, SM_CYDRAG, SWP_FRAMECHANGED, SWP_NOACTIVATE,
        SWP_NOZORDER, WINDOWPLACEMENT, WM_CAPTURECHANGED, WM_ENTERSIZEMOVE, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST, WM_NCLBUTTONDOWN,
        WS_MAXIMIZE,
    };

    /// Per-window DWM transitions. Windows animates a maximize/restore over
    /// about 50 ms; that animation, not the app, is the slide a grabbed
    /// maximized launcher still showed after the press was handled natively.
    const DWMWA_TRANSITIONS_FORCEDISABLED: u32 = 3;
    /// The launcher draws its own frame, so Windows has no standard caption
    /// to infer corners from. Ask DWM for the system radius explicitly; the
    /// preference is ignored before Windows 11, where square is the norm.
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: i32 = 2;

    type WndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

    static ORIGINAL: AtomicIsize = AtomicIsize::new(0);
    static INSTALLED: AtomicBool = AtomicBool::new(false);
    /// The registered message Slick Window Arrangement uses to remove its
    /// per-window subclass. Registering the same name returns the same id.
    static SLICK_UNSUBCLASS_MESSAGE: AtomicU32 = AtomicU32::new(0);
    /// Logical insets from the client edges to the empty caption band.
    static CAPTION_LEFT: AtomicI32 = AtomicI32::new(crate::theme::TOPBAR_DRAG_LEFT as i32);
    static CAPTION_RIGHT: AtomicI32 = AtomicI32::new(crate::theme::TOPBAR_DRAG_RIGHT as i32);
    /// The non-interactive logo, title and page label, between the rail toggle and the status chips.
    static BRAND_LEFT: AtomicI32 = AtomicI32::new(40);
    static BRAND_RIGHT: AtomicI32 = AtomicI32::new(250);
    /// Fallback for a maximized caption press: when `DragDetect` does not take
    /// the non-client press, the press waits for the system drag threshold
    /// here. A click that never moves must leave the window maximized.
    static MAXIMIZED_PRESS: AtomicBool = AtomicBool::new(false);
    static PRESS_X: AtomicI32 = AtomicI32::new(0);
    static PRESS_Y: AtomicI32 = AtomicI32::new(0);

    /// A launcher-side log for the native setup: `%APPDATA%\EmblemWeave\logs\launcher.log`.
    /// The DWM result is written once at install, because a silent failure there
    /// is the difference between a direct restore and an animated one.
    fn launcher_log(line: &str) {
        let dir = crate::jobs::log_dir().unwrap_or_else(std::env::temp_dir);
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("launcher.log")) {
            let _ = writeln!(file, "{line}");
        }
        #[cfg(debug_assertions)]
        eprintln!("{line}");
    }

    unsafe fn slick_unsubclass_message() -> u32 {
        let cached = SLICK_UNSUBCLASS_MESSAGE.load(Ordering::Relaxed);
        if cached != 0 {
            return cached;
        }
        let name: Vec<u16> = "Windhawk_Unsubclass_slick-window-arrangement\0".encode_utf16().collect();
        let message = RegisterWindowMessageW(name.as_ptr());
        if message != 0 {
            SLICK_UNSUBCLASS_MESSAGE.store(message, Ordering::Relaxed);
        }
        message
    }

    /// The painted top bar and this hit test share the same insets; the
    /// non-interactive brand strip and the empty centre are caption, so the
    /// chips and window controls keep their own clicks.
    /// A maximized window has no caption left to move from. This window is
    /// borderless, so a caption press goes through winit's procedure, which
    /// starts the move only for a restored window. Restore it under the pointer
    /// once the press has crossed the drag threshold, then start the move from
    /// the place it was grabbed. A plain click must not restore it.
    unsafe fn restore_under_cursor(hwnd: HWND, cx: i32) {
        let mut maximized: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut maximized) == 0 {
            return;
        }
        let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
        placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
        if GetWindowPlacement(hwnd, &mut placement) == 0 {
            return;
        }
        let normal = placement.rcNormalPosition;
        // A maximized window's frame rect includes the invisible DWM border,
        // so its top is a few pixels above the visible caption. Keep the
        // pointer's offset from the client's top edge instead of scaling its Y
        // by the whole window height: scaling makes the caption jump upward
        // when the cursor is in the 44 px title bar.
        let mut client_origin = POINT { x: 0, y: 0 };
        let client_top = if ClientToScreen(hwnd, &mut client_origin) != 0 {
            client_origin.y
        } else {
            maximized.top
        };
        let mw = (maximized.right - maximized.left).max(1) as f32;
        let nw = (normal.right - normal.left).max(1);
        let nh = (normal.bottom - normal.top).max(1);
        // Horizontally the pointer keeps the same fraction of the window, so
        // the caption does not jump out from under it. Vertically the restored
        // window starts at the client top, preserving the pointer's offset
        // inside the caption.
        let fx = ((cx - maximized.left) as f32 / mw).clamp(0.0, 1.0);
        let x = cx - (fx * nw as f32).round() as i32;
        let y = client_top;
        // Clear the maximized style and set the rect directly. Both
        // `ShowWindow(SW_RESTORE)` and `SetWindowPlacement` run a window-manager
        // restore transition; the move loop then anchors on an intermediate
        // frame and the first pointer movement makes the window jump. Clearing
        // `WS_MAXIMIZE` has no transition, so the position the loop starts from
        // is the position under the pointer.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        if style != 0 {
            SetWindowLongPtrW(hwnd, GWL_STYLE, (style & !(WS_MAXIMIZE as isize)) as isize);
        }
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            x,
            y,
            nw,
            nh,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }

    /// How wide the resize band is, in logical pixels. It is scaled by the
    /// window's DPI, so the grab area is the same size on every monitor.
    const RESIZE_EDGE: f32 = 7.0;

    /// The non-client code for a point in one of the window's four edge bands.
    /// Windows' own hit-test turns these into the resize cursors and starts the
    /// sizing loop on the press, so the edge follows the pointer from the first
    /// movement; the old `shell_resize` only sent `BeginResize` after the frame
    /// that saw the press, which left the edge a frame behind.
    fn edge_at(x: i32, y: i32, width: i32, height: i32, edge: i32) -> Option<isize> {
        if width <= 0 || height <= 0 {
            return None;
        }
        let edge_x = edge.min(width / 2).max(1);
        let edge_y = edge.min(height / 2).max(1);
        let left = x >= 0 && x < edge_x;
        let right = x < width && x >= width - edge_x;
        let top = y >= 0 && y < edge_y;
        let bottom = y < height && y >= height - edge_y;
        Some(match (left, right, top, bottom) {
            (true, _, true, _) => HTTOPLEFT as isize,
            (_, true, true, _) => HTTOPRIGHT as isize,
            (true, _, _, true) => HTBOTTOMLEFT as isize,
            (_, true, _, true) => HTBOTTOMRIGHT as isize,
            (true, ..) => HTLEFT as isize,
            (_, true, ..) => HTRIGHT as isize,
            (_, _, true, _) => HTTOP as isize,
            (_, _, _, true) => HTBOTTOM as isize,
            _ => return None,
        })
    }

    unsafe fn begin_caption_move(hwnd: HWND, lparam: LPARAM) {
        let original = ORIGINAL.load(Ordering::Relaxed);
        if original != 0 {
            let original: WndProc = std::mem::transmute(original);
            CallWindowProcW(Some(original), hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, lparam);
        } else {
            DefWindowProcW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, lparam);
        }
    }

    fn point_from_lparam(lparam: LPARAM) -> POINT {
        POINT {
            x: (lparam & 0xffff) as u16 as i16 as i32,
            y: ((lparam >> 16) & 0xffff) as u16 as i16 as i32,
        }
    }

    fn pack_point(point: POINT) -> LPARAM {
        (((point.y & 0xffff) as u32) << 16 | ((point.x & 0xffff) as u32)) as LPARAM
    }

    fn key_down(key: u16) -> bool {
        (unsafe { GetAsyncKeyState(key as i32) } as u16 & 0x8000) != 0
    }

    /// Restore the window under the pointer and hand the press to the native
    /// caption move loop, from the place it was grabbed.
    unsafe fn start_maximized_move(hwnd: HWND, point: POINT) {
        restore_under_cursor(hwnd, point.x);
        ReleaseCapture();
        begin_caption_move(hwnd, pack_point(point));
    }

    /// The fallback after `DragDetect` returns without having tracked the
    /// press: take the capture and wait for the system drag threshold on the
    /// client move messages, exactly as before.
    unsafe fn begin_manual_caption_tracking(hwnd: HWND, point: POINT) {
        SetCapture(hwnd);
        PRESS_X.store(point.x, Ordering::Relaxed);
        PRESS_Y.store(point.y, Ordering::Relaxed);
        MAXIMIZED_PRESS.store(true, Ordering::Relaxed);
    }

    unsafe extern "system" fn hit_test(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // Windhawk's Slick Window Arrangement injects a subclass into every
        // window entering a native move. On WM_EXITSIZEMOVE it deliberately
        // keeps moving the HWND from a 10 ms timer, which is the launcher's
        // post-release glide. The mod's own unsubclass message removes that
        // subclass for this drag, leaving Windows' native move and snap intact.
        // Sending it is harmless when the mod is not installed.
        if msg == WM_ENTERSIZEMOVE {
            let message = slick_unsubclass_message();
            if message != 0 {
                SendMessageW(hwnd, message, 0, 0);
            }
        }
        if msg == WM_NCLBUTTONDOWN && wparam as u32 == HTCAPTION && IsZoomed(hwnd) != 0 {
            // A maximized window has no caption to move from, and this
            // borderless window's original procedure starts a move only for a
            // restored one. `DragDetect` owns the wait: it blocks until the
            // pointer leaves the system drag rectangle, the button is released,
            // or Escape is pressed. A click stays maximized; a drag restores
            // under the pointer and starts the native move loop. Its rectangle
            // starts at the press point, so a flick that is already past the
            // threshold when this handler runs still counts.
            //
            // It is documented for client presses but tracks this non-client
            // caption press too. If it ever returns without tracking while the
            // button is still down, the explicit threshold tracking below
            // keeps the tested behaviour as a fallback.
            let press = point_from_lparam(lparam);
            SetForegroundWindow(hwnd);
            let dragged = DragDetect(hwnd, press) != 0;
            let mut current = press;
            GetCursorPos(&mut current);
            if dragged {
                start_maximized_move(hwnd, current);
                return 0;
            }
            if key_down(VK_LBUTTON) && !key_down(VK_ESCAPE) {
                begin_manual_caption_tracking(hwnd, current);
            } else {
                ReleaseCapture();
            }
            return 0;
        }
        // The fallback holds the capture, so the press can only arrive here as
        // client-area move and up messages; no non-client variants are needed.
        if msg == WM_MOUSEMOVE && MAXIMIZED_PRESS.load(Ordering::Relaxed) {
            let mut point = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut point) != 0
                && crossed_drag_threshold(
                    point.x - PRESS_X.load(Ordering::Relaxed),
                    point.y - PRESS_Y.load(Ordering::Relaxed),
                    GetSystemMetrics(SM_CXDRAG),
                    GetSystemMetrics(SM_CYDRAG),
                )
            {
                MAXIMIZED_PRESS.store(false, Ordering::Relaxed);
                start_maximized_move(hwnd, point);
            }
            return 0;
        }
        if msg == WM_LBUTTONUP && MAXIMIZED_PRESS.swap(false, Ordering::Relaxed) {
            ReleaseCapture();
            return 0;
        }
        if msg == WM_CAPTURECHANGED {
            MAXIMIZED_PRESS.store(false, Ordering::Relaxed);
        }
        if msg == WM_NCHITTEST {
            let mut point = point_from_lparam(lparam);
            let mut client: RECT = std::mem::zeroed();
            if ScreenToClient(hwnd, &mut point) != 0 && GetClientRect(hwnd, &mut client) != 0 {
                let dpi = GetDpiForWindow(hwnd);
                let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
                // The edges win over the caption at the four corners: a press
                // on the band resizes, exactly as a decorated window's frame
                // does, while the centre of the title bar still moves.
                if IsZoomed(hwnd) == 0 {
                    let edge = (RESIZE_EDGE * scale).round() as i32;
                    if let Some(ht) = edge_at(point.x, point.y, client.right, client.bottom, edge) {
                        return ht as LRESULT;
                    }
                }
                let top = (crate::theme::TOPBAR_HEIGHT * scale).round() as i32;
                let left = (CAPTION_LEFT.load(Ordering::Relaxed) as f32 * scale).round() as i32;
                let right = client.right - (CAPTION_RIGHT.load(Ordering::Relaxed) as f32 * scale).round() as i32;
                let brand_left = (BRAND_LEFT.load(Ordering::Relaxed) as f32 * scale).round() as i32;
                let brand_right = (BRAND_RIGHT.load(Ordering::Relaxed) as f32 * scale).round() as i32;
                let in_band = |start: i32, end: i32| point.x >= start && point.x < end;
                if point.y >= 0 && point.y < top && (in_band(left, right) || in_band(brand_left, brand_right)) {
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

    pub fn set_caption_bounds(left: f32, right: f32) {
        CAPTION_LEFT.store(left.max(0.0).round() as i32, Ordering::Relaxed);
        CAPTION_RIGHT.store(right.max(0.0).round() as i32, Ordering::Relaxed);
    }

    pub fn set_brand_caption_bounds(left: f32, right: f32) {
        BRAND_LEFT.store(left.max(0.0).round() as i32, Ordering::Relaxed);
        BRAND_RIGHT.store(right.max(0.0).round() as i32, Ordering::Relaxed);
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
            let mut title = [0u16; 64];
            let title_len = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32).max(0) as usize;
            let mut class = [0u16; 64];
            let class_len = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32).max(0) as usize;
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            let original = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
            if original == 0 {
                launcher_log("native drag: GetWindowLongPtrW returned 0; not installed");
                return;
            }
            ORIGINAL.store(original, Ordering::Relaxed);
            if SetWindowLongPtrW(hwnd, GWLP_WNDPROC, hit_test as *const () as usize as isize) == 0 {
                ORIGINAL.store(0, Ordering::Relaxed);
                launcher_log("native drag: SetWindowLongPtrW failed; not installed");
                return;
            }
            launcher_log(&format!(
                "native drag installed hwnd=0x{:X} pid={} title={:?} class={:?} visible={} rect={},{} {}x{}",
                hwnd as usize,
                GetCurrentProcessId(),
                String::from_utf16_lossy(&title[..title_len]),
                String::from_utf16_lossy(&class[..class_len]),
                IsWindowVisible(hwnd),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top
            ));
            // The launcher is not an animated window: the transitions Windows
            // adds to a maximize/restore are exactly the delayed slide the
            // caption fix is meant to remove. The attribute is per window, so
            // no other application is affected. Its HRESULT is recorded: a
            // failure must be visible instead of silently leaving the animation
            // enabled.
            let disabled: i32 = 1;
            let transitions_hr = DwmSetWindowAttribute(
                hwnd,
                DWMWA_TRANSITIONS_FORCEDISABLED,
                &disabled as *const i32 as *const std::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
            let transitions_line = format!(
                "DwmSetWindowAttribute(TRANSITIONS_FORCEDISABLED) hr=0x{:08X} ({})",
                transitions_hr as u32,
                if transitions_hr >= 0 { "success" } else { "failed" }
            );
            launcher_log(&transitions_line);
            crate::crash::note(&transitions_line);
            // The custom frame would otherwise stay square against a desktop
            // full of rounded Windows 11 windows. DWM clips the window itself,
            // so the corners get its antialiasing and shadow for free; an
            // unsupported preference is a no-op, but its result is still logged.
            let rounded: i32 = DWMWCP_ROUND;
            let corner_hr = DwmSetWindowAttribute(
                hwnd,
                DWMWA_WINDOW_CORNER_PREFERENCE,
                &rounded as *const i32 as *const std::ffi::c_void,
                std::mem::size_of::<i32>() as u32,
            );
            launcher_log(&format!(
                "DwmSetWindowAttribute(WINDOW_CORNER_PREFERENCE) hr=0x{:08X} ({})",
                corner_hr as u32,
                if corner_hr >= 0 { "success" } else { "unsupported or failed" }
            ));
            INSTALLED.store(true, Ordering::Relaxed);
        }
    }

    fn crossed_drag_threshold(dx: i32, dy: i32, threshold_x: i32, threshold_y: i32) -> bool {
        dx.abs() >= threshold_x.max(1) || dy.abs() >= threshold_y.max(1)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_caption_click_waits_for_the_drag_threshold() {
            assert!(!crossed_drag_threshold(0, 0, 4, 4));
            assert!(!crossed_drag_threshold(3, -3, 4, 4));
            assert!(crossed_drag_threshold(4, 0, 4, 4));
            assert!(crossed_drag_threshold(0, -4, 4, 4));
            assert!(crossed_drag_threshold(-9, 1, 4, 4));
        }

        #[test]
        fn every_window_edge_and_corner_has_a_resize_band() {
            let (w, h, e) = (1440, 900, 7);
            assert_eq!(edge_at(3, 450, w, h, e), Some(HTLEFT as isize));
            assert_eq!(edge_at(w - 3, 450, w, h, e), Some(HTRIGHT as isize));
            assert_eq!(edge_at(700, 3, w, h, e), Some(HTTOP as isize));
            assert_eq!(edge_at(700, h - 3, w, h, e), Some(HTBOTTOM as isize));
            assert_eq!(edge_at(3, 3, w, h, e), Some(HTTOPLEFT as isize));
            assert_eq!(edge_at(w - 3, 3, w, h, e), Some(HTTOPRIGHT as isize));
            assert_eq!(edge_at(3, h - 3, w, h, e), Some(HTBOTTOMLEFT as isize));
            assert_eq!(edge_at(w - 3, h - 3, w, h, e), Some(HTBOTTOMRIGHT as isize));
        }

        #[test]
        fn the_middle_of_the_window_is_not_an_edge() {
            let (w, h, e) = (1440, 900, 7);
            assert_eq!(edge_at(700, 450, w, h, e), None);
            assert_eq!(edge_at(700, 22, w, h, e), None, "the caption band is not a resize edge");
            assert_eq!(edge_at(-10, 450, w, h, e), None, "a point outside the client is not claimed");
            assert_eq!(edge_at(700, h + 10, w, h, e), None);
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
