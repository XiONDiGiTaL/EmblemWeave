//! What this PC's processor can run, and which build of the lifted game code
//! that calls for.
//!
//! fefw-recomp compiles the guest code in one of two variants (the engine's
//! `game-build/release/fefw_build.py` CPU_VARIANTS):
//!
//! - `fma`: `-mfma`, so the guest's FMADD / FMLA family is one fused
//!   multiply-add. Needs FMA3 and AVX (which implies SSE4.2 and POPCNT on
//!   every processor that has it) and CMPXCHG16B. Intel Haswell (2013) and
//!   AMD Piledriver (2012) and newer.
//! - `compat`: no FMA3, so `fma` / `fmaf` stay correctly rounded library
//!   calls. Needs SSE4.2, POPCNT and CMPXCHG16B: Intel Nehalem (2008) and AMD
//!   Bulldozer (2011) and newer, which includes Sandy Bridge, Ivy Bridge and
//!   the Xeon E5 v1 / v2.
//!
//! CMPXCHG16B is in both (the guest's 128-bit exclusive pairs). A processor
//! without the `compat` set cannot run the game at all, and the launcher says
//! which feature is missing instead of letting the build produce a binary
//! that dies on its first instruction.
//!
//! The engine re-checks at start (`fefw_cpu.h`) and ends with code 4 and a
//! message naming the missing feature, so a build that was moved to another
//! PC still says what is wrong.
//!
//! The rule here is the one the engine documents; `choose` takes a feature
//! set so it can be tested without the processor the tests run on.

/// The processor features either build cares about, and its name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Features {
    /// The CPUID brand string, trimmed (`Intel(R) Core(TM) i5-2500K CPU ...`).
    pub brand: String,
    pub cx16: bool,
    /// SSE4.2, with SSE3, SSSE3 and SSE4.1, which the engine's check covers together.
    pub sse42: bool,
    pub popcnt: bool,
    /// AVX with the YMM state enabled by the operating system.
    pub avx: bool,
    /// FMA3, only ever set together with AVX.
    pub fma: bool,
}

/// Which build of the lifted code this processor needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    /// FMA3 + AVX, the default and faster build.
    Fma,
    /// No FMA3: SSE4.2 + POPCNT, for Sandy Bridge, Ivy Bridge and older Xeons.
    Compat,
}

impl Variant {
    /// The value the build reads from `FEFW_CPU_VARIANT`.
    pub fn key(self) -> &'static str {
        match self {
            Variant::Fma => "fma",
            Variant::Compat => "compat",
        }
    }

    /// What the build requires, for the screen and the log.
    pub fn label(self) -> &'static str {
        match self {
            Variant::Fma => "FMA3 + AVX",
            Variant::Compat => "SSE4.2 + POPCNT (no FMA3)",
        }
    }
}

/// The build to use, or the reason this processor cannot run the game.
pub fn choose(f: &Features) -> Result<Variant, String> {
    let mut missing = Vec::new();
    if !f.cx16 {
        missing.push("CMPXCHG16B");
    }
    if !f.sse42 {
        missing.push("SSE4.2");
    }
    if !f.popcnt {
        missing.push("POPCNT");
    }
    if !missing.is_empty() {
        let brand = if f.brand.is_empty() { "this processor".to_string() } else { f.brand.clone() };
        return Err(format!(
            "{brand} lacks {}, which both builds of the game code need. The game cannot run on it: the engine needs an Intel Nehalem (2008) or AMD Bulldozer (2011) or newer processor.",
            missing.join(", ")
        ));
    }
    // FMA3 needs AVX, and the engine's own check treats them together; a
    // processor that reports FMA3 without a usable YMM state is not one of
    // those either.
    if f.fma && f.avx {
        Ok(Variant::Fma)
    } else {
        Ok(Variant::Compat)
    }
}

/// Reads the features of the processor this launcher runs on.
#[cfg(target_arch = "x86_64")]
pub fn probe() -> Features {
    use std::arch::x86_64::{__cpuid, _xgetbv};

    let mut f = Features::default();
    // SAFETY: CPUID and XGETBV are available on every x86-64 processor (CPUID
    // is how the architecture identifies itself), and the feature bits are
    // read with the leaves the architecture defines.
    unsafe {
        let leaf0 = __cpuid(0);
        let mut brand = [0u8; 48];
        if __cpuid(0x8000_0000).eax >= 0x8000_0004 {
            for (i, at) in (0..3).enumerate() {
                let r = __cpuid(0x8000_0002 + at);
                let bytes = [r.eax, r.ebx, r.ecx, r.edx];
                for (j, word) in bytes.iter().enumerate() {
                    let b = word.to_le_bytes();
                    brand[i * 16 + j * 4..i * 16 + j * 4 + 4].copy_from_slice(&b);
                }
            }
        }
        f.brand = String::from_utf8_lossy(&brand).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string();
        let _ = leaf0;

        let leaf1 = __cpuid(1);
        let ecx = leaf1.ecx;
        f.cx16 = ecx & (1 << 13) != 0;
        let sse3 = ecx & 1 != 0;
        let ssse3 = ecx & (1 << 9) != 0;
        let sse41 = ecx & (1 << 19) != 0;
        let sse42 = ecx & (1 << 20) != 0;
        f.sse42 = sse3 && ssse3 && sse41 && sse42;
        f.popcnt = ecx & (1 << 23) != 0;
        let osxsave = ecx & (1 << 27) != 0;
        let mut ymm = false;
        if osxsave {
            let xcr0 = _xgetbv(0);
            ymm = xcr0 & 0b110 == 0b110; // XMM and YMM state enabled by the OS
        }
        f.avx = ecx & (1 << 28) != 0 && ymm;
        f.fma = ecx & (1 << 12) != 0 && f.avx;
    }
    f
}

/// Reads the features of the processor this launcher runs on.
#[cfg(not(target_arch = "x86_64"))]
pub fn probe() -> Features {
    Features::default()
}

/// One line for the log and the Diagnostics page.
pub fn summary(f: &Features, chosen: &Result<Variant, String>) -> String {
    let brand = if f.brand.is_empty() { "unknown processor".to_string() } else { f.brand.clone() };
    let yes = |b: bool| if b { "yes" } else { "no" };
    let build = match chosen {
        Ok(v) => format!("{} build ({})", v.key(), v.label()),
        Err(_) => "no supported build".to_string(),
    };
    format!(
        "{brand}: SSE4.2 {}, POPCNT {}, CMPXCHG16B {}, AVX {}, FMA3 {} -> {build}",
        yes(f.sse42),
        yes(f.popcnt),
        yes(f.cx16),
        yes(f.avx),
        yes(f.fma)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sandy() -> Features {
        Features { brand: "Intel(R) Core(TM) i5-2500K".into(), cx16: true, sse42: true, popcnt: true, avx: true, fma: false }
    }

    fn haswell() -> Features {
        Features { fma: true, ..sandy() }
    }

    #[test]
    fn a_processor_with_fma3_and_avx_gets_the_fast_build() {
        assert_eq!(choose(&haswell()), Ok(Variant::Fma));
        assert_eq!(Variant::Fma.key(), "fma");
    }

    #[test]
    fn sandy_bridge_gets_the_compatible_build() {
        assert_eq!(choose(&sandy()), Ok(Variant::Compat));
    }

    #[test]
    fn fma3_without_a_usable_ymm_state_is_not_enough() {
        let f = Features { avx: false, ..haswell() };
        assert_eq!(choose(&f), Ok(Variant::Compat));
    }

    #[test]
    fn core2_is_refused_and_named() {
        let f = Features { brand: "Intel(R) Core(TM)2 Duo".into(), cx16: false, sse42: false, popcnt: false, avx: false, fma: false };
        let why = choose(&f).unwrap_err();
        assert!(why.contains("Core(TM)2 Duo"), "{why}");
        assert!(why.contains("SSE4.2") && why.contains("POPCNT") && why.contains("CMPXCHG16B"), "{why}");
    }

    #[test]
    fn a_processor_without_cmpxchg16b_is_refused() {
        let f = Features { cx16: false, ..sandy() };
        let why = choose(&f).unwrap_err();
        assert!(why.contains("CMPXCHG16B"), "{why}");
    }

    #[test]
    fn the_summary_names_the_features_and_the_build() {
        let s = summary(&haswell(), &choose(&haswell()));
        assert!(s.contains("FMA3 yes"), "{s}");
        assert!(s.contains("fma build"), "{s}");
        let c = summary(&sandy(), &choose(&sandy()));
        assert!(c.contains("FMA3 no"), "{c}");
        assert!(c.contains("compat build"), "{c}");
    }

    #[test]
    fn this_pc_probe_reports_a_supported_build() {
        // The test machine is an x86-64 Windows PC; whatever it is, the probe
        // has to answer with a build or a reason, never a panic.
        let f = probe();
        match choose(&f) {
            Ok(v) => assert!(matches!(v, Variant::Fma | Variant::Compat)),
            Err(why) => assert!(!why.is_empty()),
        }
    }
}
