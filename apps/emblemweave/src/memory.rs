//! Memory facts and the Low Memory graphics profile.
//!
//! The facts come from Windows (installed and available physical memory, the
//! commit limit) and from the engine's last `fefw.log` (the GPU it picked,
//! whether it is integrated and sharing system memory, its device-local heap
//! and the OS video-memory budget). A log line is evidence, not a guarantee:
//! when the GPU is not in the log, the profile still works but the shared-memory
//! recommendation is withheld rather than guessed.
//!
//! The profile only changes settings the launcher already writes to the
//! engine's own ini files. It does not invent keys, and it never touches audio,
//! controls, saves, mods or the game folder. The previous graphics settings are
//! kept beside it so one restore puts them back.

use crate::config::Graphics;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Gpu {
    pub name: String,
    pub shared: Option<bool>,
    pub heap_mb: Option<u64>,
    pub budget_mb: Option<u64>,
    pub used_mb: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub installed_mb: Option<u64>,
    pub total_mb: Option<u64>,
    pub available_mb: Option<u64>,
    pub commit_limit_mb: Option<u64>,
    pub commit_free_mb: Option<u64>,
    pub gpu: Option<Gpu>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Assessment {
    /// At least one memory fact was available.
    pub known: bool,
    /// The hardware is the shared-memory class where the profile is the
    /// conservative choice: a 16 GB (or smaller) system whose GPU shares that
    /// memory, or a video-memory budget of 12 GB or less on such a GPU.
    pub recommend_low_memory: bool,
    /// Windows or the GPU looks constrained right now. This is a warning, not a
    /// diagnosis: it never claims a crash was caused by memory.
    pub pressure: bool,
    pub reasons: Vec<String>,
}

fn mib(bytes: u64) -> u64 {
    bytes / (1024 * 1024)
}

#[cfg(windows)]
fn platform_memory() -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
    use windows_sys::Win32::System::SystemInformation::{
        GetPhysicallyInstalledSystemMemory, GlobalMemoryStatusEx, MEMORYSTATUSEX,
    };
    unsafe {
        let mut status: MEMORYSTATUSEX = std::mem::zeroed();
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        let (total, available, commit_limit, commit_free) = if GlobalMemoryStatusEx(&mut status) != 0 {
            (
                Some(mib(status.ullTotalPhys)),
                Some(mib(status.ullAvailPhys)),
                (status.ullTotalPageFile > 0).then(|| mib(status.ullTotalPageFile)),
                (status.ullTotalPageFile > 0).then(|| mib(status.ullAvailPageFile)),
            )
        } else {
            (None, None, None, None)
        };
        let mut kb: u64 = 0;
        let installed = (GetPhysicallyInstalledSystemMemory(&mut kb) != 0 && kb > 0).then(|| kb / 1024);
        (installed, total, available, commit_limit, commit_free)
    }
}

#[cfg(not(windows))]
fn platform_memory() -> (Option<u64>, Option<u64>, Option<u64>, Option<u64>, Option<u64>) {
    (None, None, None, None, None)
}

fn number_before(text: &str, unit: &str) -> Option<f64> {
    text.split(unit).next()?.trim().split_whitespace().last()?.parse().ok()
}

/// The GPU facts the engine writes into `fefw.log`. Later lines win: the memory
/// report is written every 30 s, and its budget is the newest one.
pub fn gpu_from_log(text: &str) -> Option<Gpu> {
    let mut gpu = Gpu::default();
    let mut seen = false;
    for line in text.lines() {
        if gpu.name.is_empty() {
            if let Some((_, rest)) = line.split_once("using GPU ") {
                let rest = rest.split_once(": ").map(|(_, r)| r).unwrap_or(rest);
                let name = rest.split(" (").next().unwrap_or(rest).trim();
                if !name.is_empty() {
                    gpu.name = name.to_string();
                    seen = true;
                }
            } else if let Some((_, rest)) = line.split_once("NVN back end up on ") {
                let name = rest.split(" (").next().unwrap_or(rest).trim();
                if !name.is_empty() {
                    gpu.name = name.to_string();
                    seen = true;
                }
            }
        }
        if line.contains("integrated GPU") {
            gpu.shared = Some(true);
            seen = true;
        } else if line.contains("discrete GPU") {
            gpu.shared = Some(false);
            seen = true;
        }
        if let Some(rest) = line.split("video memory heap ").nth(1) {
            if let Some(gib) = number_before(rest, " GiB") {
                gpu.heap_mb = Some((gib * 1024.0).round() as u64);
                seen = true;
            }
        }
        if let Some(rest) = line.split("largest device-local heap ").nth(1) {
            if let Some(mb) = number_before(rest, " MiB") {
                gpu.heap_mb = Some(mb.round() as u64);
                seen = true;
            }
        }
        if let Some(rest) = line.split("video memory: ").nth(1) {
            if let Some((used, tail)) = rest.split_once(" GB in use of a ") {
                if let (Ok(used), Some(budget)) = (used.trim().parse::<f64>(), number_before(tail, " GB budget")) {
                    gpu.used_mb = Some((used * 1024.0).round() as u64);
                    gpu.budget_mb = Some((budget * 1024.0).round() as u64);
                    seen = true;
                }
            }
        }
    }
    seen.then_some(gpu)
}

fn read_log() -> Option<String> {
    let dir = crate::engine::run_dir()?;
    std::fs::read_to_string(dir.join("fefw.log")).ok()
}

/// The facts, cached for five seconds so a page can ask every frame without
/// reading the log or calling Windows each time.
pub fn snapshot() -> Snapshot {
    static CACHE: Mutex<Option<(Instant, Snapshot)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, snap)) = cache.as_ref() {
        if at.elapsed() < Duration::from_secs(5) {
            return snap.clone();
        }
    }
    let (installed_mb, total_mb, available_mb, commit_limit_mb, commit_free_mb) = platform_memory();
    let snap = Snapshot {
        installed_mb,
        total_mb,
        available_mb,
        commit_limit_mb,
        commit_free_mb,
        gpu: read_log().as_deref().and_then(gpu_from_log),
    };
    *cache = Some((Instant::now(), snap.clone()));
    snap
}

/// The selection is deterministic and based only on the facts passed in. A
/// 16 GB shared-memory handheld gets the recommendation; a 24 GB one does not
/// just because it is a handheld. A discrete card with 32 GB of RAM and room
/// does not get a warning. Unknown facts never become a warning.
pub fn assess(s: &Snapshot) -> Assessment {
    let mut a = Assessment { known: s.installed_mb.is_some() || s.total_mb.is_some() || s.gpu.is_some(), ..Assessment::default() };
    let installed = s.installed_mb.or(s.total_mb);
    let shared = s.gpu.as_ref().and_then(|g| g.shared);
    if shared == Some(true) {
        if let Some(mb) = installed.filter(|mb| *mb <= 16 * 1024) {
            a.recommend_low_memory = true;
            a.reasons.push(format!("{} of installed memory is shared with the GPU", gb(mb)));
        }
        if let Some(budget) = s.gpu.as_ref().and_then(|g| g.budget_mb).filter(|mb| *mb <= 12 * 1024) {
            a.recommend_low_memory = true;
            a.reasons.push(format!("the OS video-memory budget is {}", gb(budget)));
        }
    }
    if let Some(free) = s.available_mb.filter(|mb| *mb < 2048) {
        a.pressure = true;
        a.reasons.push(format!("only {} of physical memory is free now", gb(free)));
    }
    if let Some(free) = s.commit_free_mb.filter(|mb| *mb < 2048) {
        a.pressure = true;
        a.reasons.push(format!("only {} of commit is free now", gb(free)));
    }
    if let Some(g) = &s.gpu {
        if let (Some(used), Some(budget)) = (g.used_mb, g.budget_mb.filter(|b| *b > 0)) {
            if used.saturating_mul(100) >= budget.saturating_mul(90) {
                a.pressure = true;
                a.reasons.push(format!("the last run used {} of its {} video-memory budget", gb(used), gb(budget)));
            }
        }
    }
    a
}

pub fn gb(mb: u64) -> String {
    if mb >= 1024 {
        format!("{:.1} GB", mb as f64 / 1024.0)
    } else {
        format!("{mb} MB")
    }
}

/// The settings the profile writes. Every one is an existing Graphics value
/// with a matching engine key; nothing outside these fields is touched.
pub fn low_memory(g: &Graphics) -> Graphics {
    let mut n = g.clone();
    n.resolution = "1280 × 720".into();
    n.render_scale = g.render_scale.min(0.75);
    n.scene_scale = g.scene_scale.min(75);
    if n.upscaling == "Off" {
        // FSR 1 is spatial, any GPU, and needs no vendor DLL that might be
        // absent; it fills the smaller picture without a vendor file.
        n.upscaling = "FSR 1.0".into();
    }
    if matches!(n.upscaling.as_str(), "FSR 4.1.2" | "FSR 3.1" | "DLSS" | "DLAA" | "XeSS") {
        n.upscale_quality = "Performance".into();
    }
    n.frame_gen = "Off".into();
    n.astc_quality = "Fast".into();
    n.compile_threads = 1;
    n.compile_priority = "Idle".into();
    n.compile_eco = true;
    n
}

/// The values that differ, in the words the page uses. A list rather than a
/// claim: only settings that actually change are named.
pub fn changes(before: &Graphics, after: &Graphics) -> Vec<String> {
    let mut out = Vec::new();
    if before.resolution != after.resolution {
        out.push(format!("Output: {} (was {})", after.resolution, before.resolution));
    }
    if (before.render_scale - after.render_scale).abs() > 0.001 {
        out.push(format!("Render scale: {:.2}× (was {:.2}×)", after.render_scale, before.render_scale));
    }
    if before.scene_scale != after.scene_scale {
        out.push(format!("Scene scale: {} % (was {} %)", after.scene_scale, before.scene_scale));
    }
    if before.upscaling != after.upscaling {
        out.push(format!("Upscaling: {} (was {})", after.upscaling, before.upscaling));
    }
    if before.upscale_quality != after.upscale_quality {
        out.push(format!("Upscaler quality: {} (was {})", after.upscale_quality, before.upscale_quality));
    }
    if before.frame_gen != after.frame_gen {
        out.push(format!("Frame generation: {} (was {})", after.frame_gen, before.frame_gen));
    }
    if before.astc_quality != after.astc_quality {
        out.push(format!("Texture quality: {} (was {})", after.astc_quality, before.astc_quality));
    }
    if before.compile_threads != after.compile_threads || before.compile_priority != after.compile_priority || before.compile_eco != after.compile_eco {
        out.push(format!(
            "Shader compilation: {} thread{}, {} priority, efficiency mode {}",
            after.compile_threads,
            if after.compile_threads == 1 { "" } else { "s" },
            after.compile_priority.to_lowercase(),
            if after.compile_eco { "on" } else { "off" }
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared(budget_mb: u64, installed_mb: u64) -> Snapshot {
        Snapshot {
            installed_mb: Some(installed_mb),
            total_mb: Some(installed_mb - 1024),
            available_mb: Some(installed_mb / 2),
            commit_limit_mb: Some(installed_mb * 2),
            commit_free_mb: Some(installed_mb / 2),
            gpu: Some(Gpu { name: "Test iGPU".into(), shared: Some(true), heap_mb: Some(11264), budget_mb: Some(budget_mb), used_mb: Some(budget_mb / 2) }),
        }
    }

    #[test]
    fn sixteen_gb_shared_memory_is_flagged() {
        let a = assess(&shared(10900, 16384));
        assert!(a.known && a.recommend_low_memory, "{a:?}");
        assert!(!a.pressure);
    }

    #[test]
    fn twenty_four_gb_shared_memory_is_not_flagged() {
        let a = assess(&shared(16384, 24576));
        assert!(a.known && !a.recommend_low_memory, "{a:?}");
        assert!(!a.pressure);
    }

    #[test]
    fn constrained_memory_warns_without_calling_it_low_memory_hardware() {
        let s = Snapshot {
            installed_mb: Some(32768),
            total_mb: Some(32768),
            available_mb: Some(1400),
            commit_limit_mb: Some(65536),
            commit_free_mb: Some(1200),
            gpu: Some(Gpu { name: "Discrete".into(), shared: Some(false), heap_mb: Some(20480), budget_mb: Some(19000), used_mb: Some(5000) }),
        };
        let a = assess(&s);
        assert!(a.pressure && !a.recommend_low_memory, "{a:?}");
        assert_eq!(a.reasons.len(), 2);
    }

    #[test]
    fn a_normal_discrete_system_gets_no_warning() {
        let s = Snapshot {
            installed_mb: Some(32768),
            total_mb: Some(32768),
            available_mb: Some(12000),
            commit_limit_mb: Some(65536),
            commit_free_mb: Some(30000),
            gpu: Some(Gpu { name: "Discrete".into(), shared: Some(false), heap_mb: Some(20480), budget_mb: Some(19000), used_mb: Some(5000) }),
        };
        let a = assess(&s);
        assert!(!a.recommend_low_memory && !a.pressure, "{a:?}");
    }

    #[test]
    fn a_large_shared_memory_system_gets_no_warning() {
        let s = shared(16000, 32768);
        let a = assess(&s);
        assert!(a.known && !a.recommend_low_memory && !a.pressure, "{a:?}");
    }

    #[test]
    fn unknown_memory_is_not_a_warning() {
        let a = assess(&Snapshot::default());
        assert!(!a.known && !a.recommend_low_memory && !a.pressure, "{a:?}");
        assert!(a.reasons.is_empty());
    }

    #[test]
    fn the_profile_preserves_other_sections_and_restores_graphics() {
        let mut c = crate::config::Config::default();
        let before = c.graphics.clone();
        c.audio.master = 0.42;
        c.theme = crate::theme::Scheme::Frost;
        c.input.deadzone = 0.31;
        let changes = c.apply_low_memory();
        assert!(!changes.is_empty(), "{changes:?}");
        assert_eq!(c.audio.master, 0.42);
        assert_eq!(c.theme, crate::theme::Scheme::Frost);
        assert_eq!(c.input.deadzone, 0.31);
        assert!(c.graphics_preset.low_memory);
        assert!(!c.low_memory_drifted());
        assert_eq!(c.graphics.resolution, "1280 × 720");
        assert_eq!(c.graphics.compile_threads, 1);
        assert_eq!(c.graphics.astc_quality, "Fast");
        assert!(c.restore_low_memory());
        assert_eq!(c.graphics, before);
        assert!(!c.graphics_preset.low_memory);
        assert!(!c.restore_low_memory(), "nothing left to restore");
    }

    #[test]
    fn dismissing_the_recommendation_changes_nothing_else() {
        let mut c = crate::config::Config::default();
        let graphics = c.graphics.clone();
        c.audio.master = 0.33;
        c.theme = crate::theme::Scheme::Frost;
        c.dismiss_low_memory();
        assert!(c.graphics_preset.dismissed);
        assert_eq!(c.graphics, graphics);
        assert_eq!(c.audio.master, 0.33);
        assert_eq!(c.theme, crate::theme::Scheme::Frost);
    }

    #[test]
    fn applying_twice_keeps_the_original_snapshot() {
        let mut c = crate::config::Config::default();
        let before = c.graphics.clone();
        c.apply_low_memory();
        c.graphics.vsync = !c.graphics.vsync;
        c.apply_low_memory();
        assert!(c.restore_low_memory());
        assert_eq!(c.graphics, before);
    }

    #[test]
    fn the_log_reader_takes_the_gpu_and_budget() {
        let text = "\
[3.664 I] nvn.tex: ASTC storage: BC1 for opaque textures ([gfx] astc_bc = auto; integrated GPU, video memory heap 11.2 GiB)
[30.4 I] memory: system: RAM 1.2 GB free of 13.6 GB; video memory: 5.17 GB in use of a 10.65 GB budget (driver figures, heap 11.2 GB)
";
        let g = gpu_from_log(text).unwrap();
        assert_eq!(g.shared, Some(true));
        assert_eq!(g.heap_mb, Some(11469));
        assert_eq!(g.used_mb, Some(5294));
        assert_eq!(g.budget_mb, Some(10906));
    }
}
