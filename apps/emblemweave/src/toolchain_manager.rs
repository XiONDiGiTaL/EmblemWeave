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
                version_cmd: Some(vec!["vswhere".to_string(), "-path".to_string(), vs.path.display().to_string(), "-property".to_string(), "installationVersion".to_string()]),
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

        // .NET SDK (for SpirvHarness)
        tools.push(ToolSpec {
            name: "dotnet".to_string(),
            display_name: ".NET SDK".to_string(),
            category: ToolCategory::Required,
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
            description: Some("Required for SpirvHarness shader translator".to_string()),
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

            // Verify version
            if let Some(version) = self.get_tool_version(tool, path)? {
                verification.found_version = Some(version.clone());
                if self.version_satisfies(&version, &tool.min_version.as_ref().unwrap_or(&tool.version)) {
                    verification.status = VerificationStatus::Verified;
                } else {
                    verification.status = VerificationStatus::VersionMismatch;
                    verification.error = Some(format!("Version {} does not satisfy minimum {}", version, tool.min_version.as_ref().unwrap_or(&tool.version)));
                }
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

        // Check toolchain_root/tools/bin for bundled tools
        let bundled = self.toolchain_root.join("tools").join("bin").join(format!("{}.exe", tool.name));
        if bundled.exists() {
            return Ok(Some(bundled));
        }

        // Search in PATH if on_path
        if tool.on_path {
            if let Ok(path_var) = std::env::var("PATH") {
                for dir in path_var.split(';') {
                    let exe_path = PathBuf::from(dir).join(format!("{}.exe", tool.name));
                    if exe_path.exists() {
                        return Ok(Some(exe_path));
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
                        if exe_path.exists() {
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
            let mut command = Command::new(path);
            if cmd.len() > 1 {
                command.args(&cmd[1..]);
            }
            let output = command.output()?;
            let stdout = String::from_utf8_lossy(&output.stdout);

            if let Some(regex_str) = &tool.version_regex {
                let regex = regex::Regex::new(regex_str)?;
                if let Some(caps) = regex.captures(&stdout) {
                    return Ok(caps.get(1).map(|m| m.as_str().to_string()));
                }
            }

            // Fallback: first line with version-like pattern
            for line in stdout.lines() {
                if line.chars().any(|c| c.is_ascii_digit()) && line.contains('.') {
                    return Ok(Some(line.trim().to_string()));
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

        // Add toolchain-specific env vars from tools.lock
        if let Some(lock) = &self.tools_lock {
            for tool in &lock.tools {
                for (k, v) in &tool.env_vars {
                    env.push((k.clone(), v.clone()));
                }
            }
        }

        // Ensure sccache env vars
        env.push(("SCCACHE_DIR".to_string(), get_sccache_dir()));
        env.push(("SCCACHE_CACHE_SIZE".to_string(), "4G".to_string()));
        env.push(("SCCACHE_IDLE_TIMEOUT".to_string(), "600".to_string()));

        Ok(env)
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
    Command::new("git")
        .args(["rev-parse", "HEAD"])
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
}