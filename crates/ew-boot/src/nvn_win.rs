//! NVN windows: the swap chain the game renders into and presents.
//!
//! A window owns the textures the game handed its builder. Acquiring returns
//! the next texture in turn; presenting records the frame. The frames are what
//! the host display shows once the renderer executes the game's commands; until
//! then the run report counts them, so a present is never mistaken for an
//! image on screen.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use crate::nvn::called;
use ew_lift::State;
use std::collections::BTreeMap;

const NVN_TRUE: u64 = 1;
/// `NVN_WINDOW_ACQUIRE_TEXTURE_RESULT_SUCCESS`.
const ACQUIRE_SUCCESS: u64 = 0;

#[derive(Debug, Clone, Default)]
pub struct Window {
    pub textures: Vec<u64>,
    pub active: u32,
    pub next: u32,
    pub present_interval: u32,
    pub acquire_timeout: u64,
    pub crop: [i32; 4],
    pub native: u64,
    pub presented: u64,
    pub last_presented: Option<u32>,
}

#[derive(Default)]
pub struct Windows {
    pub windows: BTreeMap<u64, Window>,
    /// Every present, as (window, texture index), most recent last, capped.
    pub frames: u64,
}

fn win<R>(state: &State, f: impl FnOnce(&mut Windows) -> R) -> R {
    state.host.ext.with::<Windows, R>(f)
}

/// Frames presented so far, across every window.
pub fn frames_presented(state: &State) -> u64 {
    win(state, |w| w.frames)
}

fn recorded_args(state: &State, obj: u64, setter: &str) -> Option<Vec<u64>> {
    state.host.ext.with::<crate::nvn::Nvn, _>(|n| n.state.get(&obj).and_then(|m| m.get(setter)).cloned())
}

/// `nvnWindowInitialize(NVNwindow*, const NVNwindowBuilder*)`.
fn window_initialize(state: &mut State) -> HostResult {
    let (w, b) = (state.x[0], state.x[1]);
    let Some(t) = recorded_args(state, b, "nvnWindowBuilderSetTextures") else {
        return HostResult::Unavailable("nvnWindowInitialize with no textures set".into());
    };
    let (count, list) = (t[0] & 0xFFFF_FFFF, t[1]);
    if count == 0 || count > 16 {
        return HostResult::Unavailable(format!("nvnWindowInitialize with {count} textures"));
    }
    let bytes = match read_bytes(state, list, count * 8) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let textures: Vec<u64> = bytes.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())).collect();
    let interval = recorded_args(state, b, "nvnWindowBuilderSetPresentInterval").map(|a| a[0] as u32).unwrap_or(1);
    let native = recorded_args(state, b, "nvnWindowBuilderSetNativeWindow").map(|a| a[0]).unwrap_or(0);
    let size = state.host.ext.with::<crate::nvn_tex::Textures, _>(|x| {
        x.textures.get(&textures[0]).map(|d| (d.width, d.height, d.format))
    });
    let Some((width, height, format)) = size else {
        return HostResult::Unavailable(format!("nvnWindowInitialize: texture 0x{:x} was never initialized", textures[0]));
    };
    state.host.log.push(format!(
        "gfx: window 0x{w:x}: {count} textures of {width}x{height}, format 0x{format:x}, present interval {interval}"
    ));
    win(state, |x| {
        x.windows.insert(
            w,
            Window {
                active: count as u32,
                textures,
                present_interval: interval,
                acquire_timeout: u64::MAX,
                crop: [0, 0, width as i32, height as i32],
                native,
                ..Window::default()
            },
        )
    });
    HostResult::Returned(NVN_TRUE)
}

fn with_window(state: &mut State, f: impl FnOnce(&mut Window) -> u64) -> HostResult {
    let w = state.x[0];
    match win(state, |x| x.windows.get_mut(&w).map(f)) {
        Some(v) => HostResult::Returned(v),
        None => HostResult::Unavailable(format!("{} on window 0x{w:x}, which was never initialized", called(state))),
    }
}

/// `nvnWindowAcquireTexture(NVNwindow*, NVNsync*, int* textureIndex)`: the next
/// texture in turn. Its previous frame has been presented already, so the sync
/// is signalled at once.
fn window_acquire_texture(state: &mut State) -> HostResult {
    let (w, sync, out) = (state.x[0], state.x[1], state.x[2]);
    let Some(index) = win(state, |x| {
        x.windows.get_mut(&w).map(|win| {
            let i = win.next % win.active.max(1);
            win.next = (i + 1) % win.active.max(1);
            i
        })
    }) else {
        return HostResult::Unavailable(format!("nvnWindowAcquireTexture on window 0x{w:x}, which was never initialized"));
    };
    state.host.ext.with::<crate::nvn_mem::Memory, _>(|m| m.signalled.insert(sync, true));
    match write_bytes(state, out, &(index as i32).to_le_bytes()) {
        Ok(()) => HostResult::Returned(ACQUIRE_SUCCESS),
        Err(e) => e,
    }
}

/// The window the game's frames are shown in, once one is asked for.
#[derive(Default)]
pub struct Display {
    enabled: bool,
    title: String,
    #[cfg(windows)]
    presenter: Option<ew_gfx::present::Presenter>,
    /// Why the frames cannot be shown, kept so it is logged once.
    problem: Option<String>,
    pub frames: u64,
}

// The presenter owns a mapped Vulkan buffer, whose address is a raw pointer.
// Only the run loop's thread presents, and the window is touched nowhere else,
// so the state can carry it.
unsafe impl Send for Display {}

fn display<R>(state: &State, f: impl FnOnce(&mut Display) -> R) -> R {
    state.host.ext.with::<Display, R>(f)
}

/// Show the game's frames in a window for the rest of the run. The window
/// itself opens on the first present, when the frame's size is known.
pub fn show_frames_in_a_window(state: &mut State, title: &str) {
    display(state, |d| {
        d.enabled = true;
        d.title = title.to_string();
    });
}

/// Put the frame the game just presented on screen, if a window is wanted.
fn present_frame(state: &mut State, window: u64, index: u32) {
    if !display(state, |d| d.enabled && d.problem.is_none()) {
        return;
    }
    let Some(texture) = win(state, |x| x.windows.get(&window).and_then(|w| w.textures.get(index as usize).copied())) else {
        return;
    };
    let (width, height, pixels) = match crate::nvn_tex::frame_bgra(state, texture) {
        Ok(frame) => frame,
        Err(e) => {
            let first = display(state, |d| d.problem.replace(e.clone()).is_none());
            if first {
                state.host.log.push(format!("gfx: the game's frames cannot be shown: {e}"));
            }
            return;
        }
    };
    #[cfg(windows)]
    let shown = display(state, |d| {
        use ew_gfx::present::{Frame, Presenter};
        if d.presenter.is_none() {
            match Presenter::open(&d.title, 1280, 720) {
                Ok(p) => d.presenter = Some(p),
                Err(e) => {
                    d.problem = Some(e.clone());
                    return Some(Err(e));
                }
            }
        }
        let p = d.presenter.as_mut()?;
        Some(p.show(Frame { width, height, bgra: &pixels }))
    });
    #[cfg(not(windows))]
    let shown: Option<Result<bool, String>> = {
        let _ = (&pixels, width, height);
        None
    };
    match shown {
        None => {}
        Some(Ok(true)) => {
            let n = display(state, |d| {
                d.frames += 1;
                d.frames
            });
            if n == 1 {
                state.host.log.push(format!("gfx: first frame on screen ({width}x{height})"));
            }
        }
        Some(Ok(false)) => {
            display(state, |d| {
                d.enabled = false;
                #[cfg(windows)]
                {
                    d.presenter = None;
                }
            });
            state.host.log.push("gfx: the window was closed; frames are counted only from here".to_string());
        }
        Some(Err(e)) => {
            let first = display(state, |d| d.problem.replace(e.clone()).is_none());
            if first {
                state.host.log.push(format!("gfx: the game's frames cannot be shown: {e}"));
            }
        }
    }
}

/// `nvnQueuePresentTexture(NVNqueue*, NVNwindow*, int textureIndex)`.
fn queue_present_texture(state: &mut State) -> HostResult {
    let (w, index) = (state.x[1], state.x[2] as u32);
    present_frame(state, w, index);
    let (first, frames) = win(state, |x| {
        x.frames += 1;
        let first = x.windows.get_mut(&w).map(|win| {
            win.presented += 1;
            win.last_presented = Some(index);
            win.presented == 1
        });
        (first, x.frames)
    });
    if frames % 300 == 0 {
        state.host.log.push(format!("gfx: {frames} frames presented"));
    }
    match first {
        Some(true) => {
            state.host.log.push(format!("gfx: first frame presented on window 0x{w:x} (texture {index})"));
            HostResult::Returned(0)
        }
        Some(false) => HostResult::Returned(0),
        None => HostResult::Unavailable(format!("nvnQueuePresentTexture on window 0x{w:x}, which was never initialized")),
    }
}

fn get_present_interval(state: &mut State) -> HostResult {
    with_window(state, |w| w.present_interval as u64)
}

fn set_present_interval(state: &mut State) -> HostResult {
    let v = state.x[1] as u32;
    with_window(state, |w| {
        w.present_interval = v;
        0
    })
}

fn get_acquire_timeout(state: &mut State) -> HostResult {
    with_window(state, |w| w.acquire_timeout)
}

fn set_acquire_timeout(state: &mut State) -> HostResult {
    let v = state.x[1];
    with_window(state, |w| {
        w.acquire_timeout = v;
        0
    })
}

fn set_crop(state: &mut State) -> HostResult {
    let c = [state.x[1] as i32, state.x[2] as i32, state.x[3] as i32, state.x[4] as i32];
    with_window(state, |w| {
        w.crop = c;
        0
    })
}

/// `nvnWindowGetCrop(const NVNwindow*, NVNrectangle*)`.
fn get_crop(state: &mut State) -> HostResult {
    let (w, out) = (state.x[0], state.x[1]);
    let Some(c) = win(state, |x| x.windows.get(&w).map(|w| w.crop)) else {
        return HostResult::Unavailable(format!("nvnWindowGetCrop on window 0x{w:x}, which was never initialized"));
    };
    let bytes: Vec<u8> = c.iter().flat_map(|v| v.to_le_bytes()).collect();
    match write_bytes(state, out, &bytes) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn set_num_active(state: &mut State) -> HostResult {
    let n = state.x[1] as u32;
    with_window(state, |w| {
        w.active = n.clamp(1, w.textures.len() as u32);
        0
    })
}

fn get_num_active(state: &mut State) -> HostResult {
    with_window(state, |w| w.active as u64)
}

fn get_num_textures(state: &mut State) -> HostResult {
    with_window(state, |w| w.textures.len() as u64)
}

fn get_native_window(state: &mut State) -> HostResult {
    with_window(state, |w| w.native)
}

fn window_finalize(state: &mut State) -> HostResult {
    let w = state.x[0];
    win(state, |x| x.windows.remove(&w));
    HostResult::Returned(0)
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("nvnWindowInitialize", window_initialize, "a swap chain over the game's textures"),
        ("nvnWindowFinalize", window_finalize, "forgets the window"),
        ("nvnWindowAcquireTexture", window_acquire_texture, "next texture in turn, sync signalled"),
        ("nvnQueuePresentTexture", queue_present_texture, "the frame goes to the window when one is open"),
        ("nvnWindowGetPresentInterval", get_present_interval, "window state on the host"),
        ("nvnWindowSetPresentInterval", set_present_interval, "window state on the host"),
        ("nvnWindowGetAcquireTimeout", get_acquire_timeout, "window state on the host"),
        ("nvnWindowSetAcquireTimeout", set_acquire_timeout, "window state on the host"),
        ("nvnWindowSetCrop", set_crop, "window state on the host"),
        ("nvnWindowGetCrop", get_crop, "window state on the host"),
        ("nvnWindowSetNumActiveTextures", set_num_active, "window state on the host"),
        ("nvnWindowGetNumActiveTextures", get_num_active, "window state on the host"),
        ("nvnWindowGetNumTextures", get_num_textures, "window state on the host"),
        ("nvnWindowGetNativeWindow", get_native_window, "the native window handle the builder was given"),
    ];
    for (name, handler, claim) in table {
        s.register(&format!("nvn:{name}"), *handler, claim);
    }
}
