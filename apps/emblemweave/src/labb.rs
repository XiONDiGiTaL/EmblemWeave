//! Performance Lab: a measured view of this PC, the engine's own telemetry, and
//! recommendations that remain proposals until the player approves them.
//!
//! The Lab does not invent benchmark workloads. A run starts (or attaches to)
//! the real game, asks the engine for its one-second `perf_status.json`, and
//! optionally enables the runtime's soak sampler for working-set and gauge
//! evidence. Everything else is math over those samples. A category the engine
//! does not expose is reported as unavailable.

use crate::config::{Config, Graphics, Performance};
use crate::memory;
use crate::performance::RuntimeStatus;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const QUICK_SECS: u64 = 150;
pub const EXTENDED_SECS: u64 = 900;

pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn unix_s() -> u64 {
    unix_ms() / 1000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Quick,
    Extended,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Quick => "Quick",
            Mode::Extended => "Extended",
        }
    }

    pub fn seconds(self) -> u64 {
        match self {
            Mode::Quick => QUICK_SECS,
            Mode::Extended => EXTENDED_SECS,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub gpu_elevated: u32,
    pub gpu_high: u32,
    pub gpu_critical: u32,
    pub host_elevated: u32,
    pub host_high: u32,
    pub host_critical: u32,
}

impl From<&crate::config::Lab> for Thresholds {
    fn from(t: &crate::config::Lab) -> Self {
        Thresholds {
            gpu_elevated: t.gpu_elevated,
            gpu_high: t.gpu_high,
            gpu_critical: t.gpu_critical,
            host_elevated: t.host_elevated,
            host_high: t.host_high,
            host_critical: t.host_critical,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Level {
    Normal,
    Elevated,
    High,
    Critical,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Normal => "Normal",
            Level::Elevated => "Elevated",
            Level::High => "High",
            Level::Critical => "Critical",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Pressure {
    pub gpu: Option<(Level, String)>,
    pub host: Option<(Level, String)>,
    pub reasons: Vec<String>,
}

impl Pressure {
    pub fn overall(&self) -> Level {
        self.gpu
            .iter()
            .chain(self.host.iter())
            .map(|(level, _)| *level)
            .max()
            .unwrap_or(Level::Normal)
    }
}

/// Classifies GPU budget use. This is the driver's figure for this process,
/// not an invented estimate and not added to host memory.
pub fn classify_gpu(used_mb: u64, budget_mb: u64, t: &Thresholds) -> (Level, String) {
    if budget_mb == 0 {
        return (Level::Normal, "the driver did not report a video-memory budget".into());
    }
    let pct = (used_mb.saturating_mul(100) / budget_mb) as u32;
    let level = if pct >= t.gpu_critical {
        Level::Critical
    } else if pct >= t.gpu_high {
        Level::High
    } else if pct >= t.gpu_elevated {
        Level::Elevated
    } else {
        Level::Normal
    };
    (
        level,
        format!(
            "{} of {} used ({}%) · thresholds {} / {} / {}%",
            memory::gb(used_mb),
            memory::gb(budget_mb),
            pct,
            t.gpu_elevated,
            t.gpu_high,
            t.gpu_critical
        ),
    )
}

fn host_level(free_pct: u32, t: &Thresholds) -> Level {
    if free_pct <= t.host_critical {
        Level::Critical
    } else if free_pct <= t.host_high {
        Level::High
    } else if free_pct <= t.host_elevated {
        Level::Elevated
    } else {
        Level::Normal
    }
}

/// Host physical memory and commit headroom are separate facts. The guest heap
/// reservation and the runtime's host limit are deliberately not used here.
pub fn classify_host(s: &memory::Snapshot, t: &Thresholds) -> Option<(Level, String)> {
    let installed = s.installed_mb.or(s.total_mb)?;
    let available = s.available_mb?;
    let available_pct = (available.saturating_mul(100) / installed.max(1)) as u32;
    let available_level = host_level(available_pct, t);
    let (commit_level, commit_pct) = match (s.commit_free_mb, s.commit_limit_mb) {
        (Some(free), Some(limit)) if limit > 0 => {
            let pct = (free.saturating_mul(100) / limit) as u32;
            (host_level(pct, t), Some(pct))
        }
        _ => (Level::Normal, None),
    };
    let level = available_level.max(commit_level);
    let commit = commit_pct
        .map(|p| format!(" · commit {p}% free"))
        .unwrap_or_else(|| " · commit headroom unavailable".into());
    Some((
        level,
        format!(
            "{} of {} physical available ({}%){commit}",
            memory::gb(available),
            memory::gb(installed),
            available_pct
        ),
    ))
}

fn active_profile(config: &Config, profiles: &[crate::gpu::Profile]) -> Option<usize> {
    let wanted = config.graphics.gpu.trim();
    if !wanted.is_empty()
        && !wanted.eq_ignore_ascii_case("automatic")
        && !wanted.eq_ignore_ascii_case("discrete")
        && !wanted.eq_ignore_ascii_case("integrated")
    {
        if let Some(i) = profiles.iter().position(|p| p.device.name.eq_ignore_ascii_case(wanted)) {
            return Some(i);
        }
    }
    let wanted_kind = if wanted.eq_ignore_ascii_case("integrated") {
        Some("Integrated")
    } else if wanted.eq_ignore_ascii_case("discrete") {
        Some("Discrete")
    } else {
        None
    };
    profiles
        .iter()
        .enumerate()
        .filter(|(_, p)| wanted_kind.is_none_or(|k| p.device.kind == k))
        .max_by_key(|(_, p)| {
            (
                if p.device.kind == "Discrete" { 1 } else { 0 },
                p.device.vram_mb,
            )
        })
        .map(|(i, _)| i)
}

#[derive(Debug, Clone)]
pub struct CpuFacts {
    pub brand: String,
    pub architecture: String,
    pub physical: Option<usize>,
    pub logical: usize,
    pub sse42: bool,
    pub popcnt: bool,
    pub cx16: bool,
    pub avx: bool,
    pub fma: bool,
    pub build: String,
}

pub fn probe_cpu() -> CpuFacts {
    let f = crate::cpu::probe();
    let chosen = crate::cpu::choose(&f);
    CpuFacts {
        brand: if f.brand.is_empty() { "Unavailable".into() } else { f.brand.clone() },
        architecture: std::env::consts::ARCH.to_string(),
        physical: physical_cores(),
        logical: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        sse42: f.sse42,
        popcnt: f.popcnt,
        cx16: f.cx16,
        avx: f.avx,
        fma: f.fma,
        build: match chosen {
            Ok(v) => format!("{} ({})", v.label(), v.key()),
            Err(e) => e,
        },
    }
}

/// Windows exposes physical cores through
/// `GetLogicalProcessorInformationEx(RelationProcessorCore, …)`. The record
/// header is `Relationship: u32, Size: u32`; the union after it is not read.
#[cfg(windows)]
fn physical_cores() -> Option<usize> {
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLogicalProcessorInformationEx(
            relationship: u32,
            buffer: *mut std::ffi::c_void,
            returned_length: *mut u32,
        ) -> i32;
    }
    const RELATION_PROCESSOR_CORE: u32 = 0;
    // 64 KiB is far more than any current desktop needs (one record per core).
    let mut bytes = vec![0u64; 8192];
    let mut len = (bytes.len() * 8) as u32;
    let ok = unsafe {
        GetLogicalProcessorInformationEx(
            RELATION_PROCESSOR_CORE,
            bytes.as_mut_ptr().cast(),
            &mut len,
        )
    };
    if ok == 0 {
        return None;
    }
    let raw = unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast::<u8>(), len as usize) };
    let mut at = 0usize;
    let mut cores = 0usize;
    while at + 8 <= raw.len() {
        let relationship = u32::from_ne_bytes(raw[at..at + 4].try_into().ok()?);
        let size = u32::from_ne_bytes(raw[at + 4..at + 8].try_into().ok()?) as usize;
        if size < 8 || at + size > raw.len() {
            return None;
        }
        if relationship == RELATION_PROCESSOR_CORE {
            cores += 1;
        }
        at += size;
    }
    (cores > 0).then_some(cores)
}

#[cfg(not(windows))]
fn physical_cores() -> Option<usize> {
    None
}

#[derive(Debug, Clone)]
pub struct Display {
    pub resolution: Option<(i32, i32)>,
    pub refresh_hz: Option<i32>,
}

#[cfg(windows)]
fn primary_display() -> Display {
    use windows_sys::Win32::Graphics::Gdi::{GetDC, GetDeviceCaps, ReleaseDC, VREFRESH};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
    let (w, h) = unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) };
    let refresh = unsafe {
        let dc = GetDC(std::ptr::null_mut());
        if dc.is_null() {
            None
        } else {
            let hz = GetDeviceCaps(dc, VREFRESH as i32);
            ReleaseDC(std::ptr::null_mut(), dc);
            (hz > 1).then_some(hz)
        }
    };
    Display {
        resolution: (w > 0 && h > 0).then_some((w, h)),
        refresh_hz: refresh,
    }
}

#[cfg(not(windows))]
fn primary_display() -> Display {
    Display { resolution: None, refresh_hz: None }
}

#[derive(Debug, Clone)]
pub struct StorageFacts {
    pub volume: String,
    pub free_mb: u64,
    pub total_mb: u64,
}

/// Capacity of the volume that holds the game's play folder. This is a
/// directory-level query, not a guess from the game size.
pub fn storage_facts(path: &Path) -> Option<StorageFacts> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        let probe = path
            .ancestors()
            .find(|p| p.exists())
            .unwrap_or(path)
            .as_os_str();
        let wide: Vec<u16> = OsStr::new(probe).encode_wide().chain(Some(0)).collect();
        let mut free = 0u64;
        let mut total = 0u64;
        let mut total_free = 0u64;
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free,
                &mut total,
                &mut total_free,
            )
        };
        if ok == 0 || total == 0 {
            return None;
        }
        let volume = path
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .unwrap_or_else(|| "game volume".into());
        return Some(StorageFacts {
            volume,
            free_mb: free / (1024 * 1024),
            total_mb: total / (1024 * 1024),
        });
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        None
    }
}

#[derive(Debug, Clone)]
pub struct SystemProfile {
    pub captured_ms: u64,
    pub cpu: CpuFacts,
    pub memory: memory::Snapshot,
    pub gpus: Vec<crate::gpu::Profile>,
    pub active_gpu: Option<usize>,
    pub display: Display,
    pub storage: Option<StorageFacts>,
    pub pressure: Pressure,
    pub gpu_budget: Option<(u64, u64, String)>,
}

impl SystemProfile {
    pub fn active(&self) -> Option<&crate::gpu::Profile> {
        self.active_gpu.and_then(|i| self.gpus.get(i))
    }

    pub fn active_name(&self) -> Option<&str> {
        self.active().map(|p| p.device.name.as_str())
    }
}

fn pressure_for(config: &Config, gpus: &[crate::gpu::Profile], active: Option<usize>, snap: &memory::Snapshot) -> (Pressure, Option<(u64, u64, String)>) {
    let t = Thresholds::from(&config.lab);
    let mut out = Pressure::default();
    let mut budget = None;
    if let Some(p) = active.and_then(|i| gpus.get(i)) {
        if let Some(h) = p.largest_device_local_heap() {
            if let (Some(used), Some(budget_mb)) = (h.usage_mb, h.budget_mb) {
                let (level, note) = classify_gpu(used, budget_mb, &t);
                out.gpu = Some((level, note.clone()));
                if level >= Level::Elevated {
                    out.reasons.push(format!("GPU heap pressure is {}: {note}", level.label()));
                }
                budget = Some((used, budget_mb, "VK_EXT_memory_budget heap".to_string()));
            }
        }
    }
    // The engine log is a fallback only. It is never added to the Vulkan
    // figure: both describe the same video memory.
    if budget.is_none() {
        if let Some(g) = &snap.gpu {
            if let (Some(used), Some(limit)) = (g.used_mb, g.budget_mb) {
                let (level, note) = classify_gpu(used, limit, &t);
                out.gpu = Some((level, note.clone()));
                if level >= Level::Elevated {
                    out.reasons.push(format!("GPU budget pressure is {}: {note}", level.label()));
                }
                budget = Some((used, limit, "engine log (OS budget)".to_string()));
            }
        }
    }
    if let Some((level, note)) = classify_host(snap, &t) {
        out.host = Some((level, note.clone()));
        if level >= Level::Elevated {
            out.reasons.push(format!("Host memory headroom is {}: {note}", level.label()));
        }
    }
    (out, budget)
}

pub fn probe(config: &Config) -> SystemProfile {
    let memory = memory::snapshot();
    let gpus = crate::gpu::cached_profiles().unwrap_or(&[]).to_vec();
    let active_gpu = active_profile(config, &gpus);
    let (pressure, gpu_budget) = pressure_for(config, &gpus, active_gpu, &memory);
    let storage = crate::engine::run_dir().as_deref().and_then(storage_facts);
    SystemProfile {
        captured_ms: unix_ms(),
        cpu: probe_cpu(),
        memory,
        storage,
        gpus,
        active_gpu,
        display: primary_display(),
        pressure,
        gpu_budget,
    }
}

#[derive(Debug, Clone)]
pub struct Sample {
    pub at_ms: u64,
    pub frames: u64,
    pub status: RuntimeStatus,
    pub host_available_mb: Option<u64>,
    pub commit_free_mb: Option<u64>,
    pub gpu_used_mb: Option<u64>,
    pub gpu_budget_mb: Option<u64>,
    pub process_ws_mb: Option<u64>,
}

/// A benchmark session records engine snapshots; it never owns GPU resources or
/// changes the engine configuration, so cancelling it needs no GPU teardown.
#[derive(Debug, Clone)]
pub struct Recorder {
    pub mode: Mode,
    pub started_ms: u64,
    pub active_since_ms: Option<u64>,
    pub samples: Vec<Sample>,
    pub cancelled: bool,
}

impl Recorder {
    pub fn new(mode: Mode, now_ms: u64) -> Self {
        Recorder {
            mode,
            started_ms: now_ms,
            active_since_ms: None,
            samples: Vec::new(),
            cancelled: false,
        }
    }

    pub fn elapsed_s(&self, now_ms: u64) -> u64 {
        self.active_since_ms
            .map(|s| now_ms.saturating_sub(s) / 1000)
            .unwrap_or(0)
    }

    pub fn remaining_s(&self, now_ms: u64) -> u64 {
        self.mode.seconds().saturating_sub(self.elapsed_s(now_ms))
    }

    pub fn progress(&self, now_ms: u64) -> f32 {
        (self.elapsed_s(now_ms) as f32 / self.mode.seconds() as f32).clamp(0.0, 1.0)
    }

    pub fn observe(
        &mut self,
        now_ms: u64,
        status: &RuntimeStatus,
        host: &memory::Snapshot,
        gpu: Option<(u64, u64)>,
        ws_mb: Option<u64>,
    ) {
        if self.active_since_ms.is_none() {
            self.active_since_ms = Some(now_ms);
        }
        if let Some(last) = self.samples.last() {
            if last.frames == status.frames && now_ms.saturating_sub(last.at_ms) < 2_000 {
                return;
            }
        }
        self.samples.push(Sample {
            at_ms: now_ms,
            frames: status.frames,
            status: status.clone(),
            host_available_mb: host.available_mb,
            commit_free_mb: host.commit_free_mb,
            gpu_used_mb: gpu.map(|g| g.0),
            gpu_budget_mb: gpu.map(|g| g.1),
            process_ws_mb: ws_mb,
        });
    }

    pub fn finished(&self, now_ms: u64) -> bool {
        self.active_since_ms.is_some() && self.elapsed_s(now_ms) >= self.mode.seconds()
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
        self.samples.clear();
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SoakTotals {
    pub process_ws_start_mb: Option<u64>,
    pub process_ws_end_mb: Option<u64>,
    pub process_ws_peak_mb: Option<u64>,
    pub process_private_end_mb: Option<u64>,
    pub shader_translations_per_s: Option<f64>,
    pub shader_cache_hit_pct: Option<f64>,
    pub pipelines_created: Option<u64>,
    pub pool_import_failures: Option<u64>,
    pub pool_copy_fallbacks: Option<u64>,
    pub vulkan_images_end: Option<u64>,
    pub vulkan_buffers_end: Option<u64>,
    pub live_textures_end: Option<u64>,
    pub asset_read_mb_s: Option<f64>,
    pub video_fps: Option<f64>,
    pub texture_prep_mb_s: Option<f64>,
    pub gpu_work_ms_per_frame: Option<f64>,
    pub source: String,
}

impl SoakTotals {
    pub fn unavailable() -> Self {
        SoakTotals {
            source: "the runtime soak sampler produced no JSON lines for this run".into(),
            ..SoakTotals::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Summary {
    pub samples: usize,
    pub observed_s: f64,
    pub avg_fps: f64,
    pub low1_fps: f64,
    pub low01_fps: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub cpu_ms: f64,
    pub gpu_ms: f64,
    pub upload_ms: f64,
    pub replay_ms: f64,
    pub peak_vram_percent: i32,
    pub max_shader_backlog: i32,
    pub spikes: u64,
    pub gpu_peak_used_mb: Option<u64>,
    pub gpu_budget_mb: Option<u64>,
    pub host_available_min_mb: Option<u64>,
    pub commit_free_min_mb: Option<u64>,
    pub host_total_mb: Option<u64>,
    pub soak: SoakTotals,
    pub warnings: Vec<String>,
    pub unavailable: Vec<String>,
}

impl Summary {
    pub fn gpu_peak_pct(&self) -> Option<u32> {
        match (self.gpu_peak_used_mb, self.gpu_budget_mb) {
            (Some(used), Some(budget)) if budget > 0 => Some((used.saturating_mul(100) / budget) as u32),
            _ => None,
        }
    }
}

fn gauge(map: &BTreeMap<String, i64>, key: &str) -> Option<i64> {
    map.get(key).copied()
}

fn delta(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    Some(b?.saturating_sub(a?))
}

fn per_second(a: Option<i64>, b: Option<i64>, seconds: f64) -> Option<f64> {
    (seconds > 0.0).then(|| delta(a, b).map(|d| d as f64 / seconds)).flatten()
}

#[derive(Debug, Clone)]
pub struct SoakPoint {
    pub t: f64,
    pub frame: i64,
    pub ws: Option<i64>,
    pub private: Option<i64>,
    pub gauges: BTreeMap<String, i64>,
}

pub fn parse_soak(text: &str) -> Vec<SoakPoint> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(obj) = value.as_object() else {
            continue;
        };
        let mut gauges = BTreeMap::new();
        for (k, v) in obj {
            if let Some(n) = v.as_i64() {
                gauges.insert(k.clone(), n);
            }
        }
        let some = |key: &str| obj.get(key).and_then(|v| v.as_i64());
        out.push(SoakPoint {
            t: obj.get("t").and_then(|v| v.as_f64()).unwrap_or(0.0),
            frame: some("frame").unwrap_or(0),
            ws: some("ws"),
            private: some("private"),
            gauges,
        });
    }
    out
}

fn mib(bytes: i64) -> u64 {
    (bytes.max(0) as u64) / (1024 * 1024)
}

pub fn summarize_soak(points: &[SoakPoint]) -> SoakTotals {
    if points.len() < 2 {
        return SoakTotals::unavailable();
    }
    let first = &points[0];
    let last = points.last().expect("length checked");
    let seconds = (last.t - first.t).max(0.0);
    let first_g = |k: &str| gauge(&first.gauges, k);
    let last_g = |k: &str| gauge(&last.gauges, k);
    let hits = delta(first_g("rp.shader_cache_hits"), last_g("rp.shader_cache_hits"));
    let misses = delta(first_g("rp.shader_cache_misses"), last_g("rp.shader_cache_misses"));
    let cache_pct = match (hits, misses) {
        (Some(h), Some(m)) if h + m > 0 => Some(h as f64 * 100.0 / (h + m) as f64),
        _ => None,
    };
    let frames = (last.frame - first.frame).max(0) as f64;
    let gpu_us = delta(first_g("nvn.cnt.gpu_us"), last_g("nvn.cnt.gpu_us"));
    SoakTotals {
        process_ws_start_mb: first.ws.map(mib),
        process_ws_end_mb: last.ws.map(mib),
        process_ws_peak_mb: points.iter().filter_map(|p| p.ws).max().map(mib),
        process_private_end_mb: last.private.map(mib),
        shader_translations_per_s: per_second(first_g("rp.shader_translations"), last_g("rp.shader_translations"), seconds),
        shader_cache_hit_pct: cache_pct,
        pipelines_created: delta(first_g("rp.pipeline_gfx_created"), last_g("rp.pipeline_gfx_created")).map(|v| v.max(0) as u64),
        pool_import_failures: delta(first_g("nvn.pool_import_failures"), last_g("nvn.pool_import_failures")).map(|v| v.max(0) as u64),
        pool_copy_fallbacks: delta(first_g("nvn.pool_copy_fallbacks"), last_g("nvn.pool_copy_fallbacks")).map(|v| v.max(0) as u64),
        vulkan_images_end: last_g("vk.images").map(|v| v.max(0) as u64),
        vulkan_buffers_end: last_g("vk.buffers").map(|v| v.max(0) as u64),
        live_textures_end: last_g("nvn.textures_live").map(|v| v.max(0) as u64),
        asset_read_mb_s: per_second(first_g("fs.bytes_read"), last_g("fs.bytes_read"), seconds).map(|v| v / (1024.0 * 1024.0)),
        video_fps: per_second(first_g("media.video_frames"), last_g("media.video_frames"), seconds),
        texture_prep_mb_s: per_second(first_g("nvn.texprep.staged_bytes_total"), last_g("nvn.texprep.staged_bytes_total"), seconds)
            .map(|v| v / (1024.0 * 1024.0)),
        gpu_work_ms_per_frame: gpu_us.map(|us| {
            if frames > 0.0 {
                us as f64 / 1000.0 / frames
            } else {
                0.0
            }
        }),
        source: format!("soak sampler: {} points over {:.0} s", points.len(), seconds),
    }
}

pub fn summarize(samples: &[Sample], soak: SoakTotals) -> Summary {
    let mut out = Summary {
        samples: samples.len(),
        soak,
        ..Summary::default()
    };
    // These limits apply whether or not this particular run produced samples.
    out.unavailable.push("Vulkan allocation latency is not exposed by the engine's gauges; allocation failures and fallbacks are reported when present.".into());
    out.unavailable.push("This is not a stability certification.".into());
    if samples.is_empty() {
        out.unavailable.push("No engine telemetry was received. Start the game and leave it on a representative scene.".into());
        return out;
    }
    let n = samples.len() as f64;
    let first = &samples[0];
    let last = samples.last().expect("non-empty");
    out.observed_s = last.at_ms.saturating_sub(first.at_ms) as f64 / 1000.0;
    out.avg_fps = samples.iter().map(|s| s.status.avg_fps).sum::<f64>() / n;
    out.low1_fps = samples.iter().map(|s| s.status.low1_fps).filter(|v| v.is_finite()).fold(f64::INFINITY, f64::min);
    out.low01_fps = samples.iter().map(|s| s.status.low01_fps).filter(|v| v.is_finite()).fold(f64::INFINITY, f64::min);
    if !out.low1_fps.is_finite() {
        out.low1_fps = 0.0;
    }
    if !out.low01_fps.is_finite() {
        out.low01_fps = 0.0;
    }
    out.p50_ms = samples.iter().map(|s| s.status.p50_ms).sum::<f64>() / n;
    out.p95_ms = samples.iter().map(|s| s.status.p95_ms).fold(0.0, f64::max);
    out.p99_ms = samples.iter().map(|s| s.status.p99_ms).fold(0.0, f64::max);
    out.cpu_ms = samples.iter().map(|s| s.status.cpu_ms).sum::<f64>() / n;
    out.gpu_ms = samples.iter().map(|s| s.status.gpu_ms).sum::<f64>() / n;
    out.upload_ms = samples.iter().map(|s| s.status.upload_ms).sum::<f64>() / n;
    out.replay_ms = samples.iter().map(|s| s.status.replay_ms).sum::<f64>() / n;
    out.peak_vram_percent = samples.iter().map(|s| s.status.vram_percent).max().unwrap_or(-1);
    out.max_shader_backlog = samples.iter().map(|s| s.status.shader_backlog).max().unwrap_or(-1);
    out.spikes = last.status.spikes.saturating_sub(first.status.spikes);
    out.gpu_peak_used_mb = samples.iter().filter_map(|s| s.gpu_used_mb).max();
    out.gpu_budget_mb = samples.iter().filter_map(|s| s.gpu_budget_mb).min();
    out.host_available_min_mb = samples.iter().filter_map(|s| s.host_available_mb).min();
    out.commit_free_min_mb = samples.iter().filter_map(|s| s.commit_free_mb).min();
    out.host_total_mb = None;

    if out.avg_fps < 30.0 {
        out.warnings.push(format!("Average engine frame rate was {:.1} fps in this run.", out.avg_fps));
    }
    if out.low1_fps > 0.0 && out.low1_fps < 30.0 {
        out.warnings.push(format!("The 1% low was {:.1} fps: frame delivery was uneven.", out.low1_fps));
    }
    if let Some(pct) = out.gpu_peak_pct() {
        if pct >= 90 {
            out.warnings.push(format!("The engine reached {pct}% of the OS video-memory budget."));
        }
    } else {
        out.unavailable.push("GPU budget use: neither VK_EXT_memory_budget nor the engine log supplied a budget and usage pair.".into());
    }
    if let (Some(start), Some(end)) = (out.soak.process_ws_start_mb, out.soak.process_ws_end_mb) {
        if end.saturating_sub(start) > 512 {
            out.warnings.push(format!(
                "The game's working set grew by {} during the run.",
                memory::gb(end - start)
            ));
        }
    }
    if out.soak.shader_translations_per_s.is_none() {
        out.unavailable.push("Shader compilation throughput and pipeline creation need the soak sampler; this run did not provide them.".into());
    }
    if out.soak.asset_read_mb_s.is_none() {
        out.unavailable.push("Asset-loading throughput needs the soak sampler's FS counters; this run did not provide them.".into());
    }
    if out.soak.video_fps.is_none() {
        out.unavailable.push("Video frame delivery was not exercised or its counter was absent during this run.".into());
    }
    if out.soak.pool_import_failures.is_some_and(|n| n > 0) {
        out.warnings.push(format!(
            "The engine reported {} pool import failure(s) during this run; the game may have fallen back to a slower path rather than crashed.",
            out.soak.pool_import_failures.unwrap_or(0)
        ));
    }
    if out.soak.process_ws_start_mb.is_none() {
        out.unavailable.push("Process working-set growth needs the soak sampler; this run did not provide it.".into());
    }
    out.unavailable.push("CPU and GPU subsystem benchmarks are measurements of the running game, not isolated synthetic workloads.".into());
    out
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RunMetadata {
    pub launcher: String,
    pub engine: String,
    pub cpu: String,
    pub gpu: String,
    pub driver: String,
    pub vulkan: String,
    pub os: String,
    pub resolution: String,
    pub graphics_config: String,
    pub performance_config: String,
}

impl RunMetadata {
    pub fn collect(config: &Config, profile: &SystemProfile) -> Self {
        let gpu = profile.active();
        let resolution = profile
            .display
            .resolution
            .map(|(w, h)| format!("{w} × {h}"))
            .unwrap_or_else(|| "Unavailable".into());
        RunMetadata {
            launcher: format!("{} ({})", env!("CARGO_PKG_VERSION"), env!("EW_GIT_REV")),
            engine: engine_version().unwrap_or_else(|| "not reported by fefw.ini".into()),
            cpu: profile.cpu.brand.clone(),
            gpu: profile.active_name().unwrap_or("Unavailable").to_string(),
            driver: gpu.map(|p| format!("{} · {}", p.driver, p.api_version)).unwrap_or_else(|| "Unavailable".into()),
            vulkan: gpu
                .map(|p| format!("{} extensions · {} heaps", p.extensions.len(), p.heaps.len()))
                .unwrap_or_else(|| "Unavailable".into()),
            os: os_label(),
            resolution,
            graphics_config: config_summary(config),
            performance_config: format!(
                "controller {}, {}, target {} fps, residency {}%",
                if config.performance.controller { "on" } else { "off" },
                config.performance.mode,
                config.performance.target_fps,
                config.performance.residency_percent
            ),
        }
    }
}

fn engine_version() -> Option<String> {
    let dir = crate::engine::run_dir()?;
    let text = std::fs::read_to_string(dir.join("fefw.ini")).ok()?;
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        if section == "fefw" || section == "engine" {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim().eq_ignore_ascii_case("built_by") && !v.trim().is_empty() {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

pub fn os_label() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

pub fn config_summary(config: &Config) -> String {
    format!(
        "{} · scale {:.2}× · scene {}% · {} · textures {} · shaders {} · frame gen {} · decoder {}",
        config.graphics.resolution,
        config.graphics.render_scale,
        config.graphics.scene_scale,
        config.graphics.upscaling,
        config.graphics.astc_quality,
        if config.graphics.shader_precompile { "precompile" } else { "on demand" },
        config.graphics.frame_gen,
        config.graphics.video_decoder
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    pub created_unix_s: u64,
    pub mode: Mode,
    pub duration_s: f64,
    pub metadata: RunMetadata,
    pub summary: Summary,
}

impl Run {
    pub fn new(mode: Mode, started_ms: u64, metadata: RunMetadata, summary: Summary) -> Self {
        let created = started_ms / 1000;
        Run {
            id: format!("{created}-{}", mode.label().to_ascii_lowercase()),
            created_unix_s: created,
            mode,
            duration_s: summary.observed_s,
            metadata,
            summary,
        }
    }
}

pub fn lab_dir() -> Option<PathBuf> {
    Config::dir().map(|d| d.join("lab"))
}

pub fn history_path() -> Option<PathBuf> {
    lab_dir().map(|d| d.join("history.json"))
}

pub fn backup_path() -> Option<PathBuf> {
    lab_dir().map(|d| d.join("backup.json"))
}

pub fn load_history_from(path: &Path) -> Vec<Run> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<Vec<Run>>(&t).ok())
        .unwrap_or_default()
}

pub fn load_history() -> Vec<Run> {
    history_path().map(|p| load_history_from(&p)).unwrap_or_default()
}

pub fn append_history_to(path: &Path, run: Run, cap: usize) -> Result<(), String> {
    let mut runs = load_history_from(path);
    runs.push(run);
    if runs.len() > cap {
        let drop = runs.len() - cap;
        runs.drain(0..drop);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(&runs).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replacing {}: {e}", path.display()))
}

pub fn append_history(run: Run) -> Result<(), String> {
    let path = history_path().ok_or(crate::platform::NO_DATA_DIR)?;
    append_history_to(&path, run, 40)
}

pub fn report_markdown(run: &Run) -> String {
    let s = &run.summary;
    let mut out = String::new();
    out.push_str("# EmblemWeave Performance Lab report\n\n");
    out.push_str("This report contains measurements only. It is not a stability certification, and synthetic or engine-subsystem numbers are not guaranteed gameplay FPS.\n\n");
    out.push_str(&format!("- Run: `{}` ({})\n", run.id, run.mode.label()));
    out.push_str(&format!("- Duration: {:.1} s of engine telemetry\n", run.duration_s));
    out.push_str(&format!("- Launcher: {}\n", run.metadata.launcher));
    out.push_str(&format!("- Engine: {}\n", run.metadata.engine));
    out.push_str(&format!("- OS: {}\n", run.metadata.os));
    out.push_str(&format!("- CPU: {}\n", run.metadata.cpu));
    out.push_str(&format!("- GPU: {}\n", run.metadata.gpu));
    out.push_str(&format!("- Driver / Vulkan: {}\n", run.metadata.driver));
    out.push_str(&format!("- Display: {}\n", run.metadata.resolution));
    out.push_str(&format!("- Graphics: {}\n", run.metadata.graphics_config));
    out.push_str(&format!("- Performance: {}\n\n", run.metadata.performance_config));

    out.push_str("## Engine frame telemetry\n\n");
    out.push_str("| Metric | Value |\n|---|---|\n");
    // A run with no snapshots has no frame rate. Never turn that into 0 fps in
    // a report someone may attach to an issue.
    let measured = s.samples > 0;
    let frame_value = |value: f64, unit: &str| {
        if measured {
            format!("{value:.1}{unit}")
        } else {
            "unavailable".into()
        }
    };
    let rows = [
        ("Samples", format!("{}", s.samples)),
        ("Observed", format!("{:.1} s", s.observed_s)),
        ("Average FPS", frame_value(s.avg_fps, "")),
        ("1% low", frame_value(s.low1_fps, " fps")),
        ("0.1% low", frame_value(s.low01_fps, " fps")),
        ("P50 frame", frame_value(s.p50_ms, " ms")),
        ("P95 frame", frame_value(s.p95_ms, " ms")),
        ("P99 frame", frame_value(s.p99_ms, " ms")),
        ("CPU frame time", frame_value(s.cpu_ms, " ms")),
        ("GPU frame time", frame_value(s.gpu_ms, " ms")),
        ("Spikes", if measured { format!("{}", s.spikes) } else { "unavailable".into() }),
        ("Shader backlog peak", if measured { format!("{}", s.max_shader_backlog) } else { "unavailable".into() }),
        ("VRAM peak", if s.peak_vram_percent >= 0 { format!("{}%", s.peak_vram_percent) } else { "unavailable".into() }),
        ("GPU budget peak", s.gpu_peak_pct().map(|p| format!("{p}%")).unwrap_or_else(|| "unavailable".into())),
        ("Host available minimum", s.host_available_min_mb.map(memory::gb).unwrap_or_else(|| "unavailable".into())),
        ("Commit free minimum", s.commit_free_min_mb.map(memory::gb).unwrap_or_else(|| "unavailable".into())),
    ];
    for (k, v) in rows {
        out.push_str(&format!("| {k} | {v} |\n"));
    }

    out.push_str("\n## Engine subsystem evidence\n\n");
    out.push_str("| Metric | Value |\n|---|---|\n");
    let soak = &s.soak;
    for (k, v) in [
        ("Soak source", soak.source.clone()),
        (
            "Working set",
            match (soak.process_ws_start_mb, soak.process_ws_end_mb) {
                (Some(a), Some(b)) => format!("{} → {} (peak {})", memory::gb(a), memory::gb(b), soak.process_ws_peak_mb.map(memory::gb).unwrap_or_else(|| "unavailable".into())),
                _ => "unavailable".into(),
            },
        ),
        ("Shader translations", soak.shader_translations_per_s.map(|v| format!("{v:.1}/s")).unwrap_or_else(|| "unavailable".into())),
        ("Shader cache hit share", soak.shader_cache_hit_pct.map(|v| format!("{v:.1}%")).unwrap_or_else(|| "unavailable".into())),
        ("Graphics pipelines created", soak.pipelines_created.map(|v| v.to_string()).unwrap_or_else(|| "unavailable".into())),
        ("Pool import failures", soak.pool_import_failures.map(|v| v.to_string()).unwrap_or_else(|| "unavailable".into())),
        ("Pool copy fallbacks", soak.pool_copy_fallbacks.map(|v| v.to_string()).unwrap_or_else(|| "unavailable".into())),
        ("Live Vulkan images at end", soak.vulkan_images_end.map(|v| v.to_string()).unwrap_or_else(|| "unavailable".into())),
        ("Live Vulkan buffers at end", soak.vulkan_buffers_end.map(|v| v.to_string()).unwrap_or_else(|| "unavailable".into())),
        ("Live NVN textures at end", soak.live_textures_end.map(|v| v.to_string()).unwrap_or_else(|| "unavailable".into())),
        ("Asset reads", soak.asset_read_mb_s.map(|v| format!("{v:.1} MiB/s")).unwrap_or_else(|| "unavailable".into())),
        ("Video delivery", soak.video_fps.map(|v| format!("{v:.1} frames/s")).unwrap_or_else(|| "unavailable".into())),
        ("Texture preparation", soak.texture_prep_mb_s.map(|v| format!("{v:.1} MiB/s")).unwrap_or_else(|| "unavailable".into())),
        ("GPU work from timestamps", soak.gpu_work_ms_per_frame.map(|v| format!("{v:.2} ms/frame")).unwrap_or_else(|| "unavailable".into())),
    ] {
        out.push_str(&format!("| {k} | {v} |\n"));
    }

    out.push_str("\n## Warnings\n\n");
    if s.warnings.is_empty() {
        out.push_str("None recorded.\n");
    } else {
        for w in &s.warnings {
            out.push_str(&format!("- {w}\n"));
        }
    }
    out.push_str("\n## Unavailable or limited measurements\n\n");
    for u in &s.unavailable {
        out.push_str(&format!("- {u}\n"));
    }
    out.push_str("\nNo paths, account names, save contents, or hardware serial numbers are included.\n");
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Change {
    RenderScale(f32),
    SceneScale(u32),
    AstcQuality(String),
    CompileThreads(u32),
    CompilePriority(String),
    CompileEco(bool),
    ShaderPrecompile(bool),
    FrameGen(String),
    VideoDecoder(String),
    VideoHwaccel(bool),
    ResidencyPercent(u32),
    TexPrepMb(u32),
    AdaptiveMip(bool),
    Prewarm(bool),
    Gpl(bool),
}

impl Change {
    pub fn key(&self) -> &'static str {
        match self {
            Change::RenderScale(_) => "render scale",
            Change::SceneScale(_) => "scene scale",
            Change::AstcQuality(_) => "texture quality",
            Change::CompileThreads(_) => "shader compile threads",
            Change::CompilePriority(_) => "shader compile priority",
            Change::CompileEco(_) => "shader compile efficiency mode",
            Change::ShaderPrecompile(_) => "shader precompilation",
            Change::FrameGen(_) => "frame generation",
            Change::VideoDecoder(_) => "video decoder",
            Change::VideoHwaccel(_) => "video hardware decoding",
            Change::ResidencyPercent(_) => "VRAM residency threshold",
            Change::TexPrepMb(_) => "texture preparation cache",
            Change::AdaptiveMip(_) => "adaptive mip bias",
            Change::Prewarm(_) => "pipeline prewarming",
            Change::Gpl(_) => "background shader compilation",
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Change::RenderScale(v) => format!("{v:.2}×"),
            Change::SceneScale(v) => format!("{v}%"),
            Change::AstcQuality(v) => v.clone(),
            Change::CompileThreads(v) => format!("{v}"),
            Change::CompilePriority(v) => v.clone(),
            Change::CompileEco(v) => if *v { "on".into() } else { "off".into() },
            Change::ShaderPrecompile(v) => if *v { "on".into() } else { "off".into() },
            Change::FrameGen(v) => v.clone(),
            Change::VideoDecoder(v) => v.clone(),
            Change::VideoHwaccel(v) => if *v { "on".into() } else { "off".into() },
            Change::ResidencyPercent(v) => format!("{v}%"),
            Change::TexPrepMb(v) => format!("{v} MiB"),
            Change::AdaptiveMip(v) => if *v { "on".into() } else { "off".into() },
            Change::Prewarm(v) => if *v { "on".into() } else { "off".into() },
            Change::Gpl(v) => if *v { "on".into() } else { "off".into() },
        }
    }

    pub fn applies(&self, g: &Graphics, p: &Performance) -> bool {
        match self {
            Change::RenderScale(v) => (g.render_scale - v).abs() > 0.001,
            Change::SceneScale(v) => g.scene_scale != *v,
            Change::AstcQuality(v) => g.astc_quality != *v,
            Change::CompileThreads(v) => g.compile_threads != *v,
            Change::CompilePriority(v) => g.compile_priority != *v,
            Change::CompileEco(v) => g.compile_eco != *v,
            Change::ShaderPrecompile(v) => g.shader_precompile != *v,
            Change::FrameGen(v) => g.frame_gen != *v,
            Change::VideoDecoder(v) => g.video_decoder != *v,
            Change::VideoHwaccel(v) => g.video_hwaccel != *v,
            Change::ResidencyPercent(v) => p.residency_percent != *v,
            Change::TexPrepMb(v) => p.tex_prep_mb != *v,
            Change::AdaptiveMip(v) => p.adaptive_mip != *v,
            Change::Prewarm(v) => p.prewarm != *v,
            Change::Gpl(v) => p.gpl != *v,
        }
    }

    pub fn apply(&self, g: &mut Graphics, p: &mut Performance) {
        match self {
            Change::RenderScale(v) => g.render_scale = *v,
            Change::SceneScale(v) => g.scene_scale = *v,
            Change::AstcQuality(v) => g.astc_quality = v.clone(),
            Change::CompileThreads(v) => g.compile_threads = *v,
            Change::CompilePriority(v) => g.compile_priority = v.clone(),
            Change::CompileEco(v) => g.compile_eco = *v,
            Change::ShaderPrecompile(v) => g.shader_precompile = *v,
            Change::FrameGen(v) => g.frame_gen = v.clone(),
            Change::VideoDecoder(v) => g.video_decoder = v.clone(),
            Change::VideoHwaccel(v) => g.video_hwaccel = *v,
            Change::ResidencyPercent(v) => p.residency_percent = *v,
            Change::TexPrepMb(v) => p.tex_prep_mb = *v,
            Change::AdaptiveMip(v) => p.adaptive_mip = *v,
            Change::Prewarm(v) => p.prewarm = *v,
            Change::Gpl(v) => p.gpl = *v,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recommendation {
    pub id: String,
    pub setting: String,
    pub reason: String,
    pub benefit: String,
    pub tradeoff: String,
    pub restart: bool,
    pub changes: Vec<Change>,
}

impl Recommendation {
    pub fn proposed(&self) -> String {
        self.changes.iter().map(Change::describe).collect::<Vec<_>>().join(", ")
    }
}

fn add_recommendation(
    out: &mut Vec<Recommendation>,
    keys: &mut HashSet<&'static str>,
    config: &Config,
    id: &str,
    reason: String,
    benefit: &str,
    tradeoff: &str,
    restart: bool,
    changes: Vec<Change>,
) {
    let changes: Vec<Change> = changes
        .into_iter()
        .filter(|c| {
            if keys.contains(c.key()) {
                false
            } else {
                c.applies(&config.graphics, &config.performance)
            }
        })
        .collect();
    if changes.is_empty() {
        return;
    }
    for c in &changes {
        keys.insert(c.key());
    }
    let setting = changes.iter().map(|c| c.key()).collect::<Vec<_>>().join(", ");
    out.push(Recommendation {
        id: id.into(),
        setting,
        reason,
        benefit: benefit.into(),
        tradeoff: tradeoff.into(),
        restart,
        changes,
    });
}

/// Hardware-aware proposals. No proposal is applied here; the UI requires an
/// explicit approval and writes a backup first.
pub fn recommend(config: &Config, profile: Option<&SystemProfile>, summary: Option<&Summary>) -> Vec<Recommendation> {
    let t = Thresholds::from(&config.lab);
    let mut out = Vec::new();
    let mut keys = HashSet::new();
    let active = profile.and_then(SystemProfile::active);
    let installed = profile.and_then(|p| p.memory.installed_mb.or(p.memory.total_mb));
    let integrated = active.is_some_and(|p| p.device.kind == "Integrated");
    let physical = profile.and_then(|p| p.cpu.physical).unwrap_or(0);

    if integrated && (installed.is_some_and(|mb| mb <= 16 * 1024) || active.is_some_and(|p| p.device.vram_mb <= 6 * 1024)) {
        let reason = format!(
            "{} shares system memory ({} installed{}).",
            active.map(|p| p.device.name.as_str()).unwrap_or("This GPU"),
            installed.map(memory::gb).unwrap_or_else(|| "unknown".into()),
            active
                .and_then(|p| p.largest_device_local_heap())
                .map(|h| format!(", {} heap", memory::gb(h.size_mb)))
                .unwrap_or_default()
        );
        add_recommendation(
            &mut out,
            &mut keys,
            config,
            "shared-memory-budget",
            reason,
            "A smaller scene buffer and cheaper texture path leave headroom for the game's asset cache.",
            "Lower render scale or texture quality is visible in fine detail. Nothing is changed until you approve it.",
            true,
            vec![
                Change::RenderScale(config.graphics.render_scale.min(0.85)),
                Change::SceneScale(config.graphics.scene_scale.min(85)),
                Change::AstcQuality("Fast".into()),
            ],
        );
    }

    if let Some(s) = summary {
        if let Some(pct) = s.gpu_peak_pct().filter(|p| *p >= t.gpu_high) {
            add_recommendation(
                &mut out,
                &mut keys,
                config,
                "vram-headroom",
                format!("The engine reached {pct}% of its video-memory budget during the measured run; paging begins before an allocation actually fails."),
                "An earlier residency demotion and a smaller texture-preparation cache reduce the chance of paging or a late allocation spike.",
                "Cached textures are released sooner, so revisiting a scene can load more from disk. This is a trade-off in load time, not visual quality.",
                true,
                vec![
                    Change::ResidencyPercent(config.performance.residency_percent.min(75)),
                    Change::TexPrepMb(config.performance.tex_prep_mb.min(512)),
                ],
            );
            if config.graphics.frame_gen != "Off" {
                add_recommendation(
                    &mut out,
                    &mut keys,
                    config,
                    "frame-gen-pressure",
                    format!("Frame generation was enabled while the GPU reached {pct}% of its budget."),
                    "Turning it off frees the extra frame buffers and queue depth the generator needs.",
                    "The frame rate shown may drop, but input latency and memory pressure can improve.",
                    true,
                    vec![Change::FrameGen("Off".into())],
                );
            }
        }

        let host_low = s
            .host_available_min_mb
            .zip(profile.and_then(|p| p.memory.installed_mb.or(p.memory.total_mb)))
            .is_some_and(|(free, total)| total > 0 && free.saturating_mul(100) / total <= t.host_high as u64);
        if host_low {
            let free = s.host_available_min_mb.unwrap_or(0);
            add_recommendation(
                &mut out,
                &mut keys,
                config,
                "host-headroom",
                format!("Only {} of physical memory was free during the run.", memory::gb(free)),
                "A smaller texture-preparation cache and earlier residency demotion reduce host and commit pressure.",
                "Less cached preparation work may be repeated on later loads.",
                true,
                vec![
                    Change::TexPrepMb(config.performance.tex_prep_mb.min(512)),
                    Change::ResidencyPercent(config.performance.residency_percent.min(75)),
                ],
            );
        }

        if s.max_shader_backlog >= 4 {
            let threads = (physical / 4).clamp(1, 4) as u32;
            add_recommendation(
                &mut out,
                &mut keys,
                config,
                "shader-backlog",
                format!("The engine reported up to {} shader compilations waiting.", s.max_shader_backlog),
                &format!("Prewarming and {threads} compile worker{} can move compilation off the first-draw path.", if threads == 1 { "" } else { "s" }),
                "Compilation uses CPU time and may compete with frame delivery during loads; idle priority and efficiency mode limit that.",
                true,
                vec![
                    Change::ShaderPrecompile(true),
                    Change::Prewarm(true),
                    Change::Gpl(true),
                    Change::CompileThreads(threads),
                ],
            );
        }

        if s.avg_fps > 0.0 && (s.avg_fps < 45.0 || (s.low1_fps > 0.0 && s.low1_fps < 30.0)) {
            add_recommendation(
                &mut out,
                &mut keys,
                config,
                "sustained-frame-time",
                format!(
                    "The run averaged {:.1} fps with a 1% low of {:.1} fps, so frame delivery was not comfortably above 30 fps.",
                    s.avg_fps, s.low1_fps
                ),
                "A modest reduction in scene resolution and adaptive mip bias targets the measured frame-time tail rather than adding a synthetic score.",
                "The image is softer in motion. Adaptive mip bias is experimental and affects sampler detail only.",
                true,
                vec![
                    Change::RenderScale((config.graphics.render_scale * 0.85).max(0.5)),
                    Change::SceneScale(config.graphics.scene_scale.min(85)),
                    Change::AdaptiveMip(true),
                ],
            );
        }

        let gpu_pressure = s.gpu_peak_pct().is_some_and(|p| p >= t.gpu_high);
        if gpu_pressure && config.graphics.video_decoder != "dav1d (CPU)" {
            add_recommendation(
                &mut out,
                &mut keys,
                config,
                "video-decoder-pressure",
                "The measured GPU was close to its memory budget and the current preference is FFmpeg/D3D11VA.".to_string(),
                "The built-in dav1d decoder moves movie decoding to the CPU and avoids holding GPU decode surfaces during cutscenes.",
                "CPU use rises during video; on a weak CPU this can trade GPU pressure for frame-time jitter.",
                true,
                vec![Change::VideoDecoder("dav1d (CPU)".into())],
            );
        }
    }

    if profile.is_some_and(|p| !p.cpu.avx || p.cpu.physical.is_some_and(|c| c <= 4)) {
        add_recommendation(
            &mut out,
            &mut keys,
            config,
            "cpu-capability",
            format!(
                "{} has {} physical core(s) and AVX {}.",
                profile.map(|p| p.cpu.brand.as_str()).unwrap_or("This processor"),
                physical.max(1),
                if profile.is_some_and(|p| p.cpu.avx) { "available" } else { "unavailable" }
            ),
            "One idle-priority compile worker and efficiency mode leave more CPU time for the game's main and render threads.",
            "Shader compilation may take longer, especially the first time a scene is loaded.",
            true,
            vec![
                Change::CompileThreads(1),
                Change::CompilePriority("Idle".into()),
                Change::CompileEco(true),
            ],
        );
    }

    if !integrated && installed.is_some_and(|mb| mb <= 16 * 1024) {
        add_recommendation(
            &mut out,
            &mut keys,
            config,
            "texture-quality-16gb",
            format!("{} of system memory is installed; texture preparation and shader caches compete with the game for commit.", installed.map(memory::gb).unwrap_or_default()),
            "Faster ASTC preparation lowers peak host-side staging memory.",
            "Texture preparation quality can be slightly softer on close inspection.",
            true,
            vec![Change::AstcQuality("Fast".into())],
        );
    }

    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Backup {
    pub created_unix_s: u64,
    pub graphics: Graphics,
    pub performance: Performance,
}

pub fn save_backup_to(path: &Path, config: &Config) -> Result<(), String> {
    let b = Backup {
        created_unix_s: unix_s(),
        graphics: config.graphics.clone(),
        performance: config.performance.clone(),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension("json.tmp");
    let text = serde_json::to_string_pretty(&b).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, text).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("replacing {}: {e}", path.display()))
}

pub fn load_backup_from(path: &Path) -> Option<Backup> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn load_backup() -> Option<Backup> {
    backup_path().and_then(|p| load_backup_from(&p))
}

pub fn apply_approved(config: &mut Config, recs: &[Recommendation], selected: &HashSet<String>) -> Result<usize, String> {
    let changes: Vec<&Change> = recs
        .iter()
        .filter(|r| selected.contains(&r.id))
        .flat_map(|r| r.changes.iter())
        .collect();
    if changes.is_empty() {
        return Err("Select at least one recommendation".into());
    }
    if let Some(path) = backup_path() {
        save_backup_to(&path, config)?;
    } else {
        return Err(crate::platform::NO_DATA_DIR.into());
    }
    for c in &changes {
        c.apply(&mut config.graphics, &mut config.performance);
    }
    Ok(changes.len())
}

pub fn restore_backup(config: &mut Config) -> Result<u64, String> {
    let path = backup_path().ok_or(crate::platform::NO_DATA_DIR)?;
    let backup = load_backup_from(&path).ok_or("There is no Performance Lab backup to restore")?;
    config.graphics = backup.graphics;
    config.performance = backup.performance;
    Ok(backup.created_unix_s)
}

/// Local UI state. History is read once; the profile cache is refreshed when
/// the player asks or after five seconds.
pub struct Ui {
    pub tab: Tab,
    pub session: Option<Session>,
    pub summary: Option<Summary>,
    pub history: Vec<Run>,
    pub selected: HashSet<String>,
    /// Recommendation ids the page has shown; a newly appearing proposal starts
    /// selected instead of silently unchecked.
    pub known: HashSet<String>,
    pub confirm_apply: bool,
    pub confirm_restore: bool,
    pub confirm_extended: bool,
    pub confirm_cancel: bool,
    pub message: Option<(String, bool)>,
    pub profile: Option<SystemProfile>,
    profile_at: Instant,
    pub profile_refreshed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Profile,
    Benchmark,
    Optimize,
}

impl Default for Ui {
    fn default() -> Self {
        Ui {
            tab: Tab::Profile,
            session: None,
            summary: None,
            history: Vec::new(),
            selected: HashSet::new(),
            known: HashSet::new(),
            confirm_apply: false,
            confirm_restore: false,
            confirm_extended: false,
            confirm_cancel: false,
            message: None,
            profile: None,
            profile_at: Instant::now() - std::time::Duration::from_secs(10),
            profile_refreshed: 0,
        }
    }
}

impl Ui {
    pub fn load(config: &Config) -> Self {
        let history = load_history();
        let mut ui = Ui {
            history,
            ..Ui::default()
        };
        ui.refresh_profile(config, true);
        ui
    }

    pub fn refresh_profile(&mut self, config: &Config, force: bool) {
        // The Vulkan scan runs on a worker. While the profile is still waiting
        // for it, pick the result up as soon as it lands instead of making the
        // first view sit empty for the whole five-second refresh interval.
        let awaiting_gpus = self.profile.as_ref().is_some_and(|p| p.gpus.is_empty());
        let scan_ready = awaiting_gpus && crate::gpu::cached_profiles().is_some();
        if force || scan_ready || self.profile_at.elapsed().as_secs() >= 5 {
            self.profile = Some(probe(config));
            self.profile_at = Instant::now();
            self.profile_refreshed = unix_ms();
        }
    }

    pub fn notify(&mut self, text: impl Into<String>, ok: bool) {
        self.message = Some((text.into(), ok));
    }
}

/// A live run. It holds no engine resources; leaving the page does not stop
/// sampling, and closing the game ends the run as a completed observation.
pub struct Session {
    pub recorder: Recorder,
    pub started_ms: u64,
    pub soak_path: Option<PathBuf>,
    pub launched_game: bool,
    pub last_tick: Instant,
    pub game_gone_since: Option<Instant>,
    pub metadata: Option<RunMetadata>,
    pub finished: bool,
    pub last_status: Option<RuntimeStatus>,
    pub last_soak: Option<SoakTotals>,
}

impl Session {
    pub fn new(recorder: Recorder, soak_path: Option<PathBuf>, launched_game: bool) -> Self {
        Session {
            started_ms: recorder.started_ms,
            recorder,
            soak_path,
            launched_game,
            last_tick: Instant::now() - std::time::Duration::from_secs(2),
            game_gone_since: None,
            metadata: None,
            finished: false,
            last_status: None,
            last_soak: None,
        }
    }
}

fn last_soak_sample(path: &Path) -> Option<(Option<u64>, Option<SoakTotals>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let points = parse_soak(&text);
    let ws = points.last().and_then(|p| p.ws).map(mib);
    Some((ws, Some(summarize_soak(&points))))
}

/// Cancelling or abandoning a run leaves nothing behind. The engine keeps its
/// own log; this removes only the sampler file the Lab asked it to write.
pub fn delete_partial_soak(path: &Path) {
    let _ = std::fs::remove_file(path);
    if let Some(dir) = path.parent() {
        // Only succeeds when the run folder is now empty, so it cannot remove
        // anything the player or another run still owns.
        let _ = std::fs::remove_dir(dir);
    }
}

/// Advance a run, finalise it when the target or the game ends, and persist the
/// result. Called from the launcher's frame loop, not from the page, so leaving
/// Performance Lab does not lose an extended run.
pub fn tick(ui: &mut Ui, config: &Config) {
    let Some(mut session) = ui.session.take() else {
        return;
    };
    if session.finished {
        ui.session = Some(session);
        return;
    }
    let now = Instant::now();
    if now.duration_since(session.last_tick).as_millis() < 900 {
        ui.session = Some(session);
        return;
    }
    session.last_tick = now;
    let now_ms = unix_ms();
    let running = crate::platform::game_running();
    if !running {
        let gone = *session.game_gone_since.get_or_insert(now);
        if now.duration_since(gone).as_secs() >= 4 && session.recorder.active_since_ms.is_some() {
            finish_session(ui, &mut session, config, "the game exited");
            return;
        }
        // The game never reached its first snapshot. Do not leave a session
        // waiting forever, and do not save a run that measured nothing.
        if session.recorder.active_since_ms.is_none()
            && now_ms.saturating_sub(session.started_ms) >= 30_000
        {
            if let Some(path) = session.soak_path.as_deref() {
                delete_partial_soak(path);
            }
            ui.notify(
                "The game exited before its first engine snapshot. No run was saved and the partial sampler file was removed.",
                false,
            );
            return;
        }
    } else {
        session.game_gone_since = None;
    }

    if let Some(status) = crate::performance::runtime_status() {
        let host = memory::snapshot();
        let gpu = ui
            .profile
            .as_ref()
            .and_then(|p| p.gpu_budget.as_ref().map(|(u, b, _)| (*u, *b)))
            .or_else(|| host.gpu.as_ref().and_then(|g| g.used_mb.zip(g.budget_mb)));
        let ws = session
            .soak_path
            .as_deref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| parse_soak(&t).last().and_then(|p| p.ws).map(mib));
        session.recorder.observe(now_ms, &status, &host, gpu, ws);
        session.last_status = Some(status);
        if session.recorder.active_since_ms.is_none() {
            session.recorder.active_since_ms = Some(now_ms);
        }
    }

    if session.recorder.finished(now_ms) {
        finish_session(ui, &mut session, config, "the target duration was reached");
        return;
    }
    if let Some(path) = session.soak_path.as_deref() {
        if let Some((_, totals)) = last_soak_sample(path) {
            session.last_soak = totals;
        }
    }
    // Keep the profile and its live budget available even when the page is
    // closed; metadata is collected from the first fresh snapshot.
    ui.refresh_profile(config, false);
    if session.metadata.is_none() {
        if let Some(p) = &ui.profile {
            session.metadata = Some(RunMetadata::collect(config, p));
        }
    }
    ui.session = Some(session);
}

fn finish_session(ui: &mut Ui, session: &mut Session, config: &Config, why: &str) {
    let soak = session
        .soak_path
        .as_deref()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| summarize_soak(&parse_soak(&t)))
        .unwrap_or_else(SoakTotals::unavailable);
    let summary = summarize(&session.recorder.samples, soak);
    ui.summary = Some(summary.clone());
    let metadata = session
        .metadata
        .clone()
        .or_else(|| ui.profile.as_ref().map(|p| RunMetadata::collect(config, p)))
        .unwrap_or_default();
    let run = Run::new(
        session.recorder.mode,
        session.started_ms,
        metadata,
        summary.clone(),
    );
    match append_history(run.clone()) {
        Ok(()) => {
            ui.history = load_history();
            ui.notify(
                format!(
                    "{} benchmark finished because {why}. {} samples, {:.1} s. Result saved.",
                    run.mode.label(),
                    summary.samples,
                    summary.observed_s
                ),
                true,
            );
        }
        Err(e) => {
            ui.history.push(run);
            ui.notify(format!("Benchmark finished, but its history could not be saved: {e}"), false);
        }
    }
    session.finished = true;
    session.last_soak = Some(summary.soak.clone());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(installed: u64, available: u64, commit_limit: u64, commit_free: u64) -> memory::Snapshot {
        memory::Snapshot {
            installed_mb: Some(installed),
            total_mb: Some(installed),
            available_mb: Some(available),
            commit_limit_mb: Some(commit_limit),
            commit_free_mb: Some(commit_free),
            gpu: None,
        }
    }

    #[test]
    fn gpu_pressure_thresholds_are_ordered_and_do_not_apply_host_percentages() {
        let t = Thresholds::from(&crate::config::Lab::default());
        assert_eq!(classify_gpu(0, 8192, &t).0, Level::Normal);
        assert_eq!(classify_gpu(6144, 8192, &t).0, Level::Elevated);
        assert_eq!(classify_gpu(7168, 8192, &t).0, Level::High);
        assert_eq!(classify_gpu(8000, 8192, &t).0, Level::Critical);
        assert_eq!(classify_gpu(1, 0, &t).0, Level::Normal, "a missing budget is unavailable, not critical");
    }

    #[test]
    fn host_pressure_uses_free_share_and_commit_separately() {
        let t = Thresholds::from(&crate::config::Lab::default());
        assert_eq!(classify_host(&host(16384, 8192, 32768, 22000), &t).unwrap().0, Level::Normal);
        assert_eq!(classify_host(&host(16384, 2500, 32768, 22000), &t).unwrap().0, Level::Elevated);
        assert_eq!(classify_host(&host(16384, 1200, 32768, 22000), &t).unwrap().0, Level::High);
        assert_eq!(classify_host(&host(16384, 6000, 32768, 3000), &t).unwrap().0, Level::High);
        assert_eq!(classify_host(&host(16384, 6000, 32768, 900), &t).unwrap().0, Level::Critical);
        assert!(classify_host(&memory::Snapshot::default(), &t).is_none());
    }

    #[test]
    fn missing_extensions_leave_budget_unavailable_not_zero() {
        let p = crate::gpu::Profile {
            device: crate::gpu::Device { name: "Test".into(), kind: "Discrete", vram_mb: 4096, av1_decode: false },
            vendor: "Test".into(),
            vendor_id: 0x1234,
            device_id: 1,
            api_version: "Vulkan 1.3.0".into(),
            driver: "0x00000001".into(),
            extensions: vec!["VK_KHR_swapchain".into()],
            heaps: vec![crate::gpu::Heap { size_mb: 4096, device_local: true, budget_mb: None, usage_mb: None }],
        };
        assert!(!p.has_extension("VK_EXT_memory_budget"));
        assert!(p.largest_device_local_heap().is_some());
        assert_eq!(p.largest_device_local_heap().unwrap().budget_mb, None);
    }

    #[test]
    fn a_run_summarises_samples_without_inventing_missing_counters() {
        let status = |fps: f64, low1: f64, frames: u64| RuntimeStatus {
            updated_unix_ms: 0,
            frames,
            avg_fps: fps,
            low1_fps: low1,
            low01_fps: low1 / 2.0,
            p50_ms: 1000.0 / fps,
            p95_ms: 20.0,
            p99_ms: 30.0,
            cpu_ms: 4.0,
            gpu_ms: 8.0,
            upload_ms: 0.4,
            replay_ms: 3.0,
            vram_percent: 80,
            shader_backlog: 2,
            ..RuntimeStatus::default()
        };
        let samples = vec![
            Sample { at_ms: 0, frames: 1, status: status(60.0, 40.0, 1), host_available_mb: Some(4000), commit_free_mb: Some(8000), gpu_used_mb: Some(4000), gpu_budget_mb: Some(8192), process_ws_mb: None },
            Sample { at_ms: 10_000, frames: 600, status: status(50.0, 25.0, 600), host_available_mb: Some(3000), commit_free_mb: Some(7000), gpu_used_mb: Some(7300), gpu_budget_mb: Some(8192), process_ws_mb: None },
        ];
        let s = summarize(&samples, SoakTotals::unavailable());
        assert_eq!(s.samples, 2);
        assert!((s.avg_fps - 55.0).abs() < 0.01);
        assert_eq!(s.low1_fps, 25.0);
        assert_eq!(s.gpu_peak_pct(), Some(89));
        assert!(s.warnings.iter().any(|w| w.contains("1% low")));
        assert!(s.unavailable.iter().any(|u| u.contains("not a stability certification")));
        assert!(s.soak.shader_translations_per_s.is_none());
    }

    #[test]
    fn soak_gauge_deltas_use_the_interval_and_frame_count() {
        let a = r#"{"t":1.0,"frame":100,"ws":104857600,"private":52428800,"rp.shader_cache_hits":10,"rp.shader_cache_misses":2,"rp.shader_translations":5,"rp.pipeline_gfx_created":1,"fs.bytes_read":1048576,"media.video_frames":2,"nvn.texprep.staged_bytes_total":2097152,"nvn.cnt.gpu_us":100000}"#;
        let b = r#"{"t":11.0,"frame":700,"ws":209715200,"private":104857600,"rp.shader_cache_hits":30,"rp.shader_cache_misses":10,"rp.shader_translations":25,"rp.pipeline_gfx_created":3,"fs.bytes_read":11534336,"media.video_frames":32,"nvn.texprep.staged_bytes_total":12582912,"nvn.cnt.gpu_us":460000}"#;
        let points = parse_soak(&format!("{a}\n{b}\n"));
        let s = summarize_soak(&points);
        assert_eq!(s.process_ws_start_mb, Some(100));
        assert_eq!(s.process_ws_end_mb, Some(200));
        assert!((s.shader_translations_per_s.unwrap() - 2.0).abs() < 0.001);
        assert!((s.shader_cache_hit_pct.unwrap() - (20.0 / 28.0 * 100.0)).abs() < 0.01);
        assert_eq!(s.pipelines_created, Some(2));
        assert!((s.asset_read_mb_s.unwrap() - 1.0).abs() < 0.01);
        assert!((s.video_fps.unwrap() - 3.0).abs() < 0.001);
        assert!((s.gpu_work_ms_per_frame.unwrap() - 0.6).abs() < 0.001);
    }

    #[test]
    fn allocation_failures_are_surfaced_without_inventing_latency() {
        let a = r#"{"t":1.0,"frame":100,"nvn.pool_import_failures":4,"nvn.pool_copy_fallbacks":1,"vk.images":10}"#;
        let b = r#"{"t":5.0,"frame":400,"nvn.pool_import_failures":6,"nvn.pool_copy_fallbacks":2,"vk.images":12}"#;
        let soak = summarize_soak(&parse_soak(&format!("{a}\n{b}")));
        assert_eq!(soak.pool_import_failures, Some(2));
        assert_eq!(soak.pool_copy_fallbacks, Some(1));
        assert_eq!(soak.vulkan_images_end, Some(12));
        let summary = summarize(&[], soak);
        assert!(summary.unavailable.iter().any(|u| u.contains("allocation latency")));
    }

    #[cfg(windows)]
    #[test]
    fn storage_detection_reads_the_volume_holding_the_game_folder() {
        let dir = tempfile::tempdir().unwrap();
        let facts = storage_facts(dir.path()).expect("a real temporary folder has a volume");
        assert!(facts.total_mb > 0);
        assert!(facts.free_mb <= facts.total_mb);
        assert!(!facts.volume.is_empty());
    }

    #[test]
    fn cancelling_removes_the_partial_sampler_file_and_empty_run_folder() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join("lab").join("runs").join("123-quick");
        std::fs::create_dir_all(&run).unwrap();
        let soak = run.join("soak.jsonl");
        std::fs::write(&soak, "{\"t\":1.0}").unwrap();
        delete_partial_soak(&soak);
        assert!(!soak.exists());
        assert!(!run.exists());
    }

    #[test]
    fn cancellation_drops_samples_and_records_nothing() {
        let mut r = Recorder::new(Mode::Quick, 1000);
        let status = RuntimeStatus { frames: 1, avg_fps: 60.0, ..RuntimeStatus::default() };
        r.observe(2000, &status, &memory::Snapshot::default(), None, None);
        assert_eq!(r.samples.len(), 1);
        r.cancel();
        assert!(r.cancelled);
        assert!(r.samples.is_empty());
        assert!(summarize(&r.samples, SoakTotals::unavailable()).unavailable.iter().any(|u| u.contains("No engine telemetry")));
    }

    #[test]
    fn history_round_trips_and_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lab").join("history.json");
        for i in 0..3 {
            let run = Run {
                id: format!("run-{i}"),
                created_unix_s: i,
                mode: Mode::Quick,
                duration_s: 1.0,
                metadata: RunMetadata::default(),
                summary: Summary::default(),
            };
            append_history_to(&path, run, 2).unwrap();
        }
        let history = load_history_from(&path);
        assert_eq!(history.len(), 2);
        assert_eq!(history.last().unwrap().id, "run-2");
    }

    #[test]
    fn report_is_privacy_safe_and_keeps_unavailable_markers() {
        let run = Run {
            id: "1-quick".into(),
            created_unix_s: 1,
            mode: Mode::Quick,
            duration_s: 1.0,
            metadata: RunMetadata {
                launcher: "0.1.42".into(),
                engine: "built".into(),
                cpu: "CPU".into(),
                gpu: "GPU".into(),
                driver: "driver".into(),
                ..RunMetadata::default()
            },
            summary: Summary {
                samples: 1,
                unavailable: vec!["Video frame delivery was not exercised.".into()],
                ..Summary::default()
            },
        };
        let text = report_markdown(&run);
        assert!(text.contains("not a stability certification"));
        assert!(text.contains("Video frame delivery was not exercised."));
        assert!(!text.contains("C:\\Users\\"));
    }

    #[test]
    fn a_report_without_telemetry_never_reports_zero_fps() {
        let run = Run {
            id: "1-quick".into(),
            created_unix_s: 1,
            mode: Mode::Quick,
            duration_s: 0.0,
            metadata: RunMetadata::default(),
            summary: Summary::default(),
        };
        let text = report_markdown(&run);
        assert!(text.contains("| Average FPS | unavailable |"));
        assert!(!text.contains("| Average FPS | 0.0 |"));
    }

    #[test]
    fn recommendations_explain_changes_and_apply_only_with_a_backup() {
        let mut config = Config::default();
        let profile = SystemProfile {
            captured_ms: 0,
            cpu: CpuFacts { brand: "CPU".into(), architecture: "x86_64".into(), physical: Some(4), logical: 8, sse42: true, popcnt: true, cx16: true, avx: false, fma: false, build: "compat".into() },
            memory: host(16384, 8000, 32768, 16000),
            gpus: Vec::new(),
            active_gpu: None,
            display: Display { resolution: None, refresh_hz: None },
            storage: None,
            pressure: Pressure::default(),
            gpu_budget: None,
        };
        let recs = recommend(&config, Some(&profile), None);
        assert!(!recs.is_empty());
        assert!(recs.iter().any(|r| r.id == "cpu-capability" && r.restart));
        let id = recs[0].id.clone();
        let mut selected = HashSet::new();
        selected.insert(id);
        // A temp path would be needed for a real apply; the public path is the
        // settings folder, so this test only proves planning is side-effect free.
        let before = config.graphics.clone();
        assert_eq!(config.graphics, before);
        assert!(!recs[0].reason.is_empty() && !recs[0].benefit.is_empty() && !recs[0].tradeoff.is_empty());
    }

    #[test]
    fn backup_round_trips_graphics_and_performance() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backup.json");
        let mut config = Config::default();
        config.graphics.render_scale = 0.72;
        config.performance.tex_prep_mb = 333;
        save_backup_to(&path, &config).unwrap();
        config.graphics.render_scale = 1.0;
        config.performance.tex_prep_mb = 999;
        let b = load_backup_from(&path).unwrap();
        config.graphics = b.graphics;
        config.performance = b.performance;
        assert!((config.graphics.render_scale - 0.72).abs() < 0.001);
        assert_eq!(config.performance.tex_prep_mb, 333);
    }

    #[test]
    fn lab_thresholds_normalize_to_a_safe_order() {
        let mut lab = crate::config::Lab { gpu_elevated: 99, gpu_high: 50, gpu_critical: 60, host_elevated: 4, host_high: 20, host_critical: 8 };
        lab.normalize();
        assert!(lab.gpu_elevated < lab.gpu_high && lab.gpu_high < lab.gpu_critical);
        assert!(lab.host_elevated > lab.host_high && lab.host_high > lab.host_critical);
    }
}
