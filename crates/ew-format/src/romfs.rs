//! romfs inventory and content probing.
//!
//! The inventory walks the extracted romfs and reports file counts and byte
//! totals per top-level directory and per extension. Probing inspects the bytes
//! of a small sample of files, transparently decompressing standard Zstandard
//! assets so that the *inner* container format is reported rather than the
//! wrapper.

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct DirStat {
    pub name: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtStat {
    pub ext: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Inventory {
    pub root: PathBuf,
    pub total_files: u64,
    pub total_bytes: u64,
    /// Per top-level directory, largest first.
    pub dirs: Vec<DirStat>,
    /// Per extension, most frequent first.
    pub extensions: Vec<ExtStat>,
}

impl Inventory {
    /// Walk the romfs collecting counts and sizes. Metadata only: no file
    /// contents are read, so this stays cheap on large dumps.
    pub fn scan(root: &Path) -> Result<Inventory> {
        let mut dirs: BTreeMap<String, DirStat> = BTreeMap::new();
        let mut exts: BTreeMap<String, ExtStat> = BTreeMap::new();
        let mut total_files = 0u64;
        let mut total_bytes = 0u64;

        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) if dir == root => {
                    return Err(e).with_context(|| format!("reading romfs {}", root.display()))
                }
                Err(_) => continue,
            };
            for entry in entries {
                let entry = match entry {
                    Ok(e) => e,
                    Err(_) => continue,
                };
                let path = entry.path();
                let ft = match entry.file_type() {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                if ft.is_dir() {
                    stack.push(path);
                    continue;
                }
                let len = entry.metadata().map(|m| m.len()).unwrap_or(0);

                // Attribute the file to its top-level directory.
                let top = path
                    .strip_prefix(root)
                    .ok()
                    .and_then(|rel| rel.components().next())
                    .map(|c| c.as_os_str().to_string_lossy().to_string())
                    .unwrap_or_else(|| ".".to_string());
                let d = dirs.entry(top.clone()).or_insert_with(|| DirStat {
                    name: top,
                    files: 0,
                    bytes: 0,
                });
                d.files += 1;
                d.bytes += len;

                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_ascii_lowercase())
                    .unwrap_or_else(|| "(none)".to_string());
                // Asset names contain dots, so keep only the last suffix.
                let e = exts.entry(ext.clone()).or_insert_with(|| ExtStat {
                    ext,
                    files: 0,
                    bytes: 0,
                });
                e.files += 1;
                e.bytes += len;

                total_files += 1;
                total_bytes += len;
            }
        }

        let mut dirs: Vec<DirStat> = dirs.into_values().collect();
        dirs.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
        let mut extensions: Vec<ExtStat> = exts.into_values().collect();
        extensions.sort_by(|a, b| b.files.cmp(&a.files).then_with(|| a.ext.cmp(&b.ext)));

        Ok(Inventory { root: root.to_path_buf(), total_files, total_bytes, dirs, extensions })
    }
}

/// What a file's bytes actually are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FormatGuess {
    /// Short machine-friendly identifier, e.g. `bfres`, `msbt`, `zstd`.
    pub id: String,
    /// Human-readable description.
    pub description: String,
    /// True when the payload was wrapped in Zstandard and unwrapped for the
    /// probe.
    pub zstd_wrapped: bool,
}

impl FormatGuess {
    fn new(id: &str, description: &str, zstd_wrapped: bool) -> Self {
        FormatGuess {
            id: id.to_string(),
            description: description.to_string(),
            zstd_wrapped,
        }
    }
}

/// Identify a payload from its leading bytes.
pub fn identify(payload: &[u8], name_hint: &str) -> FormatGuess {
    let zstd = payload.len() >= 4 && payload[0..4] == [0x28, 0xb5, 0x2f, 0xfd];
    if zstd {
        return FormatGuess::new("zstd", "Zstandard stream (standard)", false);
    }
    match () {
        _ if payload.starts_with(b"FRES") => FormatGuess::new("bfres", "NintendoWare BFRES resource", false),
        _ if payload.starts_with(b"Gfx2") => FormatGuess::new("bfres2", "NintendoWare BFRES (Switch 2) resource", false),
        _ if payload.starts_with(b"MsgStdBn") => FormatGuess::new("msbt", "Nintendo text message archive", false),
        _ if payload.starts_with(b"MsgPrjBn") => FormatGuess::new("msbt_project", "Nintendo message project", false),
        _ if payload.starts_with(b"BNTX") => FormatGuess::new("bntx", "NintendoWare texture container", false),
        _ if payload.starts_with(b"FSAR") || payload.starts_with(b"BFSAR") => {
            FormatGuess::new("bfsar", "NintendoWare sound archive", false)
        }
        _ if payload.starts_with(b"FSTM") || payload.starts_with(b"FWAV") => {
            FormatGuess::new("bntx_audio", "NintendoWare audio bank", false)
        }
        _ if payload.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) => {
            FormatGuess::new("webm", "Matroska/WebM container", false)
        }
        _ if payload.starts_with(b"OggS") => FormatGuess::new("ogg", "Ogg container", false),
        _ if payload.starts_with(b"AVA0") || payload.starts_with(b"AVM0") => {
            FormatGuess::new("nvn_shader", "NVN shader container", false)
        }
        _ if payload.starts_with(b"ZBCS") || payload.starts_with(b"ZBIC") => {
            FormatGuess::new("zbic", "Nintendo Zstandard variant (ZBIC)", false)
        }
        _ if name_hint.ends_with(".opus") => {
            FormatGuess::new("opus_bank", "Opus/HCA-style audio stream", false)
        }
        _ if name_hint.ends_with(".lips") => FormatGuess::new("lips", "Lip-sync data", false),
        _ if name_hint.ends_with(".matanimb") => {
            FormatGuess::new("matanimb", "NintendoWare material animation", false)
        }
        _ => FormatGuess::new("unknown", "Unrecognised payload", false),
    }
}

/// Read the first `limit` bytes of a file, unwrapping a standard Zstandard
/// stream if that is what the file holds.
pub fn read_probe_payload(path: &Path, limit: usize) -> Result<(Vec<u8>, bool)> {
    use std::io::{Read, Seek};
    let mut f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut head = vec![0u8; 4];
    let n = f.read(&mut head)?;
    head.truncate(n);
    if head == [0x28, 0xb5, 0x2f, 0xfd] {
        f.rewind()?;
        let bytes = std::fs::read(path)?;
        let mut out = Vec::with_capacity(limit);
        let mut dec = ruzstd::StreamingDecoder::new(std::io::Cursor::new(&bytes[..]))
            .with_context(|| format!("opening zstd stream in {}", path.display()))?;
        let mut buf = vec![0u8; limit];
        while out.len() < limit {
            let want = (limit - out.len()).min(buf.len());
            match dec.read(&mut buf[..want]) {
                Ok(0) => break,
                Ok(k) => out.extend_from_slice(&buf[..k]),
                Err(e) => return Err(e).with_context(|| format!("zstd decode {}", path.display())),
            }
        }
        return Ok((out, true));
    }
    let mut buf = vec![0u8; limit];
    let k = f.read(&mut buf)?;
    buf.truncate(k);
    Ok((buf, false))
}

/// Probe a sample of files, one per extension encountered.
pub fn probe_samples(root: &Path, per_ext: usize) -> Result<Vec<ProbeResult>> {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut results = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() => {
                    stack.push(path);
                    continue;
                }
                Ok(_) => {}
                Err(_) => continue,
            }
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_else(|| "(none)".to_string());
            let count = seen.entry(ext.clone()).or_insert(0);
            if *count >= per_ext {
                continue;
            }
            if let Ok((payload, wrapped)) = read_probe_payload(&path, 64) {
                let mut guess = identify(&payload, &path.to_string_lossy());
                guess.zstd_wrapped = wrapped;
                if wrapped {
                    // Report both the wrapper and the inner container.
                    guess.description = format!("zstd -> {}", guess.description);
                    if guess.id == "unknown" {
                        guess.id = "zstd-unidentified".to_string();
                    }
                }
                *count += 1;
                results.push(ProbeResult {
                    relative: path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string(),
                    format: guess,
                });
            }
        }
    }
    results.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(results)
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub relative: String,
    pub format: FormatGuess,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifies_common_magics() {
        assert_eq!(identify(b"FRES\0\0\0\0", "x.bfres").id, "bfres");
        assert_eq!(identify(b"MsgStdBn", "x.msbt").id, "msbt");
        assert_eq!(identify(&[0x28, 0xb5, 0x2f, 0xfd, 0, 0], "x.zst").id, "zstd");
        assert_eq!(identify(b"\x1a\x45\xdf\xa3rest", "x.webm").id, "webm");
        assert_eq!(identify(b"", "x.opus").id, "opus_bank");
        assert_eq!(identify(b"????", "x.weird").id, "unknown");
    }

    #[test]
    fn inventory_scans_directory_tree() {
        let dir = std::env::temp_dir().join(format!("ew_inv_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Sound/stream")).unwrap();
        std::fs::create_dir_all(dir.join("Msg/CNzh")).unwrap();
        std::fs::write(dir.join("Sound/stream/a.opus"), vec![0u8; 10]).unwrap();
        std::fs::write(dir.join("Sound/b.opus"), vec![0u8; 5]).unwrap();
        std::fs::write(dir.join("Msg/CNzh/c.msbt.zst"), vec![0u8; 7]).unwrap();

        let inv = Inventory::scan(&dir).unwrap();
        assert_eq!(inv.total_files, 3);
        assert_eq!(inv.total_bytes, 22);
        assert_eq!(inv.dirs[0].name, "Sound");
        assert_eq!(inv.dirs[0].files, 2);
        let opus = inv.extensions.iter().find(|e| e.ext == "opus").unwrap();
        assert_eq!(opus.files, 2);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
