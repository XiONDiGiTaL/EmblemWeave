# EmblemWeave Toolchain Management

## Overview

The EmblemWeave toolchain manager provides a unified, reproducible way to discover, validate, install, and execute all build tools required by the project. It replaces ad-hoc environment detection with a versioned `tools.lock` manifest and structured verification.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    ToolchainManager                          │
├─────────────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │  tools.lock  │  │  Discovery   │  │  Verification    │  │
│  │  (manifest)  │──│  (probe)     │──│  (version/check) │  │
│  └──────────────┘  └──────────────┘  └──────────────────┘  │
│         │                │                   │              │
│         ▼                ▼                   ▼              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │              BuildEventEmitter                        │  │
│  │  (JSON Lines structured events)                       │  │
│  └──────────────────────────────────────────────────────┘  │
│         │                                                   │
│         ▼                                                   │
│  ┌──────────────────────────────────────────────────────┐  │
│  │            ConcurrencyCalculator                      │  │
│  │  (memory-aware adaptive job count)                    │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## tools.lock Manifest

The `tools.lock` file is the single source of truth for all toolchain components. It lives at `engine/fefw-recomp/tools.lock` and contains:

### Schema (Version 1)

```json
{
  "version": 1,
  "generated_at": "2026-11-20T00:00:00Z",
  "git_commit": null,
  "platform": "windows",
  "tools": [
    {
      "name": "clang_cl",
      "display_name": "LLVM clang-cl",
      "category": "required",
      "version": "23.1.0",
      "min_version": "23.0.0",
      "platform": "windows",
      "source": {
        "type": "winget",
        "id": "LLVM.LLVM"
      },
      "expected_path": null,
      "sha256": null,
      "archive_sha256": null,
      "version_cmd": ["clang-cl.exe", "--version"],
      "version_regex": "clang version (\\d+\\.\\d+\\.\\d+)",
      "on_path": true,
      "env_vars": {},
      "depends_on": [],
      "description": "C/C++ compiler compatible with MSVC"
    }
  ],
  "metadata": {
    "schema_version": "1",
    "description": "Toolchain lockfile for EmblemWeave build reproducibility"
  }
}
```

### Tool Categories

| Category | Description | Failure Behavior |
|----------|-------------|------------------|
| `required` | Essential for build | Hard error if missing/failed |
| `optional` | Enhances build (caching, etc.) | Warning only |
| `build_only` | Compile-time only | Warning only |
| `dev_only` | Development/testing | Warning only |

### Tool Sources

| Type | Description | Example |
|------|-------------|---------|
| `system` | Pre-installed on system | Visual Studio, Windows SDK |
| `winget` | Windows Package Manager | LLVM, CMake, Ninja, .NET SDK |
| `download` | Direct URL download | uv (Python package manager) |
| `cargo` | Rust package | Rust tools |
| `vcpkg` | C++ package manager | libvpx |

### Verification Process

1. **Load** `tools.lock` from disk (or generate if missing)
2. **Discover** each tool via:
   - `version_cmd` execution + `version_regex` parsing
   - PATH lookup (`on_path: true`)
   - Known installation locations
3. **Validate**:
   - Version ≥ `min_version`
   - SHA-256 checksum match (if provided)
   - Required dependencies present
4. **Emit** `VerificationResult` for each tool

### Verification Statuses

| Status | Meaning |
|--------|---------|
| `Verified` | Tool found, version OK, checksum OK |
| `VersionMismatch` | Tool found but version < min_version |
| `ChecksumMismatch` | Tool found but SHA-256 doesn't match |
| `Missing` | Tool not found on system |
| `NotChecked` | Tool not yet verified |

## Unified ToolchainManager (Rust)

### Location
`apps/emblemweave/src/toolchain_manager.rs`

### Key Types

```rust
// Core manager
pub struct ToolchainManager {
    pub root: PathBuf,
    pub tools_lock: Option<ToolsLock>,
    pub event_emitter: Option<Arc<BuildEventEmitter>>,
}

// Tool specification
pub struct ToolSpec {
    pub name: String,
    pub display_name: String,
    pub category: ToolCategory,
    pub version: String,
    pub min_version: Option<String>,
    pub platform: Platform,
    pub source: ToolSource,
    pub expected_path: Option<PathBuf>,
    pub sha256: Option<String>,
    pub archive_sha256: Option<String>,
    pub version_cmd: Option<Vec<String>>,
    pub version_regex: Option<String>,
    pub on_path: bool,
    pub env_vars: HashMap<String, String>,
    pub depends_on: Vec<String>,
    pub description: String,
}

// Verification result
pub struct VerificationResult {
    pub tool_name: String,
    pub status: VerificationStatus,
    pub found_version: Option<String>,
    pub expected_version: String,
    pub found_path: Option<PathBuf>,
    pub expected_path: Option<PathBuf>,
    pub sha256_match: Option<bool>,
    pub error: Option<String>,
}
```

### Main Methods

| Method | Description |
|--------|-------------|
| `new(root)` | Create manager for engine root |
| `load_tools_lock()` | Load existing `tools.lock` |
| `generate_tools_lock()` | Generate default `tools.lock` for platform |
| `save_tools_lock()` | Write `tools.lock` to disk |
| `verify_all()` | Verify all tools, return `Vec<VerificationResult>` |
| `prepare_build_env()` | Get environment variables for build |
| `calculate_jobs(override)` | Get adaptive job count |
| `with_event_emitter(emitter)` | Attach event emitter |

### Usage in engine.rs

```rust
// Initialize
let mut toolchain_mgr = ToolchainManager::new(toolchain_root);
toolchain_mgr.load_tools_lock().ok();
if toolchain_mgr.tools_lock.is_none() {
    toolchain_mgr.generate_tools_lock()?;
}

// Verify
let verifications = toolchain_mgr.verify_all()?;
for v in &verifications {
    match v.status {
        VerificationStatus::Verified => { /* log success */ }
        VerificationStatus::Missing if v.tool_name != "sccache" => {
            return Err(format!("Required tool '{}' not found", v.tool_name));
        }
        _ => { /* handle */ }
    }
}

// Prepare build environment
let resolved_env = toolchain_mgr.prepare_build_env()?;

// Get adaptive concurrency
let concurrency = toolchain_mgr.calculate_jobs(None);
let jobs = concurrency.calculated_jobs;
```

## Structured Build Events (JSON Lines)

### Protocol

All build events are emitted as JSON Lines (one JSON object per line) to stdout. This enables:
- Real-time CI/CD monitoring
- Structured log aggregation
- Progress tracking in GUI
- Automated failure analysis

### Event Types

| Event | Description | Fields |
|-------|-------------|--------|
| `build_started` | Build begins | `build_id`, `toolchain_fingerprint`, `config` |
| `stage_started` | Stage begins | `stage_id`, `stage_name`, `stage_index`, `total_stages` |
| `stage_progress` | Stage progress update | `stage_id`, `progress` (0.0-1.0), `message` |
| `stage_completed` | Stage finished successfully | `stage_id`, `duration_ms`, `success`, `output` |
| `stage_failed` | Stage failed | `stage_id`, `duration_ms`, `error` |
| `warning` | Non-fatal warning | `stage_id`, `message` |
| `error` | Recoverable/non-recoverable error | `stage_id`, `error`, `recoverable` |
| `build_completed` | Build finished | `build_id`, `duration_ms`, `success`, `summary` |
| `build_cancelled` | Build cancelled | `build_id`, `reason` |

### Example Events

```json
{"event":"build_started","timestamp":"2026-11-20T10:30:00.000Z","build_id":"a1b2c3d4","toolchain_fingerprint":"C:\\engine|a1b2c3d4","config":{"jobs":8,"memory_limit_mb":16384,"sccache_enabled":true,"incremental":true,"target":"Release","features":[]}}
{"event":"stage_started","timestamp":"2026-11-20T10:30:00.100Z","stage_id":"recompile","stage_name":"Recompile the game code","stage_index":3,"total_stages":8}
{"event":"stage_progress","timestamp":"2026-11-20T10:30:05.000Z","stage_id":"recompile","progress":0.3,"message":"starting build_game.cmd"}
{"event":"stage_completed","timestamp":"2026-11-20T10:35:00.000Z","stage_id":"recompile","duration_ms":300000,"success":true,"output":"six modules compiled (8 jobs)"}
{"event":"build_completed","timestamp":"2026-11-20T10:45:00.000Z","build_id":"a1b2c3d4","duration_ms":900000,"success":true,"summary":{"stages_completed":8,"stages_failed":0,"total_warnings":0,"total_errors":0,"cache_hits":150,"cache_misses":10,"peak_memory_mb":8192}}
```

### Consuming Events

```bash
# From command line
cargo run -- build 2>&1 | jq -c 'select(.event == "stage_completed")'

# From PowerShell
$events = & cargo run -- build 2>&1 | ConvertFrom-Json
$events | Where-Object { $_.event -eq "stage_completed" }

# From Python
import json, subprocess
proc = subprocess.Popen(["cargo", "run", "--", "build"], stdout=subprocess.PIPE, text=True)
for line in proc.stdout:
    event = json.loads(line)
    if event["event"] == "stage_completed":
        print(f"Stage {event['stage_id']} completed in {event['duration_ms']}ms")
```

## Adaptive Memory-Aware Concurrency

### Algorithm

The `ConcurrencyCalculator` computes optimal job count based on:

```
jobs = min(
    cores - headroom_cores,
    floor((total_ram_gb - headroom_gb) / mem_per_job_gb)
)
```

### Modes

| Mode | Core Headroom | RAM Headroom | Use Case |
|------|---------------|--------------|----------|
| `conservative` | 2 cores | 4 GB | Default, safe for all machines |
| `aggressive` | 0 cores | 2 GB | High-RAM machines, CI |
| `max` | 0 cores | 0 GB | Maximum throughput, may OOM |

### Configuration

```rust
let config = ConcurrencyConfig {
    mode: "conservative".to_string(),
    manual_override: None,
    memory_per_job_mb: Some(1536),  // 1.5 GB per clang-cl job
    memory_headroom_mb: Some(4096), // 4 GB for system
    min_jobs: 1,
    max_jobs: Some(32),
};

let calc = ConcurrencyCalculator::new(config);
let result = calc.calculate();
// result.calculated_jobs
// result.available_memory_mb
// result.mode
```

### In engine.rs

```rust
// Uses conservative mode by default
let concurrency = toolchain_mgr.calculate_jobs(None);
let jobs = concurrency.calculated_jobs;

// Can be overridden via NX_JOBS env var or --jobs flag
```

### In build.ps1

```powershell
# Conservative (default)
$jobs = Get-AdaptiveJobCount -Mode conservative

# Aggressive
$jobs = Get-AdaptiveJobCount -Mode aggressive

# Manual override
$jobs = 16
```

## sccache Integration

### Configuration

sccache is configured as an **optional** tool in `tools.lock`:

```json
{
  "name": "sccache",
  "category": "optional",
  "version": "0.8.1",
  "min_version": "0.7.0",
  "env_vars": {
    "SCCACHE_DIR": "%LOCALAPPDATA%\\sccache",
    "SCCACHE_CACHE_SIZE": "4G",
    "SCCACHE_IDLE_TIMEOUT": "600"
  }
}
```

### Rust (Cargo)

In `.cargo/config.toml`:
```toml
[build]
rustc-wrapper = "sccache"
```

### C++ (CMake)

In `recomp/cmake/clang-cl.cmake`:
```cmake
if(NOT DEFINED ENV{NX_NO_SCCACHE})
  find_program(NX_SCCACHE sccache)
  if(NX_SCCACHE)
    set(CMAKE_C_COMPILER_LAUNCHER "${NX_SCCACHE}")
    set(CMAKE_CXX_COMPILER_LAUNCHER "${NX_SCCACHE}")
  endif()
endif()
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SCCACHE_DIR` | `%LOCALAPPDATA%\sccache` | Cache directory |
| `SCCACHE_CACHE_SIZE` | `4G` | Max cache size |
| `SCCACHE_IDLE_TIMEOUT` | `600` | Server idle timeout (seconds) |
| `SCCACHE_SERVER_PORT` | `24339` | Server port (for sharing) |

### Build Scripts

Both `runtime/scripts/build.ps1` and `recomp/scripts/build_game.cmd`:
- Set `SCCACHE_DIR` to `<root>/work/build/sccache`
- Set `SCCACHE_CACHE_SIZE` to `4G` (PowerShell) / `20G` (cmd)
- Pass `-DCMAKE_C_COMPILER_LAUNCHER=sccache` to CMake

## Integration Points

### engine.rs Build Pipeline

Stages with structured events:
1. `python_env` - Python virtual environment
2. `exefs` - ExeFS preparation
3. `recompile` - Game code recompilation
4. `textures` - Texture transcoders
5. `fefw_exe` - fefw.exe build
6. `shader_translator` - SpirvHarness build
7. `shaders` - Shader translation
8. `fefw_ini` - Configuration file

### nx_env.cmd / build_game.cmd

Neither script accepts `--structured-events`. The launcher emits the game build's events
itself as the `recompile` stage above; `build_game.cmd <id>` builds the lifted modules and
`nx_env.cmd <command>` runs one command with the toolchain's environment.
The launcher already ran `vcvars64.bat` once for the build (`toolchain.rs`) and sets `NX_ENV_READY`, so
`nx_env.cmd` keeps that environment instead of running `vswhere` and `vcvars64.bat` again; started directly, it
runs them itself. Either way it hands `vcvars64.bat` a short `PATH` and merges the result afterwards, so a `PATH`
close to cmd's 8191-character line limit no longer fails the build with `The input line is too long.`

### PowerShell BuildEvents.psm1

Located at `engine/fefw-recomp/runtime/scripts/BuildEvents.psm1`. Provides:
- `Start-BuildEvent`, `Start-StageEvent`, `Progress-StageEvent`
- `Complete-StageEvent`, `Fail-StageEvent`
- `Warning-Event`, `Error-Event`
- `Complete-BuildEvent`, `Cancel-BuildEvent`
- `Initialize-EventWriter`, `Close-EventWriter`

## Troubleshooting

### Tool Not Found

```
Required tool 'clang_cl' not found. Install via: winget install LLVM.LLVM
```

**Fix**: Install missing tool via winget or ensure it's on PATH.

### Version Mismatch

```
Tool 'cmake': version mismatch - found 3.24.0, expected >= 3.25.0
```

**Fix**: Upgrade tool to minimum version.

### Checksum Mismatch

```
Tool 'clang_cl': CHECKSUM MISMATCH - expected abc123..., got different
```

**Fix**: Reinstall tool from official source. Critical tools (clang, lld, cmake, ninja) will fail the build.

### Concurrency Issues

```
Build concurrency: 1 jobs (mode: conservative, cores: 4, RAM: 8 GB)
```

**Fix**: Increase RAM or use `--jobs` override:
```cmd
set NX_JOBS=4
# or
cargo run -- build -- --jobs 4
```

## Future Enhancements

- [ ] SHA-256 verification for all tools (currently null)
- [ ] Auto-install missing optional tools
- [ ] Cross-platform tools.lock (Linux/macOS)
- [ ] Toolchain fingerprint caching
- [ ] Build event persistence to file
- [ ] sccache statistics in build summary