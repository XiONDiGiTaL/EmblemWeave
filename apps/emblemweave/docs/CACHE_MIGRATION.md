# Pipeline Cache Migration Guide

## Overview

The Pipeline Cache Migration system provides inventory tracking, migration, and management of Vulkan pipeline caches across build versions. It integrates with the Build Versioning system to associate pipeline caches with specific build identities, enabling cache reuse and performance optimization.

## Architecture

### Components

1. **PipelineCacheManager** - Core manager for cache inventory and operations
2. **PipelineCacheInventory** - Persistent JSON inventory of all known caches
3. **PipelineCacheEntry** - Individual cache metadata with GPU association

### Data Structures

```rust
/// Pipeline cache entry in the inventory
pub struct PipelineCacheEntry {
    pub sha256: String,              // Content-addressed hash
    pub size_bytes: u64,             // Cache file size
    pub device_uuid: Option<String>, // GPU UUID (VkPhysicalDeviceProperties.pipelineCacheUUID)
    pub gpu_name: Option<String>,    // Human-readable GPU name
    pub driver_version: Option<String>,
    pub created_unix: u64,           // Creation timestamp
    pub last_accessed_unix: u64,     // Last access timestamp
    pub build_timestamp: Option<u64>, // Associated build
    pub build_config: Option<String>, // Release/Debug
    pub git_commit_short: Option<String>,
    pub relative_path: String,       // Path relative to cache root
    pub is_active: bool,             // Currently linked to a build
    pub format_version: u32,         // VkPipelineCache header version
}
```

## Inventory Management

### Scanning for Caches

```bash
# Scan build directories for pipeline_cache.bin files
ew-pipeline-cache scan --search-paths build/versions
```

The scanner recursively searches (max depth 3) for `pipeline_cache.bin` files and adds them to the inventory with computed SHA-256 hashes.

### Cache Deduplication

The inventory uses content-addressed storage (SHA-256) to automatically deduplicate identical caches:
- Scanning the same cache twice only creates one inventory entry
- Last accessed time is updated on subsequent scans
- Identical caches across builds share the same entry

## Build Integration

### Automatic Migration on Promote

When a build is promoted via `BuildVersionManager::promote_current()`:

1. The runtime pipeline cache (`engine/fefw-recomp/runtime/pipeline_cache.bin`) is detected
2. Added to inventory if not present
3. Migrated to the promoted build's version directory
4. Associated with build metadata (timestamp, config, git commit)
5. Marked as active

```rust
// In pipeline_cache.rs
pub fn migrate_cache_on_promote(
    version_manager: &BuildVersionManager,
    cache_manager: &mut PipelineCacheManager,
    build_timestamp: u64,
) -> Result<Option<PathBuf>>
```

### Manual Cache Linking

```bash
# Link a specific cache to a build
ew-pipeline-cache link <sha256> --build-timestamp <ts> --config Release --git-commit <hash>
```

## GPU Association

Caches are associated with specific GPUs via:
- `device_uuid`: 16-byte VkPhysicalDeviceProperties.pipelineCacheUUID
- `gpu_name`: Human-readable name (e.g., "NVIDIA GeForce RTX 4090")
- `driver_version`: Driver version string

The system maintains a minimum number of caches per GPU (`min_per_gpu`, default 2) to ensure fallback options.

## Retention and Cleanup

### Configuration

```rust
pub struct PipelineCacheConfig {
    pub cache_root: PathBuf,
    pub max_total_size: u64,      // Default: 2 GB
    pub max_caches: usize,         // Default: 20
    pub min_per_gpu: usize,        // Default: 2
    pub auto_migrate: bool,        // Default: true
}
```

### Cleanup Policy

The cleanup algorithm runs in two passes:

1. **Per-GPU retention**: Keep `min_per_gpu` most recently accessed caches per GPU
2. **Global size limit**: Remove oldest caches until `max_total_size` is satisfied
3. **Global count limit**: Keep only `max_caches` total entries

```bash
# Run cleanup manually
ew-pipeline-cache cleanup

# Dry-run to preview
ew-pipeline-cache cleanup --dry-run
```

## CLI Reference

### Commands

| Command | Description |
|---------|-------------|
| `scan` | Scan directories for pipeline caches |
| `list` | List all cached entries |
| `stats` | Show inventory statistics |
| `show <sha256>` | Show detailed entry info |
| `migrate <sha256>` | Migrate cache to build directory |
| `link <sha256>` | Link cache to a build |
| `associate <sha256>` | Associate cache with build metadata |
| `cleanup` | Run retention cleanup |
| `export` | Export inventory to JSON |
| `auto-migrate` | Auto-migrate runtime cache on promote |

### Examples

```bash
# Scan all version directories
ew-pipeline-cache scan --search-paths build/versions --recursive

# List caches with table output
ew-pipeline-cache list --format table

# Show cache details
ew-pipeline-cache show a1b2c3d4e5f6...

# Export inventory for backup
ew-pipeline-cache export --output pipeline_cache_inventory.json
```

## Integration Points

### With Build Versioning

The migration is triggered automatically during `BuildVersionManager::promote_current()`:

```rust
// In build_versioning.rs promote_current()
if let Some(cache_manager) = &mut self.cache_manager {
    let _ = pipeline_cache::migrate_cache_on_promote(self, cache_manager, build.timestamp_unix);
}
```

### With Object/Lift Store

Pipeline caches can be stored as objects in the Object Store for additional deduplication:
- Cache content stored once in object store
- Inventory entry references object by SHA-256
- Lift store can record cache generation as a "lift" operation

## Performance Considerations

### Cache Reuse Strategy

1. **Exact GPU Match**: Prefer cache with identical `device_uuid`
2. **Same Vendor**: Fallback to same GPU vendor (NVIDIA/AMD/Intel)
3. **Any Compatible**: Use any cache with same Vulkan version
4. **Cold Start**: Generate new cache if no suitable match

### Size Management

- Typical pipeline cache: 10-50 MB
- 20 caches ≈ 200-1000 MB
- Cleanup runs automatically on promote (if `auto_migrate=true`)
- Manual cleanup recommended weekly or before major builds

## Troubleshooting

### Cache Not Found

```bash
# Verify inventory
ew-pipeline-cache list

# Re-scan directories
ew-pipeline-cache scan --search-paths build/versions --recursive
```

### Corrupted Inventory

```bash
# Backup and regenerate
cp build/pipeline_cache/pipeline_cache_inventory.json build/pipeline_cache/inventory.backup.json
ew-pipeline-cache scan --search-paths build/pipeline_cache --recursive
```

### GPU Association Issues

- Ensure Vulkan SDK is installed for device UUID detection
- Check driver version compatibility
- Manually associate using `ew-pipeline-cache associate`

## Best Practices

1. **Enable auto-migration** in CI/CD pipelines
2. **Run cleanup** before large builds to free space
3. **Monitor cache hit rates** via build logs
4. **Keep min_per_gpu ≥ 2** for driver update resilience
5. **Export inventory** before major system changes