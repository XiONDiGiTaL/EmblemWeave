// SPDX-License-Identifier: GPL-3.0-or-later
//! Linux: frame generation by LSFG-VK (https://lsfg-vk.dev), a Vulkan layer that works on the finished picture. It needs two things we
//! do not ship: the layer itself, and `lsfg-vk.dll` from the `lsfg-vk` beta branch of Lossless Scaling on Steam (the default branch only
//! has the Windows `Lossless.dll`, which the layer cannot read). This finds both and builds the environment the layer is configured
//! with (`LSFGVK_ENV=1`), so the player's own lsfg-vk config file stays untouched.

use serde::Serialize;
use std::path::{Path, PathBuf};

/// LSFG-VK's install instructions (the layer is not something the launcher can install)
pub const INSTALL_URL: &str = "https://lsfg-vk.dev/docs/installation/";

#[derive(Serialize, Clone, Default)]
pub struct Status {
    /// the Vulkan layer is installed (an lsfg-vk manifest in a vulkan/implicit_layer.d folder)
    pub layer: bool,
    /// `lsfg-vk.dll` in the Lossless Scaling folder, forward slashes
    pub dll: String,
    /// Lossless Scaling is installed but only has Lossless.dll: it is on the default branch
    pub wrong_branch: bool,
}

fn data_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(d) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        v.push(d);
    } else if let Some(h) = std::env::var_os("HOME") {
        v.push(PathBuf::from(h).join(".local/share"));
    }
    match std::env::var_os("XDG_DATA_DIRS") {
        Some(d) if !d.is_empty() => v.extend(std::env::split_paths(&d)),
        _ => v.extend(["/usr/local/share", "/usr/share"].map(PathBuf::from)),
    }
    v.push(PathBuf::from("/etc"));
    v
}

fn has_layer() -> bool {
    data_dirs().iter().any(|d| {
        std::fs::read_dir(d.join("vulkan/implicit_layer.d")).is_ok_and(|rd| {
            rd.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                n.contains("lsfg") && n.ends_with(".json")
            })
        })
    })
}

fn lossless_dirs() -> Vec<PathBuf> {
    let Some((_, libs)) = crate::play::steam_libraries() else { return vec![] };
    libs.iter().map(|l| l.join("steamapps/common/Lossless Scaling")).filter(|d| d.is_dir()).collect()
}

pub fn status() -> Status {
    if !cfg!(target_os = "linux") {
        return Status::default();
    }
    let dirs = lossless_dirs();
    let dll = dirs.iter().map(|d| d.join("lsfg-vk.dll")).find(|p| p.is_file());
    Status {
        layer: has_layer(),
        wrong_branch: dll.is_none() && dirs.iter().any(|d| d.join("Lossless.dll").is_file()),
        dll: dll.map(|p| crate::dump::fwd(&p)).unwrap_or_default(),
    }
}

/// Why LSFG-VK cannot be used now, for the player.
pub fn problem(s: &Status) -> Option<String> {
    if !s.layer {
        return Some(format!("LSFG-VK frame generation is on, but the LSFG-VK Vulkan layer is not installed (see {INSTALL_URL}). Install it or turn the setting off."));
    }
    if s.dll.is_empty() {
        return Some(if s.wrong_branch {
            "LSFG-VK frame generation is on, but Lossless Scaling is on Steam's default branch. In Steam open Lossless Scaling > Properties > Betas and pick the \"lsfg-vk\" branch, or turn the setting off."
        } else {
            "LSFG-VK frame generation is on, but Lossless Scaling was not found in your Steam libraries (it must be installed, on its \"lsfg-vk\" branch). Install it or turn the setting off."
        }
        .into());
    }
    None
}

/// The environment that starts the layer for this run. The game's own frame generation is turned off so two are not stacked.
pub fn env(dll: &Path, multiplier: u32, flow_scale: &str, performance: bool) -> Vec<(String, String)> {
    let flow = flow_scale.trim().parse::<f32>().ok().filter(|f| (0.25..=1.0).contains(f)).unwrap_or(1.0);
    vec![
        ("LSFGVK_ENV".into(), "1".into()),
        ("LSFGVK_DLL_PATH".into(), dll.to_string_lossy().into_owned()),
        ("LSFGVK_MULTIPLIER".into(), multiplier.clamp(2, 4).to_string()),
        ("LSFGVK_FLOW_SCALE".into(), format!("{flow}")),
        ("LSFGVK_PERFORMANCE_MODE".into(), if performance { "1" } else { "0" }.into()),
        ("NXRT_DLSS_FRAME_GEN".into(), "0".into()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_clamps_and_turns_the_games_frame_generation_off() {
        let e = env(Path::new("/x/lsfg-vk.dll"), 9, "5", true);
        let get = |k: &str| e.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        assert_eq!(get("LSFGVK_MULTIPLIER"), Some("4"));
        assert_eq!(get("LSFGVK_FLOW_SCALE"), Some("1"));
        assert_eq!(get("LSFGVK_PERFORMANCE_MODE"), Some("1"));
        assert_eq!(get("NXRT_DLSS_FRAME_GEN"), Some("0"));
        assert_eq!(get("LSFGVK_DLL_PATH"), Some("/x/lsfg-vk.dll"));
    }

    #[test]
    fn problems_are_named() {
        assert!(problem(&Status::default()).unwrap().contains("layer"));
        let s = Status { layer: true, dll: String::new(), wrong_branch: true };
        assert!(problem(&s).unwrap().contains("Betas"));
        assert!(problem(&Status { layer: true, dll: "/a".into(), wrong_branch: false }).is_none());
    }
}
