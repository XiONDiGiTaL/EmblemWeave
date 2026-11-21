//! CLI commands for Unified Storage Manager

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::storage_manager::{
    StorageManager, StorageCleanupConfig, UnifiedStorageStats, CleanupResult,
};

#[derive(Parser)]
#[command(name = "ew-storage", about = "EmblemWeave Unified Storage Manager CLI")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Build versions root directory
    #[arg(long, default_value = "build/versions")]
    versions_root: PathBuf,

    /// Pipeline cache root directory
    #[arg(long, default_value = "build/pipeline_cache")]
    pipeline_cache_root: PathBuf,

    /// Object store root directory
    #[arg(long, default_value = "build/object_store")]
    object_store_root: PathBuf,

    /// Lift store root directory
    #[arg(long, default_value = "build/lift_store")]
    lift_store_root: PathBuf,

    /// Hard-link store root directory
    #[arg(long, default_value = "build/hardlink_store")]
    hardlink_store_root: PathBuf,

    /// Minimum successful versions to keep
    #[arg(long, default_value = "3")]
    min_successful_versions: usize,

    /// Maximum versions to keep
    #[arg(long, default_value = "20")]
    max_versions: usize,

    /// Maximum pipeline cache size (bytes)
    #[arg(long, default_value = "5368709120")]
    max_pipeline_cache_size: u64,

    /// Maximum object store size (bytes)
    #[arg(long, default_value = "10737418240")]
    max_object_store_size: u64,

    /// Maximum lift store size (bytes)
    #[arg(long, default_value = "5368709120")]
    max_lift_store_size: u64,

    /// Maximum hard-link store size (bytes)
    #[arg(long, default_value = "21474836480")]
    max_hardlink_store_size: u64,

    /// Minimum free space to maintain (bytes)
    #[arg(long, default_value = "10737418240")]
    min_free_space: u64,

    /// Dry run mode
    #[arg(long, default_value = "false")]
    dry_run: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Show unified storage statistics
    Stats {
        /// Output format (json, table)
        #[arg(long, default_value = "table")]
        format: String,

        /// Output file (for JSON)
        #[arg(long)]
        output: Option<PathBuf>,
    },

    /// Run cleanup across all storage systems
    Cleanup {
        /// Force cleanup even if not auto-enabled
        #[arg(long, default_value = "false")]
        force: bool,
    },

    /// Dry-run cleanup (preview what would be removed)
    DryRun,

    /// Export unified storage report
    Export {
        /// Output file
        #[arg(default_value = "storage_report.json")]
        output: PathBuf,
    },

    /// Store build artifacts across all stores
    StoreBuild {
        /// Build directory
        dir: PathBuf,

        /// Build timestamp
        build_ts: u64,

        /// Build config (Release, Debug, etc.)
        #[arg(long, default_value = "Release")]
        config: String,

        /// Git commit short hash
        #[arg(long, default_value = "unknown")]
        git_commit: String,
    },

    /// Check disk space
    DiskSpace,
}

pub fn run(cli: Cli) -> Result<()> {
    let config = StorageCleanupConfig {
        min_successful_versions: cli.min_successful_versions,
        max_versions: cli.max_versions,
        max_pipeline_cache_size: cli.max_pipeline_cache_size,
        max_object_store_size: cli.max_object_store_size,
        max_lift_store_size: cli.max_lift_store_size,
        max_hardlink_store_size: cli.max_hardlink_store_size,
        min_free_space: cli.min_free_space,
        auto_cleanup: false,
        dry_run: cli.dry_run,
    };

    let mut manager = StorageManager::with_paths(
        config,
        cli.versions_root,
        cli.pipeline_cache_root,
        cli.object_store_root,
        cli.lift_store_root,
        cli.hardlink_store_root,
    )?;

    match cli.command {
        Commands::Stats { format, output } => {
            let stats = manager.collect_stats()?;

            if format == "json" {
                let json = serde_json::to_string_pretty(&stats)?;
                if let Some(output_path) = output {
                    std::fs::write(&output_path, &json)?;
                    println!("Exported stats to {}", output_path.display());
                } else {
                    println!("{}", json);
                }
            } else {
                print_stats_table(&stats);
            }
        }

        Commands::Cleanup { force } => {
            if !force && !manager.config.auto_cleanup {
                println!("Cleanup not forced and auto_cleanup is disabled. Use --force to run anyway.");
                return Ok(());
            }

            println!("Running cleanup...");
            let result = manager.cleanup()?;
            print_cleanup_result(&result);
        }

        Commands::DryRun => {
            println!("Running dry-run cleanup (preview)...");
            let result = manager.dry_run_cleanup()?;
            print_cleanup_result(&result);
        }

        Commands::Export { output } => {
            println!("Exporting storage report to {}...", output.display());
            manager.export_report(&output)?;
            println!("Done.");
        }

        Commands::StoreBuild { dir, build_ts, config: build_config, git_commit } => {
            println!("Storing build artifacts for build {}...", build_ts);

            // Create a minimal BuildIdentity for storage purposes
            let build_identity = crate::build_versioning::BuildIdentity {
                build_id: uuid::Uuid::new_v4().to_string(),
                git_commit: git_commit.clone(),
                git_commit_short: git_commit,
                timestamp: chrono::DateTime::from_timestamp(build_ts as i64, 0)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| "unknown".to_string()),
                timestamp_unix: build_ts,
                config: build_config,
                toolchain_fingerprint: "unknown".to_string(),
                concurrency_info: crate::toolchain_manager::ConcurrencyInfo {
                    total_memory_mb: 0,
                    available_memory_mb: 0,
                    cpu_cores: 1,
                    estimated_per_job_mb: 1536,
                    calculated_jobs: 1,
                    reserved_memory_mb: 0,
                },
                build_config: crate::toolchain_manager::BuildConfig {
                    jobs: 1,
                    memory_limit_mb: None,
                    sccache_enabled: false,
                    incremental: false,
                    target: "Release".to_string(),
                    features: vec![],
                },
                status: crate::build_versioning::BuildStatus::Success,
                duration_ms: None,
                error: None,
                validation: None,
            };

            manager.store_build_artifacts(&build_identity, &dir)?;
            println!("Done.");
        }

        Commands::DiskSpace => {
            print_disk_space(&manager)?;
        }
    }

    Ok(())
}

fn print_stats_table(stats: &UnifiedStorageStats) {
    println!("Unified Storage Statistics");
    println!("==========================");
    println!("Collected: {}", chrono::DateTime::from_timestamp(stats.collected_unix as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_string()));
    println!();

    // Aggregate
    println!("Aggregate:");
    println!("  Total size:     {} ({:.2} GB)", format_size(stats.total.total_size_bytes), stats.total.total_size_bytes as f64 / 1_073_741_824.0);
    println!("  Total items:    {}", stats.total.total_items);
    println!();

    // Per system
    for (key, sys) in &stats.total.by_system {
        println!("{}:", sys.name);
        println!("  Size:           {} ({:.2} GB)", format_size(sys.size_bytes), sys.size_bytes as f64 / 1_073_741_824.0);
        println!("  Items:          {}", sys.item_count);
        println!("  Percentage:     {:.1}%", sys.percentage);
        println!();
    }

    // Detailed stats if available
    if let Some(bv) = &stats.build_versions {
        println!("Build Versions Detail:");
        println!("  Total versions:      {}", bv.total_versions);
        println!("  Successful:          {}", bv.successful_versions);
        println!("  Failed:              {}", bv.failed_versions);
        println!("  Current:             {:?}", bv.current_version);
        println!("  Previous:            {:?}", bv.previous_version);
        println!("  Staging size:        {}", format_size(bv.staging_size_bytes));
        println!("  Versions size:       {}", format_size(bv.versions_size_bytes));
        println!();
    }

    if let Some(pc) = &stats.pipeline_cache {
        println!("Pipeline Cache Detail:");
        println!("  Total caches:        {}", pc.total_caches);
        println!("  Total size:          {}", format_size(pc.total_size_bytes));
        println!("  By GPU:              {}", pc.by_gpu.len());
        println!();
    }

    if let Some(os) = &stats.object_store {
        println!("Object Store Detail:");
        println!("  Total objects:       {}", os.total_objects);
        println!("  Total size:          {}", format_size(os.total_size_bytes));
        println!("  Total refs:          {}", os.total_refs);
        println!("  Unreferenced:        {}", os.unreferenced_objects);
        println!();
    }

    if let Some(ls) = &stats.lift_store {
        println!("Lift Store Detail:");
        println!("  Total lifts:         {}", ls.total_lifts);
        println!("  Valid lifts:         {}", ls.valid_lifts);
        println!("  Invalid lifts:       {}", ls.invalid_lifts);
        println!("  Total output size:   {}", format_size(ls.total_output_size_bytes));
        println!();
    }

    if let Some(hl) = &stats.hardlink_store {
        println!("Hard-link Store Detail:");
        println!("  Unique content:      {}", hl.total_unique_content);
        println!("  Total references:    {}", hl.total_references);
        println!("  Total size:          {}", format_size(hl.total_size_bytes));
        println!("  Deduplication ratio: {:.2}", hl.deduplication_ratio);
        println!();
    }
}

fn print_cleanup_result(result: &CleanupResult) {
    println!("Cleanup Result");
    println!("==============");
    println!("Dry run:         {}", result.dry_run);
    println!("Timestamp:       {}", chrono::DateTime::from_timestamp(result.timestamp_unix as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "unknown".to_string()));
    println!("Bytes freed:     {}", format_size(result.bytes_freed));
    println!();

    if result.items_removed.is_empty() {
        println!("No items removed.");
    } else {
        println!("Items removed per system:");
        for (system, count) in &result.items_removed {
            println!("  {}: {}", system, count);
        }
    }

    if !result.errors.is_empty() {
        println!();
        println!("Errors:");
        for error in &result.errors {
            println!("  - {}", error);
        }
    }
}

fn print_disk_space(manager: &StorageManager) -> Result<()> {
    println!("Disk Space");
    println!("==========");

    let roots = [
        ("Versions", &manager.versions_root),
        ("Pipeline Cache", &manager.pipeline_cache_root),
        ("Object Store", &manager.object_store_root),
        ("Lift Store", &manager.lift_store_root),
        ("Hard-link Store", &manager.hardlink_store_root),
    ];

    for (name, path) in roots {
        if path.exists() {
            let size = crate::storage_manager::dir_size(path)?;
            println!("{}: {} ({})", name, path.display(), format_size(size));
        } else {
            println!("{}: {} (not found)", name, path.display());
        }
    }

    // Try to get free space
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::ffi::OsStr;
        let wide: Vec<u16> = OsStr::new(&manager.versions_root).encode_wide().chain(Some(0)).collect();
        let mut free_bytes: u64 = 0;
        let mut total_bytes: u64 = 0;
        let mut total_free_bytes: u64 = 0;
        unsafe {
            let success = windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_bytes,
                &mut total_bytes,
                &mut total_free_bytes,
            );
            if success != 0 {
                println!();
                println!("Free space:      {}", format_size(free_bytes));
                println!("Total space:     {}", format_size(total_bytes));
            }
        }
    }

    Ok(())
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
    fn test_format_size() {
        assert_eq!(format_size(0), "0.0 B");
        assert_eq!(format_size(512), "512.0 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }
}