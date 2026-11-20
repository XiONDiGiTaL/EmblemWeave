//! Discord Rich Presence over Discord's local IPC pipe.
//!
//! Discord exposes a named pipe (`\\.\pipe\discord-ipc-N`, also reachable as
//! `\\?\pipe\discord-ipc-N`) to programs on the same PC. There is no network
//! connection and no token involved: the pipe is only there while the Discord
//! desktop client is running. The application id is EmblemWeave's own; it is
//! public and built in, so the player only chooses whether presence is shown.
//!
//! The connection lives on a worker thread so a closed Discord client can
//! never delay the launcher. It reconnects while the launcher or game runs, and
//! switches back to the launcher's presence when the game exits.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// EmblemWeave's public Discord application id. Rich Presence uses Discord's
/// local IPC pipe and needs no client secret or token, so this is safe to
/// embed. Players never enter or override it.
pub const APPLICATION_ID: &str = "1573082800373108826";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Launcher,
    Game,
}

pub struct Presence {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    /// The id, pid and mode currently owned by the worker. Comparing this makes
    /// the per-frame calls cheap and stops a 700 ms poll from reconnecting.
    current: Option<(String, u32, Mode)>,
}

impl Default for Presence {
    fn default() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(false)),
            worker: None,
            current: None,
        }
    }
}

impl Presence {
    /// Show the launcher's own presence while it is open.
    pub fn start_launcher(&mut self) {
        self.start(std::process::id(), Mode::Launcher);
    }

    /// Show the game's presence while `pid` runs.
    pub fn start_game(&mut self, pid: u32) {
        self.start(pid, Mode::Game);
    }

    fn start(&mut self, pid: u32, mode: Mode) {
        let key = (APPLICATION_ID.to_string(), pid, mode);
        if self.worker.is_some() && self.current.as_ref() == Some(&key) {
            return;
        }
        self.stop();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let owned = key.0.clone();
        self.worker = Some(std::thread::spawn(move || run(&owned, pid, mode, flag)));
        self.stop = stop;
        self.current = Some(key);
    }

    pub fn stop(&mut self) {
        self.current = None;
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    pub fn active(&self) -> bool {
        self.worker.is_some()
    }
}

impl Drop for Presence {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(client_id: &str, pid: u32, mode: Mode, stop: Arc<AtomicBool>) {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    while !stop.load(Ordering::Relaxed) {
        let Some(mut pipe) = connect() else {
            sleep(&stop, Duration::from_secs(10));
            continue;
        };
        if send(
            &mut pipe,
            0,
            &format!("{{\"v\":1,\"client_id\":\"{client_id}\"}}"),
        )
        .is_err()
        {
            sleep(&stop, Duration::from_secs(5));
            continue;
        }
        // Discord needs a moment to process the handshake before the first
        // SET_ACTIVITY frame. We do not block waiting for READY: a client that
        // is closing must not hold the worker.
        sleep(&stop, Duration::from_millis(250));
        if !service(&mut pipe) {
            sleep(&stop, Duration::from_secs(5));
            continue;
        }
        let activity = activity(pid, started, mode);
        while !stop.load(Ordering::Relaxed) {
            if send(&mut pipe, 1, &activity).is_err() {
                break;
            }
            if !service(&mut pipe) {
                break;
            }
            sleep(&stop, Duration::from_secs(15));
        }
    }
}

fn sleep(stop: &AtomicBool, time: Duration) {
    let until = std::time::Instant::now() + time;
    while std::time::Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn connect() -> Option<std::fs::File> {
    // Discord documents `\\?\pipe\`; `\\.\pipe\` is the same namespace and is
    // what older clients expose. Try both so a client update cannot break it.
    for prefix in [r"\\.\pipe", r"\\?\pipe"] {
        for n in 0..10 {
            let path = format!("{prefix}\\discord-ipc-{n}");
            if let Ok(f) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(path)
            {
                return Some(f);
            }
        }
    }
    None
}

/// Read whatever Discord has already put in the pipe and answer its PING
/// frames. The pipe is peeked first so a quiet Discord client never blocks the
/// presence worker on a read. Returns false when Discord closed the connection.
#[cfg(windows)]
fn service(pipe: &mut std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;

    loop {
        let mut available = 0u32;
        let ok = unsafe {
            PeekNamedPipe(
                pipe.as_raw_handle() as _,
                std::ptr::null_mut(),
                0,
                std::ptr::null_mut(),
                &mut available,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || available < 8 {
            return ok != 0;
        }
        let mut header = [0u8; 8];
        if pipe.read_exact(&mut header).is_err() {
            return false;
        }
        let opcode = u32::from_le_bytes(header[0..4].try_into().unwrap());
        let len = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
        if len > 1024 * 1024 {
            return false;
        }
        let mut payload = vec![0u8; len];
        if pipe.read_exact(&mut payload).is_err() {
            return false;
        }
        match opcode {
            3 => {
                // Discord echoes the ping payload in its PONG.
                let text = String::from_utf8_lossy(&payload);
                if send(pipe, 4, &text).is_err() {
                    return false;
                }
            }
            2 => return false,
            _ => {}
        }
    }
}

#[cfg(not(windows))]
fn service(_pipe: &mut std::fs::File) -> bool {
    true
}

fn send(pipe: &mut std::fs::File, opcode: u32, json: &str) -> std::io::Result<()> {
    let mut frame = Vec::with_capacity(8 + json.len());
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&(json.len() as u32).to_le_bytes());
    frame.extend_from_slice(json.as_bytes());
    pipe.write_all(&frame)?;
    pipe.flush()
}

fn activity(pid: u32, started: u64, mode: Mode) -> String {
    // No images: an asset key only works after it has been uploaded to the
    // Discord application, and asking for a missing one makes Discord reject
    // the whole activity.
    let (details, state) = match mode {
        Mode::Launcher => ("In the launcher", "Fire Emblem: Fortune's Weave"),
        Mode::Game => ("Playing the campaign", "Fire Emblem: Fortune's Weave"),
    };
    format!(
        "{{\"cmd\":\"SET_ACTIVITY\",\"args\":{{\"pid\":{pid},\"activity\":{{\
         \"details\":\"{details}\",\
         \"state\":\"{state}\",\
         \"timestamps\":{{\"start\":{started}}}}}}},\
         \"nonce\":\"{started}-{pid}\"}}"
    )
}
