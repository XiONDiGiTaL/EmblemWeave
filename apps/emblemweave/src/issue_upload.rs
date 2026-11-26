//! Opt-in, anonymised issue reports after a run (ported from fefw-recomp's launcher, `launcher/src-tauri/src/report.rs`).
//!
//! Nothing is sent unless the player turned it on (Settings: About > Issue reports, `config.reports.send`) **and** a report
//! server is configured: `config.reports.endpoint`, else `EW_REPORT_URL` in the environment, else one named when the launcher
//! was built (`EW_REPORT_URL` at compile time). EmblemWeave ships with none, so out of the box this module never uploads.
//!
//! When it may send, a report is made only for a run that had a problem: a fresh crash, freeze or fatal file, a non-zero
//! exit, heavy hitches or sustained low FPS. The player's paths, user name, machine name and Steam IDs are scrubbed out
//! first, the exact JSON is saved as `fefw_last_report.json` beside the game log so it can be read, and the upload is
//! limited: at most three crash-type reports a day, one per signature and build, and one performance report per build a day.
//! The preference is read again from disk when the game exits, so turning it off while the game runs stops that upload.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A report server named at build time; empty in EmblemWeave's releases.
const BUILT_IN_URL: &str = match option_env!("EW_REPORT_URL") {
    Some(u) => u,
    None => "",
};

const MAX_LOG_HEAD: usize = 64 * 1024;
const MAX_LOG_TAIL: usize = 512 * 1024;
const MAX_CRASH_FILE: usize = 256 * 1024;
const MAX_COMPRESSED_BYTES: usize = 1_572_864; // 1.5 MiB, the reference server's limit
const DAY_SECS: u64 = 24 * 60 * 60;

/// The files the runtime writes beside its log when a run goes wrong.
const PROBLEM_FILES: [&str; 3] = ["fefw_crash.txt", "fefw_freeze.txt", "fefw_fatal.txt"];

/// The report server this launcher would send to, when one is configured and its address is usable. Only `https://` is
/// accepted, except `http://` to this PC (`localhost` / `127.0.0.1`) for someone testing their own server.
pub fn endpoint(config_value: &str) -> Option<String> {
    let env = std::env::var("EW_REPORT_URL").ok();
    let raw = [Some(config_value), env.as_deref(), Some(BUILT_IN_URL)]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())?;
    usable_url(raw)
}

fn usable_url(raw: &str) -> Option<String> {
    let url = raw.trim().trim_end_matches('/');
    let ok_chars = url.chars().all(|c| c.is_ascii_graphic()) && !url.contains(['"', '\'', '\\', '<', '>', '`']);
    let https = url.strip_prefix("https://").is_some_and(|h| !h.is_empty());
    let local = ["http://localhost", "http://127.0.0.1"]
        .iter()
        .any(|p| url.strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/'])));
    (ok_chars && (https || local)).then(|| url.to_string())
}

/// Called by the launch watcher once the game has exited. Decides, prepares and sends in this (background) thread;
/// every failure is silent, since a report must never get in the way of playing.
pub fn after_run(log: &Path, from: u64, started: SystemTime, code: Option<i32>) {
    let (config, _) = crate::config::Config::load();
    if !config.reports.send {
        return;
    }
    let Some(url) = endpoint(&config.reports.endpoint) else { return };
    let Some(state_dir) = crate::config::Config::dir() else { return };
    let Some(game_dir) = log.parent() else { return };
    if let Some(p) = prepare(&state_dir, game_dir, log, from, started, code, &config.paths.game_root) {
        send(&url, p);
    }
}

#[derive(Debug, Default, PartialEq)]
struct LogMetrics {
    measured_s: f64,
    mean_fps: Option<f64>,
    low1_fps: Option<f64>,
    worst_ms: Option<f64>,
    hitch_count: u32,
    gpu: String,
}

#[derive(Serialize)]
struct HeaderMeta<'a> {
    trigger: &'a str,
    build_id: &'a str,
    launcher_version: &'a str,
    os: &'a str,
    gpu: &'a str,
    exit_code: Option<i32>,
    mean_fps: Option<f64>,
    low1_fps: Option<f64>,
    worst_ms: Option<f64>,
    hitch_count: u32,
    crash_cause: Option<&'a str>,
}

#[derive(Serialize)]
struct ReportBundle<'a> {
    schema: u32,
    trigger: &'a str,
    build_id: &'a str,
    launcher_version: &'a str,
    os: &'a str,
    arch: &'a str,
    gpu: &'a str,
    exit_code: Option<i32>,
    mean_fps: Option<f64>,
    low1_fps: Option<f64>,
    worst_ms: Option<f64>,
    hitch_count: u32,
    crash_cause: Option<&'a str>,
    settings: BTreeMap<String, String>,
    crash_files: BTreeMap<String, String>,
    log: String,
}

#[derive(Serialize, Deserialize, Default)]
struct RateState {
    sent: Vec<SentEntry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct SentEntry {
    ts: u64,
    trigger: String,
    build_id: String,
    sig: String,
}

/// A finished run's report, scrubbed and held in memory so the next launch cannot change the files under it.
struct Prepared {
    state_dir: PathBuf,
    now: u64,
    trigger: &'static str,
    build_id: String,
    sig: String,
    json_bytes: Vec<u8>,
    meta_b64: String,
}

fn prepare(
    state_dir: &Path,
    game_dir: &Path,
    log_path: &Path,
    from: u64,
    started: SystemTime,
    code: Option<i32>,
    game_root: &str,
) -> Option<Prepared> {
    let log_raw = read_run_log(log_path, from);
    let metrics = parse_log_metrics(&log_raw);
    let fresh: Vec<&str> = PROBLEM_FILES
        .iter()
        .copied()
        .filter(|name| {
            fs::metadata(game_dir.join(name)).and_then(|m| m.modified()).is_ok_and(|t| t >= started)
        })
        .collect();
    let trigger = detect_trigger(code, &fresh, &metrics)?;
    let build_id = format!("EmblemWeave {} ({})", env!("CARGO_PKG_VERSION"), env!("EW_GIT_REV"));

    let now = now_unix_s();
    let crash_cause = likely_cause(game_dir, &fresh);
    let sig: String = crash_cause.as_deref().unwrap_or("").chars().take(80).collect();
    if !allow_by_rate_limit(state_dir, now, trigger, &build_id, &sig) {
        return None;
    }

    let ini = crate::engine::ini_values(&fs::read_to_string(game_dir.join("fefw.ini")).unwrap_or_default());
    let launcher_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let scrubber = Scrubber::from_env(game_dir, game_root, &launcher_dir, &ini);
    let gpu = Some(scrubber.scrub(&metrics.gpu)).filter(|g| !g.trim().is_empty()).unwrap_or_else(|| "unknown".into());
    let clean_cause = crash_cause.map(|c| scrubber.scrub(&c));
    let clean_log = scrubber.scrub(&slice_log(&log_raw, MAX_LOG_HEAD, MAX_LOG_TAIL));

    let mut crash_files = BTreeMap::new();
    for name in &fresh {
        if let Ok(bytes) = fs::read(game_dir.join(name)) {
            let text = String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_CRASH_FILE)]);
            crash_files.insert(name.to_string(), scrubber.scrub(&text));
        }
    }

    let launcher_version = env!("CARGO_PKG_VERSION");
    let bundle = ReportBundle {
        schema: 1,
        trigger,
        build_id: &build_id,
        launcher_version,
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        gpu: &gpu,
        exit_code: code,
        mean_fps: metrics.mean_fps,
        low1_fps: metrics.low1_fps,
        worst_ms: metrics.worst_ms,
        hitch_count: metrics.hitch_count,
        crash_cause: clean_cause.as_deref(),
        settings: safe_settings(&ini),
        crash_files,
        log: clean_log,
    };
    let json_bytes = serde_json::to_vec_pretty(&bundle).ok()?;
    // The exact scrubbed payload, so the player can read what was sent.
    let _ = fs::write(game_dir.join("fefw_last_report.json"), &json_bytes);

    let meta = HeaderMeta {
        trigger,
        build_id: &build_id,
        launcher_version,
        os: std::env::consts::OS,
        gpu: &gpu,
        exit_code: code,
        mean_fps: metrics.mean_fps,
        low1_fps: metrics.low1_fps,
        worst_ms: metrics.worst_ms,
        hitch_count: metrics.hitch_count,
        crash_cause: clean_cause.as_deref(),
    };
    let meta_b64 = base64_encode(&serde_json::to_vec(&meta).ok()?);
    Some(Prepared { state_dir: state_dir.to_path_buf(), now, trigger, build_id, sig, json_bytes, meta_b64 })
}

fn send(base_url: &str, p: Prepared) {
    let Ok(compressed) = zstd::stream::encode_all(p.json_bytes.as_slice(), 9) else { return };
    if compressed.is_empty() || compressed.len() > MAX_COMPRESSED_BYTES {
        return;
    }
    let tmp = std::env::temp_dir().join(format!("ew-report-{}-{}.zst", std::process::id(), p.now));
    if fs::write(&tmp, &compressed).is_err() {
        return;
    }
    let url = format!("{base_url}/v1/report");
    let ok = crate::process::hidden(&mut std::process::Command::new(crate::process::system_tool("curl.exe")))
        .args([
            "-sS",
            "--fail",
            "--max-time",
            "20",
            "--proto",
            "=https,http",
            "--request",
            "POST",
            "-A",
            concat!("EmblemWeave/", env!("CARGO_PKG_VERSION")),
            "--header",
            "Content-Type: application/zstd",
            "--header",
            &format!("X-Report-Meta: {}", p.meta_b64),
            "--data-binary",
            &format!("@{}", tmp.display()),
            &url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = fs::remove_file(&tmp);
    if ok {
        record_sent(&p.state_dir, p.now, p.trigger, &p.build_id, &p.sig);
    }
}

fn now_unix_s() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// This run's part of `fefw.log` (the runtime appends; `from` is the size before the launch). A very long run keeps its
/// first 128 KiB and last 2 MiB so the scan stays quick.
fn read_run_log(path: &Path, from: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = if from <= len { from } else { 0 };
    let span = len - start;
    let mut read_at = |at: u64, n: usize| -> Vec<u8> {
        let mut buf = vec![0u8; n];
        if f.seek(SeekFrom::Start(at)).is_err() {
            return Vec::new();
        }
        let got = f.read(&mut buf).unwrap_or(0);
        buf.truncate(got);
        buf
    };
    if span <= 8 * 1024 * 1024 {
        let mut all = Vec::new();
        let _ = f.seek(SeekFrom::Start(start)).map(|_| f.read_to_end(&mut all));
        return String::from_utf8_lossy(&all).into_owned();
    }
    let (head_len, tail_len) = (128 * 1024usize, 2 * 1024 * 1024usize);
    let head = read_at(start, head_len);
    let tail = read_at(len - tail_len as u64, tail_len);
    format!(
        "{}\n... [skipped {} bytes] ...\n{}",
        String::from_utf8_lossy(&head),
        span.saturating_sub((head.len() + tail.len()) as u64),
        String::from_utf8_lossy(&tail)
    )
}

fn slice_log(text: &str, head_max: usize, tail_max: usize) -> String {
    if text.len() <= head_max + tail_max {
        return text.to_string();
    }
    let head_end = floor_char_boundary(text, head_max);
    let tail_start = ceil_char_boundary(text, text.len().saturating_sub(tail_max));
    format!("{}\n... [trimmed {} bytes] ...\n{}", &text[..head_end], tail_start.saturating_sub(head_end), &text[tail_start..])
}

fn floor_char_boundary(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx > 0 && !s.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

fn ceil_char_boundary(s: &str, mut idx: usize) -> usize {
    idx = idx.min(s.len());
    while idx < s.len() && !s.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// "LIKELY CAUSE: <cause>", the first line the runtime writes into a crash report when it knows one.
fn likely_cause(game_dir: &Path, fresh: &[&str]) -> Option<String> {
    fresh.iter().find_map(|name| {
        let text = fs::read_to_string(game_dir.join(name)).ok()?;
        let cause = text.lines().next()?.strip_prefix("LIKELY CAUSE: ")?.trim();
        (!cause.is_empty()).then(|| cause.chars().take(200).collect())
    })
}

fn detect_trigger(code: Option<i32>, fresh: &[&str], m: &LogMetrics) -> Option<&'static str> {
    for (file, trigger) in [("fefw_crash.txt", "crash"), ("fefw_freeze.txt", "freeze"), ("fefw_fatal.txt", "fatal")] {
        if fresh.contains(&file) {
            return Some(trigger);
        }
    }
    if code.is_some_and(|c| c != 0) {
        return Some("exit_code");
    }
    if m.worst_ms.is_some_and(|w| w >= 250.0) || (m.measured_s >= 60.0 && m.hitch_count >= 10) {
        return Some("hitch");
    }
    if m.measured_s >= 120.0 && (m.mean_fps.is_some_and(|f| f < 28.0) || m.low1_fps.is_some_and(|f| f < 18.0)) {
        return Some("low_fps");
    }
    None
}

fn parse_log_metrics(log: &str) -> LogMetrics {
    let mut m = LogMetrics::default();
    let (mut weighted_fps, mut weight_s) = (0.0, 0.0);
    for line in log.lines() {
        // Hitch warnings from gauge.cpp: `[  12.345 W t123 hitch] ...`
        if line.contains(" W ") && line.contains("hitch") {
            m.hitch_count = m.hitch_count.saturating_add(1);
        }
        // gauge.cpp's summary: `3600 frames in 60.0 s: mean 60.0 fps, 1% low 52.3 fps, worst 28.4 ms, ...`
        if let Some((span_s, mean, low1, worst)) = parse_hitch_summary(line) {
            m.measured_s += span_s;
            weighted_fps += mean * span_s;
            weight_s += span_s;
            m.low1_fps = Some(m.low1_fps.map_or(low1, |prev| prev.min(low1)));
            m.worst_ms = Some(m.worst_ms.map_or(worst, |prev| prev.max(worst)));
        }
    }
    if let Some(g) = crate::engine::log_gpu_name(log) {
        m.gpu = g.chars().take(128).collect();
    }
    if weight_s > 0.0 {
        m.mean_fps = Some((weighted_fps / weight_s * 10.0).round() / 10.0);
    }
    m
}

fn parse_hitch_summary(line: &str) -> Option<(f64, f64, f64, f64)> {
    let (_, after_in) = line.split_once(" frames in ")?;
    let (span_str, after_s) = after_in.split_once(" s: mean ")?;
    let (mean_str, after_mean) = after_s.split_once(" fps, 1% low ")?;
    let (low1_str, after_low1) = after_mean.split_once(" fps, worst ")?;
    let (worst_str, _) = after_low1.split_once(" ms")?;
    let span: f64 = span_str.trim().parse().ok()?;
    let mean: f64 = mean_str.trim().parse().ok()?;
    let low1: f64 = low1_str.trim().parse().ok()?;
    let worst: f64 = worst_str.trim().parse().ok()?;
    (span > 0.0 && mean.is_finite() && low1.is_finite() && worst.is_finite()).then_some((span, mean, low1, worst))
}

/// The settings that say how the game was run, from the launcher's own keys: never a path, a file name or free text.
fn safe_settings(ini: &std::collections::HashMap<String, String>) -> BTreeMap<String, String> {
    const ALLOW: &[&str] = &[
        "fefw.output",
        "fefw.ultrawide",
        "fefw.fps60",
        "fefw.fps120",
        "fefw.smooth_step",
        "fefw.render",
        "fefw.render_ratio",
        "fefw.quality",
        "display.window_mode",
        "display.width",
        "display.height",
        "dlss.upscaler",
        "dlss.frame_gen",
        "gfx.texture_detail",
        "gfx.astc_bc",
        "gfx.hdr",
        "gfx.vsync",
        "gfx.present_mode",
        "gfx.ui_language",
        "perf.preset",
        "log.level",
    ];
    ALLOW
        .iter()
        .filter_map(|k| {
            let v = ini.get(*k)?.trim();
            let plain = !v.is_empty() && v.len() <= 64 && !v.contains(['/', '\\', ':']);
            plain.then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

fn rate_state_path(dir: &Path) -> PathBuf {
    dir.join("report_state.json")
}

fn load_rate_state(dir: &Path, now: u64) -> RateState {
    let mut st: RateState =
        fs::read_to_string(rate_state_path(dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    st.sent.retain(|e| now.saturating_sub(e.ts) < DAY_SECS);
    st
}

fn allow_by_rate_limit(dir: &Path, now: u64, trigger: &str, build_id: &str, sig: &str) -> bool {
    let st = load_rate_state(dir, now);
    let is_perf = |t: &str| matches!(t, "hitch" | "low_fps");
    if is_perf(trigger) {
        !st.sent.iter().any(|e| is_perf(&e.trigger) && e.build_id == build_id)
    } else {
        st.sent.iter().filter(|e| !is_perf(&e.trigger)).count() < 3
            && !st.sent.iter().any(|e| e.trigger == trigger && e.build_id == build_id && e.sig == sig)
    }
}

fn record_sent(dir: &Path, now: u64, trigger: &str, build_id: &str, sig: &str) {
    let mut st = load_rate_state(dir, now);
    st.sent.push(SentEntry { ts: now, trigger: trigger.into(), build_id: build_id.into(), sig: sig.into() });
    if let Ok(bytes) = serde_json::to_vec_pretty(&st) {
        let _ = fs::write(rate_state_path(dir), bytes);
    }
}

struct Scrubber {
    /// Exact literal replacements, longest first.
    literals: Vec<(String, &'static str)>,
}

impl Scrubber {
    fn from_env(game_dir: &Path, game_root: &str, launcher_dir: &Path, ini: &std::collections::HashMap<String, String>) -> Self {
        let mut s = Self { literals: Vec::new() };
        s.add_path(&game_dir.to_string_lossy(), "<GAME_DIR>");
        s.add_path(game_root, "<DUMP_DIR>");
        s.add_path(&launcher_dir.to_string_lossy(), "<LAUNCHER_DIR>");
        // Every path fefw.ini names: saves, the game data, mods, caches, upscaler files.
        for (key, value) in ini {
            let k = key.as_str();
            let path_like = value.contains(['/', '\\'])
                || k.ends_with("dir")
                || k.ends_with("_dll")
                || k.ends_with("path")
                || k.ends_with("cache")
                || k.starts_with("fs.");
            if path_like {
                s.add_path(value, "<PATH>");
            }
        }
        for var in ["HOME", "USERPROFILE", "APPDATA", "LOCALAPPDATA"] {
            if let Ok(v) = std::env::var(var) {
                s.add_path(&v, "<HOME>");
            }
        }
        for var in ["COMPUTERNAME", "HOSTNAME", "USERNAME", "USER"] {
            if let Ok(v) = std::env::var(var) {
                let v = v.trim();
                if v.len() >= 4 {
                    s.literals.push((v.to_string(), if var.contains("USER") { "<USER>" } else { "<HOST>" }));
                }
            }
        }
        s.sort();
        s
    }

    fn sort(&mut self) {
        self.literals.sort_by_key(|a| std::cmp::Reverse(a.0.len()));
        self.literals.dedup_by(|a, b| a.0 == b.0);
    }

    fn add_path(&mut self, raw: &str, token: &'static str) {
        let trimmed = raw.trim().trim_matches('"').trim_end_matches(['/', '\\']);
        if trimmed.len() < 4 {
            return;
        }
        let fwd = trimmed.replace('\\', "/");
        let bwd = trimmed.replace('/', "\\");
        self.literals.push((fwd.clone(), token));
        if bwd != fwd {
            self.literals.push((bwd, token));
        }
        if let Some(stripped) = fwd.strip_prefix("Z:").or_else(|| fwd.strip_prefix("z:")) {
            if stripped.starts_with('/') && stripped.len() >= 4 {
                self.literals.push((stripped.to_string(), token));
            }
        } else if fwd.starts_with('/') {
            for z in ["Z:", "z:"] {
                self.literals.push((format!("{z}{fwd}"), token));
                self.literals.push((format!("{z}{}", fwd.replace('/', "\\")), token));
            }
        }
    }

    fn scrub(&self, input: &str) -> String {
        let mut out = input.to_string();
        for (needle, repl) in &self.literals {
            if !needle.is_empty() && out.contains(needle.as_str()) {
                out = out.replace(needle.as_str(), repl);
            }
        }
        scrub_steam64_ids(&scrub_user_segments(&out))
    }
}

/// Replaces the user name after a home-folder prefix (`/home/<user>`, `/Users/<user>`, `Users\<user>`).
fn scrub_user_segments(input: &str) -> String {
    const PREFIXES: &[&str] = &["/home/", "/Users/", "Users\\", "Users/", "users\\", "users/"];
    let mut out = input.to_string();
    for prefix in PREFIXES {
        let mut next = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(pos) = rest.find(prefix) {
            let start = pos + prefix.len();
            next.push_str(&rest[..start]);
            let after = &rest[start..];
            let seg_len =
                after.find(['/', '\\', ' ', '\t', '\r', '\n', '"', '\'', ',', ';', ':', ')', ']']).unwrap_or(after.len());
            let seg = &after[..seg_len];
            next.push_str(if !seg.is_empty() && seg != "<USER>" && seg != "<HOME>" { "<USER>" } else { seg });
            rest = &after[seg_len..];
        }
        next.push_str(rest);
        out = next;
    }
    out
}

/// Redacts 17-digit Steam64 IDs (`7656119xxxxxxxxxx`).
fn scrub_steam64_ids(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if i + 17 <= bytes.len()
            && &bytes[i..i + 7] == b"7656119"
            && bytes[i + 7..i + 17].iter().all(|b| b.is_ascii_digit())
            && (i == 0 || !bytes[i - 1].is_ascii_digit())
            && (i + 17 == bytes.len() || !bytes[i + 17].is_ascii_digit())
        {
            out.push_str("<STEAM_ID>");
            i += 17;
        } else {
            let ch = input[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_sent_without_a_usable_server() {
        assert_eq!(usable_url(""), None);
        assert_eq!(usable_url("example.com/report"), None);
        assert_eq!(usable_url("http://example.com"), None, "plain http only to this PC");
        assert_eq!(usable_url("https://"), None);
        assert_eq!(usable_url("https://reports.example.com/"), Some("https://reports.example.com".into()));
        assert_eq!(usable_url("http://localhost:8787"), Some("http://localhost:8787".into()));
        assert_eq!(usable_url("http://localhost.evil.com"), None);
        assert_eq!(usable_url("https://a.example/x\" -o C:/x"), None, "nothing that could reach curl as another argument");
    }

    #[test]
    fn scrubs_paths_usernames_hosts_and_steam_ids() {
        let ini: std::collections::HashMap<String, String> = [
            ("fs.save_dir".to_string(), "D:/Alice Backups/FEFW".to_string()),
            ("fs.mods".to_string(), "Z:/mnt/alice_mods/fefw".to_string()),
        ]
        .into_iter()
        .collect();
        let mut s = Scrubber::from_env(Path::new("/home/alice/Games/fefw"), "", Path::new("/opt/launcher"), &ini);
        s.literals.push(("ALICE-PC".into(), "<HOST>"));
        s.sort();
        let sample = "save=D:/Alice Backups/FEFW/slot0 mods=/mnt/alice_mods/fefw Loaded Z:\\home\\alice\\Games\\fefw\\fefw.ini on ALICE-PC from C:\\Users\\bob\\dump and /home/charlie/test steam=76561198012345678";
        let clean = s.scrub(sample);
        for gone in ["Alice Backups", "alice_mods", "alice", "bob", "charlie", "ALICE-PC", "76561198012345678"] {
            assert!(!clean.contains(gone), "{gone} survived: {clean}");
        }
        for token in ["<PATH>", "<GAME_DIR>", "<USER>", "<HOST>", "<STEAM_ID>"] {
            assert!(clean.contains(token), "{token} missing: {clean}");
        }
    }

    #[test]
    fn reads_the_gauge_summary_and_picks_a_trigger() {
        let log = "\
[ 60.000 I t2 hitch] 1500 frames in 60.0 s: mean 25.0 fps, 1% low 16.5 fps, worst 80.0 ms, 4 over 2x the median (40.0 ms)\n\
[120.000 I t2 hitch] 1560 frames in 60.0 s: mean 26.0 fps, 1% low 17.0 fps, worst 90.0 ms, 2 over 2x the median (38.5 ms)\n";
        let m = parse_log_metrics(log);
        assert_eq!(m.mean_fps, Some(25.5));
        assert_eq!(m.low1_fps, Some(16.5));
        assert_eq!(m.worst_ms, Some(90.0));
        assert_eq!(detect_trigger(Some(0), &[], &m), Some("low_fps"));
        let healthy = LogMetrics { measured_s: 180.0, mean_fps: Some(60.0), low1_fps: Some(55.0), worst_ms: Some(32.0), ..Default::default() };
        assert_eq!(detect_trigger(Some(0), &[], &healthy), None);
        assert_eq!(detect_trigger(None, &[], &healthy), None, "an exit the launcher could not read is not a failure");
        assert_eq!(detect_trigger(Some(0), &["fefw_freeze.txt"], &healthy), Some("freeze"));
        assert_eq!(detect_trigger(Some(3), &[], &healthy), Some("exit_code"));
    }

    #[test]
    fn only_plain_settings_are_sent() {
        let ini: std::collections::HashMap<String, String> = [
            ("fefw.fps60", "1"),
            ("dlss.upscaler", "fsr"),
            ("fs.save_dir", "D:/Secret"),
            ("display.window_mode", "C:\\odd"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let s = safe_settings(&ini);
        assert_eq!(s.get("fefw.fps60").map(String::as_str), Some("1"));
        assert_eq!(s.get("dlss.upscaler").map(String::as_str), Some("fsr"));
        assert!(!s.contains_key("fs.save_dir"));
        assert!(!s.contains_key("display.window_mode"));
    }

    #[test]
    fn rate_limits_hold_per_day() {
        let dir = std::env::temp_dir().join(format!("ew-report-rate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let now = 1_000_000;
        assert!(allow_by_rate_limit(&dir, now, "crash", "b", "sig"));
        record_sent(&dir, now, "crash", "b", "sig");
        assert!(!allow_by_rate_limit(&dir, now, "crash", "b", "sig"), "the same crash once a day");
        record_sent(&dir, now, "crash", "b", "other");
        record_sent(&dir, now, "fatal", "b", "x");
        assert!(!allow_by_rate_limit(&dir, now, "freeze", "b", "y"), "three crash-type reports a day");
        assert!(allow_by_rate_limit(&dir, now, "hitch", "b", ""));
        record_sent(&dir, now, "hitch", "b", "");
        assert!(!allow_by_rate_limit(&dir, now, "low_fps", "b", ""), "one performance report per build a day");
        assert!(allow_by_rate_limit(&dir, now + DAY_SECS, "crash", "b", "sig"), "the limits reset after a day");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_run_log_is_only_this_runs_part() {
        let dir = std::env::temp_dir().join(format!("ew-report-log-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let p = dir.join("fefw.log");
        fs::write(&p, "old run\nnew run\n").unwrap();
        assert_eq!(read_run_log(&p, 8), "new run\n");
        assert_eq!(read_run_log(&p, 999), "old run\nnew run\n", "a log that was replaced is read whole");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crashed_run_makes_one_scrubbed_report_and_the_copy_matches_it() {
        let root = std::env::temp_dir().join(format!("ew-report-run-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let (game, state) = (root.join("game"), root.join("state"));
        fs::create_dir_all(&game).unwrap();
        fs::create_dir_all(&state).unwrap();
        let started = SystemTime::now() - std::time::Duration::from_secs(5);
        fs::write(game.join("fefw.log"), "previous run\n").unwrap();
        let from = fs::metadata(game.join("fefw.log")).unwrap().len();
        let game_s = game.display().to_string();
        fs::write(
            game.join("fefw.log"),
            format!("previous run\n[ 1.0 I t1 gfx] loading {game_s}\\romfs\\data.bin\n[ 2.0 E t1 crash] access violation\n"),
        )
        .unwrap();
        fs::write(game.join("fefw_crash.txt"), format!("LIKELY CAUSE: a texture in {game_s} is damaged\nstack...\n")).unwrap();
        fs::write(game.join("fefw.ini"), "[fefw]\nfps60 = 1\n[fs]\nsave_dir = D:/Private Saves\n").unwrap();

        let p = prepare(&state, &game, &game.join("fefw.log"), from, started, Some(-1073741819), "").expect("a crash is reported");
        assert_eq!(p.trigger, "crash");
        let saved = fs::read(game.join("fefw_last_report.json")).unwrap();
        assert_eq!(saved, p.json_bytes, "the copy beside the log is exactly the payload");
        let text = String::from_utf8(saved).unwrap();
        assert!(!text.contains("previous run"), "only this run's log");
        assert!(!text.contains(&game_s.replace('\\', "\\\\")) && !text.contains(&game_s), "the game folder is scrubbed");
        assert!(text.contains("<GAME_DIR>"));
        assert!(text.contains("\"fefw.fps60\": \"1\""));
        assert!(!text.contains("Private Saves"));
        assert!(text.contains("a texture in <GAME_DIR> is damaged"));

        // The same crash again today is not sent twice.
        record_sent(&state, p.now, p.trigger, &p.build_id, &p.sig);
        assert!(prepare(&state, &game, &game.join("fefw.log"), from, started, Some(-1073741819), "").is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn base64_encodes() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
    }
}
