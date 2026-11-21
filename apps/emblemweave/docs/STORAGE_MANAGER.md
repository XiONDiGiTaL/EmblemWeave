# Unified Storage Manager

## Overview

The Unified Storage Manager (`StorageManager`) provides a single interface for managing all storage systems in EmblemWeave:

1. **Build Versioning** (`build/versions`) - Isolated build directories with atomic promotion
2. **Pipeline Cache** (`build/pipeline_cache`) - Vulkan pipeline cache inventory and migration
3. **Object Store** (`build/object_store`) - Content-addressed artifact storage
4. **Lift Store** (`build/lift_store`) - Compilation result caching
5. **Hard-Link Store** (`build/hardlink_store`) - RomFS deduplication

It provides unified statistics, coordinated cleanup, and cross-store operations.

## Architecture

### Manager Composition

```rust
pub struct StorageManager {
    pub config: StorageCleanupConfig,
    pub versions_root: PathBuf,
    pub pipeline_cache_root: PathBuf,
    pub object_store_root: PathBuf,
    pub lift_store_root: PathBuf,
    pub hardlink_store_root: PathBuf,

    // Lazy-initialized managers (RefCell for interior mutability)
    version_manager: RefCell<Option<BuildVersionManager>>,
    pipeline_cache_manager: RefCell<Option<PipelineCacheManager>>,
    object_store_manager: RefCell<Option<ObjectLiftStoreManager>>,
    hardlink_store_manager: RefCell<Option<HardLinkStoreManager>>,
}
```

### Interior Mutability Pattern

Uses `RefCell<Option<T>>` for lazy initialization:
- Managers created on first access
- Allows `&self` methods to mutate internal state
- Thread-safe for single-threaded use (CLI context)

```rust
fn get_version_manager(&self) -> Result<RefMut<'_, BuildVersionManager>> {
    let mut borrowed = self.version_manager.borrow_mut();
    if borrowed.is_none() {
        *borrowed = Some(BuildVersionManager::new(config)?);
    }
    Ok(RefMut::map(borrowed, |opt| opt.as_mut().unwrap()))
}
```

## Unified Statistics

### Collection

```bash
# Collect and display unified statistics
ew-storage stats

# Export as JSON
ew-storage stats --format json --output storage_stats.json
```

### Output Structure

```json
{
  "collected_unix": 1700000000,
  "build_versions": {
    "total_versions": 15,
    "successful_versions": 12,
    "failed_versions": 3,
    "current_version": 1700000000,
    "previous_version": 1699990000,
    "total_size_bytes": 5368709120,
    "staging_size_bytes": 1073741824,
    "versions_size_bytes": 4294967296
  },
  "pipeline_cache": {
    "total_caches": 8,
    "active_caches": 5,
    "total_size_bytes": 2147483648,
    "by_gpu": {
      "nvidia-rtx4090": 5,
      "amd-rx7900xtx": 3
    }
  },
  "object_store": {
    "total_objects": 45231,
    "total_size_bytes": 3221225472,
    "total_refs": 128456,
    "unreferenced_objects": 23
  },
  "lift_store": {
    "total_lifts": 12456,
    "valid_lifts": 11823,
    "invalid_lifts": 633,
    "total_output_size_bytes": 2147483648
  },
  "hardlink_store": {
    "total_unique_content": 38421,
    "total_references": 156789,
    "total_size_bytes": 5368709120,
    "deduplication_ratio": 4.08
  },
  "total": {
    "total_size_bytes": 18253611008,
    "total_items": 97752,
    "by_system": {
      "build_versions": {"name": "Build Versions", "size_bytes": 5368709120, "item_count": 15, "percentage": 29.4},
      "pipeline_cache": {"name": "Pipeline Cache", "size_bytes": 2147483648, "item_count": 8, "percentage": 11.8},
      "object_store": {"name": "Object Store", "size_bytes": 3221225472, "item_count": 45231, "percentage": 17.6},
      "lift_store": {"name": "Lift Store", "size_bytes": 2147483648, "item_count": 12456, "percentage": 11.8},
      "hardlink_store": {"name": "Hard-link Store", "size_bytes": 5368709120, "item_count": 38421, "percentage": 29.4}
    }
  }
}
```

## Unified Cleanup

### Configuration

```rust
pub struct StorageCleanupConfig {
    pub min_successful_versions: usize,     // Default: 3
    pub max_versions: usize,                 // Default: 20
    pub max_pipeline_cache_size: u64,        // Default: 5 GB
    pub max_object_store_size: u64,          // Default: 10 GB
    pub max_lift_store_size: u64,            // Default: 5 GB
    pub max_hardlink_store_size: u64,        // Default: 20 GB
    pub min_free_space: u64,                 // Default: 10 GB
    pub auto_cleanup: bool,                  // Default: false
    pub dry_run: bool,                       // Default: false
}
```

### Cleanup Process

```bash
# Run cleanup (respects auto_cleanup flag)
ew-storage cleanup

# Force cleanup regardless of auto_cleanup
ew-storage cleanup --force

# Dry-run preview
ew-storage cleanup --dry-run
# Or: ew-storage dry-run
```

**Cleanup Order:**
1. Check free disk space (abort if below `min_free_space`)
2. Build versions: `BuildVersionManager::cleanup_old_versions()`
3. Pipeline cache: `PipelineCacheManager::cleanup()`
4. Object store: `ObjectLiftStoreManager::cleanup_objects()`
5. Lift store: `ObjectLiftStoreManager::cleanup_lifts()`
6. Hard-link store: `HardLinkStoreManager::cleanup()`

### Cleanup Result

```json
{
  "bytes_freed": 2147483648,
  "items_removed": {
    "build_versions": 3,
    "pipeline_cache": 2,
    "object_store": 23,
    "lift_store": 633,
    "hardlink_store": 1245
  },
  "errors": [],
  "dry_run": false,
  "timestamp_unix": 1700000000
}
```

## Cross-Store Operations

### Store Build Artifacts

Stores build outputs across all relevant stores:

```bash
ew-storage store-build <build_dir> <build_ts> --config Release --git-commit abc123
```

**Operations:**
1. **Object Store**: Store all build artifacts as `ObjectType::Artifact`
2. **Hard-Link Store**: Build RomFS from build directory
3. **Pipeline Cache**: Migrate runtime pipeline cache to build version

```rust
pub fn store_build_artifacts(&self, build: &BuildIdentity, build_dir: &Path) -> Result<()> {
    // Object store
    store_build_artifacts_on_promote(object_store, version_manager, build.timestamp_unix)?;
    
    // Hard-link store
    build_romfs_on_promote(hardlink_store, version_manager, build.timestamp_unix)?;
    
    // Pipeline cache
    migrate_cache_on_promote(version_manager, pipeline_cache, build.timestamp_unix)?;
    
    Ok(())
}
```

### Disk Space Check

```bash
ew-storage disk-space
```

**Output:**
```
Disk Space
==========
Versions:         build/versions (2.3 GB)
Pipeline Cache:   build/pipeline_cache (1.1 GB)
Object Store:     build/object_store (3.0 GB)
Lift Store:       build/lift_store (2.0 GB)
Hard-link Store:  build/hardlink_store (5.0 GB)

Free space:       150 GB
Total space:      500 GB
```

## CLI Reference

### Commands

| Command | Description |
|---------|-------------|
| `stats` | Show unified statistics |
| `cleanup` | Run coordinated cleanup |
| `dry-run` | Preview cleanup without changes |
| `export` | Export unified report to JSON |
| `store-build` | Store build artifacts across stores |
| `disk-space` | Show disk usage per store |

### Global Options

```
--versions-root <path>              # Default: build/versions
--pipeline-cache-root <path>        # Default: build/pipeline_cache
--object-store-root <path>          # Default: build/object_store
--lift-store-root <path>            # Default: build/lift_store
--hardlink-store-root <path>        # Default: build/hardlink_store

--min-successful-versions <n>       # Default: 3
--max-versions <n>                  # Default: 20
--max-pipeline-cache-size <bytes>   # Default: 5GB
--max-object-store-size <bytes>     # Default: 10GB
--max-lift-store-size <bytes>       # Default: 5GB
--max-hardlink-store-size <bytes>   # Default: 20GB
--min-free-space <bytes>            # Default: 10GB
--dry-run <bool>                    # Default: false
--format <json|table>               # Default: table
```

## Configuration

### Default Paths

All stores default to subdirectories under `build/`:

```
build/
├── versions/           # BuildVersionManager
├── pipeline_cache/     # PipelineCacheManager
├── object_store/       # ObjectLiftStoreManager (objects)
├── lift_store/         # ObjectLiftStoreManager (lifts)
└── hardlink_store/     # HardLinkStoreManager
```

### Custom Paths

```rust
let manager = StorageManager::with_paths(
    config,
    PathBuf::from("/custom/versions"),
    PathBuf::from("/custom/pipeline_cache"),
    PathBuf::from("/custom/object_store"),
    PathBuf::from("/custom/lift_store"),
    PathBuf::from("/custom/hardlink_store"),
)?;
```

## Integration with Build System

### Build Promotion Hook

```rust
// In BuildVersionManager::promote_current()
fn promote_current(&mut self) -> Result<BuildIdentity> {
    let build = self.complete_promotion()?;
    
    // Trigger cross-store operations
    if let Some(storage) = &self.storage_manager {
        storage.store_build_artifacts(&build, &build.staging_dir)?;
    }
    
    Ok(build)
}
```

### CI/CD Pipeline

```yaml
# Example GitHub Actions step
- name: EmblemWeave Storage Cleanup
  run: |
    ew-storage cleanup --force
    ew-storage stats --format json > storage_report.json
  # Runs after successful build promotion
```

## Monitoring and Alerting

### Key Metrics

| Metric | Warning Threshold | Critical Threshold |
|--------|-------------------|-------------------|
| Total storage | >80% of max | >95% of max |
| Free disk space | <20 GB | <10 GB |
| Deduplication ratio | <2.0x | <1.5x |
| Lift reuse rate | <50% | <30% |
| Unreferenced objects | >1000 | >5000 |

### Automated Cleanup

Enable `auto_cleanup: true` in config for automatic cleanup on:
- Build promotion (if storage exceeds thresholds)
- Scheduled cron (daily/weekly)
- Pre-build check (if free space low)

```rust
let config = StorageCleanupConfig {
    auto_cleanup: true,
    min_free_space: 20 * 1024 * 1024 * 1024, // 20 GB
    ..Default::default()
};
```

## Troubleshooting

### Cleanup Not Running

```bash
# Check auto_cleanup flag
ew-storage stats --format json | jq .config.auto_cleanup

# Force cleanup
ew-storage cleanup --force
```

### Statistics Missing

```bash
# Verify store directories exist
ls -la build/

# Re-initialize managers
ew-storage stats  # Triggers lazy initialization
```

### Cross-Store Operation Failures

```bash
# Check individual store health
ew-version stats
ew-pipeline-cache stats
ew-object-store stats
ew-romfs-store stats

# Run store-build with verbose error
ew-storage store-build build/output 1700000000 --config Release --git-commit abc123 2>&1
```

### Disk Space Issues

```bash
# Check free space
ew-storage disk-space

# Emergency cleanup
ew-storage cleanup --force
ew-object-store cleanup-objects
ew-object-store cleanup-lifts
ew-romfs-store cleanup
```

## Best Practices

1. **Run `ew-storage stats`** in CI after each build
2. **Enable auto_cleanup** for production environments
3. **Set min_free_space** to 2x largest build size
4. **Export reports** weekly for trend analysis
5. **Monitor deduplication ratios** across all stores
6. **Coordinate cleanup schedules** with build windows
7. **Keep configs in version control** (`.emblemweave/storage.toml`)
8. **Test dry-runs** before enabling auto_cleanup

## Advanced Usage

### Custom Cleanup Policies

```rust
// Extend StorageManager for custom policies
impl StorageManager {
    pub fn cleanup_with_policy(&self, policy: CleanupPolicy) -> Result<CleanupResult> {
        match policy {
            CleanupPolicy::Aggressive => {
                // Lower thresholds, more aggressive
            }
            CleanupPolicy::Conservative => {
                // Higher thresholds, preserve more
            }
            CleanupPolicy::Custom(config) => {
                // Use provided config
            }
        }
    }
}
```

### Report Export Automation

```bash
# Daily cron
0 2 * * * /path/to/ew-storage export --output /var/log/emblemweave/storage_$(date +\%Y\%m\%d).json

# Weekly summary
0 3 * * 0 /path/to/ew-storage stats --format json | jq -r '.total.total_size_bytes' >> /var/log/storage_trend.log
```