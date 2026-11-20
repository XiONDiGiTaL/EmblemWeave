//! The performance policy written to `fefw.ini`, and the supported ReShade
//! style bridge.
//!
//! The launcher does not reimplement the engine's optimizations. It maps its
//! controls onto keys the runtime already reads, so the same value is visible
//! in the F1 menu and in `runtime/docs/CONFIG.md`. `values()` is the single
//! mapping; `engine::write_settings` merges it with the graphics and audio
//! mappings before it touches the file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::config::Performance;

/// `key` -> value, using the dotted `section.key` names the engine writer
/// understands. Only keys the runtime actually reads are emitted.
pub fn values(p: &Performance) -> BTreeMap<String, String> {
    let mut d = BTreeMap::new();
    let b = |v: bool| if v { "1" } else { "0" };
    d.insert("perf.enabled".into(), b(p.controller).into());
    d.insert("perf.mode".into(), p.mode.to_ascii_lowercase());
    d.insert("perf.target_fps".into(), p.target_fps.to_string());
    d.insert("perf.stability".into(), b(p.stability).into());
    d.insert("perf.scene_profiles".into(), b(p.scene_profiles).into());
    d.insert("perf.scene_override".into(), p.scene_override.to_ascii_lowercase());
    d.insert("perf.adaptive_mip".into(), b(p.adaptive_mip).into());
    d.insert("perf.max_mip_bias".into(), format!("{:.2}", p.max_mip_bias));
    d.insert("nvn.staging_ring_mb".into(), p.staging_ring_mb.to_string());
    d.insert("nvn.pool_upload_ms".into(), p.pool_upload_ms.to_string());
    d.insert("nvn.tex_prep".into(), b(p.tex_prep).into());
    d.insert("nvn.tex_prep_threads".into(), p.tex_prep_threads.to_string());
    d.insert("nvn.tex_prep_mb".into(), p.tex_prep_mb.to_string());
    d.insert(
        "gfx.residency".into(),
        match p.residency.as_str() {
            "On" => "on",
            "Off" => "off",
            _ => "auto",
        }
        .into(),
    );
    d.insert("gfx.residency_percent".into(), p.residency_percent.to_string());
    d.insert("nvn.lean_draws".into(), b(p.lean_draws).into());
    d.insert("nvn.prewarm".into(), b(p.prewarm).into());
    d.insert("nvn.gpl".into(), b(p.gpl).into());
    d
}

/// `ReShade.ini` beside the game executable, when ReShade is installed there.
pub fn reshade_ini() -> Option<PathBuf> {
    let path = crate::engine::game_folder()?.join("ReShade.ini");
    path.is_file().then_some(path)
}

/// Writes the XiØN palette into ReShade's documented custom style
/// (`[STYLE] StyleIndex=4`, the `Col*` ImGui colour keys). ReShade's source
/// reads these keys for that style; no binary is touched and nothing is
/// injected. A one-time `.xion-backup` copy keeps the previous style
/// recoverable.
pub fn apply_reshade_theme() -> Result<String, String> {
    let path = reshade_ini().ok_or_else(|| {
        "ReShade.ini was not found next to the game executable; install ReShade for this game first".to_string()
    })?;
    let original = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let backup = path.with_extension("ini.xion-backup");
    if !backup.exists() {
        std::fs::write(&backup, &original).map_err(|e| format!("write {}: {e}", backup.display()))?;
    }

    let text = [0.925, 0.929, 0.937, 1.0]; // #ECEDEF
    let muted = [0.545, 0.565, 0.600, 1.0]; // #8B9099
    let bg = [0.020, 0.024, 0.027, 1.0]; // #050607
    let panel = [0.039, 0.043, 0.051, 1.0]; // #0A0B0D
    let surface = [0.047, 0.055, 0.067, 1.0]; // #0C0E11
    let surface_hi = [0.063, 0.075, 0.094, 1.0]; // #101318
    let field = [0.090, 0.106, 0.133, 1.0]; // #171B22
    let field_hover = [0.125, 0.149, 0.192, 1.0]; // #202631
    let field_active = [0.161, 0.204, 0.275, 1.0]; // #293446
    let line = [1.0, 1.0, 1.0, 0.07];
    let line_strong = [1.0, 1.0, 1.0, 0.14];
    let accent = [0.247, 0.502, 1.0, 1.0]; // #3F80FF
    let accent_hi = [0.475, 0.659, 1.0, 1.0]; // #79A8FF
    let accent_deep = [0.102, 0.239, 0.510, 1.0]; // #1A3D82
    let accent_soft = [0.247, 0.502, 1.0, 0.12];
    let accent_mid = [0.247, 0.502, 1.0, 0.20];
    let accent_strong = [0.247, 0.502, 1.0, 0.30];
    let black_55 = [0.0, 0.0, 0.0, 0.55];

    let colors: &[(&str, [f32; 4])] = &[
        ("ColText", text),
        ("ColTextDisabled", muted),
        ("ColWindowBg", [bg[0], bg[1], bg[2], 0.94]),
        ("ColChildBg", surface),
        ("ColPopupBg", [surface[0], surface[1], surface[2], 0.98]),
        ("ColBorder", line),
        ("ColBorderShadow", [0.0, 0.0, 0.0, 0.0]),
        ("ColFrameBg", field),
        ("ColFrameBgHovered", field_hover),
        ("ColFrameBgActive", field_active),
        ("ColTitleBg", panel),
        ("ColTitleBgActive", surface_hi),
        ("ColTitleBgCollapsed", bg),
        ("ColMenuBarBg", panel),
        ("ColScrollbarBg", [0.027, 0.031, 0.039, 1.0]),
        ("ColScrollbarGrab", field_active),
        ("ColScrollbarGrabHovered", accent_deep),
        ("ColScrollbarGrabActive", accent),
        ("ColCheckMark", accent_hi),
        ("ColSliderGrab", accent),
        ("ColSliderGrabActive", accent_hi),
        ("ColButton", field),
        ("ColButtonHovered", field_hover),
        ("ColButtonActive", accent_deep),
        ("ColHeader", accent_soft),
        ("ColHeaderHovered", accent_mid),
        ("ColHeaderActive", accent_strong),
        ("ColSeparator", line),
        ("ColSeparatorHovered", accent_hi),
        ("ColSeparatorActive", accent),
        ("ColResizeGrip", accent_soft),
        ("ColResizeGripHovered", accent_hi),
        ("ColResizeGripActive", accent),
        ("ColTab", [1.0, 1.0, 1.0, 0.0]),
        ("ColTabHovered", accent_mid),
        ("ColTabActive", [0.247, 0.502, 1.0, 0.16]),
        ("ColTabUnfocused", [1.0, 1.0, 1.0, 0.0]),
        ("ColTabUnfocusedActive", [0.247, 0.502, 1.0, 0.08]),
        ("ColPlotLines", accent_hi),
        ("ColPlotLinesHovered", text),
        ("ColPlotHistogram", accent),
        ("ColPlotHistogramHovered", accent_hi),
        ("ColTableHeaderBg", surface_hi),
        ("ColTableBorderStrong", line_strong),
        ("ColTableBorderLight", line),
        ("ColTableRowBg", [0.0, 0.0, 0.0, 0.0]),
        ("ColTableRowBgAlt", [1.0, 1.0, 1.0, 0.025]),
        ("ColTextSelectedBg", [0.247, 0.502, 1.0, 0.45]),
        ("ColDragDropTarget", accent_hi),
        ("ColNavHighlight", accent_hi),
        ("ColNavWindowingHighlight", [1.0, 1.0, 1.0, 0.70]),
        ("ColModalWindowDimBg", black_55),
    ];
    let mut out = original;
    out = crate::engine::set_ini_key(&out, "STYLE", "StyleIndex", "4");
    out = crate::engine::set_ini_key(&out, "STYLE", "Alpha", "1.000000");
    out = crate::engine::set_ini_key(&out, "STYLE", "ColFPSText", "0.475000,0.659000,1.000000,1.000000");
    for (key, c) in colors {
        let value = format!("{:.6},{:.6},{:.6},{:.6}", c[0], c[1], c[2], c[3]);
        out = crate::engine::set_ini_key(&out, "STYLE", key, &value);
    }
    for (key, value) in [
        ("WindowRounding", "14"),
        ("ChildRounding", "10"),
        ("FrameRounding", "8"),
        ("PopupRounding", "10"),
        ("ScrollbarRounding", "8"),
        ("GrabRounding", "8"),
        ("TabRounding", "8"),
        ("BorderSize", "1"),
        ("FrameBorderSize", "1"),
        ("WindowBorderSize", "1"),
    ] {
        out = crate::engine::set_ini_key(&out, "STYLE", key, value);
    }
    if std::path::Path::new("C:\\Windows\\Fonts\\segoeui.ttf").is_file() {
        out = crate::engine::set_ini_key(&out, "STYLE", "Font", "C:\\Windows\\Fonts\\segoeui.ttf");
        out = crate::engine::set_ini_key(&out, "STYLE", "FontSize", "16.000000");
    }
    std::fs::write(&path, out).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(format!(
        "XiØN style written to {} (previous style saved as {})",
        path.display(),
        backup.display()
    ))
}

/// The runtime's one-second controller snapshot. Missing means the game is not
/// running (or has not presented a frame since this build); stale means the same.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RuntimeStatus {
    pub updated_unix_ms: u64,
    pub frames: u64,
    pub enabled: bool,
    pub mode: String,
    pub scene: String,
    pub bottleneck: String,
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
    pub vram_percent: i32,
    pub shader_backlog: i32,
    pub target_fps: u32,
    pub mip_bias: f32,
    pub adjustments: u64,
    pub spikes: u64,
    pub action: String,
    pub profile_reason: String,
}

pub fn runtime_status() -> Option<RuntimeStatus> {
    let path = crate::engine::run_dir()?.join("perf_status.json");
    let text = std::fs::read_to_string(path).ok()?;
    let status: RuntimeStatus = serde_json::from_str(&text).ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    (now.saturating_sub(status.updated_unix_ms) < 5_000).then_some(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapping_uses_the_keys_the_runtime_reads() {
        let p = Performance::default();
        let d = values(&p);
        assert_eq!(d.get("perf.enabled").map(String::as_str), Some("1"));
        assert_eq!(d.get("perf.mode").map(String::as_str), Some("automatic"));
        assert_eq!(d.get("perf.target_fps").map(String::as_str), Some("60"));
        assert_eq!(d.get("nvn.staging_ring_mb").map(String::as_str), Some("256"));
        assert_eq!(d.get("nvn.pool_upload_ms").map(String::as_str), Some("6"));
        assert_eq!(d.get("nvn.tex_prep").map(String::as_str), Some("1"));
        assert_eq!(d.get("gfx.residency").map(String::as_str), Some("auto"));
        assert_eq!(d.get("gfx.residency_percent").map(String::as_str), Some("80"));
        assert!(!d.contains_key("nvn.gpl_threads"), "Graphics owns the shared shader worker cap");
    }

    #[test]
    fn labels_normalize_to_engine_words() {
        let mut p = Performance::default();
        p.mode = "aggressive".into();
        p.scene_override = "Crowded".into();
        p.residency = "Off".into();
        p.normalize();
        let d = values(&p);
        assert_eq!(d.get("perf.mode").map(String::as_str), Some("aggressive"));
        assert_eq!(d.get("perf.scene_override").map(String::as_str), Some("crowded"));
        assert_eq!(d.get("gfx.residency").map(String::as_str), Some("off"));
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let mut p = Performance::default();
        p.target_fps = 5;
        p.max_mip_bias = 9.0;
        p.tex_prep_mb = 1;
        p.residency_percent = 1;
        p.mode = "nonsense".into();
        p.normalize();
        assert_eq!(p.target_fps, 24);
        assert_eq!(p.max_mip_bias, 2.0);
        assert_eq!(p.tex_prep_mb, 128);
        assert_eq!(p.residency_percent, 50);
        assert_eq!(p.mode, "Automatic");
    }
}
