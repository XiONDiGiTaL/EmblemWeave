# EmblemWeave Structured Build Protocol

## Overview

The EmblemWeave build system emits **JSON Lines** (newline-delimited JSON) structured events to stdout during the build process. This protocol enables:

- **Real-time monitoring** in CI/CD pipelines
- **GUI progress tracking** with stage-level granularity
- **Automated failure analysis** and alerting
- **Structured log aggregation** (ELK, Splunk, etc.)
- **Build reproducibility** verification

## Protocol Specification

### Transport

- **Stream**: stdout (file descriptor 1)
- **Encoding**: UTF-8
- **Format**: JSON Lines (RFC 7464) - one complete JSON object per line
- **Ordering**: Strictly chronological by emission time

### Event Structure

Every event has these common fields:

```json
{
  "event": "event_type",
  "timestamp": "2026-11-20T10:30:00.000Z",
  ...
}
```

| Field | Type | Description |
|-------|------|-------------|
| `event` | string | Event type identifier |
| `timestamp` | string | ISO 8601 UTC timestamp with milliseconds |

### Event Types

#### 1. build_started

Emitted once at build start.

```json
{
  "event": "build_started",
  "timestamp": "2026-11-20T10:30:00.000Z",
  "build_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "toolchain_fingerprint": "C:\\engine\\fefw-recomp|a1b2c3d4",
  "config": {
    "jobs": 8,
    "memory_limit_mb": 16384,
    "sccache_enabled": true,
    "incremental": true,
    "target": "Release",
    "features": []
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `build_id` | string | UUID v4 identifying this build |
| `toolchain_fingerprint` | string | Hash of toolchain + build ID prefix |
| `config` | object | Build configuration |
| `config.jobs` | integer | Number of parallel jobs |
| `config.memory_limit_mb` | integer\|null | Memory limit per job |
| `config.sccache_enabled` | boolean | Whether sccache is active |
| `config.incremental` | boolean | Incremental build |
| `config.target` | string | Build target (Release/Debug) |
| `config.features` | string[] | Enabled features |

#### 2. stage_started

Emitted when a build stage begins.

```json
{
  "event": "stage_started",
  "timestamp": "2026-11-20T10:30:00.100Z",
  "stage_id": "recompile",
  "stage_name": "Recompile the game code",
  "stage_index": 3,
  "total_stages": 8
}
```

| Field | Type | Description |
|-------|------|-------------|
| `stage_id` | string | Unique stage identifier |
| `stage_name` | string | Human-readable stage name |
| `stage_index` | integer | 1-based stage number |
| `total_stages` | integer | Total stages in build |

**Stage IDs**: `python_env`, `exefs`, `recompile`, `textures`, `fefw_exe`, `shader_translator`, `shaders`, `fefw_ini`, `configure`, `build`, `test`

#### 3. stage_progress

Emitted periodically during stage execution (0.0 to 1.0).

```json
{
  "event": "stage_progress",
  "timestamp": "2026-11-20T10:30:05.000Z",
  "stage_id": "recompile",
  "progress": 0.3,
  "message": "starting build_game.cmd"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `stage_id` | string | Stage identifier |
| `progress` | float | Progress 0.0 (started) to 1.0 (complete) |
| `message` | string | Human-readable status message |

#### 4. stage_completed

Emitted when a stage finishes successfully.

```json
{
  "event": "stage_completed",
  "timestamp": "2026-11-20T10:35:00.000Z",
  "stage_id": "recompile",
  "duration_ms": 300000,
  "success": true,
  "output": "six modules compiled (8 jobs)"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `stage_id` | string | Stage identifier |
| `duration_ms` | integer | Stage duration in milliseconds |
| `success` | boolean | Always `true` for this event |
| `output` | string\|null | Optional stage output summary |

#### 5. stage_failed

Emitted when a stage fails.

```json
{
  "event": "stage_failed",
  "timestamp": "2026-11-20T10:35:00.000Z",
  "stage_id": "recompile",
  "duration_ms": 150000,
  "error": "Ninja build failed with exit code 1"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `stage_id` | string | Stage identifier |
| `duration_ms` | integer | Stage duration before failure |
| `error` | string | Error message |

#### 6. warning

Non-fatal warning during build.

```json
{
  "event": "warning",
  "timestamp": "2026-11-20T10:30:00.500Z",
  "stage_id": "recompile",
  "message": "NASM not installed: AV1 decoder built in C only"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `stage_id` | string\|null | Associated stage, or null |
| `message` | string | Warning message |

#### 7. error

Recoverable or non-recoverable error.

```json
{
  "event": "error",
  "timestamp": "2026-11-20T10:30:00.500Z",
  "stage_id": "recompile",
  "error": "Cache mismatch detected",
  "recoverable": true
}
```

| Field | Type | Description |
|-------|------|-------------|
| `stage_id` | string\|null | Associated stage |
| `error` | string | Error message |
| `recoverable` | boolean | Whether build can continue |

#### 8. build_completed

Emitted once at build end (success or failure).

```json
{
  "event": "build_completed",
  "timestamp": "2026-11-20T10:45:00.000Z",
  "build_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "duration_ms": 900000,
  "success": true,
  "summary": {
    "stages_completed": 8,
    "stages_failed": 0,
    "total_warnings": 2,
    "total_errors": 0,
    "cache_hits": 150,
    "cache_misses": 10,
    "peak_memory_mb": 8192
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `build_id` | string | Same as build_started |
| `duration_ms` | integer | Total build duration |
| `success` | boolean | Overall build success |
| `summary` | object | Build statistics |
| `summary.stages_completed` | integer | Successful stages |
| `summary.stages_failed` | integer | Failed stages |
| `summary.total_warnings` | integer | Warning count |
| `summary.total_errors` | integer | Error count |
| `summary.cache_hits` | integer | sccache hits |
| `summary.cache_misses` | integer | sccache misses |
| `summary.peak_memory_mb` | integer | Peak memory usage |

#### 9. build_cancelled

Emitted when build is explicitly cancelled.

```json
{
  "event": "build_cancelled",
  "timestamp": "2026-11-20T10:40:00.000Z",
  "build_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "reason": "User requested cancellation"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `build_id` | string | Same as build_started |
| `reason` | string | Cancellation reason |

## Stage Definitions

### Rust Build Pipeline (engine.rs)

| Stage ID | Index | Name | Description |
|----------|-------|------|-------------|
| `python_env` | 1 | Python environment | Create/verify uv venv, install requirements |
| `exefs` | 2 | Prepare the ExeFS | Apply patches, prepare main NSO |
| `recompile` | 3 | Recompile the game code | Build lifted C modules via build_game.cmd |
| `textures` | 4 | Texture transcoders | Fetch/build astcenc, bc7enc_rdo, ispc |
| `fefw_exe` | 5 | Build fefw.exe | Runtime executable |
| `shader_translator` | 6 | Shader translator | Build SpirvHarness (.NET) |
| `shaders` | 7 | Translate the shaders | Run translator on shader corpus |
| `fefw_ini` | 8 | Write fefw.ini | Generate runtime config |

### C++ Runtime Build (build.ps1)

| Stage ID | Index | Name | Description |
|----------|-------|------|-------------|
| `configure` | 1 | Configure (CMake) | CMake configure step |
| `build` | 2 | Build (Ninja) | Ninja compilation |
| `test` | 3 | Test (CTest) | Optional test execution |

## Consumer Examples

### jq (Command Line)

```bash
# Pretty print all events
cargo run -- build 2>&1 | jq .

# Filter stage completions
cargo run -- build 2>&1 | jq 'select(.event == "stage_completed")'

# Build summary only
cargo run -- build 2>&1 | jq 'select(.event == "build_completed")'

# Progress bar for specific stage
cargo run -- build 2>&1 | jq -r 'select(.event == "stage_progress" and .stage_id == "recompile") | "\(.progress * 100 | floor)% \(.message)"'
```

### PowerShell

```powershell
# Parse all events
$events = & cargo run -- build 2>&1 | ConvertFrom-Json

# Show stage timeline
$events | Where-Object { $_.event -in @('stage_started','stage_completed','stage_failed') } |
    Select-Object timestamp, event, stage_id, stage_name, duration_ms, success, output

# Build summary
$events | Where-Object { $_.event -eq 'build_completed' } |
    Select-Object -ExpandProperty summary

# Check for failures
$failures = $events | Where-Object { $_.event -eq 'stage_failed' }
if ($failures) { $failures | Format-Table timestamp, stage_id, error }
```

### Python

```python
import json, subprocess, sys

proc = subprocess.Popen(
    ["cargo", "run", "--", "build"],
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    text=True,
    bufsize=1
)

stage_times = {}
for line in proc.stdout:
    try:
        event = json.loads(line.strip())
    except json.JSONDecodeError:
        continue  # Skip non-JSON lines

    if event["event"] == "stage_started":
        stage_times[event["stage_id"]] = {"start": event["timestamp"], "name": event["stage_name"]}
    elif event["event"] == "stage_completed":
        if event["stage_id"] in stage_times:
            print(f"✓ {stage_times[event['stage_id']]['name']}: {event['duration_ms']}ms")
    elif event["event"] == "stage_failed":
        print(f"✗ {event['stage_id']}: {event['error']}")
    elif event["event"] == "build_completed":
        s = event["summary"]
        print(f"\nBuild {'succeeded' if event['success'] else 'failed'} in {event['duration_ms']}ms")
        print(f"  Stages: {s['stages_completed']} ok, {s['stages_failed']} failed")
        print(f"  Cache: {s['cache_hits']} hits, {s['cache_misses']} misses")

proc.wait()
```

### Rust (Serde)

```rust
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};

#[derive(Debug, Deserialize)]
#[serde(tag = "event")]
enum BuildEvent {
    #[serde(rename = "build_started")]
    BuildStarted { build_id: String, config: BuildConfig, .. },
    #[serde(rename = "stage_started")]
    StageStarted { stage_id: String, stage_name: String, .. },
    #[serde(rename = "stage_progress")]
    StageProgress { stage_id: String, progress: f32, message: String },
    #[serde(rename = "stage_completed")]
    StageCompleted { stage_id: String, duration_ms: u64, success: bool, output: Option<String> },
    #[serde(rename = "stage_failed")]
    StageFailed { stage_id: String, duration_ms: u64, error: String },
    #[serde(rename = "warning")]
    Warning { stage_id: Option<String>, message: String },
    #[serde(rename = "error")]
    Error { stage_id: Option<String>, error: String, recoverable: bool },
    #[serde(rename = "build_completed")]
    BuildCompleted { build_id: String, duration_ms: u64, success: bool, summary: BuildSummary },
    #[serde(rename = "build_cancelled")]
    BuildCancelled { build_id: String, reason: String },
}

#[derive(Debug, Deserialize)]
struct BuildConfig {
    jobs: u32,
    memory_limit_mb: Option<u32>,
    sccache_enabled: bool,
    incremental: bool,
    target: String,
    features: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BuildSummary {
    stages_completed: u32,
    stages_failed: u32,
    total_warnings: u32,
    total_errors: u32,
    cache_hits: u32,
    cache_misses: u32,
    peak_memory_mb: u32,
}

fn monitor_build() -> Result<(), Box<dyn std::error::Error>> {
    let mut child = std::process::Command::new("cargo")
        .args(["run", "--", "build"])
        .stdout(std::process::Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    for line in reader.lines() {
        let line = line?;
        if let Ok(event) = serde_json::from_str::<BuildEvent>(&line) {
            match event {
                BuildEvent::StageCompleted { stage_id, duration_ms, .. } => {
                    println!("✓ {} completed in {}ms", stage_id, duration_ms);
                }
                BuildEvent::StageFailed { stage_id, error, .. } => {
                    eprintln!("✗ {} failed: {}", stage_id, error);
                }
                BuildEvent::BuildCompleted { success, summary, .. } => {
                    println!("Build {}: {} stages, {} warnings",
                        if success { "succeeded" } else { "failed" },
                        summary.stages_completed, summary.total_warnings);
                }
                _ => {}
            }
        }
    }

    child.wait()?;
    Ok(())
}
```

## Integration with CI/CD

### GitHub Actions

```yaml
- name: Build EmblemWeave
  run: |
    cargo run -- build 2>&1 | tee build.log
    # Parse structured events for annotations
    python3 -c "
import json, sys
for line in open('build.log'):
    try:
        e = json.loads(line)
        if e['event'] == 'stage_failed':
            print(f'::error file=engine.rs,title=Stage {e[\"stage_id\"]} failed::{e[\"error\"]}')
        elif e['event'] == 'warning':
            print(f'::warning file=engine.rs,title=Stage {e.get(\"stage_id\", \"unknown\")}::{e[\"message\"]}')
    except: pass
"
```

### Azure Pipelines

```yaml
- script: |
    cargo run -- build 2>&1 | tee build.log
    python3 -c "
import json, sys
for line in open('build.log'):
    try:
        e = json.loads(line)
        if e['event'] == 'stage_failed':
            print(f'##vso[task.logissue type=error;]Stage {e[\"stage_id\"]} failed: {e[\"error\"]}')
        elif e['event'] == 'build_completed' and not e['success']:
            print(f'##vso[task.logissue type=error;]Build failed after {e[\"duration_ms\"]}ms')
    except: pass
"
```

## Backward Compatibility

- **Non-JSON lines** are still emitted for human-readable logs
- **Structured events** are additive - existing log parsing continues to work
- **Opt-in**: Use `--structured-events` flag or `RUST_LOG=structured` env var
- **Version**: Protocol version tracked in `build_started.config.protocol_version`

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Malformed JSON line | Skip, continue parsing |
| Missing required field | Treat as unknown event |
| Unknown event type | Ignore |
| Stream closed early | Emit `build_cancelled` if possible |
| Consumer lag | Events buffered by OS pipe (64KB typical) |

## Performance

- **Overhead**: <1ms per event emission
- **Throughput**: 10,000+ events/second
- **Memory**: Constant (streaming, no accumulation)
- **Network**: JSON Lines works over pipes, sockets, files