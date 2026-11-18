//! `.nsp` game containers: a PFS0 archive of plaintext NCAs
//! them for emulators. Nothing here is encrypted, signed or licensed: every
//! byte is read from the user's own file.
//!
//! Layout, little endian:
//!
//! * PFS0: `"PFS0"`, u32 entries, u32 string-table size, u32 zero; then one
//!   `{u64 offset, u64 size, u32 name offset, u32 zero}` per entry, the string
//!   table, and the file data, where offsets count from the end of the header.
//! * NCA: the header sits 0x200 bytes into the blob, so its magic is at
//!   `blob + 0x200`; the blob's first 0x200 bytes are reserved. In the header:
//!   `"NCA3"`, u8 distribution type, u8 content type (0 program, 2 control),
//!   u8 crypto type, u8 key index, u64 size at 0x208, u64 program id at 0x210;
//!   at 0x240 four `{u32 first sector, u32 end sector}` pairs, in 0x200-byte
//!   sectors; at 0x400 one 0x200-byte file-system header per section. The
//!   sections follow the 0xC00-byte header.
//! * ExeFS section: a 0x200-byte header (`{u16 2, u8 1, u8 2, u8 1}`, the
//!   SHA-256 of the block hashes at 8, and at 0x28 the block size, u32 2, u64
//!   zero, u64 hash size, u64 data offset, u64 data size), the block hashes,
//!   then a PFS0 at `section + 0x200 + data offset`.
//! * RomFS section: a 0x200-byte header holding `"IVFC"` at +8 and, at
//!   +0x18, six `{u64 offset, u64 size, u32 block, u32 zero}` level records;
//!   the last level is the RomFS itself: a 0x50-byte header of ten u64
//!   (header size, directory-hash offset and size, directory-table offset and
//!   size, file-hash offset and size, file-table offset and size, data
//!   offset), then entries whose offsets count from that header:
//!   * directories, 0x18 bytes plus the name: parent, next sibling, first
//!     child directory, first child file, next hash, name length;
//!   * files, 0x20 bytes plus the name: parent, next sibling, data offset
//!     (from the same header), size, next hash, name length.

use anyhow::{bail, ensure, Context, Result};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// The NCA magic sits this far into the NCA blob.
const NCA_MAGIC: u64 = 0x200;
/// The NCA header, including the reserved prefix, up to the first section.
const NCA_HEADER: u64 = 0xC00;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Program,
    Meta,
    Control,
    Manual,
    Data,
    PublicData,
    Other(u8),
}

impl ContentKind {
    fn of(byte: u8) -> ContentKind {
        match byte {
            0 => ContentKind::Program,
            1 => ContentKind::Meta,
            2 => ContentKind::Control,
            3 => ContentKind::Manual,
            4 => ContentKind::Data,
            5 => ContentKind::PublicData,
            other => ContentKind::Other(other),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ContentKind::Program => "program",
            ContentKind::Meta => "meta",
            ContentKind::Control => "control",
            ContentKind::Manual => "manual",
            ContentKind::Data => "data",
            ContentKind::PublicData => "public data",
            ContentKind::Other(_) => "unknown",
        }
    }
}

/// One file in a PFS0, with the offset of its bytes in the `.nsp`.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

/// A byte range of the `.nsp`: a file to copy out as it is.
#[derive(Debug, Clone)]
pub struct Extent {
    pub path: String,
    pub offset: u64,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct Section {
    /// Where the section's 0x200-byte header starts.
    pub offset: u64,
    pub size: u64,
    /// The file-system header in the NCA header that describes it.
    pub fs: [u8; 512],
}

impl Section {
    fn is_romfs(&self) -> bool {
        &self.fs[8..12] == b"IVFC"
    }
}

/// One plaintext NCA in the archive.
#[derive(Debug, Clone)]
pub struct Nca {
    pub name: String,
    /// Where the NCA blob starts in the `.nsp`.
    pub offset: u64,
    pub size: u64,
    pub title_id: u64,
    pub kind: ContentKind,
    /// 0 for plaintext content. Retail game-card NCAs use a non-zero value
    /// and cannot be read without the console's keys.
    pub crypto_type: u8,
    pub sections: Vec<Section>,
}

/// An opened `.nsp` archive.
#[derive(Debug)]
pub struct Nsp {
    pub path: PathBuf,
    pub size: u64,
    pub entries: Vec<Entry>,
}

/// The unpack stopped because the caller asked it to; nothing else went wrong.
#[derive(Debug)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the unpack was cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// What a dump wrote.
#[derive(Debug, Clone, Default)]
pub struct Dumped {
    pub exefs: usize,
    pub romfs: usize,
    pub romfs_bytes: u64,
    pub exefs_dir: PathBuf,
    pub romfs_dir: PathBuf,
    pub control_nacp: Option<PathBuf>,
}

fn u16_at(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes(b[o..o + 2].try_into().unwrap())
}
fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Read `len` bytes at `at` in `f`.
fn read_at(f: &mut File, at: u64, len: usize) -> Result<Vec<u8>> {
    let mut b = vec![0u8; len];
    f.seek(SeekFrom::Start(at))?;
    f.read_exact(&mut b).with_context(|| format!("reading {len} bytes at {at:#x}"))?;
    Ok(b)
}

/// Parse a PFS0 at `base`, with `limit` as the end of the region it may use.
fn pfs0(f: &mut File, base: u64, limit: u64) -> Result<Vec<Entry>> {
    let head = read_at(f, base, 16)?;
    if &head[0..4] != b"PFS0" && &head[0..4] != b"HFS0" {
        bail!("no PFS0 header at {base:#x}");
    }
    let count = u32_at(&head, 4) as u64;
    let strings = u32_at(&head, 8) as u64;
    ensure!(count < 1_000_000, "a PFS0 at {base:#x} declares {count} files");
    let table = 0x10 + count * 0x18;
    ensure!(base + table + strings <= limit, "a PFS0 at {base:#x} runs past its section");
    let body = read_at(f, base + table, strings as usize)?;
    let mut entries = Vec::with_capacity(count as usize);
    let names = read_at(f, base + 0x10, (count * 0x18) as usize)?;
    // Offsets count from the end of the header, names from the string table.
    let data = base + table + strings;
    for i in 0..count as usize {
        let e = &names[i * 0x18..];
        let (off, size) = (u64_at(e, 0), u64_at(e, 8));
        let at = u32_at(e, 16) as usize;
        ensure!(at <= body.len(), "a PFS0 name is outside its string table");
        let rest = &body[at..];
        let name = String::from_utf8_lossy(&rest[..rest.iter().position(|b| *b == 0).unwrap_or(rest.len())]).into_owned();
        ensure!(data + off + size <= limit, "a PFS0 file {name} is outside its section");
        entries.push(Entry { name, offset: data + off, size });
    }
    Ok(entries)
}

impl Nsp {
    /// Whether `path` starts with the PFS0 magic an NSP uses.
    pub fn is_nsp(path: &Path) -> bool {
        let mut m = [0u8; 4];
        File::open(path).and_then(|mut f| f.read_exact(&mut m)).is_ok() && &m == b"PFS0"
    }

    pub fn open(path: &Path) -> Result<Nsp> {
        let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let size = f.metadata()?.len();
        ensure!(size >= 0x10, "{} is too small to be a .nsp", path.display());
        let entries = pfs0(&mut f, 0, size).with_context(|| format!("{} is not a readable .nsp", path.display()))?;
        ensure!(!entries.is_empty(), "{} holds no content", path.display());
        Ok(Nsp { path: path.to_path_buf(), size, entries })
    }

    /// Every NCA in the archive, with its header parsed.
    pub fn ncas(&self) -> Result<Vec<Nca>> {
        let mut f = File::open(&self.path)?;
        let mut out = Vec::new();
        for e in &self.entries {
            if !e.name.ends_with(".nca") || e.size < NCA_HEADER {
                continue;
            }
            // nca_at names the file in every message, so do not wrap it in a context that
            // would hide the crypto-type explanation behind "reading game.nca" in the UI.
            out.push(nca(&mut f, e)?);
        }
        Ok(out)
    }

    /// The NCA that carries the game program.
    pub fn program(&self) -> Result<Nca> {
        let list = self.ncas()?;
        list.into_iter().find(|n| n.kind == ContentKind::Program).ok_or_else(|| {
            anyhow::anyhow!(
                "{} holds no program NCA; this is not a game package (a DLC or update alone cannot be recompiled)",
                self.path.display()
            )
        })
    }

    /// The NCA that carries the control metadata, when the package has one.
    pub fn control(&self) -> Result<Option<Nca>> {
        Ok(self.ncas()?.into_iter().find(|n| n.kind == ContentKind::Control))
    }

    /// Every data NCA, the content type add-on packs use. The caller narrows
    /// these to the base title's add-on range before writing them out.
    pub fn addons(&self) -> Result<Vec<Nca>> {
        Ok(self
            .ncas()?
            .into_iter()
            .filter(|n| matches!(n.kind, ContentKind::Data | ContentKind::PublicData))
            .collect())
    }

    /// Copy the ExeFS, the RomFS and the control metadata into `dest`, where the
    /// launcher reads them like an NSP extraction. `progress` is told how many
    /// bytes have been copied and how many there are to copy, about every
    /// 16 MiB and once more when the last byte lands. `cancelled` is asked
    /// between files; when it answers yes the unpack stops with [`Cancelled`]
    /// and leaves the caller to remove what it wrote.
    pub fn dump(&self, dest: &Path, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<Dumped> {
        let program = self.program()?;
        let control = self.control()?;
        dump_package(&self.path, &program, control.as_ref(), dest, progress, cancelled)
    }

    /// A one-line description of the archive for the launcher's checks.
    pub fn summary(&self) -> Result<String> {
        let ncas = self.ncas()?;
        let kinds: Vec<String> = ncas.iter().map(|n| n.kind.name().to_string()).collect();
        Ok(format!(".nsp, title {:016X}, {} NCAs ({})", ncas.first().map(|n| n.title_id).unwrap_or(0), ncas.len(), kinds.join(", ")))
    }
}

/// Copy a plaintext program NCA (and optional control NCA) into the folder the
/// launcher reads like an NSP extraction. Shared by `.nsp` and plaintext `.xci`,
/// which differ only in how their NCAs are found.
pub(crate) fn dump_package(
    source: &Path,
    program: &Nca,
    control: Option<&Nca>,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Result<Dumped> {
    let mut f = File::open(source)?;
    let exefs = program.exefs(&mut f)?;
    ensure!(!exefs.is_empty(), "{} holds no ExeFS", source.display());
    let romfs = program.romfs(&mut f)?;
    // control.nacp names the title and its display version: it sits in the
    // control NCA's RomFS, in the layout the launcher already reads.
    let nacp = match control {
        Some(control) => control.romfs(&mut f)?.into_iter().find(|e| e.path.eq_ignore_ascii_case("control.nacp")),
        None => None,
    };

    let exefs_dir = dest.join("exefs");
    let romfs_dir = dest.join("romfs");
    let nacp_to = dest.join("data").join("control").join("control.nacp");
    std::fs::create_dir_all(&exefs_dir)?;
    std::fs::create_dir_all(&romfs_dir)?;

    let total = exefs.iter().map(|e| e.size).sum::<u64>()
        + romfs.iter().map(|e| e.size).sum::<u64>()
        + nacp.as_ref().map(|e| e.size).unwrap_or(0);
    progress(0, total);
    let mut copied = 0u64;
    let mut reported = 0u64;
    let mut put = |f: &mut File, e: &Extent, to: &Path, copied: &mut u64, reported: &mut u64| -> Result<()> {
        if cancelled() {
            return Err(anyhow::Error::new(Cancelled));
        }
        safe_path(&e.path)?;
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        copy_range(f, e.offset, e.size, to)?;
        *copied += e.size;
        if *copied - *reported >= (16 << 20) || *copied == total {
            *reported = *copied;
            progress(*copied, total);
        }
        Ok(())
    };

    for e in &exefs {
        put(&mut f, e, &exefs_dir.join(e.path.replace('\\', "/")), &mut copied, &mut reported)?;
    }
    for e in &romfs {
        put(&mut f, e, &romfs_dir.join(e.path.replace('\\', "/")), &mut copied, &mut reported)?;
    }
    let mut control_nacp = None;
    if let Some(e) = &nacp {
        put(&mut f, e, &nacp_to, &mut copied, &mut reported)?;
        control_nacp = Some(nacp_to);
    }

    Ok(Dumped {
        exefs: exefs.len(),
        romfs: romfs.len(),
        romfs_bytes: romfs.iter().map(|e| e.size).sum(),
        exefs_dir,
        romfs_dir,
        control_nacp,
    })
}

pub(crate) fn nca_at(f: &mut File, name: &str, offset: u64, size: u64) -> Result<Nca> {
    ensure!(size >= NCA_HEADER, "{name} is shorter than an NCA header");
    let head = read_at(f, offset + NCA_MAGIC, (NCA_HEADER - NCA_MAGIC) as usize)?;
    ensure!(&head[0..3] == b"NCA", "{name} is not a plaintext NCA: an encrypted retail package cannot be unpacked");
    let crypto_type = head[6];
    ensure!(
        crypto_type == 0,
        "{name} is encrypted (NCA crypto type {crypto_type}): decrypt the dump first; this launcher never asks for keys"
    );
    let kind = ContentKind::of(head[5]);
    let title_id = u64_at(&head, 0x10);
    let declared = u64_at(&head, 8);
    ensure!(declared <= size, "{name} declares {declared} bytes but only {size} are in the package");
    let mut sections = Vec::new();
    for i in 0..4usize {
        let at = 0x40 + 16 * i;
        let (first, end) = (u32_at(&head, at) as u64 * 0x200, u32_at(&head, at + 4) as u64 * 0x200);
        if end <= first {
            continue;
        }
        ensure!(offset + end <= offset + size, "a section of {name} runs past its NCA");
        let mut fs = [0u8; 512];
        fs.copy_from_slice(&head[0x200 + 512 * i..0x400 + 512 * i]);
        sections.push(Section { offset: offset + first, size: end - first, fs });
    }
    Ok(Nca { name: name.to_string(), offset, size, title_id, kind, crypto_type, sections })
}

fn nca(f: &mut File, e: &Entry) -> Result<Nca> {
    nca_at(f, &e.name, e.offset, e.size)
}

impl Nca {
    /// Whether this NCA carries a RomFS section, which is what makes it an
    /// add-on pack rather than, say, a control or metadata NCA.
    pub fn has_romfs(&self) -> bool {
        self.sections.iter().any(|s| s.is_romfs())
    }

    /// Every ExeFS file, as a range to copy out.
    pub fn exefs(&self, f: &mut File) -> Result<Vec<Extent>> {
        let Some(section) = self.sections.iter().find(|s| !s.is_romfs() && u16_at(&s.fs, 0) == 2) else {
            return Ok(Vec::new());
        };
        // The file-system header in the NCA header says where the PFS0 starts;
        // the section itself holds the block hashes and then the PFS0.
        let data_off = u64_at(&section.fs, 0x40);
        let data_size = u64_at(&section.fs, 0x48);
        let base = section.offset + data_off;
        ensure!(base + data_size <= section.offset + section.size, "the ExeFS PFS0 is outside its section in {}", self.name);
        let entries = pfs0(f, base, base + data_size)?;
        Ok(entries.into_iter().map(|e| Extent { path: e.name, offset: e.offset, size: e.size }).collect())
    }

    /// Every RomFS file, as a range to copy out.
    pub fn romfs(&self, f: &mut File) -> Result<Vec<Extent>> {
        let Some(section) = self.sections.iter().find(|s| s.is_romfs()) else {
            return Ok(Vec::new());
        };
        // The last level of the integrity tree is the RomFS itself.
        let mut level = None;
        let levels = u32_at(&section.fs, 0x14).min(8);
        for i in 0..levels as usize {
            let at = 0x18 + 24 * i;
            if at + 24 > section.fs.len() {
                break;
            }
            let (off, size) = (u64_at(&section.fs, at), u64_at(&section.fs, at + 8));
            if size > 0 {
                level = Some((off, size));
            }
        }
        let Some((off, _)) = level else { return Ok(Vec::new()) };
        let level_off = off;
        let base = section.offset + off;
        ensure!(base + 0x50 <= section.offset + section.size, "the RomFS in {} is outside its section", self.name);

        let head = read_at(f, base, 0x50)?;
        let field = |i: usize| u64_at(&head, 8 * i);
        let (dir_meta_at, dir_meta_len) = (field(3), field(4));
        let (file_meta_at, file_meta_len) = (field(7), field(8));
        let data_at = field(9);
        ensure!(
            dir_meta_at + dir_meta_len <= data_at && file_meta_at + file_meta_len <= data_at,
            "the RomFS tables in {} are inconsistent",
            self.name
        );
        let dirs = read_at(f, base + dir_meta_at, dir_meta_len as usize)?;
        let files = read_at(f, base + file_meta_at, file_meta_len as usize)?;

        let name = |table: &[u8], at: usize, len: usize| -> Result<String> {
            ensure!(at + len <= table.len(), "a RomFS name is outside its table in {}", self.name);
            Ok(String::from_utf8_lossy(&table[at..at + len]).into_owned())
        };
        // Depth-first from the root, following the sibling chains the writer lays out.
        let mut out = Vec::new();
        let mut stack = vec![(0usize, String::new())];
        let mut seen = 0usize;
        while let Some((at, prefix)) = stack.pop() {
            seen += 1;
            ensure!(seen <= 4_000_000, "the RomFS in {} has a directory loop", self.name);
            ensure!(at + 0x18 <= dirs.len(), "a RomFS directory is outside its table in {}", self.name);
            let (first_dir, first_file) = (u32_at(&dirs, at + 8) as usize, u32_at(&dirs, at + 12) as usize);
            let mut d = first_dir;
            while d != 0xFFFF_FFFF {
                ensure!(d + 0x18 <= dirs.len(), "a RomFS directory is outside its table in {}", self.name);
                let name = name(&dirs, d + 0x18, u32_at(&dirs, d + 20) as usize)?;
                stack.push((d, format!("{prefix}{name}/")));
                d = u32_at(&dirs, d + 4) as usize;
            }
            let mut x = first_file;
            while x != 0xFFFF_FFFF {
                ensure!(x + 0x20 <= files.len(), "a RomFS file is outside its table in {}", self.name);
                let (off, size) = (u64_at(&files, x + 8), u64_at(&files, x + 16));
                let name = name(&files, x + 0x20, u32_at(&files, x + 28) as usize)?;
                ensure!(
                    level_off + data_at + off + size <= section.size,
                    "a RomFS file in {} runs past its section",
                    self.name
                );
                out.push(Extent { path: format!("{prefix}{name}"), offset: base + data_at + off, size });
                x = u32_at(&files, x + 4) as usize;
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(out)
    }
}

/// Copy one add-on NCA's RomFS into `dest`, preserving its paths. `dest` is the
/// pack's `romfs` folder; `progress` counts this NCA's bytes, and `cancelled` is
/// polled between files. Shared by `.nsp` and `.xci` so both containers produce
/// exactly the same managed DLC layout as a `.nup`.
pub fn dump_nca_romfs(
    source: &Path,
    nca: &Nca,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64),
    cancelled: &dyn Fn() -> bool,
) -> Result<(usize, u64)> {
    let mut f = File::open(source).with_context(|| format!("opening {}", source.display()))?;
    let files = nca.romfs(&mut f)?;
    let total = files.iter().map(|e| e.size).sum::<u64>();
    std::fs::create_dir_all(dest).with_context(|| format!("creating {}", dest.display()))?;
    progress(0, total);
    let mut done = 0u64;
    let mut reported = 0u64;
    for e in &files {
        if cancelled() {
            return Err(anyhow::Error::new(Cancelled));
        }
        safe_path(&e.path)?;
        let to = dest.join(e.path.replace('\\', "/"));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        copy_range(&mut f, e.offset, e.size, &to)?;
        done += e.size;
        if done - reported >= (16 << 20) || done == total {
            reported = done;
            progress(done, total);
        }
    }
    Ok((files.len(), done))
}

#[cfg(test)]
pub(crate) mod test_data {
    /// A plaintext NCA with one RomFS section holding `data` at `name`. It is
    /// deliberately small but follows the same IVFC/RomFS table layout the real
    /// parser reads, so `.nsp` and `.xci` add-on tests share it.
    pub(crate) fn nca_with_romfs(name: &str, data: &[u8], kind: u8, title: u64) -> Vec<u8> {
        let dir_meta_at = 0x50usize;
        let dir_meta_len = 0x18usize;
        let file_meta_at = dir_meta_at + dir_meta_len;
        let file_meta_len = 0x20 + name.len();
        let data_at = file_meta_at + file_meta_len;
        let level_off = 0x200usize;
        let romfs_len = data_at + data.len();
        let mut romfs = vec![0u8; romfs_len];
        for (field, value) in [
            (3, dir_meta_at as u64),
            (4, dir_meta_len as u64),
            (7, file_meta_at as u64),
            (8, file_meta_len as u64),
            (9, data_at as u64),
        ] {
            romfs[8 * field..8 * field + 8].copy_from_slice(&value.to_le_bytes());
        }
        // The root directory's only child is the file entry at offset 0.
        romfs[dir_meta_at + 8..dir_meta_at + 12].copy_from_slice(&u32::MAX.to_le_bytes());
        romfs[dir_meta_at + 12..dir_meta_at + 16].copy_from_slice(&0u32.to_le_bytes());
        romfs[file_meta_at + 4..file_meta_at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        romfs[file_meta_at + 8..file_meta_at + 16].copy_from_slice(&0u64.to_le_bytes());
        romfs[file_meta_at + 16..file_meta_at + 24]
            .copy_from_slice(&(data.len() as u64).to_le_bytes());
        romfs[file_meta_at + 28..file_meta_at + 32]
            .copy_from_slice(&(name.len() as u32).to_le_bytes());
        romfs[file_meta_at + 0x20..file_meta_at + 0x20 + name.len()]
            .copy_from_slice(name.as_bytes());
        romfs[data_at..].copy_from_slice(data);

        let section_at = 0xc00u64;
        let section_size = ((level_off + romfs_len + 0x1ff) & !0x1ff) as u64;
        let section_end = section_at + section_size;
        let mut nca = vec![0u8; section_end as usize];
        nca[0x200..0x204].copy_from_slice(b"NCA3");
        nca[0x205] = kind;
        nca[0x208..0x210].copy_from_slice(&section_end.to_le_bytes());
        nca[0x210..0x218].copy_from_slice(&title.to_le_bytes());
        nca[0x240..0x244].copy_from_slice(&((section_at / 0x200) as u32).to_le_bytes());
        nca[0x244..0x248].copy_from_slice(&((section_end / 0x200) as u32).to_le_bytes());
        let fs = 0x400usize;
        nca[fs + 8..fs + 12].copy_from_slice(b"IVFC");
        nca[fs + 0x14..fs + 0x18].copy_from_slice(&1u32.to_le_bytes());
        nca[fs + 0x18..fs + 0x20].copy_from_slice(&(level_off as u64).to_le_bytes());
        nca[fs + 0x20..fs + 0x28].copy_from_slice(&(romfs_len as u64).to_le_bytes());
        let at = (section_at + level_off as u64) as usize;
        nca[at..at + romfs.len()].copy_from_slice(&romfs);
        nca
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ew-nsp-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A PFS0 with the given files, as an `.nsp` stores them.
    fn pfs0_bytes(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut names = Vec::new();
        let mut offsets = Vec::new();
        for (name, _) in files {
            offsets.push(names.len() as u32);
            names.extend_from_slice(name.as_bytes());
            names.push(0);
        }
        let mut out = b"PFS0".to_vec();
        out.extend_from_slice(&(files.len() as u32).to_le_bytes());
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut data_off = 0u64;
        for (i, (_, data)) in files.iter().enumerate() {
            out.extend_from_slice(&data_off.to_le_bytes());
            out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(&offsets[i].to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            data_off += data.len() as u64;
        }
        out.extend_from_slice(&names);
        for (_, data) in files {
            out.extend_from_slice(data);
        }
        out
    }

    /// A small, structurally valid NCA with one ExeFS section. `crypto` is the
    /// NCA header's crypto type: 0 is plaintext, anything else is refused.
    fn nca_with_exefs(crypto: u8) -> Vec<u8> {
        let exefs = pfs0_bytes(&[("main", b"main-bytes"), ("main.npdm", b"npdm")]);
        let section_at = 0xc00u64;
        let data_at = 0x200u64; // the section's FS header and block hashes
        let section_end = (section_at + data_at + exefs.len() as u64 + 0x1ff) & !0x1ff;
        let mut nca = vec![0u8; section_end as usize];
        nca[0x200..0x204].copy_from_slice(b"NCA3");
        nca[0x205] = 0; // program content
        nca[0x206] = crypto;
        nca[0x210..0x218].copy_from_slice(&0x0400_0000_0000_0000u64.to_le_bytes());
        let len = nca.len() as u64;
        nca[0x208..0x210].copy_from_slice(&len.to_le_bytes());
        // Section 0: start / end in 0x200-byte sectors.
        nca[0x240..0x244].copy_from_slice(&((section_at / 0x200) as u32).to_le_bytes());
        nca[0x244..0x248].copy_from_slice(&((section_end / 0x200) as u32).to_le_bytes());
        // Its file-system header sits at blob + 0x400; the section itself is a
        // 0x200-byte header followed by the PFS0 at section + 0x200.
        let fs = 0x400usize;
        nca[fs..fs + 2].copy_from_slice(&2u16.to_le_bytes());
        nca[fs + 0x40..fs + 0x48].copy_from_slice(&data_at.to_le_bytes());
        nca[fs + 0x48..fs + 0x50].copy_from_slice(&(exefs.len() as u64).to_le_bytes());
        let exefs_at = (section_at + data_at) as usize;
        nca[exefs_at..exefs_at + exefs.len()].copy_from_slice(&exefs);
        nca
    }

    #[test]
    fn reads_a_pfs0_and_its_names() {
        let dir = temp("pfs0");
        let path = dir.join("a.pfs0");
        let mut names = Vec::new();
        for name in ["first.bin", "second.bin"] {
            names.extend_from_slice(name.as_bytes());
            names.push(0);
        }
        let mut head = b"PFS0".to_vec();
        head.extend_from_slice(&2u32.to_le_bytes());
        head.extend_from_slice(&(names.len() as u32).to_le_bytes());
        head.extend_from_slice(&0u32.to_le_bytes());
        for (i, name) in ["first.bin", "second.bin"].iter().enumerate() {
            let data = if i == 0 { b"abc".as_slice() } else { b"0123456789".as_slice() };
            let pad = (16 - data.len() % 16) % 16;
            let off: u64 = if i == 0 { 0 } else { 16 };
            head.extend_from_slice(&off.to_le_bytes());
            head.extend_from_slice(&(data.len() as u64).to_le_bytes());
            head.extend_from_slice(&if i == 0 { 0u32 } else { name.len() as u32 + 1 }.to_le_bytes());
            head.extend_from_slice(&0u32.to_le_bytes());
            let _ = pad;
        }
        head.extend_from_slice(&names);
        head.extend_from_slice(b"abc");
        head.extend_from_slice(&[0u8; 13]);
        head.extend_from_slice(b"0123456789");
        head.extend_from_slice(&[0u8; 6]);
        std::fs::File::create(&path).unwrap().write_all(&head).unwrap();

        let mut f = File::open(&path).unwrap();
        let size = head.len() as u64;
        let entries = pfs0(&mut f, 0, size).unwrap();
        assert_eq!(entries.len(), 2, "{entries:?}");
        assert_eq!(entries[0].name, "first.bin");
        assert_eq!(entries[1].size, 10);
        let at = entries[1].offset as usize;
        assert_eq!(&head[at..at + 10], b"0123456789");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plaintext_nca_with_crypto_type_zero_still_unpacks() {
        let dir = temp("plaintext");
        let nca = nca_with_exefs(0);
        let path = dir.join("plain.nsp");
        std::fs::write(&path, pfs0_bytes(&[("game.nca", &nca)])).unwrap();
        let nsp = Nsp::open(&path).unwrap();
        let program = nsp.program().unwrap();
        assert_eq!(program.crypto_type, 0);
        let out = dir.join("dump");
        let dumped = nsp.dump(&out, &mut |_, _| {}, &|| false).unwrap();
        assert_eq!(dumped.exefs, 2, "the program NCA's ExeFS should have two files");
        assert_eq!(std::fs::read_to_string(out.join("exefs").join("main")).unwrap(), "main-bytes");
        assert!(out.join("exefs").join("main.npdm").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_an_addon_nca_romfs() {
        let dir = temp("addon-romfs");
        let nca = test_data::nca_with_romfs("pack.bin", b"ADDON", 4, 0x0400_7EF0_1F8A_F001);
        let path = dir.join("addons.nsp");
        std::fs::write(&path, pfs0_bytes(&[("addon.nca", &nca)])).unwrap();
        let nsp = Nsp::open(&path).unwrap();
        let addons = nsp.addons().unwrap();
        assert_eq!(addons.len(), 1);
        assert_eq!(addons[0].kind, ContentKind::Data);
        assert!(addons[0].has_romfs());
        let out = dir.join("pack");
        let (files, bytes) = dump_nca_romfs(&path, &addons[0], &out, &mut |_, _| {}, &|| false).unwrap();
        assert_eq!((files, bytes), (1, 5));
        assert_eq!(std::fs::read(out.join("pack.bin")).unwrap(), b"ADDON");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_encrypted_nca_is_refused_with_the_crypto_type_named() {
        let dir = temp("encrypted");
        let nca = nca_with_exefs(1);
        let path = dir.join("retail.nsp");
        std::fs::write(&path, pfs0_bytes(&[("game.nca", &nca)])).unwrap();
        let err = Nsp::open(&path).unwrap().program().unwrap_err().to_string();
        assert!(err.contains("encrypted"), "{err}");
        assert!(err.contains("crypto type 1"), "{err}");
        assert!(err.contains("never asks for keys"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On a real package: `EW_NSP=<game.nsp> cargo test -p ew-format -- --ignored`.
    #[test]
    #[ignore]
    fn dumps_a_real_nsp() {
        let path = std::env::var("EW_NSP").expect("EW_NSP");
        let nsp = Nsp::open(Path::new(&path)).unwrap();
        eprintln!("{}", nsp.summary().unwrap());
        let program = nsp.program().unwrap();
        let mut f = File::open(&path).unwrap();
        let exefs = program.exefs(&mut f).unwrap();
        eprintln!("exefs: {:?}", exefs.iter().map(|e| (e.path.clone(), e.size)).collect::<Vec<_>>());
        assert!(exefs.iter().any(|e| e.path == "main"));
        assert!(exefs.iter().any(|e| e.path == "main.npdm"));
        let romfs = program.romfs(&mut f).unwrap();
        eprintln!("romfs: {} files, biggest {:?}", romfs.len(), romfs.iter().max_by_key(|e| e.size).map(|e| (e.path.clone(), e.size)));
        assert!(romfs.len() > 200_000, "only {} RomFS files", romfs.len());

        // Copy the ExeFS out and one RomFS file, which checks the extents
        // without writing the package's whole RomFS.
        let dir = temp("real");
        for e in &exefs {
            copy_range(&mut f, e.offset, e.size, &dir.join(&e.path)).unwrap();
        }
        let main = std::fs::metadata(dir.join("main")).unwrap().len();
        assert_eq!(main, exefs.iter().find(|e| e.path == "main").unwrap().size);
        let sample = romfs.iter().find(|e| e.path == "Fs.bin.zst").expect("Fs.bin.zst");
        let out = dir.join("Fs.bin.zst");
        copy_range(&mut f, sample.offset, sample.size, &out).unwrap();
        assert_eq!(std::fs::metadata(&out).unwrap().len(), sample.size);
        if let Ok(nup) = std::env::var("EW_NUP") {
            let nup = crate::nup::Nup::open(Path::new(&nup)).unwrap();
            let pkg = nup.program_package().cloned().unwrap();
            let theirs = nup.walk(pkg.exefs).into_iter().find(|(n, _)| n == "main").map(|(_, i)| nup.files[i as usize].size);
            assert_eq!(Some(main), theirs, "the ExeFS `main` differs from the .nup's");
            let count = nup.walk(pkg.romfs).len();
            eprintln!("nup romfs files: {count}");
            assert_eq!(romfs.len(), count);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// A path from inside the package must stay inside the unpack folder. Only
/// normal components are allowed: this also rejects a Windows drive prefix,
/// which `Path::join` would otherwise use to replace the destination.
fn safe_path(name: &str) -> Result<()> {
    // Check separators the same way the copy joins them, so a backslash path
    // cannot become `../` after the join on a non-Windows host.
    let name = name.replace('\\', "/");
    let path = Path::new(&name);
    ensure!(
        !name.is_empty() && path.components().all(|c| matches!(c, std::path::Component::Normal(_))),
        "{name} names an unsafe path"
    );
    Ok(())
}

/// Copy `size` bytes at `from` into a new file at `to`.
fn copy_range(f: &mut File, from: u64, size: u64, to: &Path) -> Result<()> {
    let mut out = OpenOptions::new().write(true).create(true).truncate(true).open(to).with_context(|| format!("creating {}", to.display()))?;
    f.seek(SeekFrom::Start(from))?;
    let mut left = size;
    let mut buf = vec![0u8; 4 << 20];
    while left > 0 {
        let want = left.min(buf.len() as u64) as usize;
        let n = f.read(&mut buf[..want])?;
        ensure!(n > 0, "the package ended after {} of {size} bytes", size - left);
        out.write_all(&buf[..n])?;
        left -= n as u64;
    }
    out.flush()?;
    Ok(())
}
