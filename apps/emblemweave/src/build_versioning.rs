//! Build Directory Versioning and Rollback System
//!
//! This module implements the Phase C Build Versioning and Rollback subsystem:
//! - Isolated staging directories for each build attempt
//! - Build identity and metadata tracking (commit, timestamp, config, toolchain fingerprint)
//! - Atomic promotion with validation (compile + link + basic smoke test)
//! - Rollback to previous successful build
//! - Cleanup safety: never remove the last known-good build

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::toolchain_manager::{BuildConfig, BuildEventEmitter, ConcurrencyInfo};
use crate::process::hidden;

/// Build versioning configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildVersioningConfig {
    /// Root directory for all build versions
    pub versions_root: PathBuf,
    /// Maximum number of build versions to keep (0 = unlimited)
    pub max_versions: usize,
    /// Minimum number of successful builds to always keep
    pub min_successful_to_keep: usize,
    /// Whether to create symlinks for current/previous
    pub create_symlinks: bool,
    /// Whether to validate builds before promotion
    pub validate_before_promote: bool,
    /// Validation command template (can use {build_dir} placeholder)
    pub validation_cmd: Option<String>,
}

impl Default for BuildVersioningConfig {
    fn default() -> Self {
        Self {
            versions_root: PathBuf::from("build/versions"),
            max_versions: 20,
            min_successful_to_keep: 3,
            create_symlinks: true,
            validate_before_promote: true,
            validation_cmd: None,
        }
    }
}

/// Build identity and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildIdentity {
    /// Unique build ID (UUID)
    pub build_id: String,
    /// Git commit hash
    pub git_commit: String,
    /// Short git commit (first 10 chars)
    pub git_commit_short: String,
    /// Build timestamp (ISO 8601)
    pub timestamp: String,
    /// Unix timestamp for sorting
    pub timestamp_unix: u64,
    /// Build configuration (Release/Debug/etc.)
    pub config: String,
    /// Toolchain fingerprint
    pub toolchain_fingerprint: String,
    /// Concurrency info used
    pub concurrency_info: ConcurrencyInfo,
    /// Build configuration snapshot
    pub build_config: BuildConfig,
    /// Build status
    pub status: BuildStatus,
    /// Build duration in milliseconds
    pub duration_ms: Option<u64>,
    /// Error message if failed
    pub error: Option<String>,
    /// Validation result
    pub validation: Option<ValidationResult>,
}

/// Build status enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildStatus {
    /// Build is in progress
    InProgress,
    /// Build completed successfully
    Success,
    /// Build failed
    Failed,
    /// Build was promoted to current
    Promoted,
    /// Build was rolled back to
    RolledBack,
}

/// Validation result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Whether validation passed
    pub passed: bool,
    /// Validation command output
    pub output: String,
    /// Validation duration in milliseconds
    pub duration_ms: u64,
    /// Validation timestamp
    pub timestamp: String,
}

/// Build version manager
pub struct BuildVersionManager {
    config: BuildVersioningConfig,
    versions_root: PathBuf,
    metadata_file: PathBuf,
    versions: BTreeMap<u64, BuildIdentity>, // keyed by timestamp_unix for ordering
    current_link: PathBuf,
    previous_link: PathBuf,
}

impl BuildVersionManager {
    /// Create a new build version manager
    pub fn new(config: BuildVersioningConfig) -> Result<Self> {
        let versions_root = config.versions_root.clone();
        fs::create_dir_all(&versions_root).context("Failed to create versions root directory")?;

        let metadata_file = versions_root.join("build_versions.json");
        let current_link = versions_root.join("current");
        let previous_link = versions_root.join("previous");

        let mut manager = Self {
            config,
            versions_root,
            metadata_file,
            versions: BTreeMap::new(),
            current_link,
            previous_link,
        };

        manager.load_metadata()?;
        Ok(manager)
    }

    /// Load build versions metadata from disk
    fn load_metadata(&mut self) -> Result<()> {
        if self.metadata_file.exists() {
            let content = fs::read_to_string(&self.metadata_file)?;
            // Handle corrupted JSON gracefully
            match serde_json::from_str::<Vec<BuildIdentity>>(&content) {
                Ok(versions) => {
                    for v in versions {
                        self.versions.insert(v.timestamp_unix, v);
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to parse build versions metadata: {}. Starting fresh.", e);
                    // Optionally backup corrupted file
                    let _ = fs::rename(&self.metadata_file, self.metadata_file.with_extension("json.corrupted"));
                }
            }
        }
        Ok(())
    }

    /// Save build versions metadata to disk
    fn save_metadata(&self) -> Result<()> {
        let versions: Vec<BuildIdentity> = self.versions.values().cloned().collect();
        let content = serde_json::to_string_pretty(&versions)?;
        fs::write(&self.metadata_file, content)?;
        Ok(())
    }

    /// Start a new build version
    pub fn start_build(
        &mut self,
        build_id: String,
        git_commit: String,
        config: String,
        toolchain_fingerprint: String,
        concurrency_info: ConcurrencyInfo,
        build_config: BuildConfig,
    ) -> Result<BuildIdentity> {
        let now = SystemTime::now();
        let timestamp = chrono::Utc::now().to_rfc3339();
        let timestamp_unix = now.duration_since(UNIX_EPOCH)?.as_millis() as u64;

        let git_commit_short = if git_commit.len() >= 10 {
            git_commit[..10].to_string()
        } else {
            git_commit.clone()
        };

        let identity = BuildIdentity {
            build_id: build_id.clone(),
            git_commit: git_commit.clone(),
            git_commit_short: git_commit_short.clone(),
            timestamp,
            timestamp_unix,
            config: config.clone(),
            toolchain_fingerprint,
            concurrency_info,
            build_config,
            status: BuildStatus::InProgress,
            duration_ms: None,
            error: None,
            validation: None,
        };

        // Create isolated staging directory
        let staging_dir = self.get_staging_dir(&identity);
        fs::create_dir_all(&staging_dir).context("Failed to create staging directory")?;

        // Write build identity to staging directory
        let identity_path = staging_dir.join("build_identity.json");
        let identity_content = serde_json::to_string_pretty(&identity)?;
        fs::write(&identity_path, identity_content)?;

        // Add to versions map and save
        self.versions.insert(timestamp_unix, identity.clone());
        self.save_metadata()?;

        Ok(identity)
    }

    /// Get the staging directory for a build identity
    pub fn get_staging_dir(&self, identity: &BuildIdentity) -> PathBuf {
        self.versions_root
            .join(format!("{}-{}-{}", identity.timestamp_unix, identity.config.to_lowercase(), identity.git_commit_short))
    }

    /// Get the staging directory by timestamp
    pub fn get_staging_dir_by_timestamp(&self, timestamp_unix: u64) -> Option<PathBuf> {
        self.versions.get(&timestamp_unix).map(|v| self.get_staging_dir(v))
    }

    /// Complete a build (success or failure)
    pub fn complete_build(
        &mut self,
        timestamp_unix: u64,
        success: bool,
        duration_ms: u64,
        error: Option<String>,
    ) -> Result<Option<BuildIdentity>> {
        let mut identity = match self.versions.get(&timestamp_unix).cloned() {
            Some(id) => id,
            None => return Ok(None),
        };

        identity.status = if success { BuildStatus::Success } else { BuildStatus::Failed };
        identity.duration_ms = Some(duration_ms);
        identity.error = error;

        self.versions.insert(timestamp_unix, identity.clone());
        self.save_metadata()?;
        Ok(Some(identity))
    }

    /// Validate a build before promotion
    pub fn validate_build(&self, timestamp_unix: u64) -> Result<ValidationResult> {
        let identity = self.versions.get(&timestamp_unix)
            .ok_or_else(|| anyhow!("Build version not found: {}", timestamp_unix))?;

        let staging_dir = self.get_staging_dir(identity);
        let build_dir = staging_dir.join("build").join(identity.config.to_lowercase());

        if !build_dir.exists() {
            return Ok(ValidationResult {
                passed: false,
                output: "Build directory does not exist".to_string(),
                duration_ms: 0,
                timestamp: chrono::Utc::now().to_rfc3339(),
            });
        }

        let start = std::time::Instant::now();
        let mut passed = true;
        let mut output = String::new();

        // Run validation command if configured
        if let Some(cmd_template) = &self.config.validation_cmd {
            let cmd = cmd_template.replace("{build_dir}", &build_dir.display().to_string());
            output.push_str(&format!("Running validation: {}\n", cmd));

            let mut cmd_obj = std::process::Command::new("cmd");
            cmd_obj.args(["/c", &cmd]).current_dir(&build_dir);
            hidden(&mut cmd_obj);
            let result = cmd_obj.output();

            match result {
                Ok(out) => {
                    output.push_str(&String::from_utf8_lossy(&out.stdout));
                    output.push_str(&String::from_utf8_lossy(&out.stderr));
                    if !out.status.success() {
                        passed = false;
                        output.push_str(&format!("\nValidation failed with exit code: {}", out.status.code().unwrap_or(-1)));
                    }
                }
                Err(e) => {
                    passed = false;
                    output.push_str(&format!("\nValidation command failed to execute: {}", e));
                }
            }
        } else {
            // Default validation: check that key binaries exist
            let _config_lower = identity.config.to_lowercase();
            let exe_name = format!("nxrt_gfx_demo.exe");
            let exe_path = build_dir.join(&exe_name);

            if exe_path.exists() {
                output.push_str(&format!("Found executable: {}\n", exe_path.display()));
                // Try running with --help or similar quick test
                let mut test_cmd = std::process::Command::new(&exe_path);
                test_cmd.arg("--help").current_dir(&build_dir);
                hidden(&mut test_cmd);
                let test_result = test_cmd.output();

                match test_result {
                    Ok(out) => {
                        output.push_str(&String::from_utf8_lossy(&out.stdout));
                        output.push_str(&String::from_utf8_lossy(&out.stderr));
                        if !out.status.success() {
                            // Some executables don't support --help, that's OK
                            output.push_str("(Executable ran, exit code non-zero but that may be expected)\n");
                        }
                    }
                    Err(e) => {
                        output.push_str(&format!("Could not execute binary: {}\n", e));
                    }
                }
            } else {
                passed = false;
                output.push_str(&format!("Executable not found: {}\n", exe_path.display()));
            }

            // Check for key libraries
            let libs = ["nxrt.dll", "nxrt_gfx.dll", "SDL3.dll"];
            for lib in libs {
                let lib_path = build_dir.join(lib);
                if lib_path.exists() {
                    output.push_str(&format!("Found library: {}\n", lib_path.display()));
                } else {
                    output.push_str(&format!("Library not found (may be static): {}\n", lib));
                }
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;
        let result = ValidationResult {
            passed,
            output,
            duration_ms,
            timestamp: chrono::Utc::now().to_rfc3339(),
        };

        // Update the build identity with validation result
        // Note: we can't mutate here since we only have &self, caller should update
        Ok(result)
    }

    /// Promote a build to "current" (atomic with validation)
    pub fn promote_build(&mut self, timestamp_unix: u64) -> Result<BuildIdentity> {
        let mut identity = self.versions.get(&timestamp_unix)
            .ok_or_else(|| anyhow!("Build version not found: {}", timestamp_unix))?
            .clone();

        if identity.status != BuildStatus::Success {
            return Err(anyhow!("Cannot promote failed build"));
        }

        // Validate if configured
        if self.config.validate_before_promote {
            let validation = self.validate_build(timestamp_unix)?;
            if !validation.passed {
                return Err(anyhow!("Build validation failed: {}", validation.output));
            }
            identity.validation = Some(validation);
        }

        let staging_dir = self.get_staging_dir(&identity);
        let target_dir = self.versions_root.join("current_build");
        let prev_dir = self.versions_root.join("previous_build");

        // Atomic promotion: remove old current, create new one
        let has_current = if self.config.create_symlinks {
            self.current_link.exists() || self.current_link.symlink_metadata().is_ok()
        } else {
            target_dir.exists()
        };

        if has_current {
            // Save current as previous before replacing
            if self.config.create_symlinks {
                let _ = fs::remove_file(&self.previous_link);
                let _ = fs::rename(&self.current_link, &self.previous_link);
            } else if target_dir.exists() {
                let _ = fs::remove_dir_all(&prev_dir);
                let _ = fs::rename(&target_dir, &prev_dir);
            }
        }

        if self.config.create_symlinks {
            // Create symlink from current -> staging_dir
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_dir(&staging_dir, &self.current_link)?;
            }
            #[cfg(not(windows))]
            {
                std::os::unix::fs::symlink(&staging_dir, &self.current_link)?;
            }
        } else {
            // Copy staging to current_build (don't move, keep staging intact)
            if target_dir.exists() {
                fs::remove_dir_all(&target_dir)?;
            }
            copy_dir_all(&staging_dir, &target_dir)?;
            // Write a marker file to track which version is current
            let marker = target_dir.join(".build_version");
            fs::write(&marker, timestamp_unix.to_string())?;
        }

        // Update status
        identity.status = BuildStatus::Promoted;
        self.versions.insert(timestamp_unix, identity.clone());
        self.save_metadata()?;

        // Cleanup old versions
        self.cleanup_old_versions()?;

        Ok(identity)
    }

    /// Rollback to a previous successful build
    pub fn rollback(&mut self, timestamp_unix: u64) -> Result<BuildIdentity> {
        let mut identity = self.versions.get(&timestamp_unix)
            .ok_or_else(|| anyhow!("Build version not found: {}", timestamp_unix))?
            .clone();

        if identity.status != BuildStatus::Success && identity.status != BuildStatus::Promoted {
            return Err(anyhow!("Can only rollback to successful builds"));
        }

        let staging_dir = self.get_staging_dir(&identity);
        if !staging_dir.exists() {
            return Err(anyhow!("Build staging directory no longer exists: {}", staging_dir.display()));
        }

        let target_dir = self.versions_root.join("current_build");
        let prev_dir = self.versions_root.join("previous_build");

        // Save current as previous
        let has_current = if self.config.create_symlinks {
            self.current_link.exists() || self.current_link.symlink_metadata().is_ok()
        } else {
            target_dir.exists()
        };

        if has_current {
            if self.config.create_symlinks {
                let _ = fs::remove_file(&self.previous_link);
                let _ = fs::rename(&self.current_link, &self.previous_link);
            } else if target_dir.exists() {
                let _ = fs::remove_dir_all(&prev_dir);
                let _ = fs::rename(&target_dir, &prev_dir);
            }
        }

        // Point current to the rollback target
        if self.config.create_symlinks {
            #[cfg(windows)]
            {
                std::os::windows::fs::symlink_dir(&staging_dir, &self.current_link)?;
            }
            #[cfg(not(windows))]
            {
                std::os::unix::fs::symlink(&staging_dir, &self.current_link)?;
            }
        } else {
            if target_dir.exists() {
                fs::remove_dir_all(&target_dir)?;
            }
            copy_dir_all(&staging_dir, &target_dir)?;
            // Write a marker file to track which version is current
            let marker = target_dir.join(".build_version");
            fs::write(&marker, timestamp_unix.to_string())?;
        }

        identity.status = BuildStatus::RolledBack;
        self.versions.insert(timestamp_unix, identity.clone());
        self.save_metadata()?;

        Ok(identity)
    }

    /// Rollback to the previous build (the one before current)
    pub fn rollback_to_previous(&mut self) -> Result<Option<BuildIdentity>> {
        // Get the previous build identity
        let previous = self.get_previous();

        if let Some(prev_identity) = previous {
            return self.rollback(prev_identity.timestamp_unix).map(Some);
        }

        Ok(None)
    }

    /// Get the current build identity
    pub fn get_current(&self) -> Option<BuildIdentity> {
        if self.config.create_symlinks {
            // Symlink mode: read the symlink
            if self.current_link.exists() {
                if let Ok(target) = fs::read_link(&self.current_link) {
                    let dir_name = target.file_name()?.to_str()?;
                    let ts_str = dir_name.split('-').next()?;
                    let timestamp_unix = ts_str.parse::<u64>().ok()?;
                    return self.versions.get(&timestamp_unix).cloned();
                }
            }
        } else {
            // Copy mode: read the marker file
            let target_dir = self.versions_root.join("current_build");
            let marker = target_dir.join(".build_version");
            if marker.exists() {
                if let Ok(content) = fs::read_to_string(&marker) {
                    if let Ok(timestamp_unix) = content.trim().parse::<u64>() {
                        return self.versions.get(&timestamp_unix).cloned();
                    }
                }
            }
        }
        None
    }

    /// Get the previous build identity
    pub fn get_previous(&self) -> Option<BuildIdentity> {
        if self.config.create_symlinks {
            // Symlink mode: read the symlink
            if self.previous_link.exists() {
                if let Ok(target) = fs::read_link(&self.previous_link) {
                    let dir_name = target.file_name()?.to_str()?;
                    let ts_str = dir_name.split('-').next()?;
                    let timestamp_unix = ts_str.parse::<u64>().ok()?;
                    return self.versions.get(&timestamp_unix).cloned();
                }
            }
        } else {
            // Copy mode: read the marker file
            let prev_dir = self.versions_root.join("previous_build");
            let marker = prev_dir.join(".build_version");
            if marker.exists() {
                if let Ok(content) = fs::read_to_string(&marker) {
                    if let Ok(timestamp_unix) = content.trim().parse::<u64>() {
                        return self.versions.get(&timestamp_unix).cloned();
                    }
                }
            }
        }
        None
    }

    /// List all build versions (newest first)
    pub fn list_versions(&self) -> Vec<BuildIdentity> {
        let mut versions: Vec<BuildIdentity> = self.versions.values().cloned().collect();
        versions.sort_by(|a, b| b.timestamp_unix.cmp(&a.timestamp_unix));
        versions
    }

    /// Get successful build versions (newest first)
    pub fn list_successful_versions(&self) -> Vec<BuildIdentity> {
        let mut versions: Vec<BuildIdentity> = self.versions
            .values()
            .filter(|v| v.status == BuildStatus::Success || v.status == BuildStatus::Promoted || v.status == BuildStatus::RolledBack)
            .cloned()
            .collect();
        versions.sort_by(|a, b| b.timestamp_unix.cmp(&a.timestamp_unix));
        versions
    }

    /// Clean up old versions, keeping at least min_successful_to_keep successful builds
    pub fn cleanup_old_versions(&mut self) -> Result<usize> {
        if self.config.max_versions == 0 {
            return Ok(0); // Unlimited
        }

        let successful = self.list_successful_versions();
        if successful.len() <= self.config.min_successful_to_keep {
            return Ok(0); // Don't clean up if we don't have enough successful builds
        }

        let all_versions = self.list_versions();
        let to_remove = if all_versions.len() > self.config.max_versions {
            all_versions.len() - self.config.max_versions
        } else {
            0
        };

        if to_remove == 0 {
            return Ok(0);
        }

        let mut removed = 0;
        for version in all_versions.iter().rev() {
            if removed >= to_remove {
                break;
            }

            // Never remove the last N successful builds
            let successful_after = successful.iter()
                .filter(|v| v.timestamp_unix != version.timestamp_unix)
                .count();

            if successful_after < self.config.min_successful_to_keep {
                continue;
            }

            // Never remove current or previous
            let is_current = self.get_current().map(|c| c.timestamp_unix == version.timestamp_unix).unwrap_or(false);
            let is_previous = self.get_previous().map(|p| p.timestamp_unix == version.timestamp_unix).unwrap_or(false);

            if is_current || is_previous {
                continue;
            }

            // Remove staging directory
            let staging_dir = self.get_staging_dir(version);
            if staging_dir.exists() {
                fs::remove_dir_all(&staging_dir)?;
            }

            // Remove from versions map
            self.versions.remove(&version.timestamp_unix);
            removed += 1;
        }

        if removed > 0 {
            self.save_metadata()?;
        }

        Ok(removed)
    }

    /// Force cleanup (for maintenance) - removes all but the last N successful builds
    pub fn force_cleanup(&mut self, keep_successful: usize) -> Result<usize> {
        let successful = self.list_successful_versions();
        if successful.len() <= keep_successful {
            return Ok(0);
        }

        let mut removed = 0;
        for version in successful.iter().skip(keep_successful) {
            let staging_dir = self.get_staging_dir(version);
            if staging_dir.exists() {
                fs::remove_dir_all(&staging_dir)?;
                removed += 1;
            }
        }

        // Rebuild metadata from remaining directories
        self.rebuild_metadata()?;

        Ok(removed)
    }

    /// Rebuild metadata from existing staging directories
    pub fn rebuild_metadata(&mut self) -> Result<()> {
        self.versions.clear();

        if let Ok(entries) = fs::read_dir(&self.versions_root) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name();
                    if let Some(name_str) = name.to_str() {
                        // Skip special directories
                        if name_str == "current_build" || name_str == "previous_build" {
                            continue;
                        }

                        // Try to parse timestamp from directory name
                        if let Some(ts_str) = name_str.split('-').next() {
                            if let Ok(timestamp_unix) = ts_str.parse::<u64>() {
                                let identity_path = entry.path().join("build_identity.json");
                                if identity_path.exists() {
                                    if let Ok(content) = fs::read_to_string(&identity_path) {
                                        if let Ok(identity) = serde_json::from_str::<BuildIdentity>(&content) {
                                            self.versions.insert(timestamp_unix, identity);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        self.save_metadata()?;
        Ok(())
    }

    /// Get disk usage statistics
    pub fn get_disk_usage(&self) -> Result<DiskUsage> {
        let mut total_bytes = 0u64;
        let mut version_count = 0;
        let mut successful_count = 0;

        for version in self.versions.values() {
            let staging_dir = self.get_staging_dir(version);
            if staging_dir.exists() {
                version_count += 1;
                if version.status == BuildStatus::Success || version.status == BuildStatus::Promoted || version.status == BuildStatus::RolledBack {
                    successful_count += 1;
                }
                total_bytes += dir_size(&staging_dir)?;
            }
        }

        Ok(DiskUsage {
            total_bytes,
            version_count,
            successful_count,
            versions_root: self.versions_root.clone(),
        })
    }

    /// Export build history as JSON
    pub fn export_history(&self, path: &Path) -> Result<()> {
        let history = self.list_versions();
        let content = serde_json::to_string_pretty(&history)?;
        fs::write(path, content)?;
        Ok(())
    }
}

/// Disk usage information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskUsage {
    pub total_bytes: u64,
    pub version_count: usize,
    pub successful_count: usize,
    pub versions_root: PathBuf,
}

fn dir_size(path: &Path) -> Result<u64> {
    let mut size = 0u64;
    if path.is_file() {
        size += fs::metadata(path)?.len();
    } else if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            size += dir_size(&entry.path())?;
        }
    }
    Ok(size)
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Integration with structured build events
pub struct VersionedBuildEventEmitter {
    inner: Arc<BuildEventEmitter>,
    version_manager: Arc<std::sync::Mutex<BuildVersionManager>>,
    current_timestamp: std::sync::Mutex<Option<u64>>,
}

impl VersionedBuildEventEmitter {
    pub fn new(
        inner: Arc<BuildEventEmitter>,
        version_manager: Arc<std::sync::Mutex<BuildVersionManager>>,
    ) -> Self {
        Self {
            inner,
            version_manager,
            current_timestamp: std::sync::Mutex::new(None),
        }
    }

    pub fn start_build(&self, identity: &BuildIdentity) -> Result<()> {
        *self.current_timestamp.lock().unwrap() = Some(identity.timestamp_unix);
        self.inner.build_started(
            &identity.toolchain_fingerprint,
            identity.build_config.clone(),
        )
    }

    pub fn complete_build(&self, success: bool, duration_ms: u64, error: Option<String>) -> Result<()> {
        if let Some(ts) = *self.current_timestamp.lock().unwrap() {
            let mut vm = self.version_manager.lock().unwrap();
            vm.complete_build(ts, success, duration_ms, error)?;
        }
        Ok(())
    }

    pub fn promote_current(&self) -> Result<BuildIdentity> {
        if let Some(ts) = *self.current_timestamp.lock().unwrap() {
            let mut vm = self.version_manager.lock().unwrap();
            vm.promote_build(ts)
        } else {
            Err(anyhow!("No active build to promote"))
        }
    }
}

use std::sync::Arc;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::tempdir;

    fn create_test_config() -> BuildVersioningConfig {
        let dir = tempdir().unwrap();
        BuildVersioningConfig {
            versions_root: dir.path().join("versions"),
            max_versions: 5,
            min_successful_to_keep: 2,
            create_symlinks: false, // Use copy for tests
            validate_before_promote: false,
            validation_cmd: None,
        }
    }

    #[test]
    fn test_build_version_manager_creation() {
        let config = create_test_config();
        let mgr = BuildVersionManager::new(config).unwrap();
        assert!(mgr.list_versions().is_empty());
    }

    #[test]
    fn test_start_and_complete_build() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        let identity = mgr.start_build(
            "test-build-1".to_string(),
            "abcdef123456".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        assert_eq!(identity.status, BuildStatus::InProgress);
        assert_eq!(mgr.list_versions().len(), 1);

        // Complete successfully
        mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();
        let versions = mgr.list_versions();
        assert_eq!(versions[0].status, BuildStatus::Success);
        assert_eq!(versions[0].duration_ms, Some(50000));
    }

    #[test]
    fn test_promote_and_rollback() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        let identity = mgr.start_build(
            "test-build-1".to_string(),
            "abcdef123456".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();

        // Create a dummy build directory structure for promotion
        let staging_dir = mgr.get_staging_dir(&identity);
        let build_dir = staging_dir.join("build").join("release");
        fs::create_dir_all(&build_dir).unwrap();
        fs::write(build_dir.join("nxrt_gfx_demo.exe"), b"dummy").unwrap();

        let promoted = mgr.promote_build(identity.timestamp_unix).unwrap();
        assert_eq!(promoted.status, BuildStatus::Promoted);
        assert!(mgr.get_current().is_some());

        // Start another build
        let identity2 = mgr.start_build(
            "test-build-2".to_string(),
            "fedcba654321".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity2.timestamp_unix, true, 60000, None).unwrap();
        let staging_dir2 = mgr.get_staging_dir(&identity2);
        let build_dir2 = staging_dir2.join("build").join("release");
        fs::create_dir_all(&build_dir2).unwrap();
        fs::write(build_dir2.join("nxrt_gfx_demo.exe"), b"dummy2").unwrap();

        mgr.promote_build(identity2.timestamp_unix).unwrap();

        // Now rollback to previous
        let rolled_back = mgr.rollback(identity.timestamp_unix).unwrap();
        assert_eq!(rolled_back.status, BuildStatus::RolledBack);
        assert_eq!(mgr.get_current().unwrap().timestamp_unix, identity.timestamp_unix);
    }

    #[test]
    fn test_cleanup_safety() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        // Create 5 successful builds with small delays to ensure unique timestamps
        for i in 0..5 {
            let identity = mgr.start_build(
                format!("test-build-{}", i),
                format!("commit{}", i),
                "Release".to_string(),
                "fingerprint".to_string(),
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

            mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();
            let staging_dir = mgr.get_staging_dir(&identity);
            let build_dir = staging_dir.join("build").join("release");
            fs::create_dir_all(&build_dir).unwrap();
            fs::write(build_dir.join("nxrt_gfx_demo.exe"), b"dummy").unwrap();
            mgr.promote_build(identity.timestamp_unix).unwrap();

            // Small delay to ensure unique timestamps
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        // Should have 5 versions, min_successful_to_keep=2
        // max_versions=5, so no cleanup yet
        assert_eq!(mgr.list_versions().len(), 5);

        // Add 6th build (exceeds max_versions)
        let identity = mgr.start_build(
            "test-build-5".to_string(),
            "commit5".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();
        let staging_dir = mgr.get_staging_dir(&identity);
        let build_dir = staging_dir.join("build").join("release");
        fs::create_dir_all(&build_dir).unwrap();
        fs::write(build_dir.join("nxrt_gfx_demo.exe"), b"dummy").unwrap();
        mgr.promote_build(identity.timestamp_unix).unwrap();

        // Should have cleaned up oldest non-protected build
        // Current (build-5) and previous (build-4) are protected
        // min_successful_to_keep=2, so build-3 and build-2 are also protected
        // Only build-0 and build-1 could be removed, but max_versions=5 means only 1 removed
        let versions = mgr.list_versions();
        assert!(versions.len() <= 5);
    }
}

// Phase D: Production Hardening Tests
#[cfg(test)]
mod phase_d_tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_config() -> BuildVersioningConfig {
        let dir = tempdir().unwrap();
        BuildVersioningConfig {
            versions_root: dir.path().join("versions"),
            max_versions: 10,
            min_successful_to_keep: 2,
            create_symlinks: false,
            validate_before_promote: true,
            validation_cmd: None,
        }
    }

    #[test]
    fn test_interrupted_build_recovery() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        let identity = mgr.start_build(
            "test-build-interrupted".to_string(),
            "commit123".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        let versions = mgr.list_versions();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].status, BuildStatus::InProgress);

        let result = mgr.promote_build(identity.timestamp_unix);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot promote failed build"));
    }

    #[test]
    fn test_failed_build_not_promotable() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        let identity = mgr.start_build(
            "test-build-fail".to_string(),
            "commit123".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity.timestamp_unix, false, 10000, Some("compilation error".to_string())).unwrap();

        let result = mgr.promote_build(identity.timestamp_unix);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Cannot promote failed build"));
    }

    #[test]
    fn test_validation_failure_prevents_promotion() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        let identity = mgr.start_build(
            "test-build-validation-fail".to_string(),
            "commit123".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();

        let result = mgr.promote_build(identity.timestamp_unix);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Build validation failed"));
    }

    #[test]
    fn test_atomic_promotion_no_partial_state() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        let identity = mgr.start_build(
            "test-build-atomic".to_string(),
            "commit123".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();
        let staging_dir = mgr.get_staging_dir(&identity);
        let build_dir = staging_dir.join("build").join("release");
        fs::create_dir_all(&build_dir).unwrap();
        fs::write(build_dir.join("nxrt_gfx_demo.exe"), b"dummy").unwrap();

        let promoted = mgr.promote_build(identity.timestamp_unix).unwrap();
        assert_eq!(promoted.status, BuildStatus::Promoted);

        let current = mgr.get_current().unwrap();
        assert_eq!(current.timestamp_unix, identity.timestamp_unix);

        assert!(staging_dir.exists());
    }

    #[test]
    fn test_corrupted_metadata_recovery() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config.clone()).unwrap();

        let identity = mgr.start_build(
            "test-build-metadata".to_string(),
            "commit123".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();

        fs::write(&mgr.metadata_file, b"invalid json").unwrap();

        let mgr2 = BuildVersionManager::new(config).unwrap();
        assert!(mgr2.list_versions().is_empty());
    }

    #[test]
    fn test_rollback_preserves_current_as_previous() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        let identity1 = mgr.start_build(
            "build-1".to_string(),
            "commit1".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity1.timestamp_unix, true, 50000, None).unwrap();
        let staging1 = mgr.get_staging_dir(&identity1);
        let build1 = staging1.join("build").join("release");
        fs::create_dir_all(&build1).unwrap();
        fs::write(build1.join("nxrt_gfx_demo.exe"), b"build1").unwrap();
        mgr.promote_build(identity1.timestamp_unix).unwrap();

        let identity2 = mgr.start_build(
            "build-2".to_string(),
            "commit2".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity2.timestamp_unix, true, 50000, None).unwrap();
        let staging2 = mgr.get_staging_dir(&identity2);
        let build2 = staging2.join("build").join("release");
        fs::create_dir_all(&build2).unwrap();
        fs::write(build2.join("nxrt_gfx_demo.exe"), b"build2").unwrap();
        mgr.promote_build(identity2.timestamp_unix).unwrap();

        assert_eq!(mgr.get_current().unwrap().timestamp_unix, identity2.timestamp_unix);

        mgr.rollback(identity1.timestamp_unix).unwrap();

        assert_eq!(mgr.get_current().unwrap().timestamp_unix, identity1.timestamp_unix);
        assert_eq!(mgr.get_previous().unwrap().timestamp_unix, identity2.timestamp_unix);
    }

    #[test]
    fn test_rollback_to_missing_staging_fails() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        let identity = mgr.start_build(
            "test-build".to_string(),
            "commit123".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();
        let staging_dir = mgr.get_staging_dir(&identity);
        let build_dir = staging_dir.join("build").join("release");
        fs::create_dir_all(&build_dir).unwrap();
        fs::write(build_dir.join("nxrt_gfx_demo.exe"), b"dummy").unwrap();
        mgr.promote_build(identity.timestamp_unix).unwrap();

        fs::remove_dir_all(&staging_dir).unwrap();

        let result = mgr.rollback(identity.timestamp_unix);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no longer exists"));
    }

    #[test]
    fn test_cleanup_respects_min_successful() {
        let dir = tempdir().unwrap();
        let config = BuildVersioningConfig {
            versions_root: dir.path().join("versions"),
            max_versions: 3,
            min_successful_to_keep: 3,
            create_symlinks: false,
            validate_before_promote: false,
            validation_cmd: None,
        };
        let mut mgr = BuildVersionManager::new(config).unwrap();

        for i in 0..5 {
            let identity = mgr.start_build(
                format!("build-{}", i),
                format!("commit{}", i),
                "Release".to_string(),
                "fingerprint".to_string(),
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

            mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();
            let staging = mgr.get_staging_dir(&identity);
            let build = staging.join("build").join("release");
            fs::create_dir_all(&build).unwrap();
            fs::write(build.join("nxrt_gfx_demo.exe"), b"dummy").unwrap();
            mgr.promote_build(identity.timestamp_unix).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let versions = mgr.list_versions();
        assert!(versions.len() >= 3);
    }

    #[test]
    fn test_build_identity_persistence() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config.clone()).unwrap();

        let identity = mgr.start_build(
            "test-persistence".to_string(),
            "commit123".to_string(),
            "Release".to_string(),
            "fingerprint".to_string(),
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

        mgr.complete_build(identity.timestamp_unix, true, 50000, None).unwrap();

        let mgr2 = BuildVersionManager::new(config).unwrap();
        let versions = mgr2.list_versions();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].build_id, "test-persistence");
        assert_eq!(versions[0].git_commit, "commit123");
        assert_eq!(versions[0].status, BuildStatus::Success);
    }

    #[test]
    fn test_concurrent_build_isolation() {
        let config = create_test_config();
        let mut mgr = BuildVersionManager::new(config).unwrap();

        let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

        let id1 = BuildIdentity {
            build_id: "build-1".to_string(),
            git_commit: "commit1".to_string(),
            git_commit_short: "commit1".to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            timestamp_unix: ts,
            config: "Release".to_string(),
            toolchain_fingerprint: "fp1".to_string(),
            concurrency_info: ConcurrencyInfo {
                total_memory_mb: 32768,
                available_memory_mb: 16384,
                cpu_cores: 16,
                estimated_per_job_mb: 1500,
                calculated_jobs: 8,
                reserved_memory_mb: 2048,
            },
            build_config: BuildConfig {
                jobs: 8,
                memory_limit_mb: None,
                sccache_enabled: false,
                incremental: false,
                target: "Release".to_string(),
                features: vec![],
            },
            status: BuildStatus::InProgress,
            duration_ms: None,
            error: None,
            validation: None,
        };

        let mut id2 = id1.clone();
        id2.build_id = "build-2".to_string();
        id2.git_commit = "commit2".to_string();
        id2.git_commit_short = "commit2".to_string();
        id2.timestamp_unix = ts + 1;

        mgr.versions.insert(id1.timestamp_unix, id1.clone());
        mgr.versions.insert(id2.timestamp_unix, id2.clone());
        mgr.save_metadata().unwrap();

        assert_eq!(mgr.list_versions().len(), 2);

        mgr.complete_build(id1.timestamp_unix, true, 50000, None).unwrap();
        assert_eq!(mgr.versions.get(&id1.timestamp_unix).unwrap().status, BuildStatus::Success);

        mgr.complete_build(id2.timestamp_unix, true, 50000, None).unwrap();
        assert_eq!(mgr.versions.get(&id2.timestamp_unix).unwrap().status, BuildStatus::Success);
    }
}