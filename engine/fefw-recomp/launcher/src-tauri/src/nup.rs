// SPDX-License-Identifier: GPL-3.0-or-later
//! Reads a `.nup` package (https://github.com/a55uka/nup, format 1) and unpacks it into the folder layout the rest
//! of the launcher expects: `<out>/base/{exefs,romfs}`, `<out>/updates/<version>/{exefs,romfs}` and, for the add-on content (DLC)
//! in the package, `<out>/dlc/<title id>/romfs`. Only the exefs and romfs subtrees of each package are written.

use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const NO_DIR: u32 = 0xFFFF_FFFF;
const KIND_BASE: u8 = 0;
const KIND_UPDATE: u8 = 1;
const KIND_ADDON: u8 = 2;
const MARKER: &str = ".nup_done";

struct Package {
    title_id: u64,
    root_dir: u32,
    exefs_dir: u32,
    romfs_dir: u32,
    version: u32,
    kind: u8,
}

struct Dir {
    name: String,
    first_child: u32,
    child_count: u32,
}

struct FileNode {
    name: String,
    size: u64,
    extent_offset: u64,
    extent_size: u64,
    codec: u8,
}

struct Nup {
    uuid: [u8; 16],
    file_size: u64,
    packages: Vec<Package>,
    dirs: Vec<Dir>,
    files: Vec<FileNode>,
    children: Vec<u32>,
}

fn u32_at(b: &[u8], o: usize) -> Result<u32, String> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| "The .nup file is truncated.".to_string())
}
fn u64_at(b: &[u8], o: usize) -> Result<u64, String> {
    b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap())).ok_or_else(|| "The .nup file is truncated.".to_string())
}

fn parse(f: &mut File) -> Result<Nup, String> {
    let real_size = f.metadata().map_err(|e| e.to_string())?.len();
    let mut sb = [0u8; 0x80];
    f.read_exact(&mut sb).map_err(|_| "This file is too small to be a .nup package.".to_string())?;
    if &sb[0..4] != b"NUP\x01" {
        return Err("This is not a .nup package (wrong header).".into());
    }
    let toc_offset = u64_at(&sb, 0x08)?;
    let toc_size = u64_at(&sb, 0x10)?;
    let file_size = u64_at(&sb, 0x40)?;
    if u32_at(&sb, 0x4C)? != 0 {
        return Err("This .nup uses features this launcher does not know.".into());
    }
    if file_size != real_size || toc_offset.checked_add(toc_size).map_or(true, |e| e > real_size) || toc_size > (1 << 30) {
        return Err("The .nup file is damaged or incomplete (size mismatch).".into());
    }
    let mut uuid = [0u8; 16];
    uuid.copy_from_slice(&sb[0x50..0x60]);

    let mut toc = vec![0u8; toc_size as usize];
    f.seek(SeekFrom::Start(toc_offset)).map_err(|e| e.to_string())?;
    f.read_exact(&mut toc).map_err(|e| e.to_string())?;
    if toc.get(0..4) != Some(b"TOC\x01") {
        return Err("The .nup table of contents is invalid.".into());
    }
    let pkg_count = u32_at(&toc, 4)? as usize;
    let dir_count = u64_at(&toc, 8)? as usize;
    let file_count = u64_at(&toc, 0x10)? as usize;
    let pool_off = u64_at(&toc, 0x18)? as usize;
    let pool_size = u64_at(&toc, 0x20)? as usize;
    let pool = toc.get(pool_off..pool_off.saturating_add(pool_size)).ok_or("The .nup string table is out of bounds.")?;

    let name_at = |off: u32, len: u32| -> Result<String, String> {
        let (o, l) = (off as usize, len as usize);
        let raw = pool.get(o..o.saturating_add(l)).ok_or("A name in the .nup is out of bounds.")?;
        Ok(String::from_utf8_lossy(raw).into_owned())
    };

    let mut pos = 0x30usize;
    let mut packages = Vec::with_capacity(pkg_count);
    for _ in 0..pkg_count {
        packages.push(Package {
            title_id: u64_at(&toc, pos)?,
            root_dir: u32_at(&toc, pos + 0x08)?,
            exefs_dir: u32_at(&toc, pos + 0x0C)?,
            romfs_dir: u32_at(&toc, pos + 0x10)?,
            version: u32_at(&toc, pos + 0x14)?,
            kind: *toc.get(pos + 0x18).ok_or("The .nup file is truncated.")?,
        });
        pos += 0x40;
    }
    let mut dirs = Vec::with_capacity(dir_count);
    for _ in 0..dir_count {
        dirs.push(Dir { name: name_at(u32_at(&toc, pos)?, u32_at(&toc, pos + 4)?)?, first_child: u32_at(&toc, pos + 8)?, child_count: u32_at(&toc, pos + 0xC)? });
        pos += 0x18;
    }
    let mut files = Vec::with_capacity(file_count);
    for _ in 0..file_count {
        files.push(FileNode {
            name: name_at(u32_at(&toc, pos)?, u32_at(&toc, pos + 4)?)?,
            size: u64_at(&toc, pos + 8)?,
            extent_offset: u64_at(&toc, pos + 0x10)?,
            extent_size: u64_at(&toc, pos + 0x18)?,
            codec: *toc.get(pos + 0x40).ok_or("The .nup file is truncated.")?,
        });
        pos += 0x50;
    }
    if pool_off < pos || (pool_off - pos) % 4 != 0 {
        return Err("The .nup table of contents is invalid.".into());
    }
    let children = (0..(pool_off - pos) / 4).map(|i| u32_at(&toc, pos + i * 4)).collect::<Result<Vec<_>, _>>()?;
    Ok(Nup { uuid, file_size, packages, dirs, files, children })
}

/// Names in a package are labels, never paths.
fn safe_name(n: &str) -> Result<&str, String> {
    if n.is_empty() || n == "." || n == ".." || n.contains(['/', '\\', ':', '\0']) {
        return Err(format!("The .nup contains an unsafe file name: {:?}", n));
    }
    Ok(n)
}

struct Extractor<'a> {
    nup: &'a Nup,
    src: BufReader<File>,
    done: u64,
    total: u64,
    progress: &'a mut dyn FnMut(u64, u64, &str),
    cancel: &'a dyn Fn() -> bool,
}

impl Extractor<'_> {
    fn dir_total(&self, d: u32, depth: u32) -> u64 {
        if depth > 64 {
            return 0;
        }
        let dir = &self.nup.dirs[d as usize];
        let dc = self.nup.dirs.len() as u32;
        (dir.first_child..dir.first_child + dir.child_count)
            .filter_map(|s| self.nup.children.get(s as usize).copied())
            .map(|id| if id < dc { self.dir_total(id, depth + 1) } else { self.nup.files.get((id - dc) as usize).map_or(0, |f| f.size) })
            .sum()
    }

    fn dir(&mut self, d: u32, out: &Path, depth: u32) -> Result<(), String> {
        if depth > 64 {
            return Err("The .nup folder tree is too deep.".into());
        }
        let nup = self.nup;
        let dir = nup.dirs.get(d as usize).ok_or("The .nup refers to a folder that does not exist.")?;
        fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {}", out.display(), e))?;
        let dc = nup.dirs.len() as u32;
        for slot in dir.first_child..dir.first_child.saturating_add(dir.child_count) {
            let id = *nup.children.get(slot as usize).ok_or("The .nup folder listing is out of bounds.")?;
            if id < dc {
                let sub = &nup.dirs[id as usize];
                self.dir(id, &out.join(safe_name(&sub.name)?), depth + 1)?;
            } else {
                let f = nup.files.get((id - dc) as usize).ok_or("The .nup refers to a file that does not exist.")?;
                self.file(f, &out.join(safe_name(&f.name)?))?;
            }
        }
        Ok(())
    }

    fn file(&mut self, f: &FileNode, dest: &Path) -> Result<(), String> {
        if (self.cancel)() {
            return Err("cancelled".into());
        }
        (self.progress)(self.done, self.total, &f.name);
        let mut out = BufWriter::with_capacity(1 << 20, File::create(dest).map_err(|e| format!("cannot write {}: {}", dest.display(), e))?);
        if f.size > 0 {
            if f.extent_offset.checked_add(f.extent_size).map_or(true, |e| e > self.nup.file_size) {
                return Err(format!("{} points outside the .nup file.", f.name));
            }
            self.src.seek(SeekFrom::Start(f.extent_offset)).map_err(|e| e.to_string())?;
            let mut part = (&mut self.src).take(f.extent_size);
            let written = match f.codec {
                0 => std::io::copy(&mut part, &mut out).map_err(|e| e.to_string())?,
                1 => {
                    let mut dec = zstd::stream::read::Decoder::new(&mut part).map_err(|e| e.to_string())?;
                    std::io::copy(&mut dec, &mut out).map_err(|e| format!("{} could not be decompressed: {}", f.name, e))?
                }
                c => return Err(format!("{} uses an unknown compression ({}).", f.name, c)),
            };
            if written != f.size {
                return Err(format!("{} has the wrong size after unpacking.", f.name));
            }
        }
        out.flush().map_err(|e| format!("cannot write {}: {}", dest.display(), e))?;
        self.done += f.size;
        Ok(())
    }
}

fn uuid_hex(u: &[u8; 16]) -> String {
    u.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Unpacks `nup_path` into `out`. A finished unpack of the same package is reused. `progress(done, total, current file)`
/// is called per file; `cancel` is polled between files. Returns `out`.
pub fn extract(nup_path: &Path, out: &Path, progress: &mut dyn FnMut(u64, u64, &str), cancel: &dyn Fn() -> bool) -> Result<PathBuf, String> {
    let mut file = File::open(nup_path).map_err(|e| format!("cannot open {}: {}", nup_path.display(), e))?;
    let nup = parse(&mut file)?;
    // ":dlc" marks unpacks that include the add-ons: a package with add-ons unpacked before they were extracted is unpacked again.
    let has_addons = nup.packages.iter().any(|p| p.kind == KIND_ADDON);
    let stamp = format!("{}:{}{}", uuid_hex(&nup.uuid), nup.file_size, if has_addons { ":dlc" } else { "" });
    if fs::read_to_string(out.join(MARKER)).map(|s| s.trim() == stamp).unwrap_or(false) {
        return Ok(out.to_path_buf());
    }
    if out.exists() {
        fs::remove_dir_all(out).map_err(|e| format!("cannot clear {}: {}", out.display(), e))?;
    }
    fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {}", out.display(), e))?;

    // (package, destination) pairs for the base, the updates and the add-ons (the runtime serves dlc/ as [aoc] dir).
    let mut jobs: Vec<(&Package, PathBuf)> = Vec::new();
    for p in &nup.packages {
        match p.kind {
            KIND_BASE => jobs.push((p, out.join("base"))),
            KIND_UPDATE => jobs.push((p, out.join("updates").join(p.version.to_string()))),
            KIND_ADDON => jobs.push((p, out.join("dlc").join(format!("{:016X}", p.title_id)))),
            _ => {}
        }
    }
    if !jobs.iter().any(|(p, _)| p.kind != KIND_ADDON) {
        return Err("This .nup has no base game or update in it.".into());
    }
    let mut ex = Extractor { nup: &nup, src: BufReader::with_capacity(1 << 20, file), done: 0, total: 0, progress, cancel };
    for (p, _) in &jobs {
        for d in [p.exefs_dir, p.romfs_dir] {
            if d != NO_DIR && (d as usize) < nup.dirs.len() {
                ex.total += ex.dir_total(d, 0);
            }
        }
    }
    for (p, dest) in &jobs {
        if p.root_dir as usize >= nup.dirs.len() {
            return Err("The .nup package tree is invalid.".into());
        }
        for (d, name) in [(p.exefs_dir, "exefs"), (p.romfs_dir, "romfs")] {
            if d != NO_DIR {
                if d as usize >= nup.dirs.len() {
                    return Err("The .nup package tree is invalid.".into());
                }
                ex.dir(d, &dest.join(name), 0)?;
            }
        }
    }
    (ex.progress)(ex.total, ex.total, "");
    fs::write(out.join(MARKER), stamp).map_err(|e| e.to_string())?;
    Ok(out.to_path_buf())
}
