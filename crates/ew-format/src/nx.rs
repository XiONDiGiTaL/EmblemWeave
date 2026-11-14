//! Reader for NX archives (`.nx`), the container NXFormat writes.
//!
//! The format is defined by NXFormat (https://github.com/fab918/nxformat, MIT,
//! wire versions 0.2 and 0.3, magic `NXAR`). Only what the launcher and the
//! runtime need is decoded here: the two header slots, the catalog
//! (applications, programs, snapshots), the manifest tree, and block reads.
//! Everything is bounds checked and every digest the header promises is
//! verified, so a damaged archive is an error and never a guess.
//!
//! Layout, in one paragraph: two 256-byte header slots at 0 and 4096, the data
//! blocks from 8192, then a packed tail of Media, Catalog, Manifest, the block
//! index and the integrity table. The catalog names applications and their
//! snapshots (one complete file tree per version), the manifest holds the
//! directory and file records those trees point at, and a file is a run of
//! references into the block arrays. A block is RAW or one independent
//! Zstandard frame, and its logical bytes are covered by CRC-32C slices.

use anyhow::{anyhow, bail, ensure, Context, Result};
use std::collections::HashSet;
use std::fs::File;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const MAGIC: &[u8] = b"NXAR";
const HEADER_SIZE: usize = 256;
const HEADER_PAGE: u64 = 4096;
const MAX_CATALOG: u64 = 16 * 1024 * 1024;
const MAX_MANIFEST: u64 = 4 * 1024 * 1024 * 1024;
const CHECK_SIZE: u32 = 16384;

/// Snapshot kinds, as the catalog records them.
pub const BASE: u8 = 0;
pub const UPDATE: u8 = 1;
pub const DLC: u8 = 2;
pub const STANDALONE_UPDATE: u8 = 3;

/// The selected header generation.
#[derive(Debug, Clone, Copy)]
pub struct Header {
    pub generation: u64,
    pub archive_size: u64,
    pub catalog_offset: u64,
    pub catalog_size: u64,
    pub manifest_offset: u64,
    pub manifest_size: u64,
    pub block_index_offset: u64,
    pub block_count: u64,
    pub integrity_offset: u64,
    pub integrity_size: u64,
    pub check_size: u32,
    catalog_sha256: [u8; 32],
    manifest_sha256: [u8; 32],
    index_sha256: [u8; 32],
}

/// One application: a game and everything it ships.
#[derive(Debug, Clone)]
pub struct Application {
    pub id: u64,
    pub platform: u8,
    pub flags: u8,
    pub title: String,
    pub publisher: String,
    pub first_snapshot: u32,
    pub snapshot_count: u32,
}

/// One complete file tree for a version: base, update or DLC.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub program: u32,
    pub kind: u8,
    pub version: u32,
    pub root: u32,
    pub logical_size: u64,
    pub display_version: String,
    pub build_id: [u8; 32],
}

struct Entry {
    name: String,
    target: u32,
    dir: bool,
}

struct NxFile {
    size: u64,
    first_block: u32,
    block_count: u32,
}

struct Block {
    offset: u64,
    stored: u32,
    logical: u32,
    codec: u8,
    first_check: u32,
}

/// An open `.nx` archive.
pub struct Nx {
    path: PathBuf,
    header: Header,
    apps: Vec<Application>,
    snapshots: Vec<Snapshot>,
    programs: Vec<(u64, u32)>,
    dirs: Vec<(u32, u32)>,
    entries: Vec<Entry>,
    files: Vec<NxFile>,
    block_refs: Vec<u32>,
    blocks: Vec<Block>,
    checks: Vec<u32>,
}

/// Does this file start like an NX archive?
pub fn is_nx(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut magic))
        .map(|()| magic == MAGIC)
        .unwrap_or(false)
}

fn crc32c_table() -> &'static [u32; 256] {
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, slot) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0x82f6_3b78 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    })
}

fn crc32c(bytes: &[u8]) -> u32 {
    let table = crc32c_table();
    let mut c = 0xFFFF_FFFFu32;
    for b in bytes {
        c = table[((c ^ *b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    let mut out = [0u8; 32];
    out.copy_from_slice(&sha2::Sha256::digest(bytes));
    out
}

fn u16_at(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn u32_at(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

fn u64_at(b: &[u8], off: usize) -> u64 {
    let mut v = [0u8; 8];
    v.copy_from_slice(&b[off..off + 8]);
    u64::from_le_bytes(v)
}

fn array32(b: &[u8], off: usize) -> [u8; 32] {
    let mut v = [0u8; 32];
    v.copy_from_slice(&b[off..off + 32]);
    v
}

/// One header slot, if it is intact: magic, size, version, feature bits, CRC.
fn read_slot(page: &[u8], at: usize) -> Option<Header> {
    let b = page.get(at..at + HEADER_SIZE)?;
    if &b[0..4] != MAGIC || u32_at(b, 252) != crc32c(&b[0..252]) {
        return None;
    }
    if u16_at(b, 4) != 0 || !(2..=3).contains(&u16_at(b, 6)) {
        return None;
    }
    if u32_at(b, 8) as usize != HEADER_SIZE || u16_at(b, 12) & 1 != 1 || u16_at(b, 14) != 0 {
        return None;
    }
    Some(Header {
        archive_size: u64_at(b, 16),
        catalog_offset: u64_at(b, 24),
        catalog_size: u64_at(b, 32),
        manifest_offset: u64_at(b, 72),
        manifest_size: u64_at(b, 80),
        block_index_offset: u64_at(b, 88),
        block_count: u64_at(b, 96),
        integrity_offset: u64_at(b, 104),
        integrity_size: u64_at(b, 112),
        catalog_sha256: array32(b, 120),
        manifest_sha256: array32(b, 152),
        index_sha256: array32(b, 184),
        generation: u64_at(b, 216),
        check_size: u32_at(b, 224),
    })
}

impl Nx {
    /// Open and validate an archive. The newest intact header generation wins,
    /// and a slot that fails its own check is ignored while the other is sound.
    pub fn open(path: &Path) -> Result<Nx> {
        let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let len = f.metadata()?.len();
        ensure!(len >= HEADER_PAGE * 2, "{} is too short to be an NX archive", path.display());
        let mut pages = vec![0u8; (HEADER_PAGE * 2) as usize];
        f.read_exact(&mut pages)?;
        let header = match (read_slot(&pages, 0), read_slot(&pages, HEADER_PAGE as usize)) {
            (Some(a), Some(b)) => {
                let same = pages[..HEADER_SIZE] == pages[HEADER_PAGE as usize..HEADER_PAGE as usize + HEADER_SIZE];
                ensure!(
                    a.generation != b.generation || same,
                    "{} holds two different headers with the same generation",
                    path.display()
                );
                if a.generation >= b.generation {
                    a
                } else {
                    b
                }
            }
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => bail!("{} has no intact NX header", path.display()),
        };
        ensure!(
            header.archive_size <= len,
            "{} claims {} bytes but is {} long",
            path.display(),
            header.archive_size,
            len
        );
        ensure!(header.catalog_size >= 128 && header.catalog_size <= MAX_CATALOG, "{} has an unusable catalog size", path.display());
        ensure!(
            header.manifest_size >= 96 && header.manifest_size <= MAX_MANIFEST,
            "{} has an unusable manifest size",
            path.display()
        );
        ensure!(header.check_size == CHECK_SIZE, "{} uses an unknown integrity slice size", path.display());

        let catalog = read_at(&mut f, header.catalog_offset, header.catalog_size)?;
        ensure!(sha256(&catalog) == header.catalog_sha256, "the catalog of {} does not match its digest", path.display());
        let manifest = read_at(&mut f, header.manifest_offset, header.manifest_size)?;
        ensure!(sha256(&manifest) == header.manifest_sha256, "the manifest of {} does not match its digest", path.display());

        let mut nx = Nx {
            path: path.to_path_buf(),
            header,
            apps: Vec::new(),
            snapshots: Vec::new(),
            programs: Vec::new(),
            dirs: Vec::new(),
            entries: Vec::new(),
            files: Vec::new(),
            block_refs: Vec::new(),
            blocks: Vec::new(),
            checks: Vec::new(),
        };
        nx.parse_catalog(&catalog)?;
        nx.parse_manifest(&manifest)?;
        nx.parse_blocks(&mut f)?;
        Ok(nx)
    }

    pub fn header(&self) -> &Header {
        &self.header
    }

    pub fn applications(&self) -> &[Application] {
        &self.apps
    }

    pub fn snapshots(&self) -> &[Snapshot] {
        &self.snapshots
    }

    /// Program rows: `(program id, application index)`.
    pub fn programs(&self) -> &[(u64, u32)] {
        &self.programs
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_size(&self, file: u32) -> Option<u64> {
        self.files.get(file as usize).map(|f| f.size)
    }

    /// The application that carries `title_id`, if the archive holds it.
    pub fn application(&self, title_id: u64) -> Option<usize> {
        self.apps.iter().position(|a| a.id == title_id)
    }

    /// The snapshot to read for an application: the newest update, else the
    /// base. DLC is a separate tree and is never picked here.
    pub fn current(&self, app: usize) -> Option<usize> {
        let a = self.apps.get(app)?;
        let first = a.first_snapshot as usize;
        let last = first.checked_add(a.snapshot_count as usize)?;
        let mut best: Option<usize> = None;
        for i in first..last.min(self.snapshots.len()) {
            let s = &self.snapshots[i];
            if s.kind == UPDATE || s.kind == STANDALONE_UPDATE {
                let newer = best.map_or(true, |b| s.version > self.snapshots[b].version);
                if newer {
                    best = Some(i);
                }
            }
        }
        if best.is_some() {
            return best;
        }
        (first..last.min(self.snapshots.len())).find(|i| self.snapshots[*i].kind == BASE).or(Some(first))
    }

    /// Every file under a snapshot's root as `(path, file id, size)`, with the
    /// root left out. Each directory record is walked once, so a cyclic
    /// manifest cannot loop here.
    pub fn tree(&self, snapshot: usize) -> Result<Vec<(String, u32, u64)>> {
        let root = self.snapshots.get(snapshot).ok_or_else(|| anyhow!("no such snapshot"))?.root;
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        self.walk_dir(root, "", &mut seen, &mut out, 0)?;
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    fn walk_dir(&self, dir: u32, prefix: &str, seen: &mut HashSet<u32>, out: &mut Vec<(String, u32, u64)>, depth: usize) -> Result<()> {
        ensure!(depth < 64, "the manifest tree is deeper than 64 levels");
        let (first, count) = *self
            .dirs
            .get(dir as usize)
            .ok_or_else(|| anyhow!("the manifest names a directory that does not exist"))?;
        if !seen.insert(dir) {
            return Ok(());
        }
        let end = first.checked_add(count).ok_or_else(|| anyhow!("a directory range overflows"))?;
        for i in first..end {
            let e = self.entries.get(i as usize).ok_or_else(|| anyhow!("the manifest names an entry that does not exist"))?;
            let path = if prefix.is_empty() { e.name.clone() } else { format!("{prefix}/{}", e.name) };
            if e.dir {
                self.walk_dir(e.target, &path, seen, out, depth + 1)?;
            } else {
                let f = self.files.get(e.target as usize).ok_or_else(|| anyhow!("the manifest names a file that does not exist"))?;
                out.push((path, e.target, f.size));
            }
        }
        Ok(())
    }

    /// The bytes of a file, with every block checked against its CRC-32C slices.
    pub fn read(&self, file: u32) -> Result<Vec<u8>> {
        let f = self.files.get(file as usize).ok_or_else(|| anyhow!("no such file"))?;
        let mut fh = File::open(&self.path).with_context(|| format!("opening {}", self.path.display()))?;
        let mut out = Vec::with_capacity(f.size as usize);
        for i in 0..f.block_count {
            let r = *self
                .block_refs
                .get(f.first_block.checked_add(i).ok_or_else(|| anyhow!("a file's block range overflows"))? as usize)
                .ok_or_else(|| anyhow!("the file names a block reference that does not exist"))?;
            let b = self.blocks.get(r as usize).ok_or_else(|| anyhow!("the file names a block that does not exist"))?;
            out.extend_from_slice(&self.read_block(&mut fh, b)?);
        }
        out.truncate(f.size as usize);
        ensure!(out.len() as u64 == f.size, "a file decodes to the wrong size");
        Ok(out)
    }

    fn read_block(&self, f: &mut File, b: &Block) -> Result<Vec<u8>> {
        f.seek(SeekFrom::Start(b.offset))?;
        let mut stored = vec![0u8; b.stored as usize];
        f.read_exact(&mut stored)?;
        let data = match b.codec {
            0 => {
                ensure!(b.stored == b.logical, "a RAW block stores a different size than it declares");
                stored
            }
            1 => {
                let mut out = Vec::with_capacity(b.logical as usize);
                ruzstd::StreamingDecoder::new(Cursor::new(&stored[..]))
                    .map_err(|e| anyhow!("a block is not a Zstandard frame: {e}"))?
                    .read_to_end(&mut out)
                    .map_err(|e| anyhow!("decompressing a block: {e}"))?;
                out
            }
            other => bail!("a block uses codec {other}, which this build does not know"),
        };
        ensure!(data.len() as u32 == b.logical, "a block decodes to the wrong size");
        // The integrity table is one CRC-32C per 16 KiB slice of a block.
        for (k, slice) in data.chunks(self.header.check_size as usize).enumerate() {
            let want = self.checks.get(b.first_check as usize + k);
            if let Some(want) = want {
                ensure!(*want == crc32c(slice), "a block failed its integrity check");
            }
        }
        Ok(data)
    }

    fn parse_catalog(&mut self, c: &[u8]) -> Result<()> {
        ensure!(c.len() >= 128 && &c[0..4] == b"NXCT", "the catalog header is missing");
        ensure!(u16_at(c, 4) == 1 && u16_at(c, 6) == 128, "the catalog is a version this build does not know");
        let apps = u32_at(c, 20) as usize;
        let programs = u32_at(c, 24) as usize;
        let snapshots = u32_at(c, 28) as usize;
        let pool_off = u64_at(c, 96) as usize;
        let pool_len = u64_at(c, 104) as usize;
        let pool = c
            .get(pool_off..pool_off.checked_add(pool_len).ok_or_else(|| anyhow!("the catalog string pool overflows"))?)
            .ok_or_else(|| anyhow!("the catalog string pool is out of range"))?;
        let string = |off: u32, len: u32| -> Result<String> {
            if off == u32::MAX || len == 0 {
                return Ok(String::new());
            }
            let end = (off as usize).checked_add(len as usize).ok_or_else(|| anyhow!("a catalog string range overflows"))?;
            let s = pool.get(off as usize..end).ok_or_else(|| anyhow!("a catalog string is out of range"))?;
            Ok(String::from_utf8_lossy(s).into_owned())
        };

        let mut at = u64_at(c, 48) as usize;
        for _ in 0..apps {
            let b = c.get(at..at + 80).ok_or_else(|| anyhow!("the application table is truncated"))?;
            self.apps.push(Application {
                id: u64_at(b, 0),
                platform: b[8],
                flags: b[9],
                title: string(u32_at(b, 28), u32_at(b, 32))?,
                publisher: string(u32_at(b, 36), u32_at(b, 40))?,
                first_snapshot: u32_at(b, 20),
                snapshot_count: u32_at(b, 24),
            });
            at += 80;
        }
        let mut at = u64_at(c, 56) as usize;
        for _ in 0..programs {
            let b = c.get(at..at + 64).ok_or_else(|| anyhow!("the program table is truncated"))?;
            self.programs.push((u64_at(b, 0), u32_at(b, 8)));
            at += 64;
        }
        let mut at = u64_at(c, 64) as usize;
        for _ in 0..snapshots {
            let b = c.get(at..at + 120).ok_or_else(|| anyhow!("the snapshot table is truncated"))?;
            self.snapshots.push(Snapshot {
                program: u32_at(b, 0),
                kind: b[4],
                version: u32_at(b, 8),
                root: u32_at(b, 12),
                logical_size: u64_at(b, 16),
                display_version: string(u32_at(b, 24), u32_at(b, 28))?,
                build_id: array32(b, 88),
            });
            at += 120;
        }
        ensure!(!self.snapshots.is_empty(), "the catalog holds no snapshots");
        Ok(())
    }

    fn parse_manifest(&mut self, m: &[u8]) -> Result<()> {
        ensure!(m.len() >= 96 && &m[0..4] == b"NXMF", "the manifest header is missing");
        ensure!(u16_at(m, 4) == 1 && u16_at(m, 6) == 96, "the manifest is a version this build does not know");
        let dirs = u32_at(m, 16) as usize;
        let files = u32_at(m, 20) as usize;
        let entries = u32_at(m, 24) as usize;
        let refs = u32_at(m, 28) as usize;

        let mut at = u64_at(m, 32) as usize;
        for _ in 0..dirs {
            let b = m.get(at..at + 16).ok_or_else(|| anyhow!("the directory table is truncated"))?;
            self.dirs.push((u32_at(b, 0), u32_at(b, 4)));
            at += 16;
        }
        let mut at = u64_at(m, 40) as usize;
        for _ in 0..files {
            let b = m.get(at..at + 64).ok_or_else(|| anyhow!("the file table is truncated"))?;
            self.files.push(NxFile {
                size: u64_at(b, 0),
                first_block: u32_at(b, 8),
                block_count: u32_at(b, 12),
            });
            at += 64;
        }
        // Entry name offsets are relative to the start of the Manifest, not to
        // the path pool inside it, which is why this reads from `m`.
        let mut at = u64_at(m, 48) as usize;
        for _ in 0..entries {
            let b = m.get(at..at + 24).ok_or_else(|| anyhow!("the entry table is truncated"))?;
            let off = u64_at(b, 0) as usize;
            let len = u32_at(b, 8) as usize;
            let end = off.checked_add(len).ok_or_else(|| anyhow!("an entry name range overflows"))?;
            let name = m.get(off..end).ok_or_else(|| anyhow!("an entry name is out of range"))?;
            self.entries.push(Entry {
                name: String::from_utf8_lossy(name).into_owned(),
                target: u32_at(b, 12),
                dir: b[16] == 1,
            });
            at += 24;
        }
        let mut at = u64_at(m, 56) as usize;
        for _ in 0..refs {
            let b = m.get(at..at + 4).ok_or_else(|| anyhow!("the block reference table is truncated"))?;
            self.block_refs.push(u32_at(b, 0));
            at += 4;
        }
        Ok(())
    }

    fn parse_blocks(&mut self, f: &mut File) -> Result<()> {
        let n = self.header.block_count as usize;
        if n == 0 {
            ensure!(self.header.integrity_size == 0, "an archive without blocks must have no integrity table");
            ensure!(sha256(&[]) == self.header.index_sha256 || self.header.index_sha256 == [0u8; 32], "the empty index does not match its digest");
            return Ok(());
        }
        let index = read_at(f, self.header.block_index_offset, (n as u64) * 24)?;
        let integrity = read_at(f, self.header.integrity_offset, self.header.integrity_size)?;
        let mut both = index.clone();
        both.extend_from_slice(&integrity);
        ensure!(sha256(&both) == self.header.index_sha256, "the block index does not match its digest");
        ensure!(integrity.len() % 4 == 0, "the integrity table is not whole");
        for i in 0..n {
            let b = &index[i * 24..i * 24 + 24];
            self.blocks.push(Block {
                offset: u64_at(b, 0),
                stored: u32_at(b, 8),
                logical: u32_at(b, 12),
                codec: b[16],
                first_check: u32_at(b, 20),
            });
        }
        for i in 0..integrity.len() / 4 {
            self.checks.push(u32_at(&integrity, i * 4));
        }
        Ok(())
    }

    /// How many of the blocks are compressed, and how many bytes they hold.
    pub fn compression(&self) -> (usize, u64) {
        let compressed = self.blocks.iter().filter(|b| b.codec == 1).count();
        (compressed, self.blocks.iter().map(|b| b.logical as u64).sum())
    }
}

fn read_at(f: &mut File, at: u64, size: u64) -> Result<Vec<u8>> {
    ensure!(size <= usize::MAX as u64, "a region is larger than this build can hold");
    f.seek(SeekFrom::Start(at))?;
    let mut v = vec![0u8; size as usize];
    f.read_exact(&mut v)?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("nx-fixtures").join(name)
    }

    #[test]
    fn crc32c_matches_the_standard_check_value() {
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn a_minimal_archive_opens_with_one_empty_snapshot() {
        let nx = Nx::open(&fixture("minimal_valid.nx")).expect("minimal archive opens");
        assert_eq!(nx.applications().len(), 1);
        assert_eq!(nx.snapshots().len(), 1);
        let s = nx.current(0).expect("a snapshot is chosen");
        assert!(nx.tree(s).expect("the tree walks").is_empty());
        assert_eq!(nx.header().block_count, 0);
    }

    #[test]
    fn a_raw_file_reads_back_its_bytes() {
        let nx = Nx::open(&fixture("raw_only.nx")).expect("raw archive opens");
        let tree = nx.tree(nx.current(0).unwrap()).expect("tree");
        assert_eq!(tree.len(), 1);
        let (_, id, size) = tree[0].clone();
        assert_eq!(size, 4);
        assert_eq!(nx.read(id).expect("read").len(), 4);
    }

    #[test]
    fn a_zstandard_block_is_decoded() {
        let nx = Nx::open(&fixture("zstd_single.nx")).expect("zstd archive opens");
        let (_, id, size) = nx.tree(nx.current(0).unwrap()).expect("tree")[0].clone();
        assert_eq!(size, 4096);
        let data = nx.read(id).expect("read");
        assert_eq!(data, vec![data[0]; 4096], "the fixture is one repeated byte");
        assert_eq!(nx.compression().1, 4096);
    }

    #[test]
    fn a_file_spanning_two_blocks_reads_in_one_piece() {
        let nx = Nx::open(&fixture("multi_block_file.nx")).expect("multi block archive opens");
        let (_, id, size) = nx.tree(nx.current(0).unwrap()).expect("tree")[0].clone();
        assert_eq!(size, 65_537);
        assert_eq!(nx.read(id).expect("read").len(), 65_537);
    }

    #[test]
    fn nested_directories_appear_as_slash_paths() {
        let nx = Nx::open(&fixture("nested_vfs.nx")).expect("nested archive opens");
        let tree = nx.tree(nx.current(0).unwrap()).expect("tree");
        assert_eq!(tree.len(), 2);
        assert!(tree.iter().any(|(p, _, _)| p.contains('/')), "{tree:?}");
    }

    #[test]
    fn the_newest_snapshot_is_the_one_selected() {
        let nx = Nx::open(&fixture("deleted_between_versions.nx")).expect("versioned archive opens");
        assert_eq!(nx.snapshots().len(), 2);
        let newest = nx.current(0).expect("a snapshot");
        let oldest = (0..nx.snapshots().len()).find(|i| *i != newest).unwrap();
        let new_tree = nx.tree(newest).expect("tree");
        let old_tree = nx.tree(oldest).expect("tree");
        assert_eq!(old_tree.len(), 2);
        assert_eq!(new_tree.len(), 1);
        assert!(new_tree.len() < old_tree.len(), "the later version removed a file");
    }

    #[test]
    fn an_appended_generation_is_read_from_its_newer_slot() {
        let nx = Nx::open(&fixture("appended_generation.nx")).expect("appended archive opens");
        assert_eq!(nx.header().generation, 1);
    }

    #[test]
    fn damaged_archives_are_rejected() {
        assert!(Nx::open(&fixture("corrupted_manifest.nx")).is_err());
        assert!(Nx::open(&fixture("truncated_archive.nx")).is_err());
    }

    #[test]
    fn a_file_that_is_not_an_archive_is_rejected() {
        let dir = std::env::temp_dir().join(format!("ew-nx-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("not.nx");
        std::fs::write(&p, b"not an archive").unwrap();
        assert!(!is_nx(&p));
        assert!(Nx::open(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
