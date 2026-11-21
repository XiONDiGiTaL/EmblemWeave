//! CLI commands for Object/Lift Store

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::object_store::{
    ObjectLiftStoreManager, ObjectStoreConfig, LiftStoreConfig, ObjectType, ObjectStoreStats, LiftStoreStats,
    store_build_artifacts_on_promote,
};
use crate::build_versioning::{BuildVersionManager, BuildVersioningConfig};
use crate::toolchain_manager::{ConcurrencyInfo, BuildConfig};

#[derive(Parser)]
#[command(name = "ew-object-store", about = "EmblemWeave Object/Lift Store CLI")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Object store root directory
    #[arg(long, default_value = "build/object_store")]
    object_root: PathBuf,

    /// Lift store root directory
    #[arg(long, default_value = "build/lift_store")]
    lift_root: PathBuf,

    /// Maximum object store size (bytes)
    #[arg(long, default_value = "10737418240")]
    max_object_size: u64,

    /// Maximum number of objects
    #[arg(long, default_value = "100000")]
    max_objects: usize,

    /// Maximum lift store size (bytes)
    #[arg(long, default_value = "5368709120")]
    max_lift_size: u64,

    /// Maximum number of lifts
    #[arg(long, default_value = "50000")]
    max_lifts: usize,

    /// Enable hardlinks
    #[arg(long, default_value = "true")]
    enable_hardlinks: bool,

    /// Build versions root
    #[arg(long, default_value = "build/versions")]
    versions_root: PathBuf,

    /// Maximum versions to keep
    #[arg(long, default_value = "20")]
    max_versions: usize,

    /// Minimum successful builds to keep
    #[arg(long, default_value = "3")]
    min_successful: usize,

    /// Use symlinks
    #[arg(long, default_value = "true")]
    symlinks: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Store a file in the object store
    Store {
        /// File to store
        file: PathBuf,

        /// Object type (artifact, source, config, intermediate, lift)
        #[arg(long, default_value = "artifact")]
        type_: String,

        /// Build timestamp to associate
        #[arg(long)]
        build_ts: Option<u64>,
    },

    /// Store all artifacts from a build directory
    StoreBuild {
        /// Build directory
        dir: PathBuf,

        /// Build timestamp
        build_ts: u64,
    },

    /// Store artifacts for a promoted build
    StorePromoted {
        /// Build timestamp
        timestamp: u64,
    },

    /// Retrieve an object to a file
    Retrieve {
        /// Object SHA-256
        sha256: String,

        /// Destination file
        dest: PathBuf,
    },

    /// Record a lift (incremental build unit)
    RecordLift {
        /// Lift name
        name: String,

        /// Input object hashes (comma-separated)
        #[arg(long)]
        inputs: String,

        /// Output object hashes (comma-separated)
        #[arg(long)]
        outputs: String,

        /// Build command
        #[arg(long)]
        command: String,

        /// Working directory
        #[arg(long)]
        working_dir: String,

        /// Build timestamp
        #[arg(long)]
        build_ts: u64,

        /// Environment variables (KEY=VALUE, comma-separated)
        #[arg(long)]
        env: Option<String>,
    },

    /// Check if a lift can be reused
    CheckLift {
        /// Lift hash
        lift_hash: String,

        /// Current input hashes (comma-separated)
        #[arg(long)]
        inputs: String,
    },

    /// Invalidate lifts for a build
    InvalidateLifts {
        /// Build timestamp
        build_ts: u64,
    },

    /// List objects
    ListObjects {
        /// Filter by type
        #[arg(long)]
        type_: Option<String>,

        /// Show only unreferenced
        #[arg(long)]
        unreferenced: bool,

        /// Limit results
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// List lifts
    ListLifts {
        /// Filter by build timestamp
        #[arg(long)]
        build_ts: Option<u64>,

        /// Show only valid
        #[arg(long)]
        valid: bool,

        /// Limit results
        #[arg(long, default_value = "50")]
        limit: usize,
    },

    /// Show object statistics
    ObjectStats,

    /// Show lift statistics
    LiftStats,

    /// Cleanup unreferenced objects
    CleanupObjects,

    /// Cleanup invalid lifts
    CleanupLifts,

    /// Export object index
    ExportObjects {
        /// Output file
        #[arg(default_value = "object_index.json")]
        output: PathBuf,
    },

    /// Export lift index
    ExportLifts {
        /// Output file
        #[arg(default_value = "lift_index.json")]
        output: PathBuf,
    },
}

pub fn run(cli: Cli) -> Result<()> {
    let object_config = ObjectStoreConfig {
        store_root: cli.object_root.clone(),
        max_total_size: cli.max_object_size,
        max_objects: cli.max_objects,
        min_ref_count: 0,
        enable_hardlinks: cli.enable_hardlinks,
        compress: false,
    };

    let lift_config = LiftStoreConfig {
        store_root: cli.lift_root.clone(),
        max_total_size: cli.max_lift_size,
        max_lifts: cli.max_lifts,
        enable_reuse: true,
    };

    let mut store = ObjectLiftStoreManager::new(object_config, lift_config)?;

    let version_config = BuildVersioningConfig {
        versions_root: cli.versions_root.clone(),
        max_versions: cli.max_versions,
        min_successful_to_keep: cli.min_successful,
        create_symlinks: cli.symlinks,
        validate_before_promote: false,
        validation_cmd: None,
    };

    let version_manager = BuildVersionManager::new(version_config)?;

    match cli.command {
        Commands::Store { file, type_, build_ts } => {
            let object_type = parse_object_type(&type_)?;
            let hash = store.store_object(&file, object_type, build_ts)?;
            println!("Stored object: {}", hash);
            println!("Type: {:?}", object_type);
            if let Some(ts) = build_ts {
                println!("Associated with build: {}", ts);
            }
        }

        Commands::StoreBuild { dir, build_ts } => {
            let hashes = store.store_build_artifacts(&dir, build_ts)?;
            println!("Stored {} artifacts from build {}", hashes.len(), build_ts);
            for hash in &hashes {
                println!("  {}", hash);
            }
        }

        Commands::StorePromoted { timestamp } => {
            let hashes = store_build_artifacts_on_promote(&mut store, &version_manager, timestamp)?;
            println!("Stored {} artifacts from promoted build {}", hashes.len(), timestamp);
            for hash in &hashes {
                println!("  {}", hash);
            }
        }

        Commands::Retrieve { sha256, dest } => {
            store.retrieve_object(&sha256, &dest)?;
            println!("Retrieved object {} to {}", sha256, dest.display());
        }

        Commands::RecordLift { name, inputs, outputs, command, working_dir, build_ts, env } => {
            let input_hashes: Vec<String> = inputs.split(',').map(|s| s.trim().to_string()).collect();
            let output_hashes: Vec<String> = outputs.split(',').map(|s| s.trim().to_string()).collect();

            let env_vars = if let Some(env_str) = env {
                env_str.split(',')
                    .filter_map(|s| {
                        let parts: Vec<&str> = s.splitn(2, '=').collect();
                        if parts.len() == 2 {
                            Some((parts[0].trim().to_string(), parts[1].trim().to_string()))
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                HashMap::new()
            };

            let lift_hash = store.record_lift(
                name,
                input_hashes,
                output_hashes,
                command,
                working_dir,
                env_vars,
                build_ts,
            )?;

            println!("Recorded lift: {}", lift_hash);
        }

        Commands::CheckLift { lift_hash, inputs } => {
            let input_hashes: Vec<String> = inputs.split(',').map(|s| s.trim().to_string()).collect();
            let can_reuse = store.can_reuse_lift(&lift_hash, &input_hashes);
            println!("Lift {}: {}", lift_hash, if can_reuse { "CAN REUSE" } else { "CANNOT REUSE" });
        }

        Commands::InvalidateLifts { build_ts } => {
            let count = store.invalidate_lifts_for_build(build_ts);
            println!("Invalidated {} lifts for build {}", count, build_ts);
        }

        Commands::ListObjects { type_, unreferenced, limit } => {
            let filter_type = type_.as_deref().and_then(parse_object_type_opt);
            let objects: Vec<_> = store.objects
                .values()
                .filter(|o| filter_type.map_or(true, |t| o.object_type == t))
                .filter(|o| !unreferenced || o.ref_count == 0)
                .take(limit)
                .collect();

            if objects.is_empty() {
                println!("No objects found");
                return Ok(());
            }

            println!("{:<64} {:>10} {:>8} {:>6} {:>12} {}", "SHA256", "SIZE", "TYPE", "REFS", "BUILDS", "EXT");
            println!("{}", "-".repeat(110));

            for obj in objects {
                let size_str = format_size(obj.size_bytes);
                let type_str = format!("{:?}", obj.object_type);
                let builds = obj.build_timestamps.len();
                let ext = obj.extension.as_deref().unwrap_or("");

                println!("{:<64} {:>10} {:>8} {:>6} {:>12} {}",
                    &obj.sha256[..64.min(obj.sha256.len())],
                    size_str,
                    type_str,
                    obj.ref_count,
                    builds,
                    ext
                );
            }
        }

        Commands::ListLifts { build_ts, valid, limit } => {
            let lifts: Vec<_> = store.lifts
                .values()
                .filter(|l| build_ts.map_or(true, |ts| l.build_timestamp == ts))
                .filter(|l| !valid || l.is_valid)
                .take(limit)
                .collect();

            if lifts.is_empty() {
                println!("No lifts found");
                return Ok(());
            }

            println!("{:<64} {:>20} {:>8} {:>10} {:>12} {}", "SHA256", "NAME", "VALID", "INPUTS", "OUTPUTS", "BUILD");
            println!("{}", "-".repeat(130));

            for lift in lifts {
                println!("{:<64} {:>20} {:>8} {:>10} {:>12} {}",
                    &lift.sha256[..64.min(lift.sha256.len())],
                    &lift.name[..20.min(lift.name.len())],
                    if lift.is_valid { "yes" } else { "no" },
                    lift.input_hashes.len(),
                    lift.output_hashes.len(),
                    lift.build_timestamp
                );
            }
        }

        Commands::ObjectStats => {
            let stats = store.get_object_stats();
            print_object_stats(&stats);
        }

        Commands::LiftStats => {
            let stats = store.get_lift_stats();
            print_lift_stats(&stats);
        }

        Commands::CleanupObjects => {
            let removed = store.cleanup_objects()?;
            println!("Cleaned up {} unreferenced objects", removed);
        }

        Commands::CleanupLifts => {
            let removed = store.cleanup_lifts()?;
            println!("Cleaned up {} invalid lifts", removed);
        }

        Commands::ExportObjects { output } => {
            store.export_objects(&output)?;
            println!("Exported object index to {}", output.display());
        }

        Commands::ExportLifts { output } => {
            store.export_lifts(&output)?;
            println!("Exported lift index to {}", output.display());
        }
    }

    Ok(())
}

fn parse_object_type(s: &str) -> Result<ObjectType> {
    match s.to_lowercase().as_str() {
        "artifact" => Ok(ObjectType::Artifact),
        "source" => Ok(ObjectType::Source),
        "config" => Ok(ObjectType::Config),
        "intermediate" => Ok(ObjectType::Intermediate),
        "lift" => Ok(ObjectType::Lift),
        _ => Err(anyhow::anyhow!("Invalid object type: {}", s)),
    }
}

fn parse_object_type_opt(s: &str) -> Option<ObjectType> {
    parse_object_type(s).ok()
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    format!("{:.1} {}", size, UNITS[unit])
}

fn print_object_stats(stats: &ObjectStoreStats) {
    println!("Object Store Statistics");
    println!("======================");
    println!("Total objects:       {}", stats.total_objects);
    println!("Total size:          {} ({:.2} MB)", stats.total_size_bytes, stats.total_size_bytes as f64 / 1_048_576.0);
    println!("Total references:    {}", stats.total_refs);
    println!("Unreferenced:        {}", stats.unreferenced_objects);
    println!();
    println!("By type:");
    for (obj_type, count) in &stats.by_type {
        println!("  {:?}: {}", obj_type, count);
    }
}

fn print_lift_stats(stats: &LiftStoreStats) {
    println!("Lift Store Statistics");
    println!("====================");
    println!("Total lifts:         {}", stats.total_lifts);
    println!("Valid lifts:         {}", stats.valid_lifts);
    println!("Invalid lifts:       {}", stats.invalid_lifts);
    println!("Total output size:   {} ({:.2} MB)", stats.total_output_size_bytes, stats.total_output_size_bytes as f64 / 1_048_576.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_object_type() {
        assert!(parse_object_type("artifact").is_ok());
        assert!(parse_object_type("source").is_ok());
        assert!(parse_object_type("config").is_ok());
        assert!(parse_object_type("intermediate").is_ok());
        assert!(parse_object_type("lift").is_ok());
        assert!(parse_object_type("invalid").is_err());
    }
}