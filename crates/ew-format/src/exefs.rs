//! ExeFS directory handling: the set of NSO modules that make up one title.

use crate::nso::{Nso, NsoHeader};
use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// One module found in an ExeFS directory.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleEntry {
    pub file_name: String,
    pub path: PathBuf,
    /// Internal module name recorded in the NSO (e.g. `app`, `nnrtld`).
    pub module_name: String,
    pub module_id: String,
    pub text_size: u32,
    pub rodata_size: u32,
    pub data_size: u32,
    pub bss_size: u32,
    pub flags: u32,
    pub compressions: Vec<String>,
    pub file_size: u64,
}

impl ModuleEntry {
    pub fn is_zbic(&self) -> bool {
        self.compressions.iter().any(|c| c == "zbic")
    }

    /// Decompressed footprint of the module.
    pub fn image_size(&self) -> u64 {
        self.text_size as u64 + self.rodata_size as u64 + self.data_size as u64
            + self.bss_size as u64
    }
}

/// A parsed ExeFS directory, ordered so the main program comes first.
#[derive(Debug, Clone, Serialize)]
pub struct ExeFs {
    pub root: PathBuf,
    pub modules: Vec<ModuleEntry>,
    /// Whether `main.npdm` was present next to the modules.
    pub has_npdm: bool,
}

impl ExeFs {
    /// Scan a directory that holds extracted ExeFS files. Files are identified
    /// by their `NSO0` magic rather than by name, so renamed dumps still work.
    pub fn scan_dir(root: &Path) -> Result<ExeFs> {
        let mut modules = Vec::new();
        let mut has_npdm = false;

        for entry in std::fs::read_dir(root)
            .with_context(|| format!("reading ExeFS directory {}", root.display()))?
        {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if file_name.ends_with(".npdm") {
                has_npdm = true;
                continue;
            }

            // Only the header and the module name need to be resident to
            // describe a module.
            let file_size = entry.metadata()?.len();
            let mut head = vec![0u8; 0x200];
            let read = {
                use std::io::Read;
                let mut f = std::fs::File::open(&path)?;
                f.read(&mut head)?
            };
            head.truncate(read);
            let header: NsoHeader = match Nso::parse(&head) {
                Ok(n) => n.header,
                // A short read cannot hold the name string, so retry by
                // parsing just the fixed header.
                Err(_) => match Nso::parse(&head[..crate::nso::NSO_HEADER_SIZE.min(head.len())]) {
                    Ok(n) => n.header,
                    Err(_) => continue,
                },
            };

            modules.push(ModuleEntry {
                file_name,
                path,
                module_name: header.module_name.clone(),
                module_id: header.module_id.clone(),
                text_size: header.segments[0].size,
                rodata_size: header.segments[1].size,
                data_size: header.segments[2].size,
                bss_size: header.bss_size,
                flags: header.flags,
                compressions: header
                    .compressions()
                    .into_iter()
                    .map(|c| match c {
                        crate::nso::Compression::None => "none",
                        crate::nso::Compression::Lz4 => "lz4",
                        crate::nso::Compression::Zbic => "zbic",
                    })
                    .map(str::to_string)
                    .collect(),
                file_size,
            });
        }

        // `main` first, then subsdk0..n, then sdk/rtld for readability.
        modules.sort_by_key(|m| {
            let rank = match m.file_name.as_str() {
                "main" => 0,
                "rtld" => 900,
                "sdk" => 901,
                other if other.starts_with("subsdk") => 100,
                _ => 500,
            };
            (rank, m.file_name.clone())
        });

        Ok(ExeFs { root: root.to_path_buf(), modules, has_npdm })
    }

    /// Parse the NPDM that accompanies the modules, if present.
    pub fn read_npdm(&self) -> Result<Option<NpdmInfo>> {
        let path = self.root.join("main.npdm");
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        NpdmInfo::parse(&bytes).map(Some)
    }

    pub fn find(&self, name: &str) -> Option<&ModuleEntry> {
        self.modules.iter().find(|m| m.file_name == name)
    }

    /// Modules that can currently be decompressed and lifted.
    pub fn liftable(&self) -> impl Iterator<Item = &ModuleEntry> {
        self.modules.iter().filter(|m| !m.is_zbic())
    }

    /// Modules blocked on ZBIC support.
    pub fn blocked(&self) -> impl Iterator<Item = &ModuleEntry> {
        self.modules.iter().filter(|m| m.is_zbic())
    }
}

/// Minimal NPDM (process metadata) description.
#[derive(Debug, Clone, Serialize)]
pub struct NpdmInfo {
    pub name: String,
    pub mmu_flags: u32,
    pub process_category: String,
    pub main_thread_priority: u8,
    pub default_cpu_id: u8,
    pub address_space_type: u32,
}

impl NpdmInfo {
    pub fn parse(bytes: &[u8]) -> Result<NpdmInfo> {
        anyhow::ensure!(bytes.len() >= 0x80, "NPDM too small");
        anyhow::ensure!(&bytes[0..4] == b"META", "NPDM magic mismatch");
        let read_u32 = |o: usize| -> u32 {
            u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
        };
        let name = String::from_utf8_lossy(&bytes[0x20..0x50])
            .trim_end_matches('\0')
            .to_string();
        let category = match read_u32(0x0C) {
            0 => "Regular",
            1 => "System",
            2 => "Application",
            3 => "Applet",
            _ => "Unknown",
        };
        Ok(NpdmInfo {
            name,
            mmu_flags: read_u32(0x08),
            process_category: category.to_string(),
            main_thread_priority: bytes[0x10],
            default_cpu_id: bytes[0x11],
            address_space_type: read_u32(0x0C),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npdm_parses_category_and_name() {
        let mut v = vec![0u8; 0x80];
        v[0..4].copy_from_slice(b"META");
        v[0x0C..0x10].copy_from_slice(&2u32.to_le_bytes());
        v[0x20..0x2F].copy_from_slice(b"Application\0\0\0\0".split(|_| false).next().unwrap());
        let info = NpdmInfo::parse(&v).unwrap();
        assert_eq!(info.process_category, "Application");
        assert!(info.name.starts_with("Application"));
    }

    #[test]
    fn npdm_rejects_bad_magic() {
        let v = vec![0u8; 0x80];
        assert!(NpdmInfo::parse(&v).is_err());
    }
}
