//! Discord Rich Presence over Discord's local IPC pipe.
//!
//! Discord exposes a named pipe (`\\.\pipe\discord-ipc-N`) to programs on the
//! same PC. There is no network connection and no token involved: the pipe is
//! only there while the Discord desktop client is running. A presence needs a
//! Discord application id (the public "client id" from the Developer Portal);
//! without one there is nothing to show, so the feature stays off.
//!
//! The connection lives on a worker thread so a closed Discord client can
//! never delay the launcher. It reconnects while the game runs, and stops as
//! soon as the game exits.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub struct Presence {
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Default for Presence {
    fn default() -> Self {
        Self { stop: Arc::new(AtomicBool::new(false)), worker: None }
    }
}

impl Presence {
    /// Start showing `client_id` while `pid` runs. An empty or non-numeric id
    /// does nothing: Discord ids are decimal snowflakes.
    pub fn start(&mut self, client_id: &str, pid: u32) {
        self.stop();
        let id = client_id.trim();
        if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return;
        }
        let id = id.to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        self.worker = Some(std::thread::spawn(move || run(&id, pid, flag)));
        self.stop = stop;
    }

    pub fn stop(&mut self) {
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

fn run(client_id: &str, pid: u32, stop: Arc<AtomicBool>) {
    let started = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    while !stop.load(Ordering::Relaxed) {
        let Some(mut pipe) = connect() else {
            sleep(&stop, Duration::from_secs(10));
            continue;
        };
        if send(&mut pipe, 0, &format!("{{\"v\":1,\"client_id\":\"{client_id}\"}}")).is_err() {
            sleep(&stop, Duration::from_secs(5));
            continue;
        }
        let activity = activity(pid, started);
        while !stop.load(Ordering::Relaxed) {
            if send(&mut pipe, 1, &activity).is_err() {
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
    for n in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{n}");
        if let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).open(path) {
            return Some(f);
        }
    }
    None
}

fn send(pipe: &mut std::fs::File, opcode: u32, json: &str) -> std::io::Result<()> {
    let mut frame = Vec::with_capacity(8 + json.len());
    frame.extend_from_slice(&opcode.to_le_bytes());
    frame.extend_from_slice(&(json.len() as u32).to_le_bytes());
    frame.extend_from_slice(json.as_bytes());
    pipe.write_all(&frame)?;
    pipe.flush()
}

fn activity(pid: u32, started: u64) -> String {
    // No images: an asset key only works after it has been uploaded to the
    // Discord application, and asking for a missing one makes Discord reject
    // the whole activity.
    format!(
        "{{\"cmd\":\"SET_ACTIVITY\",\"args\":{{\"pid\":{pid},\"activity\":{{\
         \"details\":\"Playing the campaign\",\
         \"state\":\"Fire Emblem: Fortune's Weave\",\
         \"timestamps\":{{\"start\":{started}}}}}}},\
         \"nonce\":\"{started}-{pid}\"}}"
    )
}
