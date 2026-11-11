//! `ew-runtime` — the native runtime's host service layer.
//!
//! The runtime has two halves:
//!
//! * **Guest execution** — lifted C (see `ew-lift`) executing guest instructions,
//!   with a dispatcher that resolves `pc` to a lifted function.
//! * **Host services** — this crate: the guest memory bus, module layout, and the
//!   HLE registry for Nintendo SDK services.
//!
//! Memory and HLE live here because they are the parts that must be *correct*
//! before any of the game can run, and because they can be tested without a
//! display, an audio device, or the game itself.
//!
//! Everything unimplemented is recorded as [`Blocked`] and returns an explicit
//! failure to the guest. Nothing here pretends a service works.

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;

pub mod hle;
pub mod image;

pub use hle::{HleRegistry, HleResult};
pub use image::{GuestMemory, ModuleLayout};

/// Why a host service cannot be provided yet.
#[derive(Debug, Clone, Serialize)]
pub struct Blocked {
    pub service: String,
    pub reason: String,
}

impl Blocked {
    pub fn new(service: &str, reason: &str) -> Self {
        Blocked { service: service.to_string(), reason: reason.to_string() }
    }
}

/// Overall runtime readiness, reported by the launcher and diagnostics.
#[derive(Debug, Clone, Serialize, Default)]
pub struct Readiness {
    /// Modules whose code has been lifted and can be dispatched.
    pub liftable_modules: Vec<String>,
    /// Modules that cannot be lifted yet.
    pub blocked_modules: Vec<String>,
    pub blocked_services: Vec<Blocked>,
    /// True only when a complete, verified boot path exists.
    pub bootable: bool,
}

impl Readiness {
    pub fn describe(&self) -> String {
        if self.bootable {
            "bootable".to_string()
        } else {
            format!(
                "not bootable ({} blocked module(s), {} blocked service(s))",
                self.blocked_modules.len(),
                self.blocked_services.len()
            )
        }
    }
}

/// Load an ExeFS directory into a set of guest module layouts.
pub fn plan_modules(exefs: &std::path::Path) -> Result<Vec<ModuleLayout>> {
    let scan = ew_format::ExeFs::scan_dir(exefs)
        .with_context(|| format!("scanning {}", exefs.display()))?;
    let mut out = Vec::new();
    for m in &scan.modules {
        out.push(ModuleLayout {
            name: m.file_name.clone(),
            module_name: m.module_name.clone(),
            text: m.text_size as u64,
            rodata: m.rodata_size as u64,
            data: m.data_size as u64,
            bss: m.bss_size as u64,
            liftable: !m.is_zbic(),
        });
    }
    if out.is_empty() {
        bail!("no NSO modules found in {}", exefs.display());
    }
    Ok(out)
}

/// Summarise what can and cannot run, given a module plan.
pub fn readiness(modules: &[ModuleLayout], hle: &HleRegistry) -> Readiness {
    let mut r = Readiness::default();
    for m in modules {
        if m.liftable {
            r.liftable_modules.push(m.name.clone());
        } else {
            r.blocked_modules.push(m.name.clone());
        }
    }
    for (name, status) in hle.statuses() {
        if let HleResult::Unimplemented(reason) = status {
            r.blocked_services.push(Blocked::new(&name, &reason));
        }
    }
    // Boot requires every module to be liftable and every service implemented.
    r.bootable = !r.liftable_modules.is_empty()
        && r.blocked_modules.is_empty()
        && r.blocked_services.is_empty();
    r
}

/// Guest address space description, used for diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct AddressSpace {
    pub regions: BTreeMap<String, AddressRange>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct AddressRange {
    pub start: u64,
    pub len: u64,
}

impl AddressRange {
    pub fn end(&self) -> u64 {
        self.start.saturating_add(self.len)
    }
    pub fn contains(&self, addr: u64) -> bool {
        addr >= self.start && addr < self.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_reports_the_current_truth() {
        let modules = vec![
            ModuleLayout {
                name: "main".into(),
                module_name: "app".into(),
                text: 0x100,
                rodata: 0x100,
                data: 0x100,
                bss: 0x100,
                liftable: false,
            },
            ModuleLayout {
                name: "sdk".into(),
                module_name: "nnSdk".into(),
                text: 0x100,
                rodata: 0x100,
                data: 0x100,
                bss: 0x100,
                liftable: true,
            },
        ];
        let hle = HleRegistry::with_defaults();
        let r = readiness(&modules, &hle);
        assert!(!r.bootable, "the game module is not liftable yet");
        assert_eq!(r.blocked_modules, vec!["main".to_string()]);
        assert!(r.blocked_services.iter().any(|b| b.service.contains("nvn")));
        assert!(r.describe().contains("not bootable"));
    }

    #[test]
    fn range_containment() {
        let r = AddressRange { start: 0x1000, len: 0x100 };
        assert!(r.contains(0x1000));
        assert!(r.contains(0x10FF));
        assert!(!r.contains(0x1100));
    }
}
