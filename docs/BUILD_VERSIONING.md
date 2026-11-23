# Build Directory Versioning and Rollback

## Overview

The Build Versioning and Rollback system provides isolated, reproducible build environments with atomic promotion, validation, and safe rollback capabilities. This is the first component of Phase C (Build Directory Versioning and Rollback) in the EmblemWeave optimization initiative.

## Architecture

```
build/versions/
├── build_versions.json          # Metadata index (all build identities)
├── current -> <staging_dir>     # Symlink to current promoted build
├── previous -> <staging_dir>    # Symlink to previous promoted build
├── 1700000000-release-abc123/   # Staging directory (timestamp-config-commit)
│   ├── build_identity.json      # Build metadata
│   └── build/                   # Actual CMake build output
│       └── release/
│           ├── nxrt_gfx_demo.exe
│           ├── nxrt.dll
│           └── ...
├── 1700000100-release-def456/   # Another build version
│   ├── build_identity.json
│   └── build/
│       └── release/
└── ...
```

## Key Concepts

### Build Identity

Each build gets a unique identity containing:

- **Build ID**: UUID for tracking
- **Git Commit**: Full commit hash + short (10-char) prefix
- **Timestamp**: ISO 8601 + Unix epoch for sorting
- **Configuration**: Release/Debug/RelWithDebInfo
- **Toolchain Fingerprint**: VS path, MSVC version, clang-cl version, CMake, Ninja
- **Concurrency Info**: Calculated job count, memory limits, CPU cores
- **Build Config**: Jobs, sccache settings, incremental flag, features
- **Status**: InProgress, Success, Failed, Promoted, RolledBack
- **Duration**: Build time in milliseconds
- **Validation Result**: Pass/fail, output, duration

### Isolated Staging Directories

Every build attempt gets its own isolated directory:
- Named: `{timestamp_unix}-{config}-{git_commit_short}`
- Contains `build_identity.json` with full metadata
- Contains `build/{config}/` with actual CMake/Ninja output
- Never shared between builds (true isolation)

### Atomic Promotion

Promotion is atomic and validated:

1. **Pre-promotion validation** (configurable):
   - Default: Check for executable + key libraries
   - Custom: User-defined validation command
2. **Atomic swap**:
   - Current → Previous (preserves rollback target)
   - Staging → Current (symlink or move)
3. **Status update**: Build marked as `Promoted`
4. **Cleanup**: Old versions pruned per retention policy

### Rollback

Two rollback modes:

1. **Explicit rollback**: `rollback(timestamp_unix)` - rollback to specific version
2. **Previous rollback**: `rollback_to_previous()` - rollback to the build before current

Rollback is instant (symlink swap) and safe:
- Current becomes Previous
- Target becomes Current
- Build status marked as `RolledBack`

### Cleanup Safety

The system **never removes the last known-good build**:

- `min_successful_to_keep` (default: 3) - minimum successful builds to retain
- `max_versions` (default: 20) - maximum total versions
- Current and Previous are **always protected**
- Cleanup runs automatically after promotion
- Force cleanup available for maintenance

## API Usage

### Creating a Version Manager

```rust
use build_versioning::{BuildVersionManager, BuildVersioningConfig};

let config = BuildVersioningConfig {
    versions_root: PathBuf::from("build/versions"),
    max_versions: 20,
    min_successful_to_keep: 3,
    create_symlinks: true,
    validate_before_promote: true,
    validation_cmd: Some("ctest --test-dir {build_dir} --output-on-failure".to_string()),
};

let mut manager = BuildVersionManager::new(config)?;
```

### Starting a Build

```rust
let identity = manager.start_build(
    build_id,           // UUID
    git_commit,         // Full commit hash
    "Release",          // Config
    toolchain_fingerprint, // From Get-ToolchainFingerprint
    concurrency_info,   // From ConcurrencyCalculator
    build_config,       // Jobs, sccache, etc.
)?;
```

The staging directory is created at `build/versions/{timestamp}-{config}-{commit_short}/`.

### Completing a Build

```rust
// On success
manager.complete_build(identity.timestamp_unix, true, duration_ms, None)?;

// On failure
manager.complete_build(identity.timestamp_unix, false, duration_ms, Some(error))?;
```

### Promoting a Build

```rust
// Validates then promotes atomically
let promoted = manager.promote_build(identity.timestamp_unix)?;
```

### Rolling Back

```rust
// Rollback to specific version
let rolled_back = manager.rollback(timestamp_unix)?;

// Rollback to previous (one before current)
let rolled_back = manager.rollback_to_previous()?;
```

### Querying Builds

```rust
// All versions (newest first)
let all = manager.list_versions();

// Only successful builds
let successful = manager.list_successful_versions();

// Current and previous
let current = manager.get_current();
let previous = manager.get_previous();
```

### Disk Usage

```rust
let usage = manager.get_disk_usage()?;
println!("Total: {} bytes, Versions: {}, Successful: {}", 
    usage.total_bytes, usage.version_count, usage.successful_count);
```

### Exporting History

```rust
manager.export_history(&PathBuf::from("build_history.json"))?;
```

## Integration with Structured Build Events

The `VersionedBuildEventEmitter` wraps the standard `BuildEventEmitter` to automatically track build versions:

```rust
use build_versioning::{VersionedBuildEventEmitter, BuildVersionManager};
use std::sync::Arc;

let version_manager = Arc::new(Mutex::new(BuildVersionManager::new(config)?));
let (emitter, rx) = BuildEventEmitter::with_channel(build_id);
let versioned_emitter = VersionedBuildEventEmitter::new(Arc::new(emitter), version_manager);

// Start build tracking
versioned_emitter.start_build(&identity)?;

// ... run configure, build, test stages ...

// Complete build tracking
versioned_emitter.complete_build(success, duration_ms, error)?;

// Promote if successful
if success {
    versioned_emitter.promote_current()?;
}
```

## Configuration Options

| Option | Default | Description |
|--------|---------|-------------|
| `versions_root` | `build/versions` | Root directory for all build versions |
| `max_versions` | `20` | Maximum total versions to keep (0 = unlimited) |
| `min_successful_to_keep` | `3` | Minimum successful builds to always retain |
| `create_symlinks` | `true` | Use symlinks for current/previous (Windows: junction) |
| `validate_before_promote` | `true` | Run validation before promotion |
| `validation_cmd` | `None` | Custom validation command template |

## Validation Commands

The `validation_cmd` supports `{build_dir}` placeholder:

```toml
# Run CTest
validation_cmd = "ctest --test-dir {build_dir} --output-on-failure"

# Run executable smoke test
validation_cmd = "{build_dir}/nxrt_gfx_demo.exe --version"

# Custom script
validation_cmd = "python validate_build.py {build_dir}"
```

Default validation (when no command specified):
1. Checks for `nxrt_gfx_demo.exe` in build output
2. Attempts to run with `--help`
3. Checks for key libraries (`nxrt.dll`, `nxrt_gfx.dll`, `SDL3.dll`)

## Retention Policy Examples

### Conservative (CI/CD)
```rust
BuildVersioningConfig {
    max_versions: 10,
    min_successful_to_keep: 5,
    ..
}
```
Keeps last 10 builds, always 5 successful. Good for CI with limited disk.

### Development (Local)
```rust
BuildVersioningConfig {
    max_versions: 50,
    min_successful_to_keep: 10,
    ..
}
```
Keeps more history for debugging and comparison.

### Minimal (Disk-constrained)
```rust
BuildVersioningConfig {
    max_versions: 5,
    min_successful_to_keep: 2,
    ..
}
```
Minimal retention, but always keeps 2 working builds.

## Symlink vs Copy Mode

### Symlinks (Default, `create_symlinks: true`)
- **Windows**: Directory junctions (requires admin or Developer Mode)
- **Unix**: Standard symlinks
- **Pros**: Instant promotion/rollback, no disk duplication
- **Cons**: Requires permissions, some tools don't follow junctions

### Copy Mode (`create_symlinks: false`)
- **Windows/Unix**: Full directory copy/move
- **Pros**: Works everywhere, no permission issues
- **Cons**: Slower promotion, uses more disk during transition

## Migration from Legacy Builds

To adopt versioning with existing builds:

1. Create `build/versions/` directory
2. Move existing `build/release` to `build/versions/<timestamp>-release-<commit>/build/release`
3. Create `build_identity.json` in the staging directory
4. Run `manager.rebuild_metadata()` to index existing builds
5. Promote the latest to establish `current` link

## Troubleshooting

### "Cannot promote failed build"
Only builds with `status == Success` can be promoted. Check build logs.

### "Build staging directory no longer exists"
The staging directory was manually deleted. Use `rebuild_metadata()` to clean index.

### Symlink creation fails on Windows
Enable Developer Mode or run as Administrator. Or set `create_symlinks: false`.

### Cleanup not removing old builds
Check `min_successful_to_keep` - builds are protected until this threshold is met.

### Validation fails unexpectedly
Run the validation command manually to debug. Check `{build_dir}` path resolution.

## Integration Points

This module integrates with:
- **ToolchainManager**: Uses `ConcurrencyInfo`, `BuildConfig`, `BuildEventEmitter`
- **build.ps1**: Can call `start_build`, `complete_build`, `promote_build` via Rust CLI
- **CI/CD**: Export history for build dashboards
- **sccache**: Each version gets isolated cache dir (future: shared cache with versioning)

## Future Enhancements (Phase C+)

- **Content-addressed storage**: Deduplicate identical build outputs across versions
- **Incremental promotion**: Promote only changed artifacts
- **Remote version storage**: Sync versions to S3/artifact registry
- **Build comparison**: Diff two versions (binaries, timestamps, sizes)
- **Automatic bisect**: Find regression-introducing build via binary search