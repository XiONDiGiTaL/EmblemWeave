// SPDX-License-Identifier: GPL-3.0-or-later
//! The graphics adapters in this PC, by PCI vendor. Settings uses it to show the NVIDIA-only options (DLSS upscaling and frame
//! generation, the Streamline folder) only where they can work.
//!   Windows  Win32_VideoController (PNPDeviceID `PCI\VEN_10DE&DEV_...`) through PowerShell, which Set up already relies on.
//!   Linux    /sys/bus/pci/devices/*: display controllers (class 0x03xxxx) and their `vendor` file.
//! The probe runs once, on a thread started with the launcher, and is kept: the Settings page does not wait for PowerShell.
//! A probe that finds nothing reports `known: false`, and the page then shows every option rather than hide one that may work.
//! FEFW_GPU_VENDOR=nvidia,amd,intel stands in for the probe (tests, or a PC the probe gets wrong).

use serde::Serialize;
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Default)]
pub struct Gpus {
    /// at least one adapter was found (or FEFW_GPU_VENDOR was set)
    pub known: bool,
    pub nvidia: bool,
    pub amd: bool,
    pub intel: bool,
}

const NVIDIA: u32 = 0x10de;
const AMD: u32 = 0x1002;
const INTEL: u32 = 0x8086;

fn from_vendors(ids: &[u32]) -> Gpus {
    Gpus {
        known: !ids.is_empty(),
        nvidia: ids.contains(&NVIDIA),
        amd: ids.contains(&AMD),
        intel: ids.contains(&INTEL),
    }
}

fn from_env(v: &str) -> Gpus {
    let names: Vec<String> = v.split(',').map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect();
    let has = |n: &str| names.iter().any(|x| x == n);
    Gpus { known: true, nvidia: has("nvidia"), amd: has("amd"), intel: has("intel") }
}

/// `PCI\VEN_10DE&DEV_2684&SUBSYS_...` -> 0x10de
#[cfg(any(windows, test))]
fn vendor_of_pnp(id: &str) -> Option<u32> {
    let up = id.to_ascii_uppercase();
    let at = up.find("VEN_")? + 4;
    u32::from_str_radix(up.get(at..at + 4)?, 16).ok()
}

#[cfg(windows)]
fn probe() -> Vec<u32> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new(crate::setup::powershell())
        .args(["-NoProfile", "-NonInteractive", "-Command", "Get-CimInstance Win32_VideoController | ForEach-Object { $_.PNPDeviceID }"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).lines().filter_map(vendor_of_pnp).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(not(windows))]
fn probe() -> Vec<u32> {
    let Ok(dir) = std::fs::read_dir("/sys/bus/pci/devices") else { return Vec::new() };
    let hex = |p: std::path::PathBuf| -> Option<u32> {
        let s = std::fs::read_to_string(p).ok()?;
        u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).ok()
    };
    dir.flatten()
        .filter(|d| hex(d.path().join("class")).is_some_and(|c| c >> 16 == 0x03))
        .filter_map(|d| hex(d.path().join("vendor")))
        .collect()
}

static GPUS: OnceLock<Gpus> = OnceLock::new();

/// The adapters, probed on the first call; a call made while the probe runs waits for it.
pub fn get() -> Gpus {
    GPUS.get_or_init(|| match std::env::var("FEFW_GPU_VENDOR") {
        Ok(v) if !v.trim().is_empty() => from_env(&v),
        _ => from_vendors(&probe()),
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pnp_ids() {
        assert_eq!(vendor_of_pnp(r"PCI\VEN_10DE&DEV_2684&SUBSYS_16F110DE&REV_A1\4&1"), Some(NVIDIA));
        assert_eq!(vendor_of_pnp(r"pci\ven_1002&dev_744c"), Some(AMD));
        assert_eq!(vendor_of_pnp(r"ROOT\BasicDisplay\0000"), None);
    }

    #[test]
    fn vendors() {
        let g = from_vendors(&[INTEL, NVIDIA]);
        assert!(g.known && g.nvidia && g.intel && !g.amd);
        assert!(!from_vendors(&[]).known);
        let e = from_env(" NVIDIA , amd");
        assert!(e.known && e.nvidia && e.amd && !e.intel);
    }
}
