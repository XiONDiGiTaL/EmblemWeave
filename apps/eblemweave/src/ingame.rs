//! The in-game menu: a window that stays on top of the game while it runs,
//! opened and closed with F10 or Back + RB on a controller. It holds the
//! quick cheats, the cheat codes and tables, and the settings that matter
//! while playing.
//!
//! It is EmblemWeave's own window, not part of the game's engine, so it works
//! the same with any engine: cheats reach the game through its memory, and
//! settings through the files the engine reads.

use crate::theme;
use crate::widgets::{self, Tone};
use crate::App;
use eframe::egui::{self, RichText};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Quick,
    Codes,
    Display,
    Controls,
    States,
}

pub struct Ingame {
    pub open: bool,
    tab: Tab,
    running: bool,
    /// The game process whose window has been given EmblemWeave's icon.
    branded: Option<u32>,
    last_check: Instant,
    key_down: bool,
    pad_down: bool,
    paused: bool,
    confirm_stop: bool,
    /// A screenshot waiting for the menu to get out of the picture.
    shot_at: Option<Instant>,
}

impl Default for Ingame {
    fn default() -> Self {
        Ingame { open: false, tab: Tab::Quick, running: false, branded: None, last_check: Instant::now() - Duration::from_secs(10), key_down: false, pad_down: false, paused: false, confirm_stop: false, shot_at: None }
    }
}

#[cfg(windows)]
fn f10_held() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_F10};
    unsafe { (GetAsyncKeyState(VK_F10 as i32) as u16) & 0x8000 != 0 }
}

/// The engine's play settings file, where window mode and the like live.
pub fn controls_ini() -> Option<PathBuf> {
    crate::engine::run_dir().map(|d| d.join("controls.ini"))
}

/// Read `key` in `[section]` of an ini file.
pub fn ini_get(path: &Path, section: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_section = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_section = l.trim_matches(['[', ']']).eq_ignore_ascii_case(section);
        } else if in_section {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.split(';').next().unwrap_or("").trim().to_string());
                }
            }
        }
    }
    None
}

/// Set `key` in `[section]`, keeping everything else in the file as it was.
pub fn ini_set(path: &Path, section: &str, key: &str, value: &str) -> Result<(), String> {
    // Keep a copy of the settings before the first change of the session, so a
    // bad one can be undone.
    crate::snapshots::before("before-engine-settings");
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut out: Vec<String> = Vec::new();
    let (mut in_section, mut seen_section, mut done) = (false, false, false);
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            if in_section && !done {
                out.push(format!("{key} = {value}"));
                done = true;
            }
            in_section = l.trim_matches(['[', ']']).eq_ignore_ascii_case(section);
            seen_section |= in_section;
        } else if in_section && !done {
            if let Some((k, _)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    out.push(format!("{key} = {value}"));
                    done = true;
                    continue;
                }
            }
        }
        out.push(line.to_string());
    }
    if !done {
        if !seen_section {
            out.push(format!("[{section}]"));
        }
        out.push(format!("{key} = {value}"));
    }
    let mut s = out.join("\n");
    s.push('\n');
    std::fs::write(path, s).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// Called every frame: watches for the game, the hotkey and the controller
/// chord, and draws the menu window while it is open.
pub fn frame(app: &mut App, ctx: &egui::Context) {
    let now = Instant::now();
    if now.duration_since(app.ingame.last_check) > Duration::from_millis(700) {
        app.ingame.last_check = now;
        let was = app.ingame.running;
        app.ingame.running = crate::cheats::process::find(crate::cheats::GAME_EXE).is_some();
        #[cfg(windows)]
        if app.ingame.running {
            if let Some(pid) = game_pid().filter(|p| app.ingame.branded != Some(*p)) {
                // The window appears a moment after the process starts.
                if session::brand(pid, "EmblemWeave — Fire Emblem: Fortune's Weave") {
                    app.ingame.branded = Some(pid);
                }
            }
        } else {
            app.ingame.branded = None;
        }
        if app.ingame.running && !was {
            app.notify("Game started: press F10, or Back + RB on a controller, for the EmblemWeave menu", Tone::Ok);
        }
        if !app.ingame.running {
            app.ingame.open = false;
            app.ingame.paused = false;
            // The next run's units live at different addresses.
            app.cheats_ui.reset_units();
        }
    }
    if !app.ingame.running {
        return;
    }
    // The launcher may be in the background: keep looking at the keys.
    ctx.request_repaint_after(Duration::from_millis(80));
    #[cfg(windows)]
    {
        let k = f10_held();
        if k && !app.ingame.key_down {
            app.ingame.open = !app.ingame.open;
        }
        app.ingame.key_down = k;
        let pad = crate::cheats::held_keys();
        let chord = pad & crate::cheats::keys::MINUS != 0 && pad & crate::cheats::keys::R != 0;
        if chord && !app.ingame.pad_down {
            app.ingame.open = !app.ingame.open;
        }
        app.ingame.pad_down = chord;
    }
    #[cfg(windows)]
    if let Some(at) = app.ingame.shot_at {
        if Instant::now() >= at {
            app.ingame.shot_at = None;
            take_screenshot(app);
            app.ingame.open = true;
        }
    }
    if !app.ingame.open {
        return;
    }
    let vp = egui::ViewportId::from_hash_of("emblemweave-ingame");
    let builder = egui::ViewportBuilder::default()
        .with_title("EmblemWeave — in-game menu")
        .with_inner_size([560.0, 680.0])
        .with_always_on_top()
        .with_resizable(true);
    let builder = match crate::window_icon() {
        Some(icon) => builder.with_icon(std::sync::Arc::new(icon)),
        None => builder,
    };
    ctx.show_viewport_immediate(vp, builder, |ctx, _class| {
        if ctx.input(|i| i.viewport().close_requested()) {
            app.ingame.open = false;
            return;
        }
        let p = app.config.theme.palette();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg).inner_margin(egui::Margin::same(14))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                for (t, label) in [(Tab::Quick, "Quick cheats"), (Tab::Codes, "Codes & tables"), (Tab::Display, "Display"), (Tab::Controls, "Controls"), (Tab::States, "Save states")] {
                    if ui.selectable_label(app.ingame.tab == t, RichText::new(label).font(theme::semibold(13.5))).clicked() {
                        app.ingame.tab = t;
                    }
                }
            });
            ui.separator();
            session_bar(app, ui, &p, ctx);
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match app.ingame.tab {
                Tab::Quick => crate::pages::cheats::quick(app, ui),
                Tab::Codes => crate::pages::cheats::show(app, ui),
                Tab::Display => display(app, ui, &p),
                Tab::Controls => controls(ui, &p),
                Tab::States => {
                    widgets::status_line(ui, &p, Tone::Neutral, "Not available yet", "The engine the game runs on today has no save states. They are planned for EmblemWeave's own engine, which holds the whole game state in one place.");
                }
            });
        });
    });
}

fn display(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette) {
    let Some(ini) = controls_ini() else { return };
    widgets::section(ui, p, "Window", |ui| {
        let mode = ini_get(&ini, "display", "window_mode").unwrap_or_else(|| "windowed".into());
        ui.label("Alt+Enter in the game switches between a window and full screen at once. The mode it starts in:");
        ui.horizontal(|ui| {
            for (m, label) in [("windowed", "Windowed"), ("borderless", "Borderless full screen"), ("exclusive", "Exclusive full screen")] {
                if ui.selectable_label(mode == m, label).clicked() {
                    match ini_set(&ini, "display", "window_mode", m) {
                        Ok(()) => app.notify(format!("{label}: applies the next time the game starts (Alt+Enter switches now)"), Tone::Ok),
                        Err(e) => app.notify(e, Tone::Bad),
                    }
                }
            }
        });
        ui.add_space(6.0);
        let mut w: u32 = ini_get(&ini, "display", "render_width").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut h: u32 = ini_get(&ini, "display", "render_height").and_then(|v| v.parse().ok()).unwrap_or(0);
        ui.label("Picture size (0 = the window's size when the game starts):");
        ui.horizontal(|ui| {
            let a = ui.add(egui::DragValue::new(&mut w).range(0..=7680).suffix(" wide"));
            let b = ui.add(egui::DragValue::new(&mut h).range(0..=4320).suffix(" high"));
            if a.changed() || b.changed() {
                let _ = ini_set(&ini, "display", "render_width", &w.to_string());
                let _ = ini_set(&ini, "display", "render_height", &h.to_string());
            }
            for (pw, ph) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160)] {
                if ui.small_button(format!("{pw}x{ph}")).clicked() {
                    let _ = ini_set(&ini, "display", "render_width", &pw.to_string());
                    let _ = ini_set(&ini, "display", "render_height", &ph.to_string());
                }
            }
        });
        ui.label(RichText::new("The picture size applies the next time the game starts.").size(12.0).color(p.muted));
    });
    widgets::section(ui, p, "Frame rate counter", |ui| {
        let mut on = ini_get(&ini, "fps", "show").as_deref() == Some("1");
        ui.horizontal(|ui| {
            if widgets::toggle(ui, p, &mut on).changed() {
                let _ = ini_set(&ini, "fps", "show", if on { "1" } else { "0" });
            }
            ui.label("Show the frame rate in a corner of the game (applies on the next frame; F2 in the game too)");
        });
    });
}

fn controls(ui: &mut egui::Ui, p: &crate::theme::Palette) {
    widgets::section(ui, p, "Controls", |ui| {
        ui.label("Keyboard and controller buttons can be rebound in the game's own settings: press F1 in the game, then Controls. The launcher's Controls page holds EmblemWeave's bindings for its own engine.");
        ui.add_space(6.0);
        egui::Grid::new("default-keys").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
            for (k, what) in [("F10, or Back + RB", "this menu"), ("F1", "the game engine's settings"), ("F2", "frame rate counter"), ("Alt+Enter", "window / full screen")] {
                ui.label(RichText::new(k).font(theme::mono(13.0)).color(p.ink));
                ui.label(what);
                ui.end_row();
            }
        });
    });
}

/// Pausing, stopping and capturing the game's process from outside it.
#[cfg(windows)]
pub mod session {
    use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::*;
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32};
    use windows_sys::Win32::System::Threading::{OpenProcess, OpenThread, ResumeThread, SuspendThread, TerminateProcess, PROCESS_TERMINATE, THREAD_SUSPEND_RESUME};
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible, SetForegroundWindow};

    fn threads(pid: u32) -> Vec<u32> {
        let mut out = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snap.is_null() || snap as isize == -1 {
                return out;
            }
            let mut e: THREADENTRY32 = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            let mut ok = Thread32First(snap, &mut e);
            while ok != 0 {
                if e.th32OwnerProcessID == pid {
                    out.push(e.th32ThreadID);
                }
                ok = Thread32Next(snap, &mut e);
            }
            CloseHandle(snap);
        }
        out
    }

    /// Suspend (`true`) or resume every thread of the process. Returns how
    /// many threads were reached.
    pub fn set_paused(pid: u32, pause: bool) -> usize {
        let mut n = 0;
        for t in threads(pid) {
            unsafe {
                let h = OpenThread(THREAD_SUSPEND_RESUME, 0, t);
                if h.is_null() {
                    continue;
                }
                let r = if pause { SuspendThread(h) } else { ResumeThread(h) };
                if r != u32::MAX {
                    n += 1;
                }
                CloseHandle(h);
            }
        }
        n
    }

    pub fn stop(pid: u32) -> bool {
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if h.is_null() {
                return false;
            }
            let ok = TerminateProcess(h, 0) != 0;
            CloseHandle(h);
            ok
        }
    }

    struct Find {
        pid: u32,
        best: HWND,
        area: i64,
    }

    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> i32 {
        let f = unsafe { &mut *(lp as *mut Find) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == f.pid && unsafe { IsWindowVisible(hwnd) } != 0 {
            let mut r: RECT = unsafe { std::mem::zeroed() };
            unsafe { GetWindowRect(hwnd, &mut r) };
            let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
            if area > f.area {
                f.area = area;
                f.best = hwnd;
            }
        }
        1
    }

    /// The game's largest visible window.
    pub fn window(pid: u32) -> Option<HWND> {
        let mut f = Find { pid, best: std::ptr::null_mut(), area: 0 };
        unsafe { EnumWindows(Some(each), &mut f as *mut Find as LPARAM) };
        (!f.best.is_null()).then_some(f.best)
    }

    /// Give the game's window EmblemWeave's icon and title. The engine's
    /// window carries its own icon otherwise. Returns whether a window was
    /// found.
    pub fn brand(pid: u32, title: &str) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{LoadImageW, SendMessageW, SetWindowTextW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_LOADFROMFILE, WM_SETICON};
        let Some(w) = window(pid) else { return false };
        let Some(path) = icon_file() else { return false };
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        unsafe {
            let big = LoadImageW(std::ptr::null_mut(), wide.as_ptr(), IMAGE_ICON, 32, 32, LR_LOADFROMFILE);
            let small = LoadImageW(std::ptr::null_mut(), wide.as_ptr(), IMAGE_ICON, 16, 16, LR_LOADFROMFILE);
            if !big.is_null() {
                SendMessageW(w, WM_SETICON, ICON_BIG as usize, big as isize);
            }
            if !small.is_null() {
                SendMessageW(w, WM_SETICON, ICON_SMALL as usize, small as isize);
            }
            let t: Vec<u16> = title.encode_utf16().chain([0]).collect();
            SetWindowTextW(w, t.as_ptr());
        }
        true
    }

    /// EmblemWeave's icon as a file Windows can load, in the settings folder.
    fn icon_file() -> Option<std::path::PathBuf> {
        let p = crate::config::Config::dir()?.join("icon.ico");
        let bytes = include_bytes!("../assets/icon.ico");
        if std::fs::read(&p).ok().as_deref() != Some(&bytes[..]) {
            std::fs::create_dir_all(p.parent()?).ok()?;
            std::fs::write(&p, bytes).ok()?;
        }
        Some(p)
    }

    pub fn focus(pid: u32) {
        if let Some(w) = window(pid) {
            unsafe { SetForegroundWindow(w) };
        }
    }

    /// The game window's own picture (its client area), as RGBA. The window
    /// draws itself into the capture (PrintWindow with full-content
    /// rendering), so nothing that overlaps it on screen is ever included.
    /// A window that cannot be captured that way gives `None`.
    pub fn capture(pid: u32) -> Option<(u32, u32, Vec<u8>)> {
        use windows_sys::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};
        use windows_sys::Win32::UI::WindowsAndMessaging::GetClientRect;
        let w = window(pid)?;
        let mut r: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetClientRect(w, &mut r) };
        let (width, height) = ((r.right - r.left).max(0), (r.bottom - r.top).max(0));
        if width == 0 || height == 0 {
            return None;
        }
        unsafe {
            let wdc = GetDC(w);
            let mem = CreateCompatibleDC(wdc);
            let bmp = CreateCompatibleBitmap(wdc, width, height);
            let old = SelectObject(mem, bmp);
            // PW_CLIENTONLY | PW_RENDERFULLCONTENT
            let ok = PrintWindow(w, mem, 1 | 2 as PRINT_WINDOW_FLAGS) != 0;
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width;
            info.bmiHeader.biHeight = -height;
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            let mut px = vec![0u8; (width * height * 4) as usize];
            let got = GetDIBits(mem, bmp, 0, height as u32, px.as_mut_ptr() as _, &mut info, DIB_RGB_COLORS);
            SelectObject(mem, old);
            DeleteObject(bmp);
            DeleteDC(mem);
            ReleaseDC(w, wdc);
            if !ok || got == 0 {
                return None;
            }
            for c in px.chunks_exact_mut(4) {
                c.swap(0, 2);
                c[3] = 255;
            }
            // An all-black picture means the window could not draw itself here.
            if px.chunks_exact(4).all(|c| c[0] == 0 && c[1] == 0 && c[2] == 0) {
                return None;
            }
            Some((width as u32, height as u32, px))
        }
    }
}

/// Where screenshots go: Pictures\EmblemWeave.
pub fn screenshot_dir() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_default().join("Pictures").join("EmblemWeave")
}

fn game_pid() -> Option<u32> {
    crate::cheats::process::find(crate::cheats::GAME_EXE)
}

#[cfg(windows)]
fn take_screenshot(app: &mut App) {
    let Some(pid) = game_pid() else { return };
    match session::capture(pid) {
        Some((w, h, px)) => {
            let dir = screenshot_dir();
            let _ = std::fs::create_dir_all(&dir);
            let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
            let path = dir.join(format!("FortunesWeave-{secs}.png"));
            match image::RgbaImage::from_raw(w, h, px).map(|img| img.save(&path)) {
                Some(Ok(())) => app.notify(format!("Screenshot saved to {}", path.display()), Tone::Ok),
                _ => app.notify("Could not save the screenshot", Tone::Bad),
            }
        }
        None => app.notify("The game window could not be captured (try windowed or borderless instead of exclusive full screen)", Tone::Bad),
    }
}

/// Pause, screenshot, restart and stop.
fn session_bar(app: &mut App, ui: &mut egui::Ui, p: &crate::theme::Palette, ctx: &egui::Context) {
    #[cfg(windows)]
    {
        let Some(pid) = game_pid() else { return };
        ui.horizontal_wrapped(|ui| {
            if widgets::button(ui, p, if app.ingame.paused { "▶ Resume" } else { "⏸ Pause" }, true).clicked() {
                let pause = !app.ingame.paused;
                if session::set_paused(pid, pause) > 0 {
                    app.ingame.paused = pause;
                    if !pause {
                        session::focus(pid);
                    }
                }
            }
            if widgets::button(ui, p, "📷 Screenshot", !app.ingame.paused).on_hover_text("Saved to the EmblemWeave folder in Pictures").clicked() {
                app.ingame.open = false;
                app.ingame.shot_at = Some(Instant::now() + Duration::from_millis(350));
            }
            if widgets::button(ui, p, "⟲ Restart", true).on_hover_text("Closes the game and starts it again, at the title screen. Unsaved progress is lost.").clicked() {
                app.ingame.confirm_stop = false;
                if app.ingame.paused {
                    session::set_paused(pid, false);
                }
                if session::stop(pid) {
                    std::thread::sleep(Duration::from_millis(600));
                    match crate::engine::play() {
                        Ok(()) => app.notify("Restarting the game", Tone::Ok),
                        Err(e) => app.notify(e, Tone::Bad),
                    }
                }
                app.ingame.paused = false;
            }
            if !app.ingame.confirm_stop {
                if widgets::button(ui, p, "⏹ Stop", true).clicked() {
                    app.ingame.confirm_stop = true;
                }
            } else {
                ui.label(RichText::new("Stop the game? Unsaved progress is lost.").color(p.warn));
                if widgets::button(ui, p, "Stop and return", true).clicked() {
                    if app.ingame.paused {
                        session::set_paused(pid, false);
                    }
                    if session::stop(pid) {
                        app.ingame.open = false;
                        app.ingame.paused = false;
                        ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
                        app.page = crate::pages::Page::Overview;
                        app.notify("The game was stopped", Tone::Ok);
                    }
                    app.ingame.confirm_stop = false;
                }
                if widgets::button(ui, p, "Cancel", true).clicked() {
                    app.ingame.confirm_stop = false;
                }
            }
        });
        if app.ingame.paused {
            ui.label(RichText::new("Paused: the game and its sound are frozen until you resume.").size(12.0).color(p.muted));
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    /// Brands a running game window and reads its title back.
    #[test]
    #[ignore]
    fn live_brand() {
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowTextW;
        let pid = crate::cheats::process::find(crate::cheats::GAME_EXE).expect("start the game first");
        assert!(super::session::brand(pid, "EmblemWeave — Fire Emblem: Fortune's Weave"));
        let w = super::session::window(pid).unwrap();
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(w, buf.as_mut_ptr(), 256) } as usize;
        println!("title now: {}", String::from_utf16_lossy(&buf[..n]));
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let (Some((w, h, px)), Ok(out)) = (super::session::capture(pid), std::env::var("EW_SHOT")) {
            image::RgbaImage::from_raw(w, h, px).unwrap().save(out).unwrap();
        }
    }
}
