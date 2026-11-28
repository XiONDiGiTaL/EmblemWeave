//! Find Units, supervised (issue #76).
//!
//! `ReadProcessMemory` has no timeout, no cancellation and no asynchronous form. Measured against this game, one call
//! sat inside `ntdll!NtReadVirtualMemory` for 20 s while the scan's own 30 s deadline could only be checked between
//! reads; the worker thread and the launcher's unit list were held for that whole time, and the game dropped to 6 fps.
//!
//! The scan therefore runs in a single-use child of the launcher (`EmblemWeave.exe --scan-units-helper <pid> …`),
//! inside a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. The launcher reads progress and the final address
//! list from the child's stdout and never calls `ReadProcessMemory` itself. Cancel, the deadline and a dropped scan
//! terminate the job: Windows then releases the child's handle and the copied pages, and a read that never returns
//! cannot hold the launcher. The child only ever reads the game (PROCESS_VM_READ/QUERY), so terminating it cannot
//! corrupt the save or the game state.
//!
//! The launcher falls back to the in-process scan when the helper cannot be started, so a locked-down machine still
//! gets a scan (with the old, uninterruptible-read behaviour).

use crate::cheats::{self, ScanCancelToken, ScanProgress, ScanUnitsConfig};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// First argument of the single-use child process.
pub const HELPER_ARG: &str = "--scan-units-helper";

/// How long the launcher waits past the scan deadline before it terminates the helper. The helper enforces the same
/// deadline itself; the extra second covers starting up and the last progress write.
const KILL_GRACE: Duration = Duration::from_secs(1);

fn zero_progress(remaining: f64) -> ScanProgress {
    ScanProgress {
        total_pieces: 0,
        completed_pieces: 0,
        units_found: 0,
        elapsed_secs: 0.0,
        remaining_secs: remaining,
        cancelled: false,
        timed_out: false,
        device_lost: false,
    }
}

fn publish(progress: Option<&Mutex<ScanProgress>>, next: &ScanProgress) {
    if let Some(p) = progress {
        let mut pg = p.lock().unwrap_or_else(|e| e.into_inner());
        *pg = *next;
    }
}

/// The launcher-side half of the helper: run one Find Units scan in a supervised child process.
///
/// Returns when the child finishes, when `cancel` is set, or at the deadline (plus a one-second grace). In the latter
/// two cases the child's Job Object is terminated, which is what makes the call return even if the child is inside a
/// `ReadProcessMemory` that never completes.
#[cfg(windows)]
pub fn scan_units_supervised(
    pid: u32,
    config: ScanUnitsConfig,
    cancel: Option<&ScanCancelToken>,
    progress: Option<&Mutex<ScanProgress>>,
) -> (Vec<u64>, ScanProgress) {
    let start = Instant::now();
    let deadline = start + Duration::from_secs_f64(config.timeout_secs);
    let mut prog = zero_progress(config.timeout_secs);
    publish(progress, &prog);

    let Some(mut run) = win::spawn(pid, &config) else {
        // No second process available: the in-process scan is the old behaviour, including its uninterruptible read.
        return match cheats::process::Process::open(pid) {
            Some(p) => cheats::scan_units(&p, config, cancel, progress),
            None => {
                prog.device_lost = true;
                publish(progress, &prog);
                (Vec::new(), prog)
            }
        };
    };

    let mut final_units: Option<Vec<u64>> = None;
    let mut cancelled = false;
    let mut timed_out = false;
    let mut lost = false;

    loop {
        while let Some(line) = run.next_line() {
            match win::parse(&line) {
                Some(win::Event::Progress(p)) => {
                    prog = p;
                    publish(progress, &prog);
                }
                Some(win::Event::Done(units, p)) => {
                    prog = p;
                    final_units = Some(units);
                }
                None => {}
            }
        }
        if let Some(units) = final_units.take() {
            break;
        }
        if cancel.is_some_and(ScanCancelToken::is_cancelled) {
            cancelled = true;
            run.terminate();
            break;
        }
        if Instant::now() >= deadline + KILL_GRACE {
            timed_out = true;
            run.terminate();
            break;
        }
        if run.exited() {
            // The child can exit between its final line and this poll; give the pipe a moment to deliver it.
            std::thread::sleep(Duration::from_millis(50));
            while let Some(line) = run.next_line() {
                if let Some(win::Event::Done(units, p)) = win::parse(&line) {
                    prog = p;
                    final_units = Some(units);
                }
            }
            if final_units.is_none() {
                lost = !cancelled && !timed_out;
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    run.finish();
    let units = final_units.unwrap_or_default();
    prog.total_pieces = prog.total_pieces.max(prog.completed_pieces);
    prog.units_found = units.len();
    prog.elapsed_secs = start.elapsed().as_secs_f64();
    prog.remaining_secs = deadline.saturating_duration_since(Instant::now()).as_secs_f64();
    // A killed helper never reports its own last state; the caller's request wins.
    prog.cancelled = cancelled || prog.cancelled;
    prog.timed_out = prog.timed_out || (timed_out && !prog.cancelled);
    prog.device_lost = prog.device_lost || lost;
    publish(progress, &prog);
    (units, prog)
}

/// Entry point of the single-use child. Reads the game, writes JSON lines to stdout and exits.
///
/// `timeout_secs`, `cap_bytes` and `threads` come from the launcher so a diagnostic run can reproduce the pre-0.1.43
/// uncapped read without changing the product default.
#[cfg(windows)]
pub fn helper_main(pid: u32, timeout_secs: f64, cap_bytes: u64, threads: usize) -> i32 {
    use std::io::Write;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let Some(game) = cheats::process::Process::open(pid) else {
        return 2;
    };
    let config = ScanUnitsConfig {
        timeout_secs,
        max_fallback_bytes: cap_bytes,
        max_threads: threads,
        ..Default::default()
    };
    let progress = Arc::new(Mutex::new(zero_progress(timeout_secs)));
    let stop = Arc::new(AtomicBool::new(false));
    let printer = {
        let (p, s) = (progress.clone(), stop.clone());
        std::thread::spawn(move || {
            use std::sync::atomic::Ordering;
            while !s.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                let pg = *p.lock().unwrap_or_else(|e| e.into_inner());
                println!("{}", win::json_progress(&pg));
                let _ = std::io::stdout().flush();
            }
        })
    };
    let (units, prog) = cheats::scan_units(&game, config, None, Some(progress.as_ref()));
    stop.store(true, Ordering::Relaxed);
    let _ = printer.join();
    println!("{}", win::json_done(&units, &prog));
    let _ = std::io::stdout().flush();
    0
}

#[cfg(not(windows))]
pub fn scan_units_supervised(
    pid: u32,
    config: ScanUnitsConfig,
    cancel: Option<&ScanCancelToken>,
    progress: Option<&Mutex<ScanProgress>>,
) -> (Vec<u64>, ScanProgress) {
    let _ = pid;
    let mut p = zero_progress(0.0);
    p.device_lost = true;
    let _ = (config, cancel, progress);
    (Vec::new(), p)
}

#[cfg(not(windows))]
pub fn helper_main(_pid: u32, _timeout_secs: f64, _cap_bytes: u64, _threads: usize) -> i32 {
    2
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Command, Stdio};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject, TerminateJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JobObjectExtendedLimitInformation,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

    /// A helper process and the Job Object that owns it.
    pub struct Run {
        child: Child,
        job: Option<HANDLE>,
        lines: std::sync::mpsc::Receiver<String>,
        reader: Option<std::thread::JoinHandle<()>>,
        finished: bool,
    }

    pub enum Event {
        Progress(ScanProgress),
        Done(Vec<u64>, ScanProgress),
    }

    fn helper_exe() -> std::path::PathBuf {
        std::env::var_os("EW_HELPER_EXE")
            .map(std::path::PathBuf::from)
            .or_else(|| std::env::current_exe().ok())
            .unwrap_or_else(|| std::path::PathBuf::from("EmblemWeave.exe"))
    }

    unsafe fn job_with_kill_on_close() -> Option<HANDLE> {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return None;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const _,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            CloseHandle(job);
            return None;
        }
        Some(job)
    }

    unsafe fn assign(job: HANDLE, child: &Child) -> bool {
        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, child.id());
        if process.is_null() {
            return false;
        }
        let ok = AssignProcessToJobObject(job, process) != 0;
        CloseHandle(process);
        ok
    }

    pub fn spawn(pid: u32, config: &ScanUnitsConfig) -> Option<Run> {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let exe = helper_exe();
        if !exe.is_file() {
            return None;
        }
        let mut child = Command::new(&exe)
            .arg(HELPER_ARG)
            .arg(pid.to_string())
            .arg(format!("{:.3}", config.timeout_secs))
            .arg(config.max_fallback_bytes.to_string())
            .arg(config.max_threads.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .ok()?;
        let job = unsafe {
            let job = job_with_kill_on_close();
            if let Some(j) = job {
                if !assign(j, &child) {
                    CloseHandle(j);
                    None
                } else {
                    Some(j)
                }
            } else {
                None
            }
        };
        let stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Some(Run {
            child,
            job,
            lines: rx,
            reader: Some(reader),
            finished: false,
        })
    }

    impl Run {
        pub fn next_line(&self) -> Option<String> {
            self.lines.try_recv().ok()
        }

        pub fn exited(&mut self) -> bool {
            matches!(self.child.try_wait(), Ok(Some(_)))
        }

        /// Terminate the whole job (or the child when no Job Object could be created) and reap it. Safe to call more
        /// than once and on a child stuck in a kernel read.
        pub fn terminate(&mut self) {
            if let Some(job) = self.job {
                unsafe { TerminateJobObject(job, 1) };
            } else {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
        }

        pub fn finish(&mut self) {
            if self.finished {
                return;
            }
            self.finished = true;
            if self.job.is_none() {
                let _ = self.child.kill();
            }
            let _ = self.child.wait();
            if let Some(job) = self.job.take() {
                // KILL_ON_JOB_CLOSE: closing the last handle also kills anything still running under it.
                unsafe { CloseHandle(job) };
            }
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        }
    }

    impl Drop for Run {
        fn drop(&mut self) {
            self.finish();
        }
    }

    fn progress_json(p: &ScanProgress) -> serde_json::Value {
        serde_json::json!({
            "total_pieces": p.total_pieces,
            "completed_pieces": p.completed_pieces,
            "units_found": p.units_found,
            "elapsed_secs": p.elapsed_secs,
            "remaining_secs": p.remaining_secs,
            "cancelled": p.cancelled,
            "timed_out": p.timed_out,
            "device_lost": p.device_lost,
        })
    }

    pub fn json_progress(p: &ScanProgress) -> String {
        serde_json::json!({ "progress": progress_json(p) }).to_string()
    }

    pub fn json_done(units: &[u64], p: &ScanProgress) -> String {
        serde_json::json!({ "units": units, "progress": progress_json(p) }).to_string()
    }

    fn parse_progress(v: &serde_json::Value) -> Option<ScanProgress> {
        let p = v.as_object()?;
        let g = |k: &str| p.get(k).cloned().unwrap_or(serde_json::Value::Null);
        Some(ScanProgress {
            total_pieces: g("total_pieces").as_u64().unwrap_or(0) as usize,
            completed_pieces: g("completed_pieces").as_u64().unwrap_or(0) as usize,
            units_found: g("units_found").as_u64().unwrap_or(0) as usize,
            elapsed_secs: g("elapsed_secs").as_f64().unwrap_or(0.0),
            remaining_secs: g("remaining_secs").as_f64().unwrap_or(0.0),
            cancelled: g("cancelled").as_bool().unwrap_or(false),
            timed_out: g("timed_out").as_bool().unwrap_or(false),
            device_lost: g("device_lost").as_bool().unwrap_or(false),
        })
    }

    pub fn parse(line: &str) -> Option<Event> {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        let progress = v.get("progress").and_then(parse_progress);
        if let Some(units) = v.get("units").and_then(|u| u.as_array()) {
            let list: Vec<u64> = units.iter().filter_map(|u| u.as_u64()).collect();
            return Some(Event::Done(list, progress.unwrap_or_else(|| super::zero_progress(0.0))));
        }
        progress.map(Event::Progress)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    fn game_pid() -> Option<u32> {
        let pid = crate::cheats::process::find(crate::cheats::GAME_EXE)?;
        if std::env::var_os("EW_HELPER_EXE").is_none() {
            eprintln!("EW_HELPER_EXE is not set; build the launcher and point it at target/release/EmblemWeave.exe");
            return None;
        }
        Some(pid)
    }

    fn config(timeout: f64, cap: u64) -> ScanUnitsConfig {
        ScanUnitsConfig {
            timeout_secs: timeout,
            max_fallback_bytes: cap,
            ..Default::default()
        }
    }

    #[test]
    #[ignore = "needs a running fefw.exe and EW_HELPER_EXE pointing at the built launcher"]
    fn supervised_scan_matches_the_in_process_scan_and_reaps_the_helper() {
        let Some(pid) = game_pid() else { return };
        let cfg = config(30.0, 2 << 30);
        let game = crate::cheats::process::Process::open(pid).expect("open the game");
        let (direct, direct_prog) = cheats::scan_units(&game, cfg, None, None);
        assert!(!direct_prog.device_lost && !direct_prog.timed_out);

        let progress = Mutex::new(zero_progress(30.0));
        let start = Instant::now();
        let (units, prog) = scan_units_supervised(pid, cfg, None, Some(&progress));
        let took = start.elapsed().as_secs_f64();
        eprintln!(
            "direct={} supervised={} taken={took:.2}s progress={prog:?}",
            direct.len(),
            units.len()
        );
        assert!(!prog.device_lost && !prog.timed_out && !prog.cancelled);
        assert_eq!(units, direct, "the helper returns the same addresses as the in-process scan");
        assert!(took < 20.0, "the capped scan finishes well inside the deadline");
        // The helper is reaped inside the call; a second scan must be able to start immediately.
        let (again, again_prog) = scan_units_supervised(pid, cfg, None, None);
        assert_eq!(again, direct);
        assert!(!again_prog.device_lost);
    }

    #[test]
    #[ignore = "needs a running fefw.exe and EW_HELPER_EXE"]
    fn cancel_mid_scan_terminates_the_helper_promptly() {
        let Some(pid) = game_pid() else { return };
        // 64 GiB cap: the scan reaches the same 30 GB the 0.1.42 build read, where a single read blocks for ~20 s.
        let cancel = Arc::new(ScanCancelToken::new());
        let c = cancel.clone();
        let after: f64 = std::env::var("EW_CANCEL_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(after));
            c.cancel();
        });
        let start = Instant::now();
        let (units, prog) = scan_units_supervised(pid, config(30.0, 64 << 30), Some(&cancel), None);
        let took = start.elapsed().as_secs_f64();
        eprintln!("cancelled units={} taken={took:.2}s progress={prog:?}", units.len());
        assert!(prog.cancelled, "the scan reports the cancellation");
        assert!(took < 6.0, "a cancel must not wait for a blocked read (took {took:.2}s)");
    }

    #[test]
    #[ignore = "needs a running fefw.exe and EW_HELPER_EXE"]
    fn the_deadline_kills_a_helper_that_is_stuck_in_a_read() {
        let Some(pid) = game_pid() else { return };
        let start = Instant::now();
        let (units, prog) = scan_units_supervised(pid, config(0.5, 64 << 30), None, None);
        let took = start.elapsed().as_secs_f64();
        eprintln!("timed_out units={} taken={took:.2}s progress={prog:?}", units.len());
        assert!(prog.timed_out, "the scan reports the deadline");
        assert!(took < 4.0, "the deadline plus grace must return even from a blocked read (took {took:.2}s)");
    }

    #[test]
    #[ignore = "needs a running fefw.exe and EW_HELPER_EXE; terminates the game"]
    fn the_game_exiting_mid_scan_is_device_lost_and_reaps_the_helper() {
        let Some(pid) = game_pid() else { return };
        std::thread::spawn(move || {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
            std::thread::sleep(Duration::from_secs(1));
            unsafe {
                let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
                if !h.is_null() {
                    TerminateProcess(h, 1);
                    CloseHandle(h);
                }
            }
        });
        let start = Instant::now();
        let (units, prog) = scan_units_supervised(pid, config(30.0, 64 << 30), None, None);
        let took = start.elapsed().as_secs_f64();
        eprintln!("device_lost units={} taken={took:.2}s progress={prog:?}", units.len());
        assert!(prog.device_lost, "the scan reports the game is gone");
        assert!(took < 8.0, "a lost game must not hold the launcher (took {took:.2}s)");
    }
}
