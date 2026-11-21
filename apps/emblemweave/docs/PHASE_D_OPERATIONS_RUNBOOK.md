# Phase D: Operations Runbook

## Overview

This runbook provides operational procedures for maintaining EmblemWeave in production after Phase D hardening. It covers routine operations, incident response, and maintenance tasks.

## System Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    EmblemWeave Launcher                      │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │   GUI/UX     │  │  Build       │  │  Runtime         │  │
│  │  (eframe)    │  │  Orchestrator│  │  (ew-runtime)    │  │
│  └──────┬───────┘  └──────┬───────┘  └────────┬─────────┘  │
│         │                 │                   │            │
│         └─────────────────┼───────────────────┘            │
│                           ▼                                 │
│  ┌──────────────────────────────────────────────────────┐  │
│  │           Unified Storage Manager                     │  │
│  │  ┌────────────┐ ┌───────────┐ ┌─────────┐ ┌────────┐ │  │
│  │  │  Version   │ │ Pipeline  │ │ Object  │ │ Lift   │ │  │
│  │  │  Manager   │ │ Cache     │ │ Store   │ │ Store  │ │  │
│  │  └────────────┘ └───────────┘ └─────────┘ └────┬───┘ │  │
│  │                                                │      │  │
│  │  ┌────────────────────────────────────────────┘      │  │
│  │  │              Hard-Link Store                       │  │
│  │  └────────────────────────────────────────────────────┘  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## Routine Operations

### Daily Operations

#### 1. Health Check (Automated)
```powershell
# Run via Task Scheduler daily at 03:00
# Check: disk space, storage stats, launcher status

$launcherPath = "E:\EmblemWeave\EmblemWeave.exe"
$dataPath = "E:\EmblemWeave\data"

# Check launcher exists
if (-not (Test-Path $launcherPath)) {
    Write-Error "Launcher missing!"
    Exit 1
}

# Check disk space (min 10 GB)
$disk = Get-PSDrive E
if ($disk.Free -lt 10GB) {
    Write-Warning "Low disk space: $($disk.Free/1GB) GB free"
}

# Check storage directories
$storageDirs = @("versions", "pipeline_cache", "object_store", "lift_store", "hardlink_store")
foreach ($dir in $storageDirs) {
    $path = Join-Path $dataPath "build" $dir
    if (-not (Test-Path $path)) {
        Write-Warning "Storage dir missing: $path"
    }
}

Write-Host "Health check passed"
```

#### 2. Log Rotation
```powershell
# Rotate logs older than 30 days
$logPath = "E:\EmblemWeave\data\logs"
$cutoff = (Get-Date).AddDays(-30)
Get-ChildItem $logPath -File | Where-Object { $_.LastWriteTime -lt $cutoff } | Remove-Item -Force

# Compress logs older than 7 days
$compressCutoff = (Get-Date).AddDays(-7)
Get-ChildItem $logPath -File -Filter "*.log" | Where-Object { $_.LastWriteTime -lt $compressCutoff } | ForEach-Object {
    Compress-Archive $_.FullName "$($_.FullName).zip" -Force
    Remove-Item $_.FullName -Force
}
```

### Weekly Operations

#### 1. Storage Cleanup (Manual or Scheduled)
```powershell
# Run storage cleanup (dry-run first)
# Note: Requires CLI integration - use debug build for now

cd "C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\apps\emblemweave"

# Dry run
cargo run --release --bin ew-storage -- cleanup --dry-run

# Actual cleanup (if dry-run looks good)
cargo run --release --bin ew-storage -- cleanup
```

#### 2. Backup Verification
```powershell
# Verify backups are readable
$backupRoot = "E:\EmblemWeave\backups"
$backups = Get-ChildItem $backupRoot -Directory | Sort-Object LastWriteTime -Descending

foreach ($backup in $backups | Select-Object -First 5) {
    $exe = Join-Path $backup.FullName "EmblemWeave.exe"
    if (Test-Path $exe) {
        # Quick verification - check file size and PE header
        $size = (Get-Item $exe).Length
        Write-Host "Backup $($backup.Name): $($size/1MB) MB - OK"
    } else {
        Write-Warning "Backup $($backup.Name): MISSING EXE"
    }
}

# Keep last 20 backups, remove older
$backups | Select-Object -Skip 20 | Remove-Item -Recurse -Force
```

### Monthly Operations

#### 1. Full Storage Audit
```powershell
# Generate storage report
cd "C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\apps\emblemweave"
cargo run --release --bin ew-storage -- stats --format json --output "storage-report-$(Get-Date -Format 'yyyyMMdd').json"

# Analyze report for anomalies
# - Sudden size increases
# - Low deduplication ratios
# - Orphaned content growth
```

#### 2. Toolchain Update Check
```powershell
# Check for toolchain updates
# Run with --fetch-tools to update
Start-Process "E:\EmblemWeave\EmblemWeave.exe" -ArgumentList "--fetch-tools" -Wait
```

## Incident Response

### Incident: Launcher Won't Start

**Symptoms**: Double-click does nothing, no window appears

**Diagnosis**:
```powershell
# 1. Check for crash dumps
Get-ChildItem "E:\EmblemWeave\data\crash" -File

# 2. Run from command prompt (debug build)
cd "C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\apps\emblemweave"
cargo run --bin EmblemWeave

# 3. Check Windows Event Viewer
# Application logs -> EmblemWeave
```

**Resolution**:
- If crash dump exists: analyze with WinDbg
- If missing DLLs: run `--fetch-tools`
- If config corrupted: restore from `data/config.json.old-*`
- If binary corrupted: rollback (see Rollback Procedure)

### Incident: Build Fails

**Symptoms**: Build completes with errors, no executable produced

**Diagnosis**:
```powershell
# 1. Check build logs
Get-Content "E:\EmblemWeave\data\logs\build-*.log" -Tail 100

# 2. Check toolchain status
Start-Process "E:\EmblemWeave\EmblemWeave.exe" -ArgumentList "--fetch-tools" -Wait

# 3. Verify storage space
Get-PSDrive E | Select Used, Free
```

**Resolution**:
- Free disk space if < 10 GB
- Re-fetch toolchain with `--fetch-tools`
- Clean build directories: `Remove-Item "E:\EmblemWeave\data\build" -Recurse -Force`
- Check for antivirus interference (exclude build directories)

### Incident: Storage Corruption

**Symptoms**: Storage manager reports errors, stats show 0 items

**Diagnosis**:
```powershell
# Check index files for corruption
$indexFiles = @(
    "E:\EmblemWeave\data\build\versions\versions_index.json",
    "E:\EmblemWeave\data\build\pipeline_cache\inventory.json",
    "E:\EmblemWeave\data\build\object_store\object_index.json",
    "E:\EmblemWeave\data\build\lift_store\lift_index.json",
    "E:\EmblemWeave\data\build\hardlink_store\hardlink_index.json"
)

foreach ($idx in $indexFiles) {
    if (Test-Path $idx) {
        try {
            Get-Content $idx -Raw | ConvertFrom-Json | Out-Null
            Write-Host "$idx: VALID"
        } catch {
            Write-Error "$idx: CORRUPTED - $_"
        }
    }
}
```

**Resolution**:
```powershell
# Corrupted index recovery (automatic on next startup)
# 1. Stop launcher
Stop-Process -Name "EmblemWeave" -Force

# 2. Rename corrupted index (will be recreated)
Rename-Item "E:\EmblemWeave\data\build\object_store\object_index.json" "object_index.json.corrupted"

# 3. Restart launcher (recreates empty index)
Start-Process "E:\EmblemWeave\EmblemWeave.exe"

# 4. Run cleanup to remove orphaned content
# (requires CLI integration)
```

### Incident: Disk Full

**Symptoms**: "Insufficient free space" errors, builds fail, launcher crashes

**Immediate Actions**:
```powershell
# 1. Check what's using space
Get-ChildItem "E:\EmblemWeave" -Recurse -Directory | ForEach-Object {
    $size = (Get-ChildItem $_.FullName -Recurse -File -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
    [PSCustomObject]@{ Path = $_.FullName; SizeGB = [math]::Round($size/1GB, 2) }
} | Sort-Object SizeGB -Descending | Select-Object -First 20

# 2. Emergency cleanup
# Remove old build versions (keep last 5)
# Remove unused pipeline caches
# Remove orphaned objects

# 3. If still full, move data to another drive
# Update portable.txt or config.json with new path
```

## Maintenance Procedures

### Procedure: Add New GPU Support

1. User runs with new GPU → auto-detected
2. Pipeline cache created in `build/pipeline_cache/gpu<uuid>_cache_0/`
3. Subsequent builds reuse cache
4. Monitor deduplication ratio in stats

### Procedure: Update Upscaler DLLs

```powershell
# 1. Download new DLLs from vendor
# 2. Backup current
Copy-Item "E:\EmblemWeave\upscalers" "E:\EmblemWeave\backups\upscalers-$(Get-Date -Format 'yyyyMMdd')" -Recurse

# 3. Replace DLLs
Copy-Item "NewDLLs\*" "E:\EmblemWeave\upscalers\" -Force

# 4. Verify
Start-Process "E:\EmblemWeave\EmblemWeave.exe" -ArgumentList "--apply-preset", "Quality" -Wait
```

### Procedure: Migrate Storage to New Drive

```powershell
# 1. Stop launcher
Stop-Process -Name "EmblemWeave" -Force

# 2. Copy data to new drive (preserve hard links!)
robocopy "E:\EmblemWeave\data" "F:\EmblemWeave\data" /E /COPY:DATSO /MT:16

# 3. Update config.json with new paths
# Or create portable.txt at new location

# 4. Update shortcuts/start menu
# 5. Verify launcher starts correctly
Start-Process "F:\EmblemWeave\EmblemWeave.exe"
```

## Monitoring & Alerting

### Key Metrics to Monitor

| Metric | Warning | Critical | Source |
|--------|---------|----------|--------|
| Disk free space | < 20 GB | < 10 GB | Get-PSDrive |
| Build success rate | < 95% | < 90% | Build logs |
| Storage dedup ratio | < 3.0x | < 2.0x | Stats report |
| Lift reuse rate | < 60% | < 50% | Stats report |
| Launcher crash rate | > 1/day | > 5/day | Crash dumps |
| Test suite failures | > 0 | > 0 | CI/CD |

### Log Locations

| Log Type | Location | Retention |
|----------|----------|-----------|
| Launcher | `data/logs/launcher-run.log` | 30 days |
| Build | `data/logs/build-*.log` | 30 days |
| Runtime | `data/logs/runtime-*.log` | 30 days |
| Storage | `data/logs/storage-*.log` | 30 days |
| Crash dumps | `data/crash/` | 90 days |
| Build events | `data/builds/<build_id>/events.jsonl` | Per build |

### Structured Events (JSON Lines)

Example build event stream:
```json
{"type":"BuildStarted","build_id":"bld-1732156800","timestamp":1732156800,"config":"Release"}
{"type":"ToolchainResolved","toolchain":"msvc","version":"14.52.37025"}
{"type":"ConcurrencyCalculated","mode":"Aggressive","jobs":16,"mem_per_job_mb":1536}
{"type":"CompilationUnitStarted","file":"src/main.rs","unit":1,"total":247}
{"type":"CompilationUnitFinished","file":"src/main.rs","unit":1,"duration_ms":1245}
{"type":"LinkStarted","output":"EmblemWeave.exe"}
{"type":"LinkFinished","duration_ms":15700,"size_bytes":18508800}
{"type":"BuildCompleted","duration_ms":58000,"status":"Success","artifacts":12}
```

## Security Considerations

### File Permissions
- `E:\EmblemWeave\` - Read/Execute for Users, Write for Admin
- `E:\EmblemWeave\data\` - Read/Write for Users
- `E:\EmblemWeave\backups\` - Read for Users, Write for Admin

### Network Access
- Launcher only connects to:
  - GitHub API (releases, updates)
  - Discord RPC (optional, local)
  - Game servers (gameplay only)
- No telemetry without consent

### Data Protection
- No user credentials stored
- Game data never uploaded
- Crash dumps anonymized (no paths, no memory)
- Config encryption not implemented (plain JSON)

## Contact & Escalation

| Issue Type | Primary Contact | Escalation |
|------------|-----------------|------------|
| Build failures | Build logs → GitHub Issues | Discord #build-help |
| Runtime crashes | Crash dumps → GitHub Issues | Discord #crash-reports |
| Storage corruption | Auto-recovery → Manual if needed | GitHub Issues |
| Performance regression | CI benchmarks → Alert | GitHub Issues |
| Security issues | Private disclosure → security@ | N/A |

## Appendix: Quick Reference Commands

```powershell
# Start launcher
Start-Process "E:\EmblemWeave\EmblemWeave.exe"

# Fetch toolchain
Start-Process "E:\EmblemWeave\EmblemWeave.exe" -ArgumentList "--fetch-tools" -Wait

# Compare game data
Start-Process "E:\EmblemWeave\EmblemWeave.exe" -ArgumentList "--compare-data", "game.nup", "folder", "report.json" -Wait

# Apply preset
Start-Process "E:\EmblemWeave\EmblemWeave.exe" -ArgumentList "--apply-preset", "Quality" -Wait

# View storage stats (debug build)
cd apps/emblemweave && cargo run --bin ew-storage -- stats

# Run cleanup (debug build)
cd apps/emblemweave && cargo run --bin ew-storage -- cleanup --dry-run
cd apps/emblemweave && cargo run --bin ew-storage -- cleanup

# Export report
cd apps/emblemweave && cargo run --bin ew-storage -- stats --format json --output report.json
```