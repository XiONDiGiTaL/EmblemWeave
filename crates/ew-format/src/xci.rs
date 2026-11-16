//! `.xci` game-card images: a root HFS0 whose `secure` partition holds the
//! game's NCAs. Only plaintext dumps are readable here. A retail cartridge's
//! NCAs are encrypted with console keys, which this project never asks for;
//! `Nca::crypto_type` makes that refusal explicit instead of copying garbage.
//!
//! Layout, all little endian:
//!
//! * Card header: a 0x100-byte RSA signature, `"HEAD"` at 0x100, the root
//!   HFS0's absolute offset at 0x130. Some full dumps put the 0x1000-byte
//!   card-key area before the header, so the header itself is looked for at 0
//!   and at 0x1000.
//! * HFS0: `"HFS0"`, u32 entries, u32 string-table size, u32 zero; then one
//!   0x40-byte entry per file (`u64 offset`, `u64 size`, `u32 name offset`,
//!   `u32 hashed size`, 8 reserved bytes, 32-byte SHA-256), the string table,
//!   and the file data. Entry offsets count from the end of the header.
//! * The root partition names `update`, `normal`, `secure` and `logo`; each
//!   of those is another HFS0. The `secure` partition holds the NCAs.

use crate::nsp::{dump_package, nca_at, ContentKind, Dumped, Nca};
use anyhow::{bail, ensure, Context, Result};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const HEADER: u64 = 0x200;
const HFS_ENTRY: u64 = 0x40;

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

fn read_at(f: &mut File, at: u64, len: usize) -> Result<Vec<u8>> {
    let mut b = vec![0u8; len];
    f.seek(SeekFrom::Start(at))?;
    f.read_exact(&mut b).with_context(|| format!("reading {len} bytes at {at:#x}"))?;
    Ok(b)
}

/// An HFS0 header and its file table. Entry offsets are absolute file offsets.
#[derive(Debug, Clone)]
pub struct Hfs0 {
    pub base: u64,
    pub header_size: u64,
    pub entries: Vec<Entry>,
}

/// One file in an HFS0. `offset` is already relative to the file, not the
/// partition header, so the caller can read it without another adjustment.
#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

fn hfs0(f: &mut File, base: u64, limit: u64) -> Result<Hfs0> {
    ensure!(base + 0x10 <= limit, "an HFS0 header at {base:#x} is outside its partition");
    let head = read_at(f, base, 0x10)?;
    ensure!(&head[0..4] == b"HFS0", "no HFS0 header at {base:#x}");
    let count = u32_at(&head, 4) as u64;
    let strings = u32_at(&head, 8) as u64;
    ensure!(count < 1_000_000, "the HFS0 at {base:#x} declares {count} files");
    let table = base
        .checked_add(0x10)
        .and_then(|v| v.checked_add(count.checked_mul(HFS_ENTRY)?))
        .ok_or_else(|| anyhow::anyhow!("the HFS0 at {base:#x} has an overflowing table"))?;
    let data_base = table
        .checked_add(strings)
        .ok_or_else(|| anyhow::anyhow!("the HFS0 at {base:#x} has an overflowing string table"))?;
    ensure!(data_base <= limit, "the HFS0 at {base:#x} runs past its partition");

    let raw = read_at(f, base + 0x10, (count * HFS_ENTRY) as usize)?;
    let names = read_at(f, table, strings as usize)?;
    let mut entries = Vec::with_capacity(count as usize);
    for i in 0..count as usize {
        let e = &raw[i * HFS_ENTRY as usize..];
        let (off, size) = (u64_at(e, 0), u64_at(e, 8));
        let at = u32_at(e, 16) as usize;
        ensure!(at <= names.len(), "an HFS0 name at {base:#x} is outside its string table");
        let rest = &names[at..];
        let name = String::from_utf8_lossy(&rest[..rest.iter().position(|b| *b == 0).unwrap_or(rest.len())]).into_owned();
        let offset = data_base
            .checked_add(off)
            .ok_or_else(|| anyhow::anyhow!("the HFS0 file {name} has an overflowing offset"))?;
        ensure!(
            offset.checked_add(size).is_some_and(|end| end <= limit),
            "the HFS0 file {name} is outside its partition"
        );
        entries.push(Entry { name, offset, size });
    }
    Ok(Hfs0 { base, header_size: data_base - base, entries })
}

/// An opened `.xci` image.
#[derive(Debug)]
pub struct Xci {
    pub path: PathBuf,
    pub size: u64,
    /// The root HFS0, whose entries are the `update`, `normal`, `secure` and
    /// `logo` partitions.
    pub root: Hfs0,
    /// The `secure` partition, whose entries are the game's NCAs.
    pub secure: Hfs0,
}

impl Xci {
    /// Whether `path` has a card header where a dumped XCI keeps one. This is
    /// deliberately only the magic check: the picker uses it to offer the file,
    /// and `open` does the full validation.
    pub fn is_xci(path: &Path) -> bool {
        let Ok(mut f) = File::open(path) else { return false };
        let Ok(size) = f.metadata().map(|m| m.len()) else { return false };
        Self::header_at(&mut f, size).is_ok()
    }

    fn header_at(f: &mut File, size: u64) -> Result<u64> {
        for at in [0u64, 0x1000] {
            if at + 0x104 > size {
                continue;
            }
            let mut magic = [0u8; 4];
            f.seek(SeekFrom::Start(at + 0x100))?;
            f.read_exact(&mut magic)?;
            if &magic == b"HEAD" {
                return Ok(at);
            }
        }
        bail!("no XCI card header (`HEAD` at 0x100) was found")
    }

    pub fn open(path: &Path) -> Result<Xci> {
        let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let size = f.metadata()?.len();
        ensure!(size >= HEADER, "{} is too small to be a .xci", path.display());
        let header_at = Self::header_at(&mut f, size).with_context(|| format!("{} is not a readable .xci", path.display()))?;
        let head = read_at(&mut f, header_at, HEADER as usize)?;
        let root_at = header_at
            .checked_add(u64_at(&head, 0x130))
            .ok_or_else(|| anyhow::anyhow!("the .xci root partition offset overflows"))?;
        ensure!(root_at >= header_at && root_at < size, "the .xci root partition is outside the file");
        let root = hfs0(&mut f, root_at, size).with_context(|| format!("reading the root partition of {}", path.display()))?;
        let secure_entry = root
            .entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case("secure"))
            .ok_or_else(|| anyhow::anyhow!("{} has no `secure` partition", path.display()))?;
        ensure!(secure_entry.size >= 0x10, "the `secure` partition of {} is empty", path.display());
        let secure = hfs0(&mut f, secure_entry.offset, secure_entry.offset + secure_entry.size)
            .with_context(|| format!("reading the secure partition of {}", path.display()))?;
        ensure!(!secure.entries.is_empty(), "the `secure` partition of {} holds no files", path.display());
        Ok(Xci { path: path.to_path_buf(), size, root, secure })
    }

    /// Every NCA in the secure partition, with its header parsed.
    pub fn ncas(&self) -> Result<Vec<Nca>> {
        let mut f = File::open(&self.path)?;
        let mut out = Vec::new();
        for e in &self.secure.entries {
            if !e.name.to_ascii_lowercase().ends_with(".nca") || e.size < 0xc00 {
                continue;
            }
            // nca_at names the file in every message, so do not wrap it in a context that
            // would hide the crypto-type explanation behind "reading game.nca" in the UI.
            out.push(nca_at(&mut f, &e.name, e.offset, e.size)?);
        }
        Ok(out)
    }

    /// The NCA that carries the game program.
    pub fn program(&self) -> Result<Nca> {
        self.ncas()?.into_iter().find(|n| n.kind == ContentKind::Program).ok_or_else(|| {
            anyhow::anyhow!(
                "{} holds no program NCA; this is not a game card image (a DLC or update alone cannot be recompiled)",
                self.path.display()
            )
        })
    }

    /// The NCA that carries the control metadata, when the image has one.
    pub fn control(&self) -> Result<Option<Nca>> {
        Ok(self.ncas()?.into_iter().find(|n| n.kind == ContentKind::Control))
    }

    /// Copy the ExeFS, RomFS and control metadata into `dest`, exactly as
    /// `.nsp` does. A cancelled unpack leaves the caller to remove what it
    /// wrote.
    pub fn dump(&self, dest: &Path, progress: &mut dyn FnMut(u64, u64), cancelled: &dyn Fn() -> bool) -> Result<Dumped> {
        let program = self.program()?;
        let control = self.control()?;
        dump_package(&self.path, &program, control.as_ref(), dest, progress, cancelled)
    }

    /// A one-line description for the launcher's checks.
    pub fn summary(&self) -> Result<String> {
        let ncas = self.ncas()?;
        let kinds: Vec<String> = ncas.iter().map(|n| n.kind.name().to_string()).collect();
        Ok(format!(".xci, title {:016X}, {} NCAs ({})", ncas.first().map(|n| n.title_id).unwrap_or(0), ncas.len(), kinds.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ew-xci-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A PFS0 with the given files, as an NCA's ExeFS section stores them.
    fn build_pfs0(files: &[(&str, &[u8])]) -> Vec<u8> {
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

    /// A small plaintext NCA carrying one ExeFS section, so the game NCA in an
    /// XCI can be parsed and unpacked without a retail image.
    fn build_nca() -> Vec<u8> {
        let exefs = build_pfs0(&[("main", b"main-bytes"), ("main.npdm", b"npdm")]);
        let section_at = 0xc00u64;
        let data_at = 0x200u64;
        let section_end = (section_at + data_at + exefs.len() as u64 + 0x1ff) & !0x1ff;
        let mut nca = vec![0u8; section_end as usize];
        nca[0x200..0x204].copy_from_slice(b"NCA3");
        nca[0x205] = 0; // program content
        nca[0x206] = 0; // plaintext
        nca[0x210..0x218].copy_from_slice(&0x0400_0000_0000_0000u64.to_le_bytes());
        let len = nca.len() as u64;
        nca[0x208..0x210].copy_from_slice(&len.to_le_bytes());
        nca[0x240..0x244].copy_from_slice(&((section_at / 0x200) as u32).to_le_bytes());
        nca[0x244..0x248].copy_from_slice(&((section_end / 0x200) as u32).to_le_bytes());
        let fs = 0x400usize;
        nca[fs..fs + 2].copy_from_slice(&2u16.to_le_bytes());
        nca[fs + 0x40..fs + 0x48].copy_from_slice(&data_at.to_le_bytes());
        nca[fs + 0x48..fs + 0x50].copy_from_slice(&(exefs.len() as u64).to_le_bytes());
        let exefs_at = (section_at + data_at) as usize;
        nca[exefs_at..exefs_at + exefs.len()].copy_from_slice(&exefs);
        nca
    }

    /// Build an HFS0 from `(name, bytes)` pairs. Offsets are relative to the
    /// data area, as the real format says.
    fn build_hfs0(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut names = Vec::new();
        let mut offsets = Vec::new();
        for (name, _) in files {
            offsets.push(names.len() as u32);
            names.extend_from_slice(name.as_bytes());
            names.push(0);
        }
        let header_size = 0x10 + files.len() * HFS_ENTRY as usize + names.len();
        let mut out = b"HFS0".to_vec();
        out.extend_from_slice(&(files.len() as u32).to_le_bytes());
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        let mut data_off = 0u64;
        for (i, (_, data)) in files.iter().enumerate() {
            out.extend_from_slice(&data_off.to_le_bytes());
            out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(&offsets[i].to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&[0u8; 8]);
            out.extend_from_slice(&[0u8; 32]);
            data_off += data.len() as u64;
        }
        out.extend_from_slice(&names);
        for (_, data) in files {
            out.extend_from_slice(data);
        }
        assert_eq!(out.len(), header_size + files.iter().map(|(_, d)| d.len()).sum::<usize>());
        out
    }

    #[test]
    fn reads_hfs0_entries_and_offsets() {
        let dir = temp("hfs0");
        let path = dir.join("root.hfs0");
        let bytes = build_hfs0(&[("secure", b"abc"), ("logo", b"0123456789")]);
        std::fs::write(&path, &bytes).unwrap();
        let mut f = File::open(&path).unwrap();
        let h = hfs0(&mut f, 0, bytes.len() as u64).unwrap();
        assert_eq!(h.entries.len(), 2);
        assert_eq!(h.entries[0].name, "secure");
        assert_eq!(h.entries[0].size, 3);
        assert_eq!(h.entries[1].name, "logo");
        let at = h.entries[1].offset as usize;
        assert_eq!(&bytes[at..at + 10], b"0123456789");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reads_a_card_header_and_secure_partition() {
        let dir = temp("open");
        let secure = build_hfs0(&[("dummy.bin", b"x")]);
        let root = build_hfs0(&[("secure", &secure)]);
        let mut image = vec![0u8; HEADER as usize];
        image[0x100..0x104].copy_from_slice(b"HEAD");
        image[0x130..0x138].copy_from_slice(&(HEADER).to_le_bytes());
        image.extend_from_slice(&root);
        let path = dir.join("game.xci");
        std::fs::write(&path, &image).unwrap();
        assert!(Xci::is_xci(&path));
        let xci = Xci::open(&path).unwrap();
        assert_eq!(xci.root.entries.len(), 1);
        assert_eq!(xci.secure.entries.len(), 1);
        assert_eq!(xci.secure.entries[0].name, "dummy.bin");
        assert_eq!(xci.secure.base, xci.root.entries[0].offset);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_card_header_at_0x1000_applies_its_root_offset() {
        let dir = temp("header-1000");
        let nca = build_nca();
        let secure = build_hfs0(&[("game.nca", &nca)]);
        let root = build_hfs0(&[("secure", &secure)]);
        let card = 0x1000u64;
        let root_off = 0x400u64;
        let mut image = vec![0u8; (card + root_off) as usize];
        image[card as usize + 0x100..card as usize + 0x104].copy_from_slice(b"HEAD");
        image[card as usize + 0x130..card as usize + 0x138].copy_from_slice(&root_off.to_le_bytes());
        image.extend_from_slice(&root);
        let path = dir.join("full.xci");
        std::fs::write(&path, &image).unwrap();
        assert!(Xci::is_xci(&path));
        let xci = Xci::open(&path).unwrap();
        assert_eq!(xci.root.base, card + root_off, "the header's root offset counts from the 0x1000 header");
        assert_eq!(xci.root.entries.len(), 1);
        assert_eq!(xci.root.entries[0].name, "secure");
        assert_eq!(xci.secure.base, xci.root.entries[0].offset);
        assert_eq!(xci.secure.entries[0].name, "game.nca");
        let program = xci.program().unwrap();
        assert_eq!(program.crypto_type, 0);
        let out = dir.join("dump");
        let dumped = xci.dump(&out, &mut |_, _| {}, &|| false).unwrap();
        assert_eq!(dumped.exefs, 2);
        assert_eq!(std::fs::read_to_string(out.join("exefs").join("main")).unwrap(), "main-bytes");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_plain_file_is_not_a_card_image() {
        let dir = temp("plain");
        let path = dir.join("not.xci");
        std::fs::write(&path, b"not an xci").unwrap();
        assert!(!Xci::is_xci(&path));
        assert!(Xci::open(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_encrypted_nca_is_named_instead_of_unpacked() {
        let dir = temp("crypto");
        let path = dir.join("encrypted.nca");
        let mut nca = vec![0u8; 0xc00];
        nca[0x200..0x204].copy_from_slice(b"NCA3");
        nca[0x206] = 1; // crypto type
        let mut f = File::options().read(true).write(true).create(true).truncate(true).open(&path).unwrap();
        f.write_all(&nca).unwrap();
        let err = nca_at(&mut f, "encrypted.nca", 0, nca.len() as u64).unwrap_err().to_string();
        assert!(err.contains("encrypted"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On a real image: `EW_XCI=<game.xci> cargo test -p ew-format -- --ignored`.
    #[test]
    #[ignore]
    fn opens_a_real_xci() {
        let path = std::env::var("EW_XCI").expect("EW_XCI");
        let xci = Xci::open(Path::new(&path)).unwrap();
        eprintln!("{}", xci.summary().unwrap());
        let program = xci.program().unwrap();
        assert_eq!(program.kind, ContentKind::Program);
    }
}
