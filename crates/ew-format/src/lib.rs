//! `ew-format` — container and asset format layer for EmblemWeave.
//!
//! This crate is the evidence-driven foundation of the project: it parses the
//! Nintendo Switch 2 executable containers found in a retail dump, decompresses
//! their segments, and inventories the extracted romfs. Nothing here contains or
//! redistributes Nintendo content; every function takes the user's own files as
//! input.

pub mod exefs;
pub mod nso;
pub mod nsp;
pub mod nup;
pub mod nx;
pub mod romfs;

pub use exefs::{ExeFs, ModuleEntry, NpdmInfo};
pub use nso::{Compression, GameImage, Mod0, Nso, NsoHeader, Segment, SegmentKind};
pub use romfs::{probe_samples, FormatGuess, Inventory, ProbeResult};

/// Errors that identify a genuine, documented blocker rather than a bug.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    /// The module's segments use Nintendo's ZBIC Zstandard variant.
    ZbicCompression,
}

impl Blocker {
    /// Stable identifier used in reports and the launcher status panel.
    pub fn id(self) -> &'static str {
        match self {
            Blocker::ZbicCompression => "ZBIC_COMPRESSION",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Blocker::ZbicCompression => {
                "Executable segments use ZBIC compression (Nintendo's modified Zstandard). \
                 Decoding needs a Binary Interpolative Coding FSE table reader."
            }
        }
    }
}
