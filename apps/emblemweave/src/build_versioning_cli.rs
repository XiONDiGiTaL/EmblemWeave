//! CLI commands for Build Versioning and Rollback

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::build_versioning::{BuildVersionManager, BuildVersioningConfig, BuildIdentity};
use crate::toolchain_manager::{ConcurrencyInfo, BuildConfig};

#[derive(Parser)]
#[command(name = "ew-build-version", about = "EmblemWeave Build Versioning CLI")]
pub struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Root directory for build versions
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

    /// Validate before promote
    #[arg(long, default_value = "true")]
    validate: bool,

    /// Custom validation command
    #[arg(long)]
    validation_cmd: Option<String>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a new build version
    Start {
        /// Build ID (UUID)
        #[arg(long)]
        build_id: String,

        /// Git commit hash
        #[arg(long)]
        git_commit: String,

        /// Build configuration (Release/Debug)
        #[arg(long, default_value = "Release")]
        config: String,

        /// Toolchain fingerprint
        #[arg(long)]
        fingerprint: String,

        /// Number of jobs
        #[arg(long, default_value = "8")]
        jobs: usize,

        /// Enable sccache
        #[arg(long, default_value = "false")]
        sccache: bool,

        /// Incremental build
        #[arg(long, default_value = "false")]
        incremental: bool,
    },

    /// Complete a build (success or failure)
    Complete {
        /// Build timestamp (from start)
        timestamp: u64,

        /// Success or failure
        #[arg(long)]
        success: bool,

        /// Build duration in milliseconds
        #[arg(long, default_value = "0")]
        duration_ms: u64,

        /// Error message (if failed)
        #[arg(long)]
        error: Option<String>,
    },

    /// Promote a build to current
    Promote {
        /// Build timestamp
        timestamp: u64,
    },

    /// Rollback to a specific build
    Rollback {
        /// Build timestamp (optional, uses previous if not specified)
        timestamp: Option<u64>,
    },

    /// List all build versions
    List {
        /// Show only successful builds
        #[arg(long)]
        successful: bool,
    },

    /// Show current build
    Current,

    /// Show previous build
    Previous,

    /// Show disk usage
    Usage,

    /// Cleanup old versions
    Cleanup,

    /// Force cleanup (keep only N successful)
    ForceCleanup {
        /// Number of successful builds to keep
        #[arg(default_value = "3")]
        keep: usize,
    },

    /// Export build history to JSON
    Export {
        /// Output file
        #[arg(default_value = "build_history.json")]
        output: PathBuf,
    },

    /// Validate a build
    Validate {
        /// Build timestamp
        timestamp: u64,
    },

    /// Rebuild metadata from existing directories
    RebuildMetadata,
}

pub fn run(cli: Cli) -> Result<()> {
    let config = BuildVersioningConfig {
        versions_root: cli.versions_root,
        max_versions: cli.max_versions,
        min_successful_to_keep: cli.min_successful,
        create_symlinks: cli.symlinks,
        validate_before_promote: cli.validate,
        validation_cmd: cli.validation_cmd,
    };

    let mut manager = BuildVersionManager::new(config)?;

    match cli.command {
        Commands::Start { build_id, git_commit, config, fingerprint, jobs, sccache, incremental } => {
            let concurrency_info = ConcurrencyInfo {
                total_memory_mb: 32768,
                available_memory_mb: 16384,
                cpu_cores: 16,
                estimated_per_job_mb: 1500,
                calculated_jobs: jobs,
                reserved_memory_mb: 2048,
            };

            let build_config = BuildConfig {
                jobs,
                memory_limit_mb: None,
                sccache_enabled: sccache,
                incremental,
                target: config.clone(),
                features: vec![],
            };

            let identity = manager.start_build(build_id, git_commit, config, fingerprint, concurrency_info, build_config)?;
            println!("Started build: {}", identity.timestamp_unix);
            println!("Staging dir: {}", manager.get_staging_dir(&identity).display());
            println!("Build ID: {}", identity.build_id);
        }

        Commands::Complete { timestamp, success, duration_ms, error } => {
            if let Some(identity) = manager.complete_build(timestamp, success, duration_ms, error)? {
                println!("Build completed: status={:?}, duration={}ms", identity.status, identity.duration_ms.unwrap_or(0));
            } else {
                eprintln!("Build not found: {}", timestamp);
                std::process::exit(1);
            }
        }

        Commands::Promote { timestamp } => {
            let identity = manager.promote_build(timestamp)?;
            println!("Promoted build: {}", identity.timestamp_unix);
            println!("Status: {:?}", identity.status);
            if let Some(current) = manager.get_current() {
                println!("Current build: {}", current.timestamp_unix);
            }
        }

        Commands::Rollback { timestamp } => {
            let identity = if let Some(ts) = timestamp {
                manager.rollback(ts)?
            } else {
                match manager.rollback_to_previous()? {
                    Some(id) => id,
                    None => {
                        eprintln!("No previous build to rollback to");
                        std::process::exit(1);
                    }
                }
            };
            println!("Rolled back to: {}", identity.timestamp_unix);
            println!("Status: {:?}", identity.status);
        }

        Commands::List { successful } => {
            let versions = if successful {
                manager.list_successful_versions()
            } else {
                manager.list_versions()
            };

            if versions.is_empty() {
                println!("No build versions found");
                return Ok(());
            }

            println!("{:<20} {:<10} {:<12} {:<10} {:<15} {:<10}", "TIMESTAMP", "CONFIG", "COMMIT", "STATUS", "DURATION", "BUILD_ID");
            println!("{}", "-".repeat(90));

            for v in versions {
                let duration = v.duration_ms.map(|d| format!("{}ms", d)).unwrap_or_else(|| "N/A".to_string());
                println!("{:<20} {:<10} {:<12} {:<10} {:<15} {:<10}",
                    v.timestamp_unix,
                    v.config,
                    &v.git_commit_short,
                    format!("{:?}", v.status),
                    duration,
                    &v.build_id[..10.min(v.build_id.len())]
                );
            }
        }

        Commands::Current => {
            if let Some(current) = manager.get_current() {
                print_build_identity(&current);
            } else {
                println!("No current build");
            }
        }

        Commands::Previous => {
            if let Some(previous) = manager.get_previous() {
                print_build_identity(&previous);
            } else {
                println!("No previous build");
            }
        }

        Commands::Usage => {
            let usage = manager.get_disk_usage()?;
            println!("Versions root: {}", usage.versions_root.display());
            println!("Total versions: {}", usage.version_count);
            println!("Successful: {}", usage.successful_count);
            println!("Total disk usage: {} bytes ({:.2} MB)", usage.total_bytes, usage.total_bytes as f64 / 1_048_576.0);
        }

        Commands::Cleanup => {
            let removed = manager.cleanup_old_versions()?;
            println!("Cleaned up {} old versions", removed);
        }

        Commands::ForceCleanup { keep } => {
            let removed = manager.force_cleanup(keep)?;
            println!("Force cleaned up {} old versions (kept {} successful)", removed, keep);
        }

        Commands::Export { output } => {
            manager.export_history(&output)?;
            println!("Exported build history to {}", output.display());
        }

        Commands::Validate { timestamp } => {
            let result = manager.validate_build(timestamp)?;
            println!("Validation: {}", if result.passed { "PASSED" } else { "FAILED" });
            println!("Duration: {}ms", result.duration_ms);
            println!("Output:\n{}", result.output);
        }

        Commands::RebuildMetadata => {
            manager.rebuild_metadata()?;
            println!("Metadata rebuilt from existing directories");
        }
    }

    Ok(())
}

fn print_build_identity(identity: &BuildIdentity) {
    println!("Build ID:       {}", identity.build_id);
    println!("Timestamp:      {} ({})", identity.timestamp, identity.timestamp_unix);
    println!("Git Commit:     {} ({})", identity.git_commit, identity.git_commit_short);
    println!("Config:         {}", identity.config);
    println!("Status:         {:?}", identity.status);
    println!("Toolchain:      {}", identity.toolchain_fingerprint);
    println!("Jobs:           {}", identity.build_config.jobs);
    println!("SCCACHE:        {}", identity.build_config.sccache_enabled);
    println!("Incremental:    {}", identity.build_config.incremental);
    if let Some(duration) = identity.duration_ms {
        println!("Duration:       {}ms", duration);
    }
    if let Some(error) = &identity.error {
        println!("Error:          {}", error);
    }
    if let Some(validation) = &identity.validation {
        println!("Validation:     {} ({}ms)", if validation.passed { "PASSED" } else { "FAILED" }, validation.duration_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_cli_start_complete_promote() {
        let dir = tempdir().unwrap();
        let config = BuildVersioningConfig {
            versions_root: dir.path().join("versions"),
            max_versions: 10,
            min_successful_to_keep: 2,
            create_symlinks: false,
            validate_before_promote: false,
            validation_cmd: None,
        };

        let mut manager = BuildVersionManager::new(config).unwrap();

        let identity = manager.start_build(
            "test-1".to_string(),
            "abc123".to_string(),
            "Release".to_string(),
            "fp".to_string(),
            ConcurrencyInfo {
                total_memory_mb: 32768,
                available_memory_mb: 16384,
                cpu_cores: 16,
                estimated_per_job_mb: 1500,
                calculated_jobs: 8,
                reserved_memory_mb: 2048,
            },
            BuildConfig {
                jobs: 8,
                memory_limit_mb: None,
                sccache_enabled: false,
                incremental: false,
                target: "Release".to_string(),
                features: vec![],
            },
        ).unwrap();

        manager.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();

        let staging = manager.get_staging_dir(&identity);
        let build_dir = staging.join("build").join("release");
        std::fs::create_dir_all(&build_dir).unwrap();
        std::fs::write(build_dir.join("nxrt_gfx_demo.exe"), b"dummy").unwrap();

        manager.promote_build(identity.timestamp_unix).unwrap();

        assert!(manager.get_current().is_some());
        assert_eq!(manager.get_current().unwrap().timestamp_unix, identity.timestamp_unix);
    }
}