//! `ew-storage`: the console front end for the unified storage manager.
//!
//! The launcher is a GUI binary, so the storage commands live in their own
//! console binary. This target compiles only the storage modules (and the
//! build-versioning types they use), not the launcher's UI tree.

// `build_versioning` depends on `toolchain_manager` for its event emitter and
// build descriptors; `toolchain` in turn asks `engine` for the launcher root
// and the Visual Studio install hint. The storage CLI never resolves a
// toolchain, so these two items are enough to keep that module compiling
// without pulling the whole launcher into this binary.
mod engine {
    use std::path::PathBuf;

    pub fn root() -> Option<PathBuf> {
        None
    }

    pub const VS_INSTALL: &str = "";
}

#[path = "../toolchain_manager.rs"]
mod toolchain_manager;

#[path = "../toolchain.rs"]
mod toolchain;

#[path = "../build_versioning.rs"]
mod build_versioning;

#[path = "../pipeline_cache.rs"]
mod pipeline_cache;

#[path = "../object_store.rs"]
mod object_store;

#[path = "../romfs_store.rs"]
mod romfs_store;

#[path = "../storage_manager.rs"]
mod storage_manager;

#[path = "../storage_manager_cli.rs"]
mod storage_manager_cli;

use clap::Parser;

fn main() {
    let cli = storage_manager_cli::Cli::parse();
    if let Err(error) = storage_manager_cli::run(cli) {
        eprintln!("ew-storage: {error:#}");
        std::process::exit(1);
    }
}
