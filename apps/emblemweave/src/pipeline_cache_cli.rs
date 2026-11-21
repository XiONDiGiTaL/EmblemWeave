//! CLI commands for Pipeline Cache Inventory and Migration

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::pipeline_cache::{PipelineCacheManager, PipelineCacheConfig, PipelineCacheStats, migrate_cache_on_promote};
use crate::build_versioning::{BuildVersionManager, BuildVersioningConfig, BuildIdentity};
use crate::toolchain_manager::{ConcurrencyInfo, BuildConfig};

#[derive(Parser)]
#[command(name = "ew-pipeline-cache", about = "EmblemWeave Pipeline Cache Management CLI")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Root directory for pipeline caches
    #[arg(long, default_value = "build/pipeline_cache")]
    cache_root: PathBuf,

    /// Maximum total cache size in bytes (0 = unlimited)
    #[arg(long, default_value = "2147483648")]
    max_size: u64,

    /// Maximum number of caches to keep
    #[arg(long, default_value = "20")]
    max_caches: usize,

    /// Minimum caches to keep per GPU
    #[arg(long, default_value = "2")]
    min_per_gpu: usize,

    /// Build versions root (for migration)
    #[arg(long, default_value = "build/versions")]
    versions_root: PathBuf,

    /// Maximum versions to keep
    #[arg(long, default_value = "20")]
    max_versions: usize,

    /// Minimum successful builds to keep
    #[arg(long, default_value = "3")]
    min_successful: usize,

    /// Use symlinks for current/previous
    #[arg(long, default_value = "true")]
    symlinks: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Scan for pipeline caches in common locations
    Scan {
        /// Additional paths to scan
        #[arg(num_args = 0..)]
        paths: Vec<PathBuf>,
    },

    /// List all cached pipeline caches
    List {
        /// Show only active caches
        #[arg(long)]
        active: bool,

        /// Filter by GPU UUID
        #[arg(long)]
        gpu: Option<String>,
    },

    /// Show cache statistics
    Stats,

    /// Show details of a specific cache
    Show {
        /// Cache SHA-256 hash
        sha256: String,
    },

    /// Migrate a cache to a build version
    Migrate {
        /// Cache SHA-256 hash
        sha256: String,

        /// Build timestamp
        timestamp: u64,
    },

    /// Link a cache to a build (copy to build directory)
    Link {
        /// Cache SHA-256 hash
        sha256: String,

        /// Build timestamp
        timestamp: u64,
    },

    /// Associate a cache with a build (tracking only)
    Associate {
        /// Cache SHA-256 hash
        sha256: String,

        /// Build timestamp
        timestamp: u64,
    },

    /// Cleanup old caches based on retention policy
    Cleanup,

    /// Force cleanup (remove all but N per GPU)
    ForceCleanup {
        /// Number of caches to keep per GPU
        #[arg(default_value = "2")]
        keep_per_gpu: usize,
    },

    /// Export inventory to JSON
    Export {
        /// Output file
        #[arg(default_value = "pipeline_cache_inventory.json")]
        output: PathBuf,
    },

    /// Auto-migrate runtime pipeline cache to current build
    AutoMigrate {
        /// Build timestamp (uses current if not specified)
        timestamp: Option<u64>,
    },
}

pub fn run(cli: Cli) -> Result<()> {
    let cache_config = PipelineCacheConfig {
        cache_root: cli.cache_root.clone(),
        max_total_size: cli.max_size,
        max_caches: cli.max_caches,
        min_per_gpu: cli.min_per_gpu,
        auto_migrate: true,
    };

    let mut cache_manager = PipelineCacheManager::new(cache_config)?;

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
        Commands::Scan { paths } => {
            let search_paths: Vec<PathBuf> = if paths.is_empty() {
                vec![
                    PathBuf::from("engine/fefw-recomp/runtime"),
                    PathBuf::from("build"),
                    cli.cache_root.clone(),
                ]
            } else {
                paths
            };

            let found = cache_manager.scan_caches(&search_paths)?;
            println!("Scanned {} paths, found {} new pipeline caches", search_paths.len(), found);
            println!("Total caches in inventory: {}", cache_manager.inventory.entries.len());
        }

        Commands::List { active, gpu } => {
            let entries: Vec<_> = cache_manager.inventory.entries
                .values()
                .filter(|e| !active || e.is_active)
                .filter(|e| gpu.as_ref().map_or(true, |g| e.device_uuid.as_deref() == Some(g)))
                .collect();

            if entries.is_empty() {
                println!("No pipeline caches found");
                return Ok(());
            }

            println!("{:<64} {:>12} {:>10} {:>8} {:>10} {}", "SHA256", "SIZE", "GPU UUID", "VERSION", "BUILD", "ACTIVE");
            println!("{}", "-".repeat(120));

            for entry in entries {
                let size_str = format_size(entry.size_bytes);
                let gpu_uuid = entry.device_uuid.as_deref().unwrap_or("unknown");
                let gpu_short = if gpu_uuid.len() > 8 { &gpu_uuid[..8] } else { gpu_uuid };
                let build_info = match (entry.build_timestamp, &entry.build_config, &entry.git_commit_short) {
                    (Some(ts), Some(cfg), Some(commit)) => format!("{} {} {}", ts, cfg, commit),
                    (Some(ts), _, _) => format!("{}", ts),
                    _ => "unlinked".to_string(),
                };
                let active_str = if entry.is_active { "yes" } else { "no" };

                println!("{:<64} {:>12} {:>10} {:>8} {:>10} {}",
                    &entry.sha256[..64.min(entry.sha256.len())],
                    size_str,
                    gpu_short,
                    entry.format_version,
                    build_info,
                    active_str
                );
            }
        }

        Commands::Stats => {
            let stats = cache_manager.get_stats();
            println!("Pipeline Cache Statistics");
            println!("=========================");
            println!("Total caches:        {}", stats.total_caches);
            println!("Active caches:       {}", stats.active_caches);
            println!("Total size:          {} ({:.2} MB)", stats.total_size_bytes, stats.total_size_bytes as f64 / 1_048_576.0);
            println!();
            println!("By GPU:");
            for (gpu, count) in &stats.by_gpu {
                println!("  {}: {}", gpu, count);
            }
        }

        Commands::Show { sha256 } => {
            if let Some(entry) = cache_manager.inventory.entries.get(&sha256) {
                print_cache_entry(entry);
            } else {
                eprintln!("Cache not found: {}", sha256);
                std::process::exit(1);
            }
        }

        Commands::Migrate { sha256, timestamp } => {
            let build = version_manager.list_versions()
                .into_iter()
                .find(|v| v.timestamp_unix == timestamp)
                .ok_or_else(|| anyhow::anyhow!("Build not found: {}", timestamp))?;

            let dest = cache_manager.migrate_to_build(&sha256, &build)?;
            println!("Migrated cache to: {}", dest.display());
        }

        Commands::Link { sha256, timestamp } => {
            let build = version_manager.list_versions()
                .into_iter()
                .find(|v| v.timestamp_unix == timestamp)
                .ok_or_else(|| anyhow::anyhow!("Build not found: {}", timestamp))?;

            let dest = cache_manager.link_to_build(&sha256, &build)?;
            println!("Linked cache to build: {}", dest.display());
        }

        Commands::Associate { sha256, timestamp } => {
            let build = version_manager.list_versions()
                .into_iter()
                .find(|v| v.timestamp_unix == timestamp)
                .ok_or_else(|| anyhow::anyhow!("Build not found: {}", timestamp))?;

            cache_manager.associate_with_build(&sha256, &build)?;
            println!("Associated cache {} with build {}", sha256, timestamp);
        }

        Commands::Cleanup => {
            let removed = cache_manager.cleanup()?;
            println!("Cleaned up {} old pipeline caches", removed);
        }

        Commands::ForceCleanup { keep_per_gpu } => {
            cache_manager.config.min_per_gpu = keep_per_gpu;
            let removed = cache_manager.cleanup()?;
            println!("Force cleaned up {} old pipeline caches (kept {} per GPU)", removed, keep_per_gpu);
        }

        Commands::Export { output } => {
            cache_manager.export_inventory(&output)?;
            println!("Exported pipeline cache inventory to {}", output.display());
        }

        Commands::AutoMigrate { timestamp } => {
            let build_ts = if let Some(ts) = timestamp {
                ts
            } else {
                version_manager.get_current()
                    .map(|b| b.timestamp_unix)
                    .ok_or_else(|| anyhow::anyhow!("No current build found"))?
            };

            if let Some(dest) = migrate_cache_on_promote(&version_manager, &mut cache_manager, build_ts)? {
                println!("Auto-migrated pipeline cache to: {}", dest.display());
            } else {
                println!("No runtime pipeline cache found to migrate");
            }
        }
    }

    Ok(())
}

fn print_cache_entry(entry: &crate::pipeline_cache::PipelineCacheEntry) {
    println!("Pipeline Cache Entry");
    println!("===================");
    println!("SHA-256:           {}", entry.sha256);
    println!("Size:              {} bytes ({:.2} MB)", entry.size_bytes, entry.size_bytes as f64 / 1_048_576.0);
    println!("Device UUID:       {}", entry.device_uuid.as_deref().unwrap_or("unknown"));
    println!("GPU Name:          {}", entry.gpu_name.as_deref().unwrap_or("unknown"));
    println!("Driver Version:    {}", entry.driver_version.as_deref().unwrap_or("unknown"));
    println!("Format Version:    {}", entry.format_version);
    println!("Created:           {}", chrono::DateTime::from_timestamp(entry.created_unix as i64, 0).map(|d| d.to_rfc3339()).unwrap_or_else(|| "unknown".to_string()));
    println!("Last Accessed:     {}", chrono::DateTime::from_timestamp(entry.last_accessed_unix as i64, 0).map(|d| d.to_rfc3339()).unwrap_or_else(|| "unknown".to_string()));
    println!("Build Timestamp:   {}", entry.build_timestamp.map(|t| t.to_string()).unwrap_or_else(|| "unlinked".to_string()));
    println!("Build Config:      {}", entry.build_config.as_deref().unwrap_or("unlinked"));
    println!("Git Commit:        {}", entry.git_commit_short.as_deref().unwrap_or("unlinked"));
    println!("Relative Path:     {}", entry.relative_path);
    println!("Active:            {}", if entry.is_active { "yes" } else { "no" });
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cli_scan() {
        let dir = tempdir().unwrap();
        let config = PipelineCacheConfig {
            cache_root: dir.path().join("pipeline_cache"),
            max_total_size: 100 * 1024 * 1024,
            max_caches: 10,
            min_per_gpu: 1,
            auto_migrate: true,
        };

        let mut manager = PipelineCacheManager::new(config).unwrap();

        let cache_file = dir.path().join("pipeline_cache.bin");
        let mut data = vec![0u8; 32];
        data[0..4].copy_from_slice(&32u32.to_le_bytes());
        data[4..8].copy_from_slice(&1u32.to_le_bytes());
        data[8..12].copy_from_slice(&0x10DEu32.to_le_bytes());
        data[12..16].copy_from_slice(&0x1234u32.to_le_bytes());
        data[16..32].copy_from_slice(&[1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16]);

        std::fs::write(&cache_file, &data).unwrap();

        let found = manager.scan_caches(&[dir.path().to_path_buf()]).unwrap();
        assert_eq!(found, 1);
    }
}