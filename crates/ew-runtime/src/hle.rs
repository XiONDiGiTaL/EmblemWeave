//! HLE (high-level emulation) registry for Nintendo SDK services.
//!
//! A service is registered as either implemented or unimplemented. Unimplemented
//! services fail loudly and carry the reason they are blocked, so the runtime can
//! never appear to boot further than it actually has.
//!
//! The service list below is drawn from the symbol names present in the retail
//! `app`/`nnSdk` rodata (e.g. `nn::os::AllocateMemoryPages`,
//! `nn::mem::StandardAllocator::Finalize`), not from guesswork.

use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum HleResult {
    /// A host implementation exists and is exercised by tests.
    Implemented(String),
    /// No implementation yet; the guest call fails explicitly.
    Unimplemented(String),
}

impl HleResult {
    pub fn is_implemented(&self) -> bool {
        matches!(self, HleResult::Implemented(_))
    }

    pub fn reason(&self) -> &str {
        match self {
            HleResult::Implemented(r) | HleResult::Unimplemented(r) => r,
        }
    }
}

/// Registry of guest-visible services.
#[derive(Debug, Default, Clone)]
pub struct HleRegistry {
    entries: BTreeMap<String, HleResult>,
}

impl HleRegistry {
    pub fn new() -> Self {
        HleRegistry::default()
    }

    /// The service set the runtime knows about.
    ///
    /// Implemented entries mirror the handlers `ew-boot` actually registers, so
    /// this list cannot claim less than the runtime delivers. Anything the
    /// single-threaded host cannot provide stays blocked with the reason.
    pub fn with_defaults() -> Self {
        let mut r = HleRegistry::new();
        for (name, claim) in [
            (
                "nn::os::AllocateMemoryPages",
                "pages come from the arena above the guest stack, 4 KiB aligned and bounded",
            ),
            (
                "nn::os::FreeMemoryPages",
                "the arena is a bump allocator, so a freed page is not handed out again",
            ),
            (
                "nn::os::SetMemoryHeapSize",
                "the heap is fixed by the dispatcher layout, and the call succeeds without resizing it",
            ),
            (
                "nn::os::Mutex",
                "single guest thread: an uncontended lock succeeds, recursion is counted and a self-deadlock is reported",
            ),
            (
                "nn::os::Event",
                "single guest thread: signalling is exact, and waiting on a clear event is reported instead of hanging",
            ),
            (
                "nn::os::ThreadLocalStorage",
                "one slot table belonging to the only guest thread",
            ),
        ] {
            r.register(name, HleResult::Implemented(claim.to_string()));
        }
        for (name, reason) in [
            (
                "nn::os::CreateThread",
                "the host runs one guest thread, so starting another one needs a scheduler",
            ),
            ("nn::mem::StandardAllocator", "allocator semantics must match nnSdk exactly"),
            ("nn::nvn::Device", "NVN2 graphics device is not implemented"),
            ("nn::nvn::MemoryPool", "NVN2 memory pools are not implemented"),
            ("nn::nvn::Window", "NVN2 window/swapchain is not implemented"),
            ("nn::vi::Display", "display service is not implemented"),
            ("nn::audio::AudioOut", "audio output is not implemented"),
            ("nn::hid::AppletResource", "input service is not implemented"),
            ("nn::fs::FileSystem", "romfs/save filesystem service is not implemented"),
        ] {
            r.register(name, HleResult::Unimplemented(reason.to_string()));
        }
        r
    }

    pub fn register(&mut self, name: &str, status: HleResult) {
        self.entries.insert(name.to_string(), status);
    }

    pub fn status(&self, name: &str) -> Option<&HleResult> {
        self.entries.get(name)
    }

    pub fn statuses(&self) -> impl Iterator<Item = (String, HleResult)> + '_ {
        self.entries.iter().map(|(k, v)| (k.clone(), v.clone()))
    }

    pub fn implemented_count(&self) -> usize {
        self.entries.values().filter(|v| v.is_implemented()).count()
    }

    pub fn unimplemented_count(&self) -> usize {
        self.entries.values().filter(|v| !v.is_implemented()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_separate_what_runs_from_what_does_not() {
        let r = HleRegistry::with_defaults();
        assert!(r.implemented_count() >= 5, "the single-threaded os services run");
        assert!(r.unimplemented_count() >= 9);
        let nvn = r.status("nn::nvn::Device").unwrap();
        assert!(!nvn.is_implemented());
        assert!(nvn.reason().contains("NVN2"));
        let pages = r.status("nn::os::AllocateMemoryPages").unwrap();
        assert!(pages.is_implemented());
        assert!(pages.reason().contains("arena"));
        let create = r.status("nn::os::CreateThread").unwrap();
        assert!(!create.is_implemented(), "a second thread still needs a scheduler");
    }

    #[test]
    fn registration_is_replaceable() {
        let mut r = HleRegistry::new();
        r.register("test", HleResult::Unimplemented("no".into()));
        assert_eq!(r.unimplemented_count(), 1);
        r.register("test", HleResult::Implemented("yes".into()));
        assert_eq!(r.implemented_count(), 1);
        assert_eq!(r.status("test").unwrap().reason(), "yes");
    }
}
