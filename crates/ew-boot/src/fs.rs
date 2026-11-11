//! `nn::fs`, served by the host filesystem.
//!
//! Mounts map a guest mount name onto a host directory:
//!
//! * `MountRom` maps onto the user's romfs, **read-only**: an extracted folder,
//!   or the RomFS inside a `.nup` container, read in place (see [`NupRom`]).
//!   The host never opens a romfs file for writing, so the user's dump cannot
//!   be modified through this layer.
//! * `MountSaveData` maps onto a separate, writable save directory chosen by the
//!   launcher (by default under `%APPDATA%\EmblemWeave\save`).
//!
//! Guest paths have the form `mount:/dir/file`. They are resolved component by
//! component; `..`, absolute components and drive prefixes are rejected, so a
//! guest path can never name a host file outside its mount.
//!
//! Failures the game is designed to handle (a missing file, an existing
//! directory) return the real `nn::fs` result codes. Conditions the host cannot
//! honour (add-on content, a mount that was never configured) stop the run.

use crate::hle::{c_string, read_bytes, write_bytes, HostResult, HostServices};
use ew_lift::State;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// `nn::fs::ResultPathNotFound` (2002-0001).
pub const RESULT_PATH_NOT_FOUND: u64 = 2 | (1 << 9);
/// `nn::fs::ResultPathAlreadyExists` (2002-0002).
pub const RESULT_PATH_ALREADY_EXISTS: u64 = 2 | (2 << 9);
/// `nn::fs::ResultOutOfRange` (2002-3005), a read past the end of a file.
pub const RESULT_OUT_OF_RANGE: u64 = 2 | (3005 << 9);

const OPEN_READ: u32 = 1;
const OPEN_WRITE: u32 = 2;
const OPEN_APPEND: u32 = 4;

/// `nn::fs::DirectoryEntry`: name[769], type at 0x304, size at 0x308.
const DIR_ENTRY_BYTES: u64 = 0x310;

// ---- a RomFS read in place from a .nup container ------------------------------

/// The RomFS of a `.nup` container. Host paths below the container's own path
/// name entries inside it. Stored files are read at their offset; compressed
/// ones are decompressed once, checked against their SHA-256, and kept in a
/// cache of at most `CACHE_BYTES`, oldest first out.
pub struct NupRom {
    pub root: PathBuf,
    pub nup: std::sync::Arc<ew_format::nup::Nup>,
    files: std::collections::HashMap<String, u32>,
    /// Every directory, by path relative to the RomFS root ("" is the root),
    /// with its entries: (name, is directory, size).
    dirs: std::collections::HashMap<String, Vec<(String, bool, u64)>>,
    cache: std::collections::VecDeque<(u32, std::sync::Arc<Vec<u8>>)>,
    cached: u64,
}

const CACHE_BYTES: u64 = 1 << 30;

#[derive(Default)]
struct NupSlot(Option<NupRom>);

impl NupRom {
    pub fn open(path: &Path) -> Result<NupRom, String> {
        let nup = ew_format::nup::Nup::open(path).map_err(|e| format!("{e:#}"))?;
        let pkg = nup.program_package().ok_or("the .nup has no base or update package")?.clone();
        let files = nup.index(pkg.romfs);
        let mut dirs: std::collections::HashMap<String, Vec<(String, bool, u64)>> = std::collections::HashMap::new();
        dirs.entry(String::new()).or_default();
        for (rel, i) in &files {
            let size = nup.files[*i as usize].size;
            let mut parent = String::new();
            let parts: Vec<&str> = rel.split('/').collect();
            for (k, part) in parts.iter().enumerate() {
                let is_file = k + 1 == parts.len();
                let here = if parent.is_empty() { part.to_string() } else { format!("{parent}/{part}") };
                let list = dirs.entry(parent.clone()).or_default();
                if !list.iter().any(|(n, _, _)| n == part) {
                    list.push((part.to_string(), !is_file, if is_file { size } else { 0 }));
                }
                if !is_file {
                    dirs.entry(here.clone()).or_default();
                }
                parent = here;
            }
        }
        for list in dirs.values_mut() {
            list.sort();
        }
        Ok(NupRom { root: path.to_path_buf(), nup: std::sync::Arc::new(nup), files, dirs, cache: Default::default(), cached: 0 })
    }

    /// The path inside the container that a host path names, if it is inside.
    fn rel(&self, path: &Path) -> Option<String> {
        let rest = path.strip_prefix(&self.root).ok()?;
        Some(rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/"))
    }

    fn file_size(&self, rel: &str) -> Option<u64> {
        self.files.get(rel).map(|i| self.nup.files[*i as usize].size)
    }

    /// `len` bytes of a file from `offset`.
    fn read(&mut self, rel: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        let i = *self.files.get(rel).ok_or_else(|| format!("{rel} is not in the .nup"))?;
        let e = self.nup.files[i as usize].clone();
        let end = (offset + len).min(e.size);
        if offset >= end {
            return Ok(Vec::new());
        }
        if !e.compressed {
            let mut f = std::fs::File::open(&self.root).map_err(|err| err.to_string())?;
            f.seek(SeekFrom::Start(e.offset + offset)).map_err(|err| err.to_string())?;
            let mut out = vec![0u8; (end - offset) as usize];
            f.read_exact(&mut out).map_err(|err| err.to_string())?;
            return Ok(out);
        }
        let data = match self.cache.iter().find(|(k, _)| *k == i) {
            Some((_, d)) => d.clone(),
            None => {
                let d = std::sync::Arc::new(self.nup.read(i).map_err(|err| format!("{err:#}"))?);
                self.cached += d.len() as u64;
                self.cache.push_back((i, d.clone()));
                while self.cached > CACHE_BYTES && self.cache.len() > 1 {
                    if let Some((_, old)) = self.cache.pop_front() {
                        self.cached -= old.len() as u64;
                    }
                }
                d
            }
        };
        Ok(data[offset as usize..end as usize].to_vec())
    }
}

fn nup<R>(state: &State, f: impl FnOnce(&mut Option<NupRom>) -> R) -> R {
    state.host.ext.with::<NupSlot, R>(|s| f(&mut s.0))
}

/// For a host path inside the `.nup` RomFS: its path inside the container.
fn in_nup(state: &State, path: &Path) -> Option<String> {
    nup(state, |n| n.as_ref().and_then(|n| n.rel(path)))
}

/// What a host path is: 0 directory, 1 file, or nothing.
fn entry_kind(state: &State, path: &Path) -> Option<u32> {
    if let Some(rel) = in_nup(state, path) {
        return nup(state, |n| {
            let n = n.as_ref()?;
            if n.dirs.contains_key(&rel) {
                Some(0)
            } else if n.files.contains_key(&rel) {
                Some(1)
            } else {
                None
            }
        });
    }
    if path.is_dir() {
        Some(0)
    } else if path.is_file() {
        Some(1)
    } else {
        None
    }
}

fn host_file_size(state: &State, path: &Path) -> Option<u64> {
    match in_nup(state, path) {
        Some(rel) => nup(state, |n| n.as_ref().and_then(|n| n.file_size(&rel))),
        None => std::fs::metadata(path).ok().map(|m| m.len()),
    }
}

fn guest_str(state: &State, addr: u64) -> Result<String, HostResult> {
    let bytes = c_string(state, addr, 0x400)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Resolve `mount:/a/b` to a host path inside that mount, with its writability.
pub(crate) fn resolve(state: &State, guest: &str) -> Result<(PathBuf, bool), HostResult> {
    let Some((mount, rest)) = guest.split_once(':') else {
        return Err(HostResult::Unavailable(format!("fs path `{guest}` has no mount name")));
    };
    let Some((root, writable)) = state.host.mounts.get(mount).cloned() else {
        return Err(HostResult::Unavailable(format!(
            "fs path `{guest}` uses mount `{mount}`, which is not mounted"
        )));
    };
    let mut path = root;
    for part in rest.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => {
                return Err(HostResult::Unavailable(format!("fs path `{guest}` climbs out of its mount")))
            }
            p if p.contains(':') => {
                return Err(HostResult::Unavailable(format!("fs path `{guest}` has a drive component")))
            }
            p => path.push(p),
        }
    }
    Ok((path, writable))
}

fn put_u64(state: &mut State, addr: u64, v: u64) -> Result<(), HostResult> {
    write_bytes(state, addr, &v.to_le_bytes())
}

fn new_handle(state: &mut State) -> u64 {
    state.host.next_handle += 1;
    // Handles are opaque pointers to the game; keep them distinctive and nonzero.
    0xF5_0000_0000 + state.host.next_handle
}

macro_rules! tryr {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

fn io_fault(what: &str, path: &Path, e: std::io::Error) -> HostResult {
    HostResult::Unavailable(format!("{what} {}: {e}", path.display()))
}

fn read_only(guest: &str) -> HostResult {
    HostResult::Unavailable(format!(
        "the guest tried to modify `{guest}` on a read-only mount; the user's game files are never written"
    ))
}

// ---- allocator, cache and priority ---------------------------------------

/// `SetAllocator(alloc, free)`: the host layer allocates nothing in guest
/// memory, so the functions are recorded for completeness and never called.
fn set_allocator(state: &mut State) -> HostResult {
    state.host.named.insert("fs.alloc".into(), state.x[0]);
    state.host.named.insert("fs.free".into(), state.x[1]);
    HostResult::Returned(0)
}

/// `QueryMountRomCacheSize(size_t*)`. The host reads romfs through the OS file
/// cache, so the guest buffer is not needed; one page keeps the allocation real.
fn query_rom_cache(state: &mut State) -> HostResult {
    tryr!(put_u64(state, state.x[0], 0x1000));
    HostResult::Returned(0)
}

fn ok(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

// ---- mounts --------------------------------------------------------------

fn mount_rom(state: &mut State) -> HostResult {
    let name = tryr!(guest_str(state, state.x[0]));
    let Some(root) = state.host.romfs_root.clone() else {
        return HostResult::Unavailable(format!(
            "MountRom(\"{name}\"): no romfs directory is configured; set the game data folder in the launcher"
        ));
    };
    if root.is_file() {
        let already = nup(state, |n| n.as_ref().is_some_and(|n| n.root == root));
        if !already {
            match NupRom::open(&root) {
                Ok(rom) => nup(state, |n| *n = Some(rom)),
                Err(e) => return HostResult::Unavailable(format!("MountRom: {e}")),
            }
        }
    } else if !root.is_dir() {
        return HostResult::Unavailable(format!("MountRom: romfs directory {} does not exist", root.display()));
    }
    state.host.log.push(format!("fs: mounted {name}: read-only on {}", root.display()));
    state.host.mounts.insert(name, (root, false));
    HostResult::Returned(0)
}

fn mount_save(state: &mut State) -> HostResult {
    let name = tryr!(guest_str(state, state.x[0]));
    let Some(root) = state.host.save_root.clone() else {
        return HostResult::Unavailable(format!("MountSaveData(\"{name}\"): no save directory is configured"));
    };
    if let Err(e) = std::fs::create_dir_all(&root) {
        return io_fault("creating save directory", &root, e);
    }
    state.host.mounts.insert(name, (root, true));
    HostResult::Returned(0)
}

fn unmount(state: &mut State) -> HostResult {
    let name = tryr!(guest_str(state, state.x[0]));
    state.host.mounts.remove(&name);
    HostResult::Returned(0)
}

fn add_on_content(_state: &mut State) -> HostResult {
    HostResult::Unavailable(
        "add-on content is not supported: the launcher has no DLC mount to offer".into(),
    )
}

// ---- entries ---------------------------------------------------------------

/// `GetEntryType(DirectoryEntryType*, const char*)`: 0 directory, 1 file.
fn get_entry_type(state: &mut State) -> HostResult {
    let guest = tryr!(guest_str(state, state.x[1]));
    let (path, _) = tryr!(resolve(state, &guest));
    let Some(kind) = entry_kind(state, &path) else {
        return HostResult::Returned(RESULT_PATH_NOT_FOUND);
    };
    tryr!(write_bytes(state, state.x[0], &kind.to_le_bytes()));
    HostResult::Returned(0)
}

fn create_directory(state: &mut State) -> HostResult {
    let guest = tryr!(guest_str(state, state.x[0]));
    let (path, writable) = tryr!(resolve(state, &guest));
    if !writable {
        return read_only(&guest);
    }
    if path.exists() {
        return HostResult::Returned(RESULT_PATH_ALREADY_EXISTS);
    }
    match std::fs::create_dir(&path) {
        Ok(()) => HostResult::Returned(0),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HostResult::Returned(RESULT_PATH_NOT_FOUND),
        Err(e) => io_fault("creating directory", &path, e),
    }
}

fn create_file(state: &mut State) -> HostResult {
    let guest = tryr!(guest_str(state, state.x[0]));
    let size = state.x[1];
    let (path, writable) = tryr!(resolve(state, &guest));
    if !writable {
        return read_only(&guest);
    }
    if path.exists() {
        return HostResult::Returned(RESULT_PATH_ALREADY_EXISTS);
    }
    match std::fs::File::create_new(&path).and_then(|f| f.set_len(size)) {
        Ok(()) => HostResult::Returned(0),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => HostResult::Returned(RESULT_PATH_NOT_FOUND),
        Err(e) => io_fault("creating file", &path, e),
    }
}

fn delete_file(state: &mut State) -> HostResult {
    let guest = tryr!(guest_str(state, state.x[0]));
    let (path, writable) = tryr!(resolve(state, &guest));
    if !writable {
        return read_only(&guest);
    }
    if !path.is_file() {
        return HostResult::Returned(RESULT_PATH_NOT_FOUND);
    }
    match std::fs::remove_file(&path) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => io_fault("deleting file", &path, e),
    }
}

/// `Commit(mount)`: writes go straight to host files, so a commit has nothing
/// left to publish. The mount must exist.
fn commit(state: &mut State) -> HostResult {
    let name = tryr!(guest_str(state, state.x[0]));
    if !state.host.mounts.contains_key(&name) {
        return HostResult::Unavailable(format!("Commit(\"{name}\") on a mount that does not exist"));
    }
    HostResult::Returned(0)
}

// ---- files -----------------------------------------------------------------

fn open_file(state: &mut State) -> HostResult {
    let out = state.x[0];
    let guest = tryr!(guest_str(state, state.x[1]));
    let mode = state.x[2] as u32;
    let (path, writable) = tryr!(resolve(state, &guest));
    if mode & (OPEN_WRITE | OPEN_APPEND) != 0 && !writable {
        return read_only(&guest);
    }
    if entry_kind(state, &path) != Some(1) {
        return HostResult::Returned(RESULT_PATH_NOT_FOUND);
    }
    let h = new_handle(state);
    let size = host_file_size(state, &path).unwrap_or(0);
    state.host.log.push(format!("fs: open {guest} ({size} bytes, mode {mode})"));
    state.host.files.insert(h, (path, mode));
    tryr!(put_u64(state, out, h));
    HostResult::Returned(0)
}

fn file(state: &State, h: u64) -> Result<(PathBuf, u32), HostResult> {
    state
        .host
        .files
        .get(&h)
        .cloned()
        .ok_or_else(|| HostResult::Unavailable(format!("fs file handle 0x{h:x} is not open")))
}

fn get_file_size(state: &mut State) -> HostResult {
    let (path, _) = tryr!(file(state, state.x[1]));
    match host_file_size(state, &path) {
        Some(len) => {
            tryr!(put_u64(state, state.x[0], len));
            HostResult::Returned(0)
        }
        None => io_fault("reading size of", &path, std::io::Error::from(std::io::ErrorKind::NotFound)),
    }
}

fn read_into(state: &mut State, h: u64, offset: u64, buf: u64, size: u64) -> Result<u64, HostResult> {
    let (path, mode) = file(state, h)?;
    if mode & OPEN_READ == 0 {
        return Err(HostResult::Unavailable(format!("ReadFile on {} opened without read access", path.display())));
    }
    if let Some(rel) = in_nup(state, &path) {
        let bytes = nup(state, |n| n.as_mut().ok_or_else(|| "the .nup RomFS is not open".to_string()).and_then(|n| n.read(&rel, offset, size)))
            .map_err(|e| HostResult::Unavailable(format!("reading {rel} from the .nup: {e}")))?;
        write_bytes(state, buf, &bytes)?;
        return Ok(bytes.len() as u64);
    }
    let mut f = std::fs::File::open(&path).map_err(|e| io_fault("opening", &path, e))?;
    let len = f.metadata().map_err(|e| io_fault("reading size of", &path, e))?.len();
    let want = size.min(len.saturating_sub(offset));
    let mut bytes = vec![0u8; want as usize];
    f.seek(SeekFrom::Start(offset)).map_err(|e| io_fault("seeking", &path, e))?;
    f.read_exact(&mut bytes).map_err(|e| io_fault("reading", &path, e))?;
    write_bytes(state, buf, &bytes)?;
    Ok(want)
}

/// `ReadFile(size_t* out, FileHandle, int64 offset, void* buf, size_t size)`.
fn read_file_counted(state: &mut State) -> HostResult {
    let (out, h, offset, buf, size) = (state.x[0], state.x[1], state.x[2], state.x[3], state.x[4]);
    let n = tryr!(read_into(state, h, offset, buf, size));
    tryr!(put_u64(state, out, n));
    HostResult::Returned(0)
}

/// `ReadFile(FileHandle, int64 offset, void* buf, size_t size)`: the whole
/// range must exist, otherwise the read fails with `ResultOutOfRange`.
fn read_file(state: &mut State) -> HostResult {
    let (h, offset, buf, size) = (state.x[0], state.x[1], state.x[2], state.x[3]);
    let (path, _) = tryr!(file(state, h));
    let len = host_file_size(state, &path).unwrap_or(0);
    if offset.saturating_add(size) > len {
        return HostResult::Returned(RESULT_OUT_OF_RANGE);
    }
    tryr!(read_into(state, h, offset, buf, size));
    HostResult::Returned(0)
}

/// `WriteFile(FileHandle, int64 offset, const void*, size_t, const WriteOption&)`.
fn write_file(state: &mut State) -> HostResult {
    let (h, offset, buf, size) = (state.x[0], state.x[1], state.x[2], state.x[3]);
    let (path, mode) = tryr!(file(state, h));
    if mode & OPEN_WRITE == 0 {
        return HostResult::Unavailable(format!("WriteFile on {} opened without write access", path.display()));
    }
    let bytes = tryr!(read_bytes(state, buf, size));
    let r = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .and_then(|mut f| {
            let len = f.metadata()?.len();
            if offset + size > len && mode & OPEN_APPEND == 0 {
                return Err(std::io::Error::other("write past the end without AllowAppend"));
            }
            f.seek(SeekFrom::Start(offset))?;
            f.write_all(&bytes)
        });
    match r {
        Ok(()) => HostResult::Returned(0),
        Err(e) => io_fault("writing", &path, e),
    }
}

fn set_file_size(state: &mut State) -> HostResult {
    let (path, mode) = tryr!(file(state, state.x[0]));
    if mode & OPEN_WRITE == 0 {
        return HostResult::Unavailable(format!("SetFileSize on {} opened without write access", path.display()));
    }
    match std::fs::OpenOptions::new().write(true).open(&path).and_then(|f| f.set_len(state.x[1])) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => io_fault("resizing", &path, e),
    }
}

fn close_file(state: &mut State) -> HostResult {
    state.host.files.remove(&state.x[0]);
    HostResult::Returned(0)
}

// ---- directories -----------------------------------------------------------

/// Directory listing in a stable order, filtered by the open mode
/// (1 directories, 2 files).
fn listing(state: &State, path: &Path, mode: u32) -> Result<Vec<(String, bool, u64)>, HostResult> {
    if let Some(rel) = in_nup(state, path) {
        let all = nup(state, |n| n.as_ref().and_then(|n| n.dirs.get(&rel).cloned())).unwrap_or_default();
        return Ok(all.into_iter().filter(|(_, d, _)| if *d { mode & 1 != 0 } else { mode & 2 != 0 }).collect());
    }
    let rd = std::fs::read_dir(path).map_err(|e| io_fault("listing", path, e))?;
    let mut out = Vec::new();
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        let is_dir = meta.is_dir();
        if (is_dir && mode & 1 == 0) || (!is_dir && mode & 2 == 0) {
            continue;
        }
        out.push((entry.file_name().to_string_lossy().into_owned(), is_dir, if is_dir { 0 } else { meta.len() }));
    }
    out.sort();
    Ok(out)
}

fn open_directory(state: &mut State) -> HostResult {
    let out = state.x[0];
    let guest = tryr!(guest_str(state, state.x[1]));
    let mode = state.x[2] as u32;
    let (path, _) = tryr!(resolve(state, &guest));
    if entry_kind(state, &path) != Some(0) {
        return HostResult::Returned(RESULT_PATH_NOT_FOUND);
    }
    let h = new_handle(state);
    state.host.dirs.insert(h, (path, mode, 0));
    tryr!(put_u64(state, out, h));
    HostResult::Returned(0)
}

fn dir(state: &State, h: u64) -> Result<(PathBuf, u32, u64), HostResult> {
    state
        .host
        .dirs
        .get(&h)
        .cloned()
        .ok_or_else(|| HostResult::Unavailable(format!("fs directory handle 0x{h:x} is not open")))
}

fn dir_entry_count(state: &mut State) -> HostResult {
    let (path, mode, _) = tryr!(dir(state, state.x[1]));
    let n = tryr!(listing(state, &path, mode)).len() as u64;
    tryr!(put_u64(state, state.x[0], n));
    HostResult::Returned(0)
}

/// `ReadDirectory(int64* out, DirectoryEntry* buf, DirectoryHandle, int64 n)`.
fn read_directory(state: &mut State) -> HostResult {
    let (out, buf, h, max) = (state.x[0], state.x[1], state.x[2], state.x[3]);
    let (path, mode, pos) = tryr!(dir(state, h));
    let all = tryr!(listing(state, &path, mode));
    let take: Vec<_> = all.iter().skip(pos as usize).take(max as usize).cloned().collect();
    for (i, (name, is_dir, size)) in take.iter().enumerate() {
        let mut entry = vec![0u8; DIR_ENTRY_BYTES as usize];
        let n = name.len().min(0x300);
        entry[..n].copy_from_slice(&name.as_bytes()[..n]);
        entry[0x304] = if *is_dir { 0 } else { 1 };
        entry[0x308..0x310].copy_from_slice(&size.to_le_bytes());
        tryr!(write_bytes(state, buf + i as u64 * DIR_ENTRY_BYTES, &entry));
    }
    if let Some(d) = state.host.dirs.get_mut(&h) {
        d.2 = pos + take.len() as u64;
    }
    tryr!(put_u64(state, out, take.len() as u64));
    HostResult::Returned(0)
}

fn close_directory(state: &mut State) -> HostResult {
    state.host.dirs.remove(&state.x[0]);
    HostResult::Returned(0)
}

pub fn register(s: &mut HostServices) {
    let table: [(&str, fn(&mut State) -> HostResult, &'static str); 27] = [
        ("_ZN2nn2fs12SetAllocatorEPFPvmEPFvS1_mE", set_allocator, "records the guest allocator; the host layer allocates nothing in guest memory"),
        ("_ZN2nn2fs22QueryMountRomCacheSizeEPm", query_rom_cache, "the host reads romfs through the OS cache; one page is requested"),
        ("_ZN2nn2fs8MountRomEPKcPvm", mount_rom, "mounts the configured romfs directory read-only"),
        ("_ZN2nn2fs7UnmountEPKc", unmount, "removes the mount"),
        ("_ZN2nn2fs26DisableGlobalFileDataCacheEv", ok, "a performance cache; the OS file cache serves the same role"),
        ("_ZN2nn2fs25EnableGlobalFileDataCacheEPvm", ok, "a performance cache; the OS file cache serves the same role"),
        ("_ZN2nn2fs12GetEntryTypeEPNS0_18DirectoryEntryTypeEPKc", get_entry_type, "directory or file from the host, ResultPathNotFound otherwise"),
        ("_ZN2nn2fs13MountSaveDataEPKcRKNS_7account3UidE", mount_save, "mounts the launcher's save directory, separate from the game files"),
        ("_ZN2nn2fs15CreateDirectoryEPKc", create_directory, "host mkdir on writable mounts with nn::fs result codes"),
        ("_ZN2nn2fs10CreateFileEPKcl", create_file, "creates a zero-filled file of the given size on writable mounts"),
        ("_ZN2nn2fs8OpenFileEPNS0_10FileHandleEPKci", open_file, "opens by mode; write access is refused on read-only mounts"),
        ("_ZN2nn2fs11SetFileSizeENS0_10FileHandleEl", set_file_size, "host set_len"),
        ("_ZN2nn2fs11GetFileSizeEPlNS0_10FileHandleE", get_file_size, "host file length"),
        ("_ZN2nn2fs8ReadFileEPmNS0_10FileHandleElPvm", read_file_counted, "reads up to size bytes and reports the count"),
        ("_ZN2nn2fs9WriteFileENS0_10FileHandleElPKvmRKNS0_11WriteOptionE", write_file, "writes at offset; growth needs AllowAppend; flush is implicit"),
        ("_ZN2nn2fs9CloseFileENS0_10FileHandleE", close_file, "releases the handle"),
        ("_ZN2nn2fs10DeleteFileEPKc", delete_file, "host remove on writable mounts"),
        ("_ZN2nn2fs6CommitEPKc", commit, "writes are already on disk, so a commit of an existing mount succeeds"),
        ("_ZN2nn2fs13OpenDirectoryEPNS0_15DirectoryHandleEPKci", open_directory, "lists the host directory in name order"),
        ("_ZN2nn2fs22GetDirectoryEntryCountEPlNS0_15DirectoryHandleE", dir_entry_count, "count filtered by the open mode"),
        ("_ZN2nn2fs13ReadDirectoryEPlPNS0_14DirectoryEntryENS0_15DirectoryHandleEl", read_directory, "fills nn::fs::DirectoryEntry records, 0x310 bytes each"),
        ("_ZN2nn2fs14CloseDirectoryENS0_15DirectoryHandleE", close_directory, "releases the handle"),
        ("_ZN2nn2fs8ReadFileENS0_10FileHandleElPvm", read_file, "reads exactly size bytes or returns ResultOutOfRange"),
        ("_ZN2nn2fs31QueryMountAddOnContentCacheSizeEPmi", add_on_content, "add-on content is not offered"),
        ("_ZN2nn2fs17MountAddOnContentEPKciPvm", add_on_content, "add-on content is not offered"),
        ("_ZN2nn2fs26SetPriorityOnCurrentThreadENS0_8PriorityE", ok, "I/O priority has no effect with one guest thread"),
        ("_ZN2nn2fs9FlushFileENS0_10FileHandleE", ok, "writes are not buffered by the host layer"),
    ];
    for (name, handler, claim) in table {
        s.register(name, handler, claim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ew-fs-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Data")).unwrap();
        std::fs::write(dir.join("Data").join("hello.txt"), b"hello romfs").unwrap();
        dir
    }

    fn put_str(s: &mut State, at: u64, text: &str) {
        s.mem[at as usize..at as usize + text.len()].copy_from_slice(text.as_bytes());
        s.mem[at as usize + text.len()] = 0;
    }

    #[test]
    fn reads_romfs_and_refuses_to_write_it() {
        let root = temp_root("rom");
        let mut s = State::default();
        s.host.romfs_root = Some(root.clone());
        put_str(&mut s, 0x100, "rom");
        s.x[0] = 0x100;
        assert_eq!(mount_rom(&mut s), HostResult::Returned(0));

        put_str(&mut s, 0x200, "rom:/Data/hello.txt");
        s.x[0] = 0x300;
        s.x[1] = 0x200;
        s.x[2] = OPEN_READ as u64;
        assert_eq!(open_file(&mut s), HostResult::Returned(0));
        let h = u64::from_le_bytes(s.mem[0x300..0x308].try_into().unwrap());

        s.x[0] = h;
        s.x[1] = 6;
        s.x[2] = 0x400;
        s.x[3] = 5;
        assert_eq!(read_file(&mut s), HostResult::Returned(0));
        assert_eq!(&s.mem[0x400..0x405], b"romfs");
        s.x[0] = h;
        s.x[1] = 6;
        s.x[2] = 0x400;
        s.x[3] = 50;
        assert_eq!(read_file(&mut s), HostResult::Returned(RESULT_OUT_OF_RANGE));

        s.x[0] = 0x300;
        s.x[1] = 0x200;
        s.x[2] = OPEN_WRITE as u64;
        assert!(matches!(open_file(&mut s), HostResult::Unavailable(_)), "rom is read-only");

        put_str(&mut s, 0x200, "rom:/Data/missing.bin");
        s.x[0] = 0x300;
        s.x[1] = 0x200;
        s.x[2] = OPEN_READ as u64;
        assert_eq!(open_file(&mut s), HostResult::Returned(RESULT_PATH_NOT_FOUND));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn paths_cannot_escape_their_mount() {
        let root = temp_root("escape");
        let mut s = State::default();
        s.host.mounts.insert("rom".into(), (root.clone(), false));
        assert!(resolve(&s, "rom:/../secret").is_err());
        assert!(resolve(&s, "rom:/C:/Windows").is_err());
        assert_eq!(resolve(&s, "rom:/Data//hello.txt").unwrap().0, root.join("Data").join("hello.txt"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn save_data_round_trips() {
        let root = temp_root("save");
        let mut s = State::default();
        s.host.save_root = Some(root.join("save"));
        put_str(&mut s, 0x100, "save");
        s.x[0] = 0x100;
        assert_eq!(mount_save(&mut s), HostResult::Returned(0));
        put_str(&mut s, 0x200, "save:/slot0.bin");
        s.x[0] = 0x200;
        s.x[1] = 4;
        assert_eq!(create_file(&mut s), HostResult::Returned(0));
        s.x[0] = 0x200;
        s.x[1] = 4;
        assert_eq!(create_file(&mut s), HostResult::Returned(RESULT_PATH_ALREADY_EXISTS));
        s.x[0] = 0x300;
        s.x[1] = 0x200;
        s.x[2] = (OPEN_READ | OPEN_WRITE) as u64;
        assert_eq!(open_file(&mut s), HostResult::Returned(0));
        let h = u64::from_le_bytes(s.mem[0x300..0x308].try_into().unwrap());
        s.mem[0x500..0x504].copy_from_slice(b"WEAV");
        s.x[0] = h;
        s.x[1] = 0;
        s.x[2] = 0x500;
        s.x[3] = 4;
        assert_eq!(write_file(&mut s), HostResult::Returned(0));
        assert_eq!(std::fs::read(root.join("save").join("slot0.bin")).unwrap(), b"WEAV");
        let _ = std::fs::remove_dir_all(root);
    }
}
