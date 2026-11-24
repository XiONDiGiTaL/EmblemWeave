//! Unified Toolchain Manager for EmblemWeave
//!
//! This module provides a comprehensive toolchain management system that handles:
//! - Discovery and validation of all required and optional tools
//! - Installation via package managers (winget, scoop, chocolatey) or direct downloads
//! - Verification via SHA-256 checksums from tools.lock
//! - Execution environment preparation with proper PATH and environment variables
//! - Structured build event protocol (JSON Lines)
//! - Adaptive memory-aware build concurrency

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, OnceLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::process::hidden;
use crate::toolchain::{self, Toolchain, VsInstall, Msvc, Sdk, Clang, Resolved};

/// The version of the tools.lock schema
pub const TOOLS_LOCK_VERSION: u32 = 1;

/// Minimum required versions for core toolchain components
pub const MIN_CLANG_MAJOR: u32 = 23;
pub const MIN_CMAKE_VERSION: (u32, u32) = (3, 25);
pub const MIN_VS_MAJOR: u32 = 17;

/// Tool categories for classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolCategory {
    /// Required for the build to succeed
    Required,
    /// Optional but recommended for features/performance
    Optional,
    /// Development/debugging tools
    Development,
}

/// Source of a tool
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "type")]
pub enum ToolSource {
    /// Windows Package Manager (winget)
    Winget { id: String },
    /// Scoop package manager
    Scoop { bucket: Option<String>, name: String },
    /// Chocolatey package manager
    Chocolatey { name: String },
    /// Direct download from URL
    Download { url: String, archive_type: ArchiveType },
    /// Bundled with the application
    Bundled { relative_path: String },
    /// System-provided (PATH, Windows SDK, etc.)
    System { hint: String },
}

/// Archive type for downloaded tools
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArchiveType {
    Zip,
    TarGz,
    TarXz,
    Msi,
    Exe,
}

/// Platform specification
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
    All,
}

/// Tool verification result
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerificationStatus {
    Verified,
    ChecksumMismatch,
    Missing,
    VersionMismatch,
    NotApplicable,
}

/// Individual tool specification in tools.lock
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    /// Unique identifier for the tool
    pub name: String,
    /// Human-readable name
    pub display_name: String,
    /// Tool category (required/optional/development)
    pub category: ToolCategory,
    /// Expected version (semver or version string)
    pub version: String,
    /// Minimum acceptable version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_version: Option<String>,
    /// Platform this tool applies to
    pub platform: Platform,
    /// Source for installation/acquisition
    pub source: ToolSource,
    /// Expected installation location (relative to toolchain root or absolute)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_path: Option<String>,
    /// SHA-256 checksum of the tool binary/archive
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// SHA-256 of the downloaded archive (for download sources)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archive_sha256: Option<String>,
    /// Command to verify version (e.g., "clang-cl --version")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_cmd: Option<Vec<String>>,
    /// Regex to extract version from version_cmd output
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_regex: Option<String>,
    /// Whether this tool must be on PATH
    #[serde(default)]
    pub on_path: bool,
    /// Environment variables to set when this tool is active
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env_vars: BTreeMap<String, String>,
    /// Dependencies on other tools (by name)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    /// Description for documentation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Complete tools.lock manifest
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsLock {
    /// Schema version
    pub version: u32,
    /// Timestamp of generation (ISO 8601)
    pub generated_at: String,
    /// Git commit hash if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    /// Platform this lock file targets
    pub platform: Platform,
    /// All tool specifications
    pub tools: Vec<ToolSpec>,
    /// Metadata about the toolchain
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// Result of tool verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolVerification {
    pub tool_name: String,
    pub status: VerificationStatus,
    pub found_path: Option<PathBuf>,
    pub found_version: Option<String>,
    pub expected_version: String,
    pub checksum_match: Option<bool>,
    pub error: Option<String>,
}

/// Build event types for structured protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum BuildEvent {
    BuildStarted {
        timestamp: String,
        build_id: String,
        toolchain_fingerprint: String,
        config: BuildConfig,
    },
    StageStarted {
        timestamp: String,
        stage_id: String,
        stage_name: String,
        stage_index: usize,
        total_stages: usize,
    },
    StageProgress {
        timestamp: String,
        stage_id: String,
        progress: f32,
        message: String,
    },
    StageCompleted {
        timestamp: String,
        stage_id: String,
        duration_ms: u64,
        success: bool,
        output: Option<String>,
    },
    StageFailed {
        timestamp: String,
        stage_id: String,
        duration_ms: u64,
        error: String,
    },
    Warning {
        timestamp: String,
        stage_id: Option<String>,
        message: String,
    },
    Error {
        timestamp: String,
        stage_id: Option<String>,
        error: String,
        recoverable: bool,
    },
    BuildCompleted {
        timestamp: String,
        build_id: String,
        duration_ms: u64,
        success: bool,
        summary: BuildSummary,
    },
    BuildCancelled {
        timestamp: String,
        build_id: String,
        reason: String,
    },
}

/// Build configuration snapshot
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildConfig {
    pub jobs: usize,
    pub memory_limit_mb: Option<u64>,
    pub sccache_enabled: bool,
    pub incremental: bool,
    pub target: String,
    pub features: Vec<String>,
}

/// Build summary for completion event
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSummary {
    pub stages_completed: usize,
    pub stages_failed: usize,
    pub total_warnings: usize,
    pub total_errors: usize,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub peak_memory_mb: u64,
}

/// Build event emitter for JSON Lines protocol
pub struct BuildEventEmitter {
    sender: Arc<std::sync::mpsc::SyncSender<BuildEvent>>,
    build_id: String,
    start_time: Instant,
    stage_start_times: Mutex<HashMap<String, Instant>>,
}

impl BuildEventEmitter {
    /// Create a new emitter that sends events through a buffered mpsc channel
    /// Buffer size of 100 allows non-blocking emission for typical build stages
    pub fn new(sender: std::sync::mpsc::SyncSender<BuildEvent>, build_id: String) -> Self {
        Self {
            sender: Arc::new(sender),
            build_id,
            start_time: Instant::now(),
            stage_start_times: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new emitter with a default buffered channel (capacity 100)
    /// Returns (emitter, receiver) for consuming events
    pub fn with_channel(build_id: String) -> (Self, std::sync::mpsc::Receiver<BuildEvent>) {
        let (tx, rx) = std::sync::mpsc::sync_channel(100);
        let emitter = Self::new(tx, build_id);
        (emitter, rx)
    }

    /// Get the build ID
    pub fn build_id(&self) -> &str {
        &self.build_id
    }

    pub fn emit(&self, event: BuildEvent) -> Result<()> {
        self.sender.send(event).map_err(|e| anyhow::anyhow!("Failed to send event: {}", e))
    }

    pub fn build_started(&self, fingerprint: &str, config: BuildConfig) -> Result<()> {
        self.emit(BuildEvent::BuildStarted {
            timestamp: chrono::Utc::now().to_rfc3339(),
            build_id: self.build_id.clone(),
            toolchain_fingerprint: fingerprint.to_string(),
            config,
        })
    }

    pub fn stage_started(&self, stage_id: &str, stage_name: &str, index: usize, total: usize) -> Result<()> {
        self.stage_start_times.lock().unwrap().insert(stage_id.to_string(), Instant::now());
        self.emit(BuildEvent::StageStarted {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage_id: stage_id.to_string(),
            stage_name: stage_name.to_string(),
            stage_index: index,
            total_stages: total,
        })
    }

    pub fn stage_progress(&self, stage_id: &str, progress: f32, message: &str) -> Result<()> {
        self.emit(BuildEvent::StageProgress {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage_id: stage_id.to_string(),
            progress,
            message: message.to_string(),
        })
    }

    pub fn stage_completed(&self, stage_id: &str, success: bool, output: Option<String>) -> Result<()> {
        let duration = self.stage_start_times.lock().unwrap()
            .remove(stage_id)
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);
        self.emit(BuildEvent::StageCompleted {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage_id: stage_id.to_string(),
            duration_ms: duration,
            success,
            output,
        })
    }

    pub fn stage_failed(&self, stage_id: &str, error: &str) -> Result<()> {
        let duration = self.stage_start_times.lock().unwrap()
            .remove(stage_id)
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);
        self.emit(BuildEvent::StageFailed {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage_id: stage_id.to_string(),
            duration_ms: duration,
            error: error.to_string(),
        })
    }

    pub fn warning(&self, stage_id: Option<&str>, message: &str) -> Result<()> {
        self.emit(BuildEvent::Warning {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage_id: stage_id.map(|s| s.to_string()),
            message: message.to_string(),
        })
    }

    pub fn error(&self, stage_id: Option<&str>, error: &str, recoverable: bool) -> Result<()> {
        self.emit(BuildEvent::Error {
            timestamp: chrono::Utc::now().to_rfc3339(),
            stage_id: stage_id.map(|s| s.to_string()),
            error: error.to_string(),
            recoverable,
        })
    }

    pub fn build_completed(&self, success: bool, summary: BuildSummary) -> Result<()> {
        self.emit(BuildEvent::BuildCompleted {
            timestamp: chrono::Utc::now().to_rfc3339(),
            build_id: self.build_id.clone(),
            duration_ms: self.start_time.elapsed().as_millis() as u64,
            success,
            summary,
        })
    }

    pub fn build_cancelled(&self, reason: &str) -> Result<()> {
        self.emit(BuildEvent::BuildCancelled {
            timestamp: chrono::Utc::now().to_rfc3339(),
            build_id: self.build_id.clone(),
            reason: reason.to_string(),
        })
    }
}

/// Adaptive concurrency calculator
pub struct ConcurrencyCalculator {
    total_memory_mb: u64,
    available_memory_mb: u64,
    cpu_cores: usize,
    estimated_per_job_mb: u64,
    min_jobs: usize,
    max_jobs: usize,
    user_limit: Option<usize>,
    reserved_memory_mb: u64,
}

impl ConcurrencyCalculator {
    pub fn new() -> Self {
        let (total_mem, avail_mem) = get_system_memory();
        let cpu_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        Self {
            total_memory_mb: total_mem,
            available_memory_mb: avail_mem,
            cpu_cores,
            estimated_per_job_mb: 1500, // Conservative estimate for clang-cl + lld-link
            min_jobs: 1,
            max_jobs: cpu_cores * 2,
            user_limit: None,
            reserved_memory_mb: 2048, // Reserve 2GB for system
        }
    }

    pub fn with_estimated_per_job(mut self, mb: u64) -> Self {
        self.estimated_per_job_mb = mb;
        self
    }

    pub fn with_user_limit(mut self, limit: usize) -> Self {
        self.user_limit = Some(limit);
        self
    }

    pub fn with_reserved_memory(mut self, mb: u64) -> Self {
        self.reserved_memory_mb = mb;
        self
    }

    pub fn calculate(&self) -> usize {
        let usable_memory = self.available_memory_mb.saturating_sub(self.reserved_memory_mb);
        let memory_based_jobs = if self.estimated_per_job_mb > 0 {
            (usable_memory / self.estimated_per_job_mb) as usize
        } else {
            self.max_jobs
        };

        let mut jobs = memory_based_jobs.clamp(self.min_jobs, self.max_jobs);

        if let Some(limit) = self.user_limit {
            jobs = jobs.min(limit);
        }

        // Also consider CPU cores
        jobs = jobs.min(self.cpu_cores * 2);

        jobs.max(1)
    }

    pub fn get_info(&self) -> ConcurrencyInfo {
        ConcurrencyInfo {
            total_memory_mb: self.total_memory_mb,
            available_memory_mb: self.available_memory_mb,
            cpu_cores: self.cpu_cores,
            estimated_per_job_mb: self.estimated_per_job_mb,
            calculated_jobs: self.calculate(),
            reserved_memory_mb: self.reserved_memory_mb,
        }
    }
}

/// Information about concurrency calculation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrencyInfo {
    pub total_memory_mb: u64,
    pub available_memory_mb: u64,
    pub cpu_cores: usize,
    pub estimated_per_job_mb: u64,
    pub calculated_jobs: usize,
    pub reserved_memory_mb: u64,
}

fn get_system_memory() -> (u64, u64) {
    #[cfg(windows)]
    {
        use std::mem;
        use windows_sys::Win32::System::SystemInformation::{
            GlobalMemoryStatusEx, MEMORYSTATUSEX,
        };

        unsafe {
            let mut mem_status: MEMORYSTATUSEX = mem::zeroed();
            mem_status.dwLength = mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut mem_status) != 0 {
                let total = (mem_status.ullTotalPhys / 1024 / 1024) as u64;
                let avail = (mem_status.ullAvailPhys / 1024 / 1024) as u64;
                return (total, avail);
            }
        }
        (8192, 4096) // Fallback
    }
    #[cfg(not(windows))]
    {
        // Linux: read from /proc/meminfo
        if let Ok(content) = fs::read_to_string("/proc/meminfo") {
            let mut total = 0;
            let mut available = 0;
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    total = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                } else if line.starts_with("MemAvailable:") {
                    available = line.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                }
            }
            if total > 0 {
                return (total / 1024, available / 1024); // Convert KB to MB
            }
        }
        (8192, 4096)
    }
}

/// Unified Toolchain Manager
pub struct ToolchainManager {
    tools_lock: Option<ToolsLock>,
    verified_tools: Mutex<HashMap<String, ToolVerification>>,
    toolchain_root: PathBuf,
    event_emitter: Option<Arc<BuildEventEmitter>>,
}

impl ToolchainManager {
    pub fn new(toolchain_root: PathBuf) -> Self {
        Self {
            tools_lock: None,
            verified_tools: Mutex::new(HashMap::new()),
            toolchain_root,
            event_emitter: None,
        }
    }

    pub fn with_event_emitter(mut self, emitter: Arc<BuildEventEmitter>) -> Self {
        self.event_emitter = Some(emitter);
        self
    }

    /// Get reference to the loaded tools.lock
    pub fn tools_lock(&self) -> Option<&ToolsLock> {
        self.tools_lock.as_ref()
    }

    /// Get the toolchain root path
    pub fn toolchain_root(&self) -> &Path {
        &self.toolchain_root
    }

    /// Load tools.lock from the toolchain root
    pub fn load_tools_lock(&mut self) -> Result<()> {
        let lock_path = self.toolchain_root.join("tools.lock");
        if !lock_path.exists() {
            return Err(anyhow!("tools.lock not found at {}", lock_path.display()));
        }
        let content = fs::read_to_string(&lock_path)?;
        self.tools_lock = Some(serde_json::from_str(&content)?);
        Ok(())
    }

    /// Load the on-disk lock when it still describes the current probe, otherwise generate a fresh one.
    ///
    /// A lock written by an older probe can name a required tool with no expected path, no version command and no
    /// PATH lookup (the Nov 21 Windows SDK entry did exactly that), or still require a tool this release made optional
    /// (.NET). Verification can only report those as missing, which would fail a build over a stale cache. Returns
    /// true when a loaded lock was discarded.
    pub fn load_tools_lock_or_generate(&mut self) -> Result<bool> {
        if self.load_tools_lock().is_err() {
            self.generate_tools_lock()?;
            return Ok(false);
        }
        let loaded = self.tools_lock.take().expect("load_tools_lock stored the lock");
        let fresh = self.generate_tools_lock()?;
        if self.lock_is_usable(&loaded, &fresh) {
            self.tools_lock = Some(loaded);
            Ok(false)
        } else {
            Ok(true)
        }
    }

    /// A lock is usable when it agrees with the fresh probe about which tools exist and which are required, and
    /// every required tool in it can still be located. Version strings may differ: verification checks the minimum.
    fn lock_is_usable(&self, loaded: &ToolsLock, fresh: &ToolsLock) -> bool {
        if loaded.version != fresh.version || loaded.platform != fresh.platform {
            return false;
        }
        for fresh_tool in &fresh.tools {
            let Some(loaded_tool) = loaded.tools.iter().find(|tool| tool.name == fresh_tool.name) else {
                return false;
            };
            if loaded_tool.category != fresh_tool.category {
                return false;
            }
            if loaded_tool.category == ToolCategory::Required
                && !matches!(self.find_tool(loaded_tool), Ok(Some(_)))
            {
                return false;
            }
        }
        // A required tool the probe no longer lists would still gate verification.
        for loaded_tool in &loaded.tools {
            if loaded_tool.category == ToolCategory::Required
                && !fresh.tools.iter().any(|tool| tool.name == loaded_tool.name)
            {
                return false;
            }
        }
        true
    }

    /// Generate a tools.lock from current system state
    pub fn generate_tools_lock(&mut self) -> Result<ToolsLock> {
        let probe_result = toolchain::probe();
        let mut tools = Vec::new();

        // Visual Studio
        if let Some(vs) = &probe_result.vs {
            tools.push(ToolSpec {
                name: "visual_studio".to_string(),
                display_name: "Visual Studio 2022".to_string(),
                category: ToolCategory::Required,
                version: vs.version.clone(),
                min_version: Some("17.0".to_string()),
                platform: Platform::Windows,
                source: ToolSource::System {
                    hint: "Install via Visual Studio Installer with 'Desktop development with C++' workload".to_string(),
                },
                expected_path: Some(vs.path.display().to_string()),
                sha256: None,
                archive_sha256: None,
                // The absolute vswhere: it is not on PATH, and the expected path above is the install folder,
                // which is not a program.
                version_cmd: Some(vec![crate::toolchain::vswhere().display().to_string(), "-path".to_string(), vs.path.display().to_string(), "-property".to_string(), "installationVersion".to_string()]),
                version_regex: Some(r"(\d+\.\d+\.\d+\.\d+)".to_string()),
                on_path: false,
                env_vars: BTreeMap::new(),
                depends_on: vec![],
                description: Some("Visual Studio C++ build tools".to_string()),
            });
        }

        // MSVC Toolset
        if let Some(msvc) = &probe_result.msvc {
            tools.push(ToolSpec {
                name: "msvc_toolset".to_string(),
                display_name: "MSVC Toolset".to_string(),
                category: ToolCategory::Required,
                version: msvc.version.clone(),
                min_version: Some("14.30".to_string()),
                platform: Platform::Windows,
                source: ToolSource::System {
                    hint: "Part of Visual Studio C++ workload".to_string(),
                },
                expected_path: Some(msvc.dir.display().to_string()),
                sha256: None,
                archive_sha256: None,
                version_cmd: None,
                version_regex: None,
                on_path: false,
                env_vars: BTreeMap::new(),
                depends_on: vec!["visual_studio".to_string()],
                description: Some("MSVC headers and libraries".to_string()),
            });
        }

        // Windows SDK
        if let Some(sdk) = &probe_result.sdk {
            tools.push(ToolSpec {
                name: "windows_sdk".to_string(),
                display_name: "Windows 10/11 SDK".to_string(),
                category: ToolCategory::Required,
                version: sdk.version.clone(),
                min_version: Some("10.0.19041".to_string()),
                platform: Platform::Windows,
                source: ToolSource::System {
                    hint: "Install via Visual Studio Installer or Windows SDK installer".to_string(),
                },
                expected_path: Some(sdk.include.display().to_string()),
                sha256: None,
                archive_sha256: None,
                version_cmd: None,
                version_regex: None,
                on_path: false,
                env_vars: BTreeMap::new(),
                depends_on: vec!["visual_studio".to_string()],
                description: Some("Windows SDK headers and libraries".to_string()),
            });
        }

        // clang-cl
        if let Some(clang) = &probe_result.clang {
            let sha256 = compute_file_sha256(&clang.exe).ok();
            tools.push(ToolSpec {
                name: "clang_cl".to_string(),
                display_name: "LLVM clang-cl".to_string(),
                category: ToolCategory::Required,
                version: clang.version.clone(),
                min_version: Some("23.0.0".to_string()),
                platform: Platform::Windows,
                source: ToolSource::Winget {
                    id: "LLVM.LLVM".to_string(),
                },
                expected_path: Some(clang.exe.display().to_string()),
                sha256,
                archive_sha256: None,
                version_cmd: Some(vec![clang.exe.display().to_string(), "--version".to_string()]),
                version_regex: Some(r"clang version (\d+\.\d+\.\d+)".to_string()),
                on_path: true,
                env_vars: BTreeMap::new(),
                depends_on: vec![],
                description: Some("C/C++ compiler compatible with MSVC".to_string()),
            });

            // lld-link
            if let Some(lld) = clang.lld_link() {
                let sha256 = compute_file_sha256(&lld).ok();
                tools.push(ToolSpec {
                    name: "lld_link".to_string(),
                    display_name: "LLVM lld-link".to_string(),
                    category: ToolCategory::Required,
                    version: clang.version.clone(),
                    min_version: Some("23.0.0".to_string()),
                    platform: Platform::Windows,
                    source: ToolSource::Winget {
                        id: "LLVM.LLVM".to_string(),
                    },
                    expected_path: Some(lld.display().to_string()),
                    sha256,
                    archive_sha256: None,
                    version_cmd: Some(vec![lld.display().to_string(), "--version".to_string()]),
                    version_regex: Some(r"lld version (\d+\.\d+\.\d+)".to_string()),
                    on_path: true,
                    env_vars: BTreeMap::new(),
                    depends_on: vec!["clang_cl".to_string()],
                    description: Some("LLVM linker for clang-cl".to_string()),
                });
            }
        }

        // CMake
        if let Some(cmake_version) = &probe_result.cmake {
            tools.push(ToolSpec {
                name: "cmake".to_string(),
                display_name: "CMake".to_string(),
                category: ToolCategory::Required,
                version: cmake_version.clone(),
                min_version: Some("3.25.0".to_string()),
                platform: Platform::All,
                source: ToolSource::Winget {
                    id: "Kitware.CMake".to_string(),
                },
                expected_path: None,
                sha256: None,
                archive_sha256: None,
                version_cmd: Some(vec!["cmake".to_string(), "--version".to_string()]),
                version_regex: Some(r"cmake version (\d+\.\d+\.\d+)".to_string()),
                on_path: true,
                env_vars: BTreeMap::new(),
                depends_on: vec![],
                description: Some("Build system generator".to_string()),
            });
        }

        // Ninja
        if let Some(ninja_version) = &probe_result.ninja {
            tools.push(ToolSpec {
                name: "ninja".to_string(),
                display_name: "Ninja Build System".to_string(),
                category: ToolCategory::Required,
                version: ninja_version.clone(),
                min_version: Some("1.11.0".to_string()),
                platform: Platform::All,
                source: ToolSource::Winget {
                    id: "Ninja-build.Ninja".to_string(),
                },
                expected_path: None,
                sha256: None,
                archive_sha256: None,
                version_cmd: Some(vec!["ninja".to_string(), "--version".to_string()]),
                version_regex: Some(r"^(\d+\.\d+\.\d+)".to_string()),
                on_path: true,
                env_vars: BTreeMap::new(),
                depends_on: vec![],
                description: Some("Fast build system for CMake".to_string()),
            });
        }

        // .NET SDK (for SpirvHarness). Optional: this release carries the shader translator and its own runtime,
        // so a PC without .NET can still build.
        tools.push(ToolSpec {
            name: "dotnet".to_string(),
            display_name: ".NET SDK".to_string(),
            category: ToolCategory::Optional,
            version: "8.0".to_string(),
            min_version: Some("8.0.0".to_string()),
            platform: Platform::All,
            source: ToolSource::Winget {
                id: "Microsoft.DotNet.SDK.8".to_string(),
            },
            expected_path: None,
            sha256: None,
            archive_sha256: None,
            version_cmd: Some(vec!["dotnet".to_string(), "--version".to_string()]),
            version_regex: Some(r"^(\d+\.\d+\.\d+)".to_string()),
            on_path: true,
            env_vars: BTreeMap::new(),
            depends_on: vec![],
            description: Some("Only for rebuilding the shader translator; the release ships it prebuilt".to_string()),
        });

        // Python (for nxrecomp)
        tools.push(ToolSpec {
            name: "python".to_string(),
            display_name: "Python 3.11+".to_string(),
            category: ToolCategory::Required,
            version: "3.11".to_string(),
            min_version: Some("3.11.0".to_string()),
            platform: Platform::All,
            source: ToolSource::Winget {
                id: "Python.Python.3.11".to_string(),
            },
            expected_path: None,
            sha256: None,
            archive_sha256: None,
            version_cmd: Some(vec!["python".to_string(), "--version".to_string()]),
            version_regex: Some(r"Python (\d+\.\d+\.\d+)".to_string()),
            on_path: true,
            env_vars: BTreeMap::new(),
            depends_on: vec![],
            description: Some("Required for nxrecomp recompiler".to_string()),
        });

        // sccache
        tools.push(ToolSpec {
            name: "sccache".to_string(),
            display_name: "sccache".to_string(),
            category: ToolCategory::Optional,
            version: "0.8".to_string(),
            min_version: Some("0.7.0".to_string()),
            platform: Platform::All,
            source: ToolSource::Winget {
                id: "Mozilla.sccache".to_string(),
            },
            expected_path: None,
            sha256: None,
            archive_sha256: None,
            version_cmd: Some(vec!["sccache".to_string(), "--version".to_string()]),
            version_regex: Some(r"sccache (\d+\.\d+\.\d+)".to_string()),
            on_path: true,
            env_vars: BTreeMap::from([
                ("SCCACHE_DIR".to_string(), r"%LOCALAPPDATA%\sccache".to_string()),
                ("SCCACHE_CACHE_SIZE".to_string(), "4G".to_string()),
            ]),
            depends_on: vec![],
            description: Some("Compiler cache for Rust and C++".to_string()),
        });

        let lock = ToolsLock {
            version: TOOLS_LOCK_VERSION,
            generated_at: chrono::Utc::now().to_rfc3339(),
            git_commit: get_git_commit(),
            platform: Platform::Windows,
            tools,
            metadata: BTreeMap::new(),
        };

        self.tools_lock = Some(lock.clone());
        Ok(lock)
    }

    /// Save tools.lock to disk
    pub fn save_tools_lock(&self) -> Result<()> {
        let lock = self.tools_lock.as_ref().ok_or_else(|| anyhow!("No tools.lock loaded"))?;
        let lock_path = self.toolchain_root.join("tools.lock");
        let content = serde_json::to_string_pretty(lock)?;
        fs::write(&lock_path, content)?;
        Ok(())
    }

    /// Verify all tools against tools.lock
    pub fn verify_all(&self) -> Result<Vec<ToolVerification>> {
        let lock = self.tools_lock.as_ref().ok_or_else(|| anyhow!("No tools.lock loaded"))?;
        let mut results = Vec::new();

        for tool in &lock.tools {
            let verification = self.verify_tool(tool)?;
            results.push(verification.clone());
            self.verified_tools.lock().unwrap().insert(tool.name.clone(), verification);
        }

        Ok(results)
    }

    /// Verify a single tool
    pub fn verify_tool(&self, tool: &ToolSpec) -> Result<ToolVerification> {
        let mut verification = ToolVerification {
            tool_name: tool.name.clone(),
            status: VerificationStatus::Missing,
            found_path: None,
            found_version: None,
            expected_version: tool.version.clone(),
            checksum_match: None,
            error: None,
        };

        // Try to find the tool
        let found_path = self.find_tool(tool)?;

        if let Some(path) = &found_path {
            verification.found_path = Some(path.clone());

            // Verify version. A probe that fails is this tool's problem, not a reason to abort the whole build.
            let mut probe_error = None;
            let version = match self.get_tool_version(tool, path) {
                Ok(v) => v,
                Err(e) => {
                    probe_error = Some(format!("its version could not be read: {e}"));
                    None
                }
            };
            // Visual Studio, the MSVC toolset and the Windows SDK are folders, not executables. The lock's version
            // came from this machine's probe, so an existing folder is the component that probe named.
            let version = version.or_else(|| path.is_dir().then(|| tool.version.clone()));
            if let Some(version) = version {
                verification.found_version = Some(version.clone());
                if self.version_satisfies(&version, &tool.min_version.as_ref().unwrap_or(&tool.version)) {
                    verification.status = VerificationStatus::Verified;
                    verification.error = None;
                } else {
                    verification.status = VerificationStatus::VersionMismatch;
                    verification.error = Some(format!("Version {} does not satisfy minimum {}", version, tool.min_version.as_ref().unwrap_or(&tool.version)));
                }
            } else {
                verification.error = probe_error;
            }

            // Verify checksum if provided
            if let Some(expected_sha256) = &tool.sha256 {
                let actual = compute_file_sha256(path)?;
                verification.checksum_match = Some(actual.eq_ignore_ascii_case(expected_sha256));
                if !verification.checksum_match.unwrap() {
                    verification.status = VerificationStatus::ChecksumMismatch;
                    verification.error = Some(format!("Checksum mismatch: expected {}, got {}", expected_sha256, actual));
                }
            }
        } else if tool.category == ToolCategory::Required {
            verification.error = Some(format!("Required tool '{}' not found", tool.display_name));
        } else {
            verification.status = VerificationStatus::NotApplicable;
        }

        Ok(verification)
    }

    fn find_tool(&self, tool: &ToolSpec) -> Result<Option<PathBuf>> {
        // Check expected path first
        if let Some(expected) = &tool.expected_path {
            let path = PathBuf::from(expected);
            if path.exists() {
                return Ok(Some(path));
            }
        }

        // The release carries CMake, Ninja, Python, clang-cl and lld-link in `<checkout>/tools/bin`. The build roots
        // the manager at the engine folder, so check the checkout beside it as well as the root itself. A carried tool
        // is the known-good one, so it wins over the process PATH — which a launcher started from Explorer may not
        // have refreshed since a tool was installed.
        let mut names = vec![format!("{}.exe", tool.name), format!("{}.cmd", tool.name)];
        let dashed = tool.name.replace('_', "-");
        if dashed != tool.name {
            names.push(format!("{dashed}.exe"));
            names.push(format!("{dashed}.cmd"));
        }
        for base in [
            self.toolchain_root.clone(),
            self.toolchain_root.join("fefw-recomp"),
        ] {
            for name in &names {
                let bundled = base.join("tools").join("bin").join(name);
                if bundled.is_file() {
                    return Ok(Some(bundled));
                }
            }
        }

        // Search in PATH if on_path
        if tool.on_path {
            if let Ok(path_var) = std::env::var("PATH") {
                for dir in path_var.split(';') {
                    let dir = dir.trim().trim_matches('"');
                    for name in &names {
                        let exe_path = PathBuf::from(dir).join(name);
                        if exe_path.is_file() {
                            return Ok(Some(exe_path));
                        }
                    }
                }
            }
        }

        // Try version command to locate
        if let Some(cmd) = &tool.version_cmd {
            if let Some(exe_name) = cmd.first() {
                if let Ok(path_var) = std::env::var("PATH") {
                    for dir in path_var.split(';') {
                        let exe_path = PathBuf::from(dir).join(exe_name);
                        if exe_path.is_file() {
                            return Ok(Some(exe_path));
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    fn get_tool_version(&self, tool: &ToolSpec, path: &Path) -> Result<Option<String>> {
        if let Some(cmd) = &tool.version_cmd {
            // Run the program the command names. Visual Studio's command is `vswhere -path <folder> -property …`;
            // running `path` (the folder) instead is what returned `Access is denied (os error 5)` before any build
            // stage could log a line.
            let program = version_program(tool, path);
            // Two attempts: the first run of the 113 MB carried clang-cl can be held up by an antivirus scan and
            // answer nothing, which used to stop a build before its own compile-and-link probe had run. Both streams
            // are read because a tool may print its version to stderr (`clang -v` does).
            for attempt in 0..2 {
                let mut command = Command::new(&program);
                if cmd.len() > 1 {
                    command.args(&cmd[1..]);
                }
                hidden(&mut command);
                let output = command.output()?;
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                if let Some(version) = version_in(tool, &text) {
                    return Ok(Some(version));
                }
                if attempt == 0 {
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        Ok(None)
    }

    fn version_satisfies(&self, found: &str, min_required: &str) -> bool {
        // Simple version comparison - parse major.minor.patch
        let found_parts = parse_version_parts(found);
        let min_parts = parse_version_parts(min_required);

        for (f, m) in found_parts.iter().zip(min_parts.iter()) {
            if f > m {
                return true;
            } else if f < m {
                return false;
            }
        }
        found_parts.len() >= min_parts.len()
    }

    /// Get the resolved toolchain environment (integrates with existing toolchain.rs)
    pub fn resolve_toolchain_env(&self) -> Result<Vec<(String, String)>, String> {
        toolchain::init().map(|r| r.env.clone())
    }

    /// Prepare build environment with all verified tools
    pub fn prepare_build_env(&self) -> Result<Vec<(String, String)>, String> {
        let mut env = self.resolve_toolchain_env()?;
        env.extend(self.build_env_extras());
        Ok(env)
    }

    /// The variables this manager adds to the host's environment, without asking the host for a toolchain: tools.lock's variables and
    /// sccache's. Kept apart from `prepare_build_env` so it can be tested on a machine that is not set up to build (CI).
    pub fn build_env_extras(&self) -> Vec<(String, String)> {
        let mut env = Vec::new();
        if let Some(lock) = &self.tools_lock {
            for tool in &lock.tools {
                for (k, v) in &tool.env_vars {
                    env.push((k.clone(), v.clone()));
                }
            }
        }
        env.push(("SCCACHE_DIR".to_string(), get_sccache_dir()));
        env.push(("SCCACHE_CACHE_SIZE".to_string(), "4G".to_string()));
        env.push(("SCCACHE_IDLE_TIMEOUT".to_string(), "600".to_string()));
        env
    }

    /// Calculate optimal job count for build
    pub fn calculate_jobs(&self, user_limit: Option<usize>) -> ConcurrencyInfo {
        let calc = ConcurrencyCalculator::new()
            .with_estimated_per_job(1500); // MB per clang-cl job
        let calc = if let Some(limit) = user_limit {
            calc.with_user_limit(limit)
        } else {
            calc
        };
        calc.get_info()
    }
}

/// The executable a tool's version command names. `path` is the file `find_tool` chose; when the command is the
/// tool itself (`clang-cl --version`, `cmake --version`) that found file is exactly what to run. When the command
/// names another program (Visual Studio's `vswhere`), that program is what to run: the tool's own `expected_path`
/// is a folder, and Windows answers a folder with `Access is denied`.
fn version_program(tool: &ToolSpec, path: &Path) -> PathBuf {
    let Some(first) = tool.version_cmd.as_ref().and_then(|c| c.first()) else {
        return path.to_path_buf();
    };
    let named = Path::new(first);
    let stem = named.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
    let own = stem.eq_ignore_ascii_case(&tool.name)
        || stem.eq_ignore_ascii_case(&tool.name.replace('_', "-"));
    if own {
        path.to_path_buf()
    } else {
        named.to_path_buf()
    }
}

/// The version a tool's output names: the lock's regex first, then the first line that looks like a version.
fn version_in(tool: &ToolSpec, text: &str) -> Option<String> {
    if let Some(regex_str) = &tool.version_regex {
        if let Ok(regex) = regex::Regex::new(regex_str) {
            if let Some(caps) = regex.captures(text) {
                return caps.get(1).map(|m| m.as_str().to_string());
            }
        }
    }
    for line in text.lines() {
        if line.chars().any(|c| c.is_ascii_digit()) && line.contains('.') {
            return Some(line.trim().to_string());
        }
    }
    None
}

/// Compute SHA-256 of a file
fn compute_file_sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

/// Parse version string into numeric parts
fn parse_version_parts(version: &str) -> Vec<u32> {
    // Find the first digit and parse from there
    let start = version.find(|c: char| c.is_ascii_digit()).unwrap_or(version.len());
    version[start..]
        .split('.')
        .filter_map(|p| {
            // Take only the leading numeric part of each segment
            p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()
        })
        .collect()
}

/// Get sccache directory
fn get_sccache_dir() -> String {
    std::env::var("SCCACHE_DIR")
        .or_else(|_| std::env::var("LOCALAPPDATA").map(|p| format!(r"{}\sccache", p)))
        .unwrap_or_else(|_| r"C:\sccache".to_string())
}

/// Get git commit hash
fn get_git_commit() -> Option<String> {
    let mut git_cmd = Command::new("git");
    git_cmd.args(["rev-parse", "HEAD"]);
    hidden(&mut git_cmd);
    git_cmd
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_parse_version_parts() {
        assert_eq!(parse_version_parts("23.1.4"), vec![23, 1, 4]);
        assert_eq!(parse_version_parts("14.44.35207"), vec![14, 44, 35207]);
        assert_eq!(parse_version_parts("4.5.0-rc3"), vec![4, 5, 0]);
        assert_eq!(parse_version_parts("v1.13.2"), vec![1, 13, 2]);
    }

    #[test]
    fn test_version_satisfies() {
        let mgr = ToolchainManager::new(PathBuf::from("."));
        assert!(mgr.version_satisfies("23.1.4", "23.0.0"));
        assert!(mgr.version_satisfies("24.0.0", "23.0.0"));
        assert!(!mgr.version_satisfies("22.1.0", "23.0.0"));
        assert!(mgr.version_satisfies("3.25.1", "3.25.0"));
        assert!(mgr.version_satisfies("14.44.35207", "14.30.0"));
    }

    #[test]
    fn test_concurrency_calculator() {
        let calc = ConcurrencyCalculator::new()
            .with_estimated_per_job(1000)
            .with_user_limit(8);
        let info = calc.get_info();
        assert!(info.calculated_jobs >= 1);
        assert!(info.calculated_jobs <= 8);
    }

    #[test]
    fn test_build_event_emitter() {
        let (emitter, rx) = BuildEventEmitter::with_channel("test-build".to_string());
        emitter.build_started("fingerprint", BuildConfig {
            jobs: 4,
            memory_limit_mb: None,
            sccache_enabled: true,
            incremental: true,
            target: "release".to_string(),
            features: vec![],
        }).unwrap();

        drop(emitter); // Close the channel

        let events: Vec<BuildEvent> = rx.iter().collect();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], BuildEvent::BuildStarted { .. }));
    }

    fn tool_spec(
        name: &str,
        version: &str,
        expected: Option<&Path>,
        version_cmd: Option<Vec<String>>,
    ) -> ToolSpec {
        ToolSpec {
            name: name.into(),
            display_name: name.into(),
            category: ToolCategory::Required,
            version: version.into(),
            min_version: Some(version.into()),
            platform: Platform::Windows,
            source: ToolSource::System {
                hint: String::new(),
            },
            expected_path: expected.map(|p| p.display().to_string()),
            sha256: None,
            archive_sha256: None,
            version_cmd,
            version_regex: Some(r"(\d+\.\d+\.\d+(?:\.\d+)?)".into()),
            on_path: false,
            env_vars: BTreeMap::new(),
            depends_on: vec![],
            description: None,
        }
    }

    /// Visual Studio's version command asks `vswhere`; its expected path is the install folder, and running a
    /// folder is what produced `Access is denied (os error 5)` before the build logged anything.
    #[test]
    fn a_version_command_runs_the_program_it_names_not_the_folder() {
        let dir = std::env::temp_dir().join(format!("ew-tool-program-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let vs = tool_spec(
            "visual_studio",
            "18.0.0",
            Some(&dir),
            Some(vec!["vswhere".into(), "-latest".into()]),
        );
        assert_eq!(version_program(&vs, &dir), PathBuf::from("vswhere"));

        // A command that names the tool itself still runs the exact file that was found.
        let clang = dir.join("clang-cl.exe");
        let own = tool_spec(
            "clang_cl",
            "23.0.0",
            Some(&clang),
            Some(vec![clang.display().to_string(), "--version".into()]),
        );
        assert_eq!(version_program(&own, &clang), clang);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The MSVC toolset and the Windows SDK are folders with no executable to ask. The version the probe recorded
    /// when the lock was generated is the version that is installed, so a present folder must verify.
    #[test]
    fn a_folder_backed_tool_is_verified_by_the_version_the_probe_recorded() {
        let dir = std::env::temp_dir().join(format!("ew-tool-folder-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let spec = tool_spec("msvc_toolset", "14.44.35207", Some(&dir), None);
        let mut mgr = ToolchainManager::new(dir.clone());
        mgr.tools_lock = Some(ToolsLock {
            version: TOOLS_LOCK_VERSION,
            generated_at: String::new(),
            git_commit: None,
            platform: Platform::Windows,
            tools: vec![spec.clone()],
            metadata: BTreeMap::new(),
        });

        let verified = mgr.verify_tool(&spec).unwrap();
        assert_eq!(verified.status, VerificationStatus::Verified);
        assert_eq!(verified.found_version.as_deref(), Some("14.44.35207"));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The whole generated lock against this machine's real tools. Ignored by default because it needs Visual Studio;
    /// run it with `--ignored` on a build PC. This is the path that returned `Access is denied` before any stage logged.
    #[cfg(windows)]
    #[test]
    #[ignore = "asks this machine for its real toolchain"]
    fn the_generated_lock_verifies_on_this_machine() {
        let root = std::env::temp_dir().join(format!("ew-tool-real-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut mgr = ToolchainManager::new(root.clone());
        mgr.generate_tools_lock().unwrap();
        let checked = mgr.verify_all().expect("verification must not abort the build");
        let missing: Vec<&str> = checked
            .iter()
            .filter(|v| v.status == VerificationStatus::Missing)
            .map(|v| v.tool_name.as_str())
            .collect();
        assert!(missing.is_empty(), "required tools missing: {missing:?}");

        // The Nov 21 lock in the engine checkout names a Windows SDK with no path and still requires .NET. It must be
        // discarded if the manager is ever rooted at the checkout, not reused until verification fails.
        let stale_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../engine/fefw-recomp/tools.lock");
        if let Ok(text) = std::fs::read_to_string(stale_path) {
            let stale: ToolsLock = serde_json::from_str(&text).unwrap();
            assert!(
                !mgr.lock_is_usable(&stale, mgr.tools_lock().unwrap()),
                "the Nov 21 tools.lock must not be reused"
            );
        }
        let _ = std::fs::remove_dir_all(root);
    }

    fn lock_with(tools: Vec<ToolSpec>) -> ToolsLock {
        ToolsLock {
            version: TOOLS_LOCK_VERSION,
            generated_at: String::new(),
            git_commit: None,
            platform: Platform::Windows,
            tools,
            metadata: BTreeMap::new(),
        }
    }

    /// A lock that cannot locate a required tool, or that disagrees with the probe about what is required, was written
    /// by an older probe. Verification can only report those tools as missing, so the lock must be discarded.
    #[test]
    fn a_lock_that_cannot_locate_a_required_tool_is_not_usable() {
        let dir = std::env::temp_dir().join(format!("ew-tool-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let present = tool_spec("windows_sdk", "10.0.28000.0", Some(&dir), None);
        let fresh = lock_with(vec![present.clone()]);
        let mgr = ToolchainManager::new(dir.clone());

        assert!(mgr.lock_is_usable(&lock_with(vec![present.clone()]), &fresh));

        // The Nov 21 shape: a required Windows SDK with no path, no command and no PATH lookup.
        let blind = tool_spec("windows_sdk", "10.0.22621.0", None, None);
        assert!(!mgr.lock_is_usable(&lock_with(vec![blind]), &fresh));

        // A tool the current probe found is absent from the lock.
        assert!(!mgr.lock_is_usable(&lock_with(vec![]), &fresh));

        // A required tool the probe no longer lists would still gate the build.
        let extra = tool_spec("nasm", "2.16.01", None, None);
        assert!(!mgr.lock_is_usable(&lock_with(vec![present.clone(), extra]), &fresh));

        // The probe made a tool optional that the lock still requires.
        let mut now_optional = present.clone();
        now_optional.category = ToolCategory::Optional;
        let fresh_optional = lock_with(vec![now_optional]);
        assert!(!mgr.lock_is_usable(&lock_with(vec![present.clone()]), &fresh_optional));

        let _ = std::fs::remove_dir_all(dir);
    }

    /// The release carries Ninja and CMake in `<checkout>/tools/bin`. A manager rooted at the checkout finds them
    /// directly, and one rooted at the engine folder above still finds them through the checkout beside it.
    #[test]
    fn a_carried_tool_is_found_in_the_engine_checkout() {
        let root = std::env::temp_dir().join(format!("ew-tool-carried-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let checkout = root.join("fefw-recomp");
        let bin = checkout.join("tools").join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("ninja.exe"), b"").unwrap();
        std::fs::write(bin.join("clang-cl.exe"), b"").unwrap();

        let ninja = tool_spec("ninja", "1.13.1", None, Some(vec!["ninja".into(), "--version".into()]));
        let clang = tool_spec("clang_cl", "23.1.5", None, Some(vec!["clang-cl".into(), "--version".into()]));

        let mgr = ToolchainManager::new(checkout.clone());
        assert_eq!(mgr.find_tool(&ninja).unwrap(), Some(bin.join("ninja.exe")));
        // `clang_cl` names the carried `clang-cl.exe`, with a hyphen.
        assert_eq!(mgr.find_tool(&clang).unwrap(), Some(bin.join("clang-cl.exe")));

        let parent = ToolchainManager::new(root.clone());
        assert_eq!(parent.find_tool(&ninja).unwrap(), Some(bin.join("ninja.exe")));
        let _ = std::fs::remove_dir_all(root);
    }

    /// `clang -v` writes to stderr; a version there is still the tool's version.
    #[cfg(windows)]
    #[test]
    fn a_version_on_stderr_is_read() {
        let spec = tool_spec(
            "cmake",
            "3.31.2",
            None,
            Some(vec!["cmd.exe".into(), "/C".into(), "echo 3.31.2 1>&2".into()]),
        );
        let mgr = ToolchainManager::new(std::env::temp_dir());
        let version = mgr.get_tool_version(&spec, Path::new("cmd.exe")).unwrap();
        assert_eq!(version.as_deref(), Some("3.31.2"));
    }

    /// End to end on Windows: the command runs, even though the tool's expected path is a folder.
    #[cfg(windows)]
    #[test]
    fn a_version_command_that_names_another_program_is_not_run_against_the_folder() {
        let dir = std::env::temp_dir().join(format!("ew-tool-run-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let spec = tool_spec(
            "visual_studio",
            "17.0.0",
            Some(&dir),
            Some(vec!["cmd.exe".into(), "/C".into(), "echo 17.1.2".into()]),
        );
        let mgr = ToolchainManager::new(dir.clone());
        assert_eq!(
            mgr.get_tool_version(&spec, &dir).unwrap().as_deref(),
            Some("17.1.2")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}