# Phase D: Deployment Guide

## Overview

This guide covers the production deployment process for EmblemWeave after Phase D hardening. It includes pre-deployment checks, deployment steps, verification, and rollback procedures.

## Prerequisites

### Build Environment
- Windows 10/11 with Visual Studio 2022 Community or later
- MSVC 14.52.37025+
- LLVM/Clang 23.1.5+
- CMake 4.4.4+
- Ninja 1.13.2+
- Rust 1.85+ (MSVC toolchain)

### Target Deployment
- Windows 10/11 (x64)
- Minimum 8 GB RAM (16 GB recommended for builds)
- 20 GB free disk space for storage systems
- GPU with Vulkan 1.3+ support (for runtime)

## Pre-Deployment Checklist

### 1. Build Verification
```bash
# Run full test suite
cd apps/emblemweave
cargo test --release

# Verify all 378+ tests pass
# Expected: 378 passed, 0 failed, 11 ignored

# Build production binary
cargo build --release

# Verify binary exists and has correct size
ls -la target/release/EmblemWeave.exe
# Expected: ~18.5 MB
```

### 2. Test Results Verification
All Phase D tests must pass:
- [ ] storage_manager: 9/9 tests
- [ ] romfs_store: 7/7 tests
- [ ] object_store: 5/5 tests
- [ ] pipeline_cache: 4/4 tests
- [ ] Full regression suite: 378+ tests

### 3. Backup Current Deployment
```powershell
# Create timestamped backup directory
$timestamp = Get-Date -Format "yyyyMMdd-HHmmss"
$backupDir = "E:\EmblemWeave\backups\pre-deploy-$timestamp"
New-Item -ItemType Directory -Path $backupDir -Force

# Backup current binary
Copy-Item "E:\EmblemWeave\EmblemWeave.exe" "$backupDir\"

# Verify backup
Get-Item "$backupDir\EmblemWeave.exe"
```

### 4. Verify User Data Directories
Ensure these directories exist and are writable:
- `E:\EmblemWeave\data\` - User config, saves, logs
- `E:\EmblemWeave\backups\` - Binary backups
- `E:\EmblemWeave\engine\` - Runtime engine
- `E:\EmblemWeave\upscalers\` - Upscaler DLLs

## Deployment Steps

### Step 1: Deploy Binary
```powershell
# Copy new release binary
Copy-Item "C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\target\release\EmblemWeave.exe" "E:\EmblemWeave\EmblemWeave.exe" -Force

# Verify deployment
Get-Item "E:\EmblemWeave\EmblemWeave.exe"
# Expected: 18,508,800 bytes (18.5 MB), current timestamp
```

### Step 2: Verify Storage Systems Initialize
Run the launcher once to initialize storage systems:
```powershell
# Start EmblemWeave (will initialize storage on first run)
Start-Process "E:\EmblemWeave\EmblemWeave.exe"

# Wait 10 seconds for initialization
Start-Sleep -Seconds 10

# Check storage directories created
Get-ChildItem "E:\EmblemWeave\data\build" -Directory
# Expected: versions, pipeline_cache, object_store, lift_store, hardlink_store
```

### Step 3: Run Storage Health Check
```powershell
# Verify storage manager can collect stats
# (Requires CLI integration - use debug build for now)
cd "C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave\apps\emblemweave"
cargo run --bin EmblemWeave -- --help 2>&1 | head -20
```

### Step 4: Verify User Data Preservation
```powershell
# Check critical user data files
Get-Item "E:\EmblemWeave\data\config.json"
Get-Item "E:\EmblemWeave\data\engine.txt"
Get-ChildItem "E:\EmblemWeave\data\save" -File
Get-ChildItem "E:\EmblemWeave\data\play" -File
```

## Post-Deployment Verification

### Functional Tests
1. **Launcher Starts**: GUI opens without errors
2. **Settings Load**: Config.json reads correctly
3. **Game Detection**: Detects game installation
4. **Build System**: Can initiate a build (dry-run)
5. **Storage Stats**: Shows initialized storage systems

### Performance Verification
```bash
# Clean build benchmark
time cargo build --release
# Target: < 60 seconds

# Incremental build benchmark
touch src/main.rs && time cargo build --release
# Target: < 20 seconds

# Test suite benchmark
time cargo test
# Target: < 30 seconds
```

### Storage Efficiency Verification
After several builds, verify:
- Hard-link deduplication: 5x reduction (40GB → 8GB for 20 builds)
- Pipeline cache deduplication: 4x reduction
- Lift reuse rate: > 70%

## Rollback Procedure

### Emergency Rollback (< 5 minutes)
```powershell
# 1. Stop EmblemWeave if running
Stop-Process -Name "EmblemWeave" -Force -ErrorAction SilentlyContinue

# 2. Restore previous binary
$latestBackup = Get-ChildItem "E:\EmblemWeave\backups\" -Directory | Sort-Object LastWriteTime -Descending | Select-Object -First 1
Copy-Item "$($latestBackup.FullName)\EmblemWeave.exe" "E:\EmblemWeave\EmblemWeave.exe" -Force

# 3. Verify rollback
Get-Item "E:\EmblemWeave\EmblemWeave.exe"
Start-Process "E:\EmblemWeave\EmblemWeave.exe"
```

### Full Rollback (with storage reset)
```powershell
# WARNING: This resets all storage systems
# 1. Emergency rollback (above)
# 2. Remove storage directories
Remove-Item "E:\EmblemWeave\data\build" -Recurse -Force -ErrorAction SilentlyContinue

# 3. Restart launcher (reinitializes storage)
Start-Process "E:\EmblemWeave\EmblemWeave.exe"
```

## Deployment Artifacts

### Files to Deploy
| File | Source | Destination | Size |
|------|--------|-------------|------|
| EmblemWeave.exe | target/release/ | E:\EmblemWeave\ | 18.5 MB |

### Directories Preserved (NOT deployed)
| Directory | Purpose |
|-----------|---------|
| data/ | User config, saves, logs, engine.txt |
| backups/ | Binary backups |
| docs/ | Documentation |
| engine/ | Runtime engine files |
| upscalers/ | DLSS, XeSS, FSR, NIS, MAKO DLLs |

## Verification Checklist

- [ ] Binary deployed and verified (18.5 MB)
- [ ] Backup created in backups/pre-deploy-<timestamp>/
- [ ] User data directory (data/) intact
- [ ] Launcher starts successfully
- [ ] Settings load from config.json
- [ ] Game detection works
- [ ] Storage systems initialize (build/ subdirectories created)
- [ ] No errors in launcher-run.log
- [ ] Performance benchmarks meet targets

## Troubleshooting

### Launcher Won't Start
1. Check `E:\EmblemWeave\launcher-run.log` for errors
2. Verify Visual C++ Redistributable installed
3. Try running from command prompt to see console output (use debug build)

### Storage Errors
1. Check disk space: `Get-PSDrive E | Select Used,Free`
2. Verify write permissions on `E:\EmblemWeave\data\`
3. Run storage cleanup: (requires CLI integration)

### Build Failures
1. Verify toolchain: `--fetch-tools` flag
2. Check toolchain.log in data/
3. Ensure 16+ GB free space for builds

## Support Contacts

- **Build Issues**: Check `data/logs/build-*.log`
- **Runtime Issues**: Check `data/logs/runtime-*.log`
- **Storage Issues**: Check `data/logs/storage-*.log`
- **Crash Reports**: `data/crash/` directory