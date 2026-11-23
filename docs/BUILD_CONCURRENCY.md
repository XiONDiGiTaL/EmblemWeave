# EmblemWeave Build Concurrency System

## Overview

The EmblemWeave build system uses **adaptive, memory-aware concurrency calculation** to determine the optimal number of parallel build jobs (`-j N`). This prevents out-of-memory (OOM) crashes on low-RAM machines while maximizing throughput on high-end systems.

## Problem

Traditional approaches use fixed formulas:
- `N = CPU cores` (ignores memory)
- `N = CPU cores - 1` (still ignores memory)
- `N = 16` (arbitrary cap)

These fail because:
- **clang-cl with debug info** uses 1-2 GB per process
- **lld-link** can use 4-8 GB for large links
- **16 jobs × 2 GB = 32 GB** - exceeds most systems
- **OOM kills** leave corrupted build state

## Solution: ConcurrencyCalculator

### Algorithm

```rust
// Available memory for builds
available_mb = total_physical_mb - headroom_mb

// Jobs limited by RAM
jobs_by_mem = floor(available_mb / mem_per_job_mb)

// Jobs limited by CPU
jobs_by_cpu = logical_cores - core_headroom

// Final result
jobs = min(jobs_by_cpu, jobs_by_mem)
jobs = clamp(jobs, min_jobs, max_jobs)
```

### Parameters

| Parameter | Conservative | Aggressive | Max | Description |
|-----------|-------------|------------|-----|-------------|
| `mem_per_job_mb` | 1536 | 1536 | 1536 | Memory per clang-cl job |
| `memory_headroom_mb` | 4096 | 2048 | 0 | System reserve |
| `core_headroom` | 2 | 0 | 0 | Cores left for OS |
| `min_jobs` | 1 | 1 | 1 | Minimum jobs |
| `max_jobs` | 32 | 32 | 32 | Maximum jobs |

### Mode Comparison

| System Specs | Conservative | Aggressive | Max |
|--------------|--------------|------------|-----|
| 8 cores, 16 GB RAM | 4 | 6 | 8 |
| 16 cores, 32 GB RAM | 8 | 14 | 16 |
| 32 cores, 64 GB RAM | 14 | 30 | 32 (capped) |
| 8 cores, 8 GB RAM | 2 | 3 | 8 |
| 4 cores, 8 GB RAM | 1 | 2 | 4 |

## Implementation

### Rust (toolchain_manager.rs)

```rust
use crate::toolchain_manager::{ConcurrencyCalculator, ConcurrencyConfig, ConcurrencyResult};

// Default conservative config
let config = ConcurrencyConfig::default();
// ConcurrencyConfig {
//     mode: "conservative",
//     manual_override: None,
//     memory_per_job_mb: Some(1536),
//     memory_headroom_mb: Some(4096),
//     min_jobs: 1,
//     max_jobs: Some(32),
// }

let calc = ConcurrencyCalculator::new(config);
let result: ConcurrencyResult = calc.calculate();

println!("Jobs: {}", result.calculated_jobs);
println!("Available RAM: {} MB", result.available_memory_mb);
println!("Mode: {}", result.mode);

// With manual override
let result = calc.calculate_with_override(Some(16));
// result.calculated_jobs == 16
```

**Result Structure:**
```rust
pub struct ConcurrencyResult {
    pub calculated_jobs: u32,
    pub available_memory_mb: u64,
    pub memory_per_job_mb: u32,
    pub memory_headroom_mb: u32,
    pub logical_cores: u32,
    pub core_headroom: u32,
    pub mode: String,
    pub limited_by: String,  // "memory" | "cpu" | "manual" | "min" | "max"
}
```

### PowerShell (build.ps1)

```powershell
# Conservative (default)
$jobs = Get-AdaptiveJobCount -Mode conservative

# Aggressive
$jobs = Get-AdaptiveJobCount -Mode aggressive

# Max (no RAM limit)
$jobs = Get-AdaptiveJobCount -Mode max

# Manual override
$jobs = Get-AdaptiveJobCount -Mode conservative -ManualOverride 12
```

**Output:**
```
Build concurrency: 8 jobs (mode: conservative, cores: 16, RAM: 32 GB)
```

### Command Line Override

```bash
# Via environment variable
export NX_JOBS=12
cargo run -- build

# Via CLI flag (if implemented)
cargo run -- build -- --jobs 12
```

## Memory Estimation

### Per-Job Memory Usage

| Component | Typical | Peak | Notes |
|-----------|---------|------|-------|
| clang-cl (compile) | 800 MB | 2 GB | With debug info |
| lld-link (link) | 2 GB | 8 GB | Large executables |
| rustc (compile) | 1 GB | 3 GB | With incremental |
| ninja (overhead) | 50 MB | 100 MB | Per job |

### Conservative Defaults

- **mem_per_job_mb = 1536** (1.5 GB) - covers clang-cl + overhead
- **memory_headroom_mb = 4096** (4 GB) - OS, browser, IDE, etc.

### Tuning for Your Machine

```rust
// High-memory workstation (64 GB+)
let config = ConcurrencyConfig {
    mode: "aggressive".to_string(),
    memory_headroom_mb: Some(8192),  // 8 GB headroom
    memory_per_job_mb: Some(2048),   // 2 GB per job
    max_jobs: Some(48),
    ..Default::default()
};

// Low-memory laptop (8 GB)
let config = ConcurrencyConfig {
    mode: "conservative".to_string(),
    memory_headroom_mb: Some(3072),  // 3 GB headroom
    memory_per_job_mb: Some(1024),   // 1 GB per job
    max_jobs: Some(4),
    ..Default::default()
};
```

## Integration Points

### engine.rs (Rust Build Pipeline)

```rust
// Get adaptive concurrency from ToolchainManager
let concurrency = toolchain_mgr.calculate_jobs(None);
let jobs = concurrency.calculated_jobs;

// Pass to build_game.cmd via NX_JOBS
let mut envs: Vec<(&str, &str)> = vec![("NX_JOBS", &jobs.to_string())];
run_env(ctx, &build_view.path, "cmd", &["/C", "recomp\\scripts\\build_game.cmd", "fefw"], &envs);
```

### build_game.cmd (Game Module Build)

```cmd
rem Uses NX_JOBS from environment, falls back to adaptive
if not defined NX_JOBS set /a NX_JOBS=%NUMBER_OF_PROCESSORS%-2
if %NX_JOBS% GTR 16 set "NX_JOBS=16"

call "%HERE%nx_env.cmd" cmake --build "%NX_BUILD%" --target %GAME%_modules -- -j %NX_JOBS%
```

### build.ps1 (C++ Runtime Build)

```powershell
# Resolve job count: explicit -Jobs > NX_JOBS env > adaptive
if ($Jobs -le 0) {
    $Jobs = if ($env:NX_JOBS) { [int]$env:NX_JOBS } else { Get-AdaptiveJobCount -Mode $ConcurrencyMode }
}

$bld = "cmake --build `"$bd`" -j $Jobs"
```

## Monitoring & Debugging

### Log Output

```
toolchain fingerprint: C:\engine\fefw-recomp|a1b2c3d4
build jobs: 8 (conservative: 8 jobs, 16 cores, 32 GB RAM)
```

### Structured Events

```json
{
  "event": "build_started",
  "config": {
    "jobs": 8,
    "memory_limit_mb": 28672,
    "sccache_enabled": true,
    "incremental": true,
    "target": "Release"
  }
}
```

### Manual Verification

```powershell
# Check calculated values
$cores = (Get-CimInstance Win32_Processor | Measure-Object -Property NumberOfLogicalProcessors -Sum).Sum
$memGB = [math]::Floor((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)
$byCores = [math]::Max(1, $cores - 2)
$byMem = [math]::Max(1, [math]::Floor(($memGB - 4) / 1.5))
$jobs = [math]::Min($byCores, $byMem)
Write-Host "Cores: $cores, RAM: $memGB GB, Jobs: $jobs (by cores: $byCores, by mem: $byMem)"
```

## Troubleshooting

### OOM During Build

**Symptoms**: Build crashes with "out of memory", system becomes unresponsive

**Solutions**:
1. Use conservative mode (default)
2. Reduce `NX_JOBS` manually: `set NX_JOBS=4`
3. Increase swap/page file
4. Close other applications
5. Disable debug info: `CMAKE_BUILD_TYPE=Release` (default)

### Too Few Jobs (Slow Build)

**Symptoms**: Build takes longer than expected, CPU underutilized

**Solutions**:
1. Check RAM: `Get-CimInstance Win32_ComputerSystem | Select-Object TotalPhysicalMemory`
2. Use aggressive mode: `-ConcurrencyMode aggressive`
3. Increase `NX_JOBS` manually
4. Verify `mem_per_job_mb` isn't overestimated

### Inconsistent Job Count

**Symptoms**: Different job counts between builds

**Causes**:
- `NX_JOBS` env var set
- Different mode selected
- Memory pressure from other processes

**Debug**:
```powershell
# Check current env
Get-ChildItem Env:NX_JOBS

# Run with verbose
.\build.ps1 -Verbose -ConcurrencyMode conservative
```

## Best Practices

### For Users

1. **Default is safe**: Conservative mode works on all machines
2. **Override if needed**: `set NX_JOBS=12` for known-good configs
3. **Monitor first build**: Watch memory usage in Task Manager
4. **Use aggressive on CI**: Known hardware, no other workloads

### For Developers

1. **Test on low-RAM**: Verify 8 GB machines work
2. **Profile memory**: Use `/usr/bin/time -v` or Windows Performance Analyzer
3. **Document assumptions**: `mem_per_job_mb = 1536` based on measurements
4. **Add telemetry**: Track `limited_by` field in build events

### For CI/CD

```yaml
# GitHub Actions - use aggressive on hosted runners
- name: Build
  run: |
    .\build.ps1 -ConcurrencyMode aggressive
  env:
    NX_JOBS: 16  # Override for known runner specs
```

## Future Enhancements

- [ ] Dynamic adjustment during build (reduce jobs on memory pressure)
- [ ] Per-stage concurrency (link stage needs more memory)
- [ ] sccache-aware scheduling (cache hits need less memory)
- [ ] NUMA-aware scheduling (multi-socket systems)
- [ ] Container-aware limits (cgroups memory limits)
- [ ] Build-time telemetry collection