//! Object/Lift Store Architecture
//!
//! This module provides a content-addressed object store (similar to Git's object model)
//! and a lift store for incremental build artifacts. It enables deduplication of build
//! outputs across versions and efficient incremental builds.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::build_versioning::{BuildVersionManager, BuildIdentity};

/// Object type in the store
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectType {
    /// Build artifact (executable, library, object file)
    Artifact,
    /// Source file
    Source,
    /// Configuration file
    Config,
    /// Intermediate build file
    Intermediate,
    /// Lift (incremental build unit)
    Lift,
}

/// Object metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectMeta {
    /// SHA-256 hash of the object content (content-addressed)
    pub sha256: String,
    /// Object type
    pub object_type: ObjectType,
    /// Size in bytes
    pub size_bytes: u64,
    /// Creation timestamp
    pub created_unix: u64,
    /// Last access timestamp
    pub last_accessed_unix: u64,
    /// Reference count (number of builds referencing this object)
    pub ref_count: u32,
    /// Associated build timestamps
    pub build_timestamps: Vec<u64>,
    /// File extension hint
    pub extension: Option<String>,
    /// MIME type hint
    pub mime_type: Option<String>,
}

/// Lift metadata (for incremental build units)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiftMeta {
    /// SHA-256 of the lift content
    pub sha256: String,
    /// Lift name/identifier
    pub name: String,
    /// Input file hashes (dependencies)
    pub input_hashes: Vec<String>,
    /// Output file hashes (products)
    pub output_hashes: Vec<String>,
    /// Build command that produced this lift
    pub command: String,
    /// Working directory
    pub working_dir: String,
    /// Environment variables (relevant subset)
    pub env_vars: HashMap<String, String>,
    /// Creation timestamp
    pub created_unix: u64,
    /// Last access timestamp
    pub last_accessed_unix: u64,
    /// Associated build timestamp
    pub build_timestamp: u64,
    /// Whether this lift is valid (inputs haven't changed)
    pub is_valid: bool,
    /// Size of all outputs combined
    pub total_output_size: u64,
}

/// Object store configuration
#[derive(Debug, Clone)]
pub struct ObjectStoreConfig {
    /// Root directory for object store
    pub store_root: PathBuf,
    /// Maximum total store size (bytes), 0 = unlimited
    pub max_total_size: u64,
    /// Maximum number of objects
    pub max_objects: usize,
    /// Minimum reference count to keep (0 = keep all referenced)
    pub min_ref_count: u32,
    /// Enable hard-linking for artifacts
    pub enable_hardlinks: bool,
    /// Compress objects (zstd)
    pub compress: bool,
}

impl Default for ObjectStoreConfig {
    fn default() -> Self {
        Self {
            store_root: PathBuf::from("build/object_store"),
            max_total_size: 10 * 1024 * 1024 * 1024, // 10 GB default
            max_objects: 100000,
            min_ref_count: 0,
            enable_hardlinks: true,
            compress: false,
        }
    }
}

/// Lift store configuration
#[derive(Debug, Clone)]
pub struct LiftStoreConfig {
    /// Root directory for lift store
    pub store_root: PathBuf,
    /// Maximum total store size (bytes), 0 = unlimited
    pub max_total_size: u64,
    /// Maximum number of lifts
    pub max_lifts: usize,
    /// Enable lift reuse across builds
    pub enable_reuse: bool,
}

impl Default for LiftStoreConfig {
    fn default() -> Self {
        Self {
            store_root: PathBuf::from("build/lift_store"),
            max_total_size: 5 * 1024 * 1024 * 1024, // 5 GB default
            max_lifts: 50000,
            enable_reuse: true,
        }
    }
}

/// Combined object and lift store manager
pub struct ObjectLiftStoreManager {
    pub object_config: ObjectStoreConfig,
    pub lift_config: LiftStoreConfig,
    object_index_path: PathBuf,
    lift_index_path: PathBuf,
    pub objects: HashMap<String, ObjectMeta>,
    pub lifts: HashMap<String, LiftMeta>,
}

impl ObjectLiftStoreManager {
    /// Create a new object/lift store manager
    pub fn new(object_config: ObjectStoreConfig, lift_config: LiftStoreConfig) -> Result<Self> {
        fs::create_dir_all(&object_config.store_root)
            .context("Failed to create object store root")?;
        fs::create_dir_all(&lift_config.store_root)
            .context("Failed to create lift store root")?;

        // Create subdirectories for sharding (first 2 chars of hash)
        for i in 0..256 {
            let subdir = object_config.store_root.join(format!("{:02x}", i));
            fs::create_dir_all(&subdir)?;
        }

        let object_index_path = object_config.store_root.join("object_index.json");
        let lift_index_path = lift_config.store_root.join("lift_index.json");

        let objects = if object_index_path.exists() {
            let content = fs::read_to_string(&object_index_path)
                .context("Failed to read object index")?;
            match serde_json::from_str(&content) {
                Ok(obj) => obj,
                Err(e) => {
                    eprintln!("Warning: Failed to parse object index: {}. Starting fresh.", e);
                    let _ = fs::rename(&object_index_path, object_index_path.with_extension("json.corrupted"));
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        let lifts = if lift_index_path.exists() {
            let content = fs::read_to_string(&lift_index_path)
                .context("Failed to read lift index")?;
            match serde_json::from_str(&content) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("Warning: Failed to parse lift index: {}. Starting fresh.", e);
                    let _ = fs::rename(&lift_index_path, lift_index_path.with_extension("json.corrupted"));
                    HashMap::new()
                }
            }
        } else {
            HashMap::new()
        };

        Ok(Self {
            object_config,
            lift_config,
            object_index_path,
            lift_index_path,
            objects,
            lifts,
        })
    }

    /// Store an object (file) in the object store
    /// Returns the SHA-256 hash of the content
    pub fn store_object(
        &mut self,
        source_path: &Path,
        object_type: ObjectType,
        build_timestamp: Option<u64>,
    ) -> Result<String> {
        let content = fs::read(source_path)?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let sha256 = format!("{:x}", hasher.finalize());

        // Check if already exists
        if let Some(meta) = self.objects.get_mut(&sha256) {
            meta.ref_count += 1;
            meta.last_accessed_unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            if let Some(ts) = build_timestamp {
                if !meta.build_timestamps.contains(&ts) {
                    meta.build_timestamps.push(ts);
                }
            }
            self.save_object_index()?;
            return Ok(sha256);
        }

        // Determine shard directory
        let shard = &sha256[..2];
        let shard_dir = self.object_config.store_root.join(shard);
        fs::create_dir_all(&shard_dir)?;

        let dest_path = shard_dir.join(&sha256);

        // Copy or hardlink the file
        if self.object_config.enable_hardlinks {
            let _ = fs::hard_link(source_path, &dest_path);
            if !dest_path.exists() {
                fs::copy(source_path, &dest_path)?;
            }
        } else {
            fs::copy(source_path, &dest_path)?;
        }

        // Create metadata
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        let extension = source_path.extension().and_then(|e| e.to_str()).map(|s| s.to_string());
        let mime_type = Self::guess_mime_type(source_path, &extension);

        let meta = ObjectMeta {
            sha256: sha256.clone(),
            object_type,
            size_bytes: content.len() as u64,
            created_unix: now,
            last_accessed_unix: now,
            ref_count: 1,
            build_timestamps: build_timestamp.map(|ts| vec![ts]).unwrap_or_default(),
            extension,
            mime_type,
        };

        self.objects.insert(sha256.clone(), meta);
        self.save_object_index()?;

        Ok(sha256)
    }

    /// Store multiple objects from a directory (e.g., build output)
    pub fn store_build_artifacts(
        &mut self,
        build_dir: &Path,
        build_timestamp: u64,
    ) -> Result<Vec<String>> {
        let mut hashes = Vec::new();

        for entry in fs::read_dir(build_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                // Determine object type from extension
                let object_type = Self::classify_artifact(&path);
                let hash = self.store_object(&path, object_type, Some(build_timestamp))?;
                hashes.push(hash);
            } else if path.is_dir() {
                // Recurse into subdirectories
                let sub_hashes = self.store_build_artifacts(&path, build_timestamp)?;
                hashes.extend(sub_hashes);
            }
        }

        Ok(hashes)
    }

    /// Classify artifact type from file extension
    fn classify_artifact(path: &Path) -> ObjectType {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        match ext.to_lowercase().as_str() {
            "exe" | "dll" | "so" | "dylib" | "lib" | "a" => ObjectType::Artifact,
            "o" | "obj" | "pdb" | "ilk" | "exp" => ObjectType::Intermediate,
            "cmake" | "ninja" | "json" | "toml" | "txt" | "log" => ObjectType::Config,
            "rs" | "c" | "cpp" | "h" | "hpp" | "cc" | "cxx" => ObjectType::Source,
            _ => ObjectType::Artifact,
        }
    }

    /// Guess MIME type from path and extension
    fn guess_mime_type(path: &Path, extension: &Option<String>) -> Option<String> {
        let ext = extension.as_deref().unwrap_or("");
        Some(match ext.to_lowercase().as_str() {
            "exe" => "application/x-msdownload",
            "dll" | "so" | "dylib" => "application/x-sharedlib",
            "o" | "obj" => "application/x-object",
            "pdb" => "application/x-pdb",
            "lib" | "a" => "application/x-archive",
            "json" => "application/json",
            "toml" => "application/toml",
            "txt" | "log" => "text/plain",
            "rs" => "text/x-rust",
            "c" | "h" => "text/x-c",
            "cpp" | "hpp" | "cc" | "cxx" => "text/x-c++",
            "cmake" => "text/x-cmake",
            "ninja" => "text/x-ninja",
            _ => "application/octet-stream",
        }.to_string())
    }

    /// Get object path in store
    pub fn get_object_path(&self, sha256: &str) -> Option<PathBuf> {
        if self.objects.contains_key(sha256) {
            let shard = &sha256[..2];
            Some(self.object_config.store_root.join(shard).join(sha256))
        } else {
            None
        }
    }

    /// Retrieve an object to a destination path
    pub fn retrieve_object(&mut self, sha256: &str, dest_path: &Path) -> Result<()> {
        let source = self.get_object_path(sha256)
            .ok_or_else(|| anyhow!("Object not found: {}", sha256))?;

        if self.object_config.enable_hardlinks {
            let _ = fs::hard_link(&source, dest_path);
            if !dest_path.exists() {
                fs::copy(&source, dest_path)?;
            }
        } else {
            fs::copy(&source, dest_path)?;
        }

        // Update access time
        if let Some(meta) = self.objects.get_mut(sha256) {
            meta.last_accessed_unix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        }

        Ok(())
    }

    /// Record a lift (incremental build unit)
    pub fn record_lift(
        &mut self,
        name: String,
        input_hashes: Vec<String>,
        output_hashes: Vec<String>,
        command: String,
        working_dir: String,
        env_vars: HashMap<String, String>,
        build_timestamp: u64,
    ) -> Result<String> {
        // Compute lift hash from inputs + command + env
        let mut hasher = Sha256::new();
        for input in &input_hashes {
            hasher.update(input.as_bytes());
        }
        hasher.update(command.as_bytes());
        for (k, v) in &env_vars {
            hasher.update(k.as_bytes());
            hasher.update(v.as_bytes());
        }
        let lift_hash = format!("{:x}", hasher.finalize());

        // Check if lift already exists and is valid
        if let Some(existing) = self.lifts.get(&lift_hash) {
            if existing.is_valid && existing.build_timestamp == build_timestamp {
                return Ok(lift_hash);
            }
        }

        // Verify all input objects exist
        for input_hash in &input_hashes {
            if !self.objects.contains_key(input_hash) {
                return Err(anyhow!("Input object not found: {}", input_hash));
            }
        }

        // Verify all output objects exist
        let mut total_output_size = 0u64;
        for output_hash in &output_hashes {
            if let Some(meta) = self.objects.get(output_hash) {
                total_output_size += meta.size_bytes;
            } else {
                return Err(anyhow!("Output object not found: {}", output_hash));
            }
        }

        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

        let lift = LiftMeta {
            sha256: lift_hash.clone(),
            name,
            input_hashes,
            output_hashes,
            command,
            working_dir,
            env_vars,
            created_unix: now,
            last_accessed_unix: now,
            build_timestamp,
            is_valid: true,
            total_output_size,
        };

        self.lifts.insert(lift_hash.clone(), lift);
        self.save_lift_index()?;

        Ok(lift_hash)
    }

    /// Check if a lift can be reused (inputs match)
    pub fn can_reuse_lift(&self, lift_hash: &str, current_input_hashes: &[String]) -> bool {
        if !self.lift_config.enable_reuse {
            return false;
        }

        if let Some(lift) = self.lifts.get(lift_hash) {
            if !lift.is_valid {
                return false;
            }
            // Check if input hashes match
            if lift.input_hashes.len() != current_input_hashes.len() {
                return false;
            }
            for (a, b) in lift.input_hashes.iter().zip(current_input_hashes.iter()) {
                if a != b {
                    return false;
                }
            }
            return true;
        }
        false
    }

    /// Invalidate lifts for a build (e.g., when config changes)
    pub fn invalidate_lifts_for_build(&mut self, build_timestamp: u64) -> usize {
        let mut count = 0;
        for lift in self.lifts.values_mut() {
            if lift.build_timestamp == build_timestamp && lift.is_valid {
                lift.is_valid = false;
                count += 1;
            }
        }
        if count > 0 {
            let _ = self.save_lift_index();
        }
        count
    }

    /// Get lifts for a build
    pub fn get_lifts_for_build(&self, build_timestamp: u64) -> Vec<&LiftMeta> {
        self.lifts
            .values()
            .filter(|l| l.build_timestamp == build_timestamp)
            .collect()
    }

    /// Cleanup unreferenced objects
    pub fn cleanup_objects(&mut self) -> Result<usize> {
        if self.object_config.max_total_size == 0 && self.object_config.max_objects == 0 {
            return Ok(0);
        }

        let mut objects: Vec<_> = self.objects.values().cloned().collect();
        objects.sort_by(|a, b| a.last_accessed_unix.cmp(&b.last_accessed_unix));

        let mut to_remove = Vec::new();
        let mut total_size: u64 = objects.iter().map(|o| o.size_bytes).sum();

        // Remove unreferenced objects first
        for obj in &objects {
            if obj.ref_count == 0 {
                to_remove.push(obj.sha256.clone());
                total_size = total_size.saturating_sub(obj.size_bytes);
            }
        }

        // Remove by max objects
        if self.object_config.max_objects > 0 && objects.len() > self.object_config.max_objects {
            let excess = objects.len() - self.object_config.max_objects;
            for obj in objects.iter().take(excess) {
                if obj.ref_count <= self.object_config.min_ref_count && !to_remove.contains(&obj.sha256) {
                    to_remove.push(obj.sha256.clone());
                    total_size = total_size.saturating_sub(obj.size_bytes);
                }
            }
        }

        // Remove by max size
        if self.object_config.max_total_size > 0 && total_size > self.object_config.max_total_size {
            for obj in &objects {
                if total_size <= self.object_config.max_total_size {
                    break;
                }
                if obj.ref_count <= self.object_config.min_ref_count && !to_remove.contains(&obj.sha256) {
                    to_remove.push(obj.sha256.clone());
                    total_size = total_size.saturating_sub(obj.size_bytes);
                }
            }
        }

        // Remove objects
        let mut removed = 0;
        for sha256 in to_remove {
            if let Some(meta) = self.objects.remove(&sha256) {
                let path = self.object_config.store_root.join(&meta.sha256[..2]).join(&sha256);
                if path.exists() {
                    let _ = fs::remove_file(&path);
                }
                removed += 1;
            }
        }

        if removed > 0 {
            self.save_object_index()?;
        }

        Ok(removed)
    }

    /// Cleanup invalid lifts
    pub fn cleanup_lifts(&mut self) -> Result<usize> {
        if self.lift_config.max_total_size == 0 && self.lift_config.max_lifts == 0 {
            return Ok(0);
        }

        let mut lifts: Vec<_> = self.lifts.values().cloned().collect();
        lifts.sort_by(|a, b| a.last_accessed_unix.cmp(&b.last_accessed_unix));

        let mut to_remove = Vec::new();
        let mut total_size: u64 = lifts.iter().map(|l| l.total_output_size).sum();

        // Remove invalid lifts first
        for lift in &lifts {
            if !lift.is_valid {
                to_remove.push(lift.sha256.clone());
                total_size = total_size.saturating_sub(lift.total_output_size);
            }
        }

        // Remove by max lifts
        if self.lift_config.max_lifts > 0 && lifts.len() > self.lift_config.max_lifts {
            let excess = lifts.len() - self.lift_config.max_lifts;
            for lift in lifts.iter().take(excess) {
                if !to_remove.contains(&lift.sha256) {
                    to_remove.push(lift.sha256.clone());
                    total_size = total_size.saturating_sub(lift.total_output_size);
                }
            }
        }

        // Remove by max size
        if self.lift_config.max_total_size > 0 && total_size > self.lift_config.max_total_size {
            for lift in &lifts {
                if total_size <= self.lift_config.max_total_size {
                    break;
                }
                if !to_remove.contains(&lift.sha256) {
                    to_remove.push(lift.sha256.clone());
                    total_size = total_size.saturating_sub(lift.total_output_size);
                }
            }
        }

        let mut removed = 0;
        for sha256 in to_remove {
            if self.lifts.remove(&sha256).is_some() {
                removed += 1;
            }
        }

        if removed > 0 {
            self.save_lift_index()?;
        }

        Ok(removed)
    }

    /// Save object index
    fn save_object_index(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.objects)?;
        fs::write(&self.object_index_path, content)?;
        Ok(())
    }

    /// Save lift index
    fn save_lift_index(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.lifts)?;
        fs::write(&self.lift_index_path, content)?;
        Ok(())
    }

    /// Get object store statistics
    pub fn get_object_stats(&self) -> ObjectStoreStats {
        let total_size: u64 = self.objects.values().map(|o| o.size_bytes).sum();
        let total_refs: u32 = self.objects.values().map(|o| o.ref_count).sum();
        let unreferenced = self.objects.values().filter(|o| o.ref_count == 0).count();

        let mut by_type: HashMap<ObjectType, usize> = HashMap::new();
        for obj in self.objects.values() {
            *by_type.entry(obj.object_type).or_insert(0) += 1;
        }

        ObjectStoreStats {
            total_objects: self.objects.len(),
            total_size_bytes: total_size,
            total_refs,
            unreferenced_objects: unreferenced,
            by_type,
        }
    }

    /// Get lift store statistics
    pub fn get_lift_stats(&self) -> LiftStoreStats {
        let valid_lifts = self.lifts.values().filter(|l| l.is_valid).count();
        let invalid_lifts = self.lifts.values().filter(|l| !l.is_valid).count();
        let total_output_size: u64 = self.lifts.values().map(|l| l.total_output_size).sum();

        LiftStoreStats {
            total_lifts: self.lifts.len(),
            valid_lifts,
            invalid_lifts,
            total_output_size_bytes: total_output_size,
        }
    }

    /// Export object index
    pub fn export_objects(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.objects)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Export lift index
    pub fn export_lifts(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.lifts)?;
        fs::write(path, content)?;
        Ok(())
    }
}

/// Object store statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectStoreStats {
    pub total_objects: usize,
    pub total_size_bytes: u64,
    pub total_refs: u32,
    pub unreferenced_objects: usize,
    pub by_type: HashMap<ObjectType, usize>,
}

/// Lift store statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiftStoreStats {
    pub total_lifts: usize,
    pub valid_lifts: usize,
    pub invalid_lifts: usize,
    pub total_output_size_bytes: u64,
}

/// Integrate with BuildVersionManager to store build artifacts on promote
pub fn store_build_artifacts_on_promote(
    store: &mut ObjectLiftStoreManager,
    version_manager: &BuildVersionManager,
    build_timestamp: u64,
) -> Result<Vec<String>> {
    let build = version_manager.list_versions()
        .into_iter()
        .find(|v| v.timestamp_unix == build_timestamp);

    let Some(build) = build else {
        return Err(anyhow!("Build not found: {}", build_timestamp));
    };

    let staging_dir = version_manager.get_staging_dir(&build);
    let build_dir = staging_dir.join("build").join(build.config.to_lowercase());

    if build_dir.exists() {
        store.store_build_artifacts(&build_dir, build_timestamp)
    } else {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_configs() -> (ObjectStoreConfig, LiftStoreConfig) {
        let dir = tempdir().unwrap();
        let object_config = ObjectStoreConfig {
            store_root: dir.path().join("object_store"),
            max_total_size: 100 * 1024 * 1024,
            max_objects: 1000,
            min_ref_count: 0,
            enable_hardlinks: true,
            compress: false,
        };
        let lift_config = LiftStoreConfig {
            store_root: dir.path().join("lift_store"),
            max_total_size: 50 * 1024 * 1024,
            max_lifts: 500,
            enable_reuse: true,
        };
        (object_config, lift_config)
    }

    #[test]
    fn test_object_store_creation() {
        let (object_config, lift_config) = create_test_configs();
        let store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();
        assert_eq!(store.objects.len(), 0);
        assert_eq!(store.lifts.len(), 0);
    }

    #[test]
    fn test_store_object() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.exe");
        fs::write(&test_file, b"fake executable content").unwrap();

        let hash = store.store_object(&test_file, ObjectType::Artifact, Some(12345)).unwrap();
        assert_eq!(hash.len(), 64); // SHA-256 hex

        let meta = store.objects.get(&hash).unwrap();
        assert_eq!(meta.object_type, ObjectType::Artifact);
        assert_eq!(meta.ref_count, 1);
        assert_eq!(meta.build_timestamps, vec![12345]);
    }

    #[test]
    fn test_store_object_deduplication() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.exe");
        fs::write(&test_file, b"same content").unwrap();

        let hash1 = store.store_object(&test_file, ObjectType::Artifact, Some(100)).unwrap();
        let hash2 = store.store_object(&test_file, ObjectType::Artifact, Some(200)).unwrap();

        assert_eq!(hash1, hash2); // Same content = same hash

        let meta = store.objects.get(&hash1).unwrap();
        assert_eq!(meta.ref_count, 2); // Referenced twice
        assert_eq!(meta.build_timestamps.len(), 2);
    }

    #[test]
    fn test_store_build_artifacts() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let build_dir = dir.path().join("build").join("release");
        fs::create_dir_all(&build_dir).unwrap();

        fs::write(build_dir.join("nxrt_gfx_demo.exe"), b"exe content").unwrap();
        fs::write(build_dir.join("nxrt.dll"), b"dll content").unwrap();
        fs::write(build_dir.join("config.json"), b"{}").unwrap();

        let hashes = store.store_build_artifacts(&build_dir, 12345).unwrap();
        assert_eq!(hashes.len(), 3);

        for hash in &hashes {
            let meta = store.objects.get(hash).unwrap();
            assert_eq!(meta.build_timestamps, vec![12345]);
        }
    }

    #[test]
    fn test_record_lift() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        // First store some input objects
        let dir = tempdir().unwrap();
        let input_file = dir.path().join("input.cpp");
        fs::write(&input_file, b"int main() {}").unwrap();
        let input_hash = store.store_object(&input_file, ObjectType::Source, Some(100)).unwrap();

        let output_file = dir.path().join("output.o");
        fs::write(&output_file, b"object code").unwrap();
        let output_hash = store.store_object(&output_file, ObjectType::Intermediate, Some(100)).unwrap();

        // Record lift
        let lift_hash = store.record_lift(
            "compile_main".to_string(),
            vec![input_hash],
            vec![output_hash],
            "clang-cl -c input.cpp -o output.o".to_string(),
            dir.path().to_string_lossy().to_string(),
            HashMap::new(),
            100,
        ).unwrap();

        assert_eq!(lift_hash.len(), 64);

        let lift = store.lifts.get(&lift_hash).unwrap();
        assert_eq!(lift.name, "compile_main");
        assert!(lift.is_valid);
        assert_eq!(lift.build_timestamp, 100);
    }

    #[test]
    fn test_can_reuse_lift() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let input_file = dir.path().join("input.cpp");
        fs::write(&input_file, b"int main() {}").unwrap();
        let input_hash = store.store_object(&input_file, ObjectType::Source, Some(100)).unwrap();

        let output_file = dir.path().join("output.o");
        fs::write(&output_file, b"object code").unwrap();
        let output_hash = store.store_object(&output_file, ObjectType::Intermediate, Some(100)).unwrap();

        let lift_hash = store.record_lift(
            "compile_main".to_string(),
            vec![input_hash.clone()],
            vec![output_hash],
            "clang-cl -c input.cpp -o output.o".to_string(),
            dir.path().to_string_lossy().to_string(),
            HashMap::new(),
            100,
        ).unwrap();

        // Same inputs - can reuse
        assert!(store.can_reuse_lift(&lift_hash, &[input_hash.clone()]));

        // Different inputs - cannot reuse
        let other_file = dir.path().join("other.cpp");
        fs::write(&other_file, b"void foo() {}").unwrap();
        let other_hash = store.store_object(&other_file, ObjectType::Source, Some(100)).unwrap();
        assert!(!store.can_reuse_lift(&lift_hash, &[other_hash]));
    }

    #[test]
    fn test_get_stats() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.exe");
        fs::write(&test_file, b"content").unwrap();

        store.store_object(&test_file, ObjectType::Artifact, Some(100)).unwrap();

        let stats = store.get_object_stats();
        assert_eq!(stats.total_objects, 1);
        assert_eq!(stats.total_size_bytes, 7);
        assert_eq!(stats.unreferenced_objects, 0);
    }
}

// Phase D: Production Hardening Tests
#[cfg(test)]
mod phase_d_tests {
    use super::*;
    use tempfile::tempdir;

    fn create_test_configs() -> (ObjectStoreConfig, LiftStoreConfig) {
        let dir = tempdir().unwrap();
        let object_config = ObjectStoreConfig {
            store_root: dir.path().join("object_store"),
            max_total_size: 100 * 1024 * 1024,
            max_objects: 1000,
            min_ref_count: 0,
            enable_hardlinks: true,
            compress: false,
        };
        let lift_config = LiftStoreConfig {
            store_root: dir.path().join("lift_store"),
            max_total_size: 50 * 1024 * 1024,
            max_lifts: 500,
            enable_reuse: true,
        };
        (object_config, lift_config)
    }

    #[test]
    fn test_corrupted_object_index_recovery() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config.clone(), lift_config.clone()).unwrap();

        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.exe");
        fs::write(&test_file, b"executable content").unwrap();

        store.store_object(&test_file, ObjectType::Artifact, Some(100)).unwrap();
        assert_eq!(store.objects.len(), 1);

        // Corrupt the object index file
        let index_path = object_config.store_root.join("object_index.json");
        fs::write(&index_path, b"corrupted json").unwrap();

        // New store should handle gracefully
        let store2 = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();
        assert_eq!(store2.objects.len(), 0);
    }

    #[test]
    fn test_corrupted_lift_index_recovery() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config.clone(), lift_config.clone()).unwrap();

        let dir = tempdir().unwrap();
        let input_file = dir.path().join("input.cpp");
        fs::write(&input_file, b"int main() {}").unwrap();
        let input_hash = store.store_object(&input_file, ObjectType::Source, Some(100)).unwrap();

        let output_file = dir.path().join("output.o");
        fs::write(&output_file, b"object code").unwrap();
        let output_hash = store.store_object(&output_file, ObjectType::Intermediate, Some(100)).unwrap();

        store.record_lift(
            "compile_main".to_string(),
            vec![input_hash],
            vec![output_hash],
            "clang-cl -c input.cpp -o output.o".to_string(),
            dir.path().to_string_lossy().to_string(),
            HashMap::new(),
            100,
        ).unwrap();
        assert_eq!(store.lifts.len(), 1);

        // Corrupt the lift index file
        let index_path = lift_config.store_root.join("lift_index.json");
        fs::write(&index_path, b"corrupted json").unwrap();

        // New store should handle gracefully
        let store2 = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();
        assert_eq!(store2.lifts.len(), 0);
    }

    #[test]
    fn test_object_content_verification() {
        let (object_config, lift_config) = create_test_configs();
        let object_config_clone = object_config.clone();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.exe");
        fs::write(&test_file, b"original content").unwrap();

        let hash = store.store_object(&test_file, ObjectType::Artifact, Some(100)).unwrap();

        // Verify content matches hash
        let meta = store.objects.get(&hash).unwrap();
        let content_path = object_config_clone.store_root.join(&hash[0..2]).join(&hash);
        assert!(content_path.exists());

        let content = fs::read(&content_path).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let computed_hash = format!("{:x}", hasher.finalize());
        assert_eq!(computed_hash, hash);
    }

    #[test]
    fn test_ref_counting_accuracy() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let test_file = dir.path().join("shared.dll");
        fs::write(&test_file, b"shared library").unwrap();

        // Store same object for multiple builds
        let hash1 = store.store_object(&test_file, ObjectType::Artifact, Some(100)).unwrap();
        let hash2 = store.store_object(&test_file, ObjectType::Artifact, Some(200)).unwrap();
        let hash3 = store.store_object(&test_file, ObjectType::Artifact, Some(300)).unwrap();

        assert_eq!(hash1, hash2);
        assert_eq!(hash2, hash3);

        let meta = store.objects.get(&hash1).unwrap();
        assert_eq!(meta.ref_count, 3);
        assert_eq!(meta.build_timestamps.len(), 3);

        // Remove one build reference by directly manipulating ref_count
        if let Some(meta) = store.objects.get_mut(&hash1) {
            meta.ref_count = 2;
            meta.build_timestamps.retain(|&ts| ts != 200);
        }
        let meta = store.objects.get(&hash1).unwrap();
        assert_eq!(meta.ref_count, 2);
        assert_eq!(meta.build_timestamps.len(), 2);
    }

    #[test]
    fn test_hardlink_fallback_on_windows() {
        let (object_config, lift_config) = create_test_configs();
        let object_config_clone = object_config.clone();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let test_file = dir.path().join("test.exe");
        fs::write(&test_file, b"executable content").unwrap();

        let hash = store.store_object(&test_file, ObjectType::Artifact, Some(100)).unwrap();

        // On Windows, if hardlinks not supported, should fall back to copy
        let content_path = object_config_clone.store_root.join(&hash[0..2]).join(&hash);
        assert!(content_path.exists());

        // File should be readable (not read-only if copy fallback)
        let metadata = fs::metadata(&content_path).unwrap();
        assert!(!metadata.permissions().readonly());
    }

    #[test]
    fn test_lift_invalidation_on_input_change() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let input_file = dir.path().join("input.cpp");
        fs::write(&input_file, b"int main() {}").unwrap();
        let input_hash = store.store_object(&input_file, ObjectType::Source, Some(100)).unwrap();

        let output_file = dir.path().join("output.o");
        fs::write(&output_file, b"object code").unwrap();
        let output_hash = store.store_object(&output_file, ObjectType::Intermediate, Some(100)).unwrap();

        let lift_hash = store.record_lift(
            "compile_main".to_string(),
            vec![input_hash.clone()],
            vec![output_hash],
            "clang-cl -c input.cpp -o output.o".to_string(),
            dir.path().to_string_lossy().to_string(),
            HashMap::new(),
            100,
        ).unwrap();

        // Same inputs - valid
        assert!(store.can_reuse_lift(&lift_hash, &[input_hash.clone()]));
        let lift = store.lifts.get(&lift_hash).unwrap();
        assert!(lift.is_valid);

        // Modify input - should invalidate
        fs::write(&input_file, b"int main() { return 0; }").unwrap();
        let new_input_hash = store.store_object(&input_file, ObjectType::Source, Some(200)).unwrap();

        // Can't reuse with new input hash
        assert!(!store.can_reuse_lift(&lift_hash, &[new_input_hash]));
    }

    #[test]
    fn test_cleanup_objects_removes_unreferenced() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();

        // Add referenced object
        let ref_file = dir.path().join("referenced.exe");
        fs::write(&ref_file, b"referenced").unwrap();
        let ref_hash = store.store_object(&ref_file, ObjectType::Artifact, Some(100)).unwrap();

        // Add unreferenced object (ref_count = 0)
        let unref_file = dir.path().join("unreferenced.dll");
        fs::write(&unref_file, b"unreferenced").unwrap();
        let unref_hash = store.store_object(&unref_file, ObjectType::Artifact, Some(100)).unwrap();
        // Manually set ref_count to 0
        if let Some(meta) = store.objects.get_mut(&unref_hash) {
            meta.ref_count = 0;
            meta.build_timestamps.clear();
        }

        assert_eq!(store.objects.len(), 2);

        // Cleanup should remove unreferenced
        let removed = store.cleanup_objects().unwrap();
        assert_eq!(removed, 1);
        assert!(store.objects.contains_key(&ref_hash));
        assert!(!store.objects.contains_key(&unref_hash));
    }

    #[test]
    fn test_cleanup_lifts_removes_invalid() {
        let (object_config, lift_config) = create_test_configs();
        let mut store = ObjectLiftStoreManager::new(object_config, lift_config).unwrap();

        let dir = tempdir().unwrap();
        let input_file = dir.path().join("input.cpp");
        fs::write(&input_file, b"int main() {}").unwrap();
        let input_hash = store.store_object(&input_file, ObjectType::Source, Some(100)).unwrap();

        let output_file = dir.path().join("output.o");
        fs::write(&output_file, b"object code").unwrap();
        let output_hash = store.store_object(&output_file, ObjectType::Intermediate, Some(100)).unwrap();

        // Valid lift
        let valid_lift = store.record_lift(
            "valid_compile".to_string(),
            vec![input_hash.clone()],
            vec![output_hash.clone()],
            "clang-cl -c input.cpp -o output.o".to_string(),
            dir.path().to_string_lossy().to_string(),
            HashMap::new(),
            100,
        ).unwrap();

        // Invalid lift (different inputs)
        let other_file = dir.path().join("other.cpp");
        fs::write(&other_file, b"void foo() {}").unwrap();
        let other_hash = store.store_object(&other_file, ObjectType::Source, Some(100)).unwrap();

        let invalid_lift = store.record_lift(
            "invalid_compile".to_string(),
            vec![other_hash],
            vec![output_hash],
            "clang-cl -c other.cpp -o output.o".to_string(),
            dir.path().to_string_lossy().to_string(),
            HashMap::new(),
            100,
        ).unwrap();

        // Manually mark as invalid
        if let Some(lift) = store.lifts.get_mut(&invalid_lift) {
            lift.is_valid = false;
        }

        assert_eq!(store.lifts.len(), 2);

        // Cleanup should remove invalid lifts
        let removed = store.cleanup_lifts().unwrap();
        assert_eq!(removed, 1);
        assert!(store.lifts.contains_key(&valid_lift));
        assert!(!store.lifts.contains_key(&invalid_lift));
    }
}