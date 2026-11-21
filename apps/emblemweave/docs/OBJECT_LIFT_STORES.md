# Object/Lift Store Architecture

## Overview

The Object/Lift Store provides content-addressed storage for build artifacts and compilation results, enabling deduplication, incremental builds, and artifact reuse across builds. It consists of two integrated stores:

1. **Object Store** - Content-addressed blob storage (SHA-256)
2. **Lift Store** - Compilation result caching with input/output tracking

## Architecture

### Object Store

The Object Store stores arbitrary blobs with content-addressed addressing:

```rust
pub enum ObjectType {
    Artifact,    // Build outputs (binaries, libraries, assets)
    Source,      // Source files
    Config,      // Configuration files
    Intermediate, // Intermediate build products
    Lift,        // Lift operation records
}
```

**Key Features:**
- SHA-256 content addressing with sharded directories (2-char prefix)
- Reference counting for garbage collection
- Hard-link deduplication with copy fallback
- Optional compression (disabled by default)
- Configurable size and count limits

### Lift Store

The Lift Store caches compilation operations (lifts) for reuse:

```
Lift = (Input Hashes + Tool Config + Environment) → Output Hashes
```

**Key Features:**
- Records input file hashes, tool command, environment
- Outputs linked to Object Store entries
- Automatic invalidation on input/config changes
- Reuse detection via exact hash matching
- Configurable retention limits

## Data Structures

### Object Metadata

```rust
pub struct ObjectMeta {
    pub sha256: String,           // Content hash (primary key)
    pub size_bytes: u64,          // Object size
    pub ref_count: u32,           // Reference count
    pub object_type: ObjectType,  // Type classification
    pub created_unix: u64,        // Creation timestamp
    pub last_accessed_unix: u64,  // Last access timestamp
    pub source_path: Option<PathBuf>, // Original source path
    pub mime_type: Option<String>,    // MIME type hint
}
```

### Lift Metadata

```rust
pub struct LiftMeta {
    pub lift_id: String,                    // UUID
    pub input_hashes: Vec<String>,          // Input object SHA-256s
    pub tool_command: String,               // Full command line
    pub tool_config_hash: String,           // Hash of tool config
    pub env_hash: String,                   // Hash of relevant env vars
    pub output_hashes: Vec<String>,         // Output object SHA-256s
    pub created_unix: u64,                  // Creation timestamp
    pub last_reused_unix: Option<u64>,      // Last successful reuse
    pub reuse_count: u32,                   // Number of reuses
    pub build_timestamp: Option<u64>,       // Associated build
    pub build_config: Option<String>,       // Build config
    pub git_commit_short: Option<String>,   // Git commit
    pub is_valid: bool,                     // Validity flag
}
```

## Storage Layout

```
build/object_store/
├── objects/
│   ├── ab/                 # Shard by first 2 chars of SHA-256
│   │   └── abcdef123456... # Full hash as filename
│   ├── cd/
│   │   └── cdef12345678...
│   └── ...
├── metadata/
│   ├── ab/
│   │   └── abcdef123456...meta.json
│   └── ...
└── index.json              # Object index (SHA-256 → metadata)

build/lift_store/
├── lifts/
│   ├── <lift_id>.json      # Lift metadata
│   └── ...
├── index.json              # Lift index (lift_id → metadata)
└── by_input/
    └── <input_hash>.json   # Reverse index: input → lift_ids
```

## Object Store Operations

### Store Object

```bash
# Store a file in the object store
ew-object-store store <file> --type Artifact --source-path <original>

# Store with custom metadata
ew-object-store store build/output.exe --type Artifact --mime-type application/x-msdownload
```

**Process:**
1. Compute SHA-256 of file content
2. Check if object exists (deduplication)
3. If new: copy to sharded directory, create metadata
4. If exists: increment ref_count, update last_accessed
5. Return object SHA-256

### Retrieve Object

```bash
# Retrieve object to destination
ew-object-store retrieve <sha256> <dest_path>
```

**Process:**
1. Look up metadata by SHA-256
2. Hard-link from store to destination (Windows: copy fallback)
3. Update last_accessed, increment ref_count

### List Objects

```bash
# List all objects
ew-object-store list

# Filter by type
ew-object-store list --type Artifact

# Show with sizes
ew-object-store list --format json --output objects.json
```

## Lift Store Operations

### Record Lift

```bash
# Record a compilation lift
ew-object-store record-lift \
    --input-hashes <sha1>,<sha2> \
    --tool-command "clang++ -O2 -c input.cpp -o output.o" \
    --tool-config-hash <config_sha> \
    --env-hash <env_sha> \
    --output-hashes <out_sha> \
    --build-timestamp <ts> \
    --build-config Release \
    --git-commit <short_hash>
```

**Process:**
1. Create composite key: `(input_hashes + tool_command + tool_config_hash + env_hash)`
2. Check if identical lift exists (reuse detection)
3. If exists: increment reuse_count, update last_reused_unix
4. If new: create LiftMeta, store in lifts/, update indexes

### Check Lift

```bash
# Check if lift can be reused
ew-object-store check-lift \
    --input-hashes <sha1>,<sha2> \
    --tool-command "clang++ -O2 -c input.cpp -o output.o" \
    --tool-config-hash <config_sha> \
    --env-hash <env_sha>
```

**Returns:** Matching lift_id and output hashes if valid, or "no match"

### Invalidate Lifts

```bash
# Invalidate lifts for a build (e.g., after toolchain change)
ew-object-store invalidate --build-timestamp <ts>

# Invalidate by input hash (source file changed)
ew-object-store invalidate --input-hash <sha256>
```

## Build Integration

### Storing Build Artifacts on Promote

When a build is promoted, artifacts are stored in the Object Store:

```rust
// In object_store.rs
pub fn store_build_artifacts_on_promote(
    object_store: &mut ObjectLiftStoreManager,
    version_manager: &BuildVersionManager,
    build_timestamp: u64,
) -> Result<()>
```

**Process:**
1. Get promoted build directory
2. Walk all files in build output
3. Store each file as ObjectType::Artifact
4. Record source_path for traceability
5. Update build manifest with object references

### Recording Compilation Lifts

During build, each compilation unit can be recorded as a lift:

```rust
// Pseudo-code for build system integration
for compilation_unit in build_plan {
    let input_hashes = hash_inputs(&compilation_unit.inputs);
    let tool_command = render_command(&compilation_unit);
    let config_hash = hash_tool_config(&compilation_unit.config);
    let env_hash = hash_relevant_env();
    
    // Check for existing lift
    if let Some(lift) = object_store.check_lift(&input_hashes, &tool_command, &config_hash, &env_hash) {
        // Reuse outputs
        materialize_outputs(&lift.output_hashes, &compilation_unit.outputs);
    } else {
        // Run compilation
        run_compilation(&compilation_unit);
        // Record new lift
        let output_hashes = store_outputs(&compilation_unit.outputs);
        object_store.record_lift(input_hashes, tool_command, config_hash, env_hash, output_hashes, ...);
    }
}
```

## Configuration

### Object Store Config

```rust
pub struct ObjectStoreConfig {
    pub store_root: PathBuf,           // Default: build/object_store
    pub max_total_size: u64,           // Default: 10 GB
    pub max_objects: usize,            // Default: 100,000
    pub min_ref_count: u32,            // Default: 0 (never auto-delete)
    pub enable_hardlinks: bool,        // Default: true
    pub compress: bool,                // Default: false
}
```

### Lift Store Config

```rust
pub struct LiftStoreConfig {
    pub store_root: PathBuf,           // Default: build/lift_store
    pub max_total_size: u64,           // Default: 5 GB
    pub max_lifts: usize,              // Default: 50,000
    pub enable_reuse: bool,            // Default: true
}
```

## Cleanup and Maintenance

### Object Cleanup

```bash
# Remove unreferenced objects (ref_count == 0)
ew-object-store cleanup-objects

# Dry-run
ew-object-store cleanup-objects --dry-run
```

**Process:**
1. Find all objects with ref_count == 0
2. Delete object file and metadata
3. Update index

### Lift Cleanup

```bash
# Remove invalid lifts
ew-object-store cleanup-lifts

# Dry-run
ew-object-store cleanup-lifts --dry-run
```

**Process:**
1. Find all lifts with is_valid == false
2. Delete lift metadata
3. Update indexes

### Statistics

```bash
# Show unified statistics
ew-object-store stats
```

**Output:**
```
Object Store Statistics
=======================
Total objects:       15,432
Total size:          2.34 GB
Total refs:          45,210
Unreferenced:        127
By type:
  Artifact:          8,234 (1.8 GB)
  Source:            4,121 (0.2 GB)
  Config:            2,012 (0.05 GB)
  Intermediate:      1,065 (0.29 GB)

Lift Store Statistics
=====================
Total lifts:         8,921
Valid lifts:         8,543
Invalid lifts:       378
Total output size:   1.87 GB
Reuse rate:          73.2%
```

## CLI Reference

### Commands

| Command | Description |
|---------|-------------|
| `store <file>` | Store file in object store |
| `retrieve <sha256> <dest>` | Retrieve object to path |
| `record-lift` | Record compilation lift |
| `check-lift` | Check for reusable lift |
| `invalidate` | Invalidate lifts by build/input |
| `list` | List objects |
| `stats` | Show statistics |
| `cleanup-objects` | Remove unreferenced objects |
| `cleanup-lifts` | Remove invalid lifts |
| `export` | Export store data |

### Global Options

```
--object-store-root <path>   # Default: build/object_store
--lift-store-root <path>     # Default: build/lift_store
--format <json|table>        # Output format
--dry-run                    # Preview without changes
```

## Performance Considerations

### Deduplication Effectiveness

- **Typical deduplication**: 40-60% for build artifacts
- **Source files**: 80-90% (unchanged between builds)
- **Intermediate**: 30-50% (varies by build config)

### Lift Reuse Rates

- **Clean rebuilds**: 0% (no prior lifts)
- **Incremental builds**: 60-85% (typical)
- **Config changes**: Drops to 0% for affected units
- **Toolchain updates**: Drops to 0% (config hash changes)

### Storage Sizing

| Project Size | Object Store | Lift Store |
|--------------|--------------|------------|
| Small (<100 files) | 100 MB | 50 MB |
| Medium (1000 files) | 2 GB | 500 MB |
| Large (10000+ files) | 15 GB | 3 GB |

## Troubleshooting

### Low Deduplication

1. Check `enable_hardlinks` is true
2. Verify filesystem supports hardlinks (NTFS, ext4, APFS)
3. Check for per-file modifications (timestamps, build IDs)

### Low Lift Reuse

1. Verify tool config hash includes all relevant flags
2. Check env hash includes CC, CXX, CFLAGS, etc.
3. Ensure input hashes are computed deterministically
4. Check for non-deterministic build outputs (timestamps, UUIDs)

### Storage Full

```bash
# Check disk usage
ew-object-store stats

# Cleanup unreferenced
ew-object-store cleanup-objects

# Cleanup invalid lifts
ew-object-store cleanup-lifts

# Reduce max limits in config
```

## Best Practices

1. **Enable hardlinks** for maximum deduplication
2. **Hash all build inputs** deterministically (source, headers, config)
3. **Include compiler version** in tool config hash
4. **Record lifts at compilation unit granularity** (per .cpp/.c file)
5. **Run cleanup** periodically in CI (weekly or per 100 builds)
6. **Monitor reuse rate** as build health metric
7. **Export indexes** before major migrations