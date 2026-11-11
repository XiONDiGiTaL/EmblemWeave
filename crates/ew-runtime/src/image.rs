//! Guest memory bus with explicit region bounds.
//!
//! Nintendo Switch 2 modules are loaded at fixed addresses; a translation that
//! silently reads outside a mapped region is a bug that corrupts emulation
//! state. Every access is range-checked and reported.

use anyhow::{bail, Result};
use serde::Serialize;

/// One loadable module's memory footprint.
#[derive(Debug, Clone, Serialize)]
pub struct ModuleLayout {
    /// ExeFS file name, e.g. `main`, `sdk`.
    pub name: String,
    /// Internal module name, e.g. `app`, `nnSdk`.
    pub module_name: String,
    pub text: u64,
    pub rodata: u64,
    pub data: u64,
    pub bss: u64,
    /// Whether the module's code can already be lifted and dispatched.
    pub liftable: bool,
}

impl ModuleLayout {
    pub fn total(&self) -> u64 {
        self.text + self.rodata + self.data + self.bss
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Perm {
    Read,
    ReadWrite,
    /// `.text` is executable and read-only.
    ReadExecute,
}

#[derive(Debug, Clone)]
struct Region {
    name: String,
    base: u64,
    data: Vec<u8>,
    perm: Perm,
}

/// The guest physical address space.
#[derive(Debug, Default)]
pub struct GuestMemory {
    regions: Vec<Region>,
}

impl GuestMemory {
    pub fn new() -> Self {
        GuestMemory::default()
    }

    /// Map a zero-filled region.
    pub fn map(&mut self, name: &str, base: u64, size: u64, perm: Perm) -> Result<()> {
        if size == 0 {
            return Ok(());
        }
        let size = usize::try_from(size).map_err(|_| anyhow::anyhow!("region too large"))?;
        let end = base.saturating_add(size as u64);
        for r in &self.regions {
            let r_end = r.base + r.data.len() as u64;
            if base < r_end && r.base < end {
                bail!("region {name} overlaps {}", r.name);
            }
        }
        self.regions.push(Region { name: name.to_string(), base, data: vec![0u8; size], perm });
        Ok(())
    }

    /// Map a region and copy initial contents into it.
    pub fn map_with(&mut self, name: &str, base: u64, bytes: &[u8], perm: Perm) -> Result<()> {
        self.map(name, base, bytes.len() as u64, perm)?;
        let region = self
            .regions
            .iter_mut()
            .find(|r| r.name == name && r.base == base)
            .expect("just mapped");
        region.data[..bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    fn region_for(&self, addr: u64, len: u64) -> Result<&Region> {
        let end = addr
            .checked_add(len)
            .ok_or_else(|| anyhow::anyhow!("address overflow at 0x{addr:x}+{len}"))?;
        for r in &self.regions {
            let r_end = r.base + r.data.len() as u64;
            if addr >= r.base && end <= r_end {
                return Ok(r);
            }
        }
        bail!("unmapped guest memory access at 0x{addr:x} ({len} bytes)")
    }

    /// Guest read of 1, 2, 4 or 8 bytes (little endian).
    pub fn read(&self, addr: u64, bytes: usize) -> Result<u64> {
        if !matches!(bytes, 1 | 2 | 4 | 8) {
            bail!("unsupported access width {bytes}");
        }
        let r = self.region_for(addr, bytes as u64)?;
        let off = (addr - r.base) as usize;
        let mut buf = [0u8; 8];
        buf[..bytes].copy_from_slice(&r.data[off..off + bytes]);
        Ok(u64::from_le_bytes(buf))
    }

    pub fn write(&mut self, addr: u64, value: u64, bytes: usize) -> Result<()> {
        if !matches!(bytes, 1 | 2 | 4 | 8) {
            bail!("unsupported access width {bytes}");
        }
        let (base, perm) = {
            let r = self.region_for(addr, bytes as u64)?;
            (r.base, r.perm)
        };
        if perm == Perm::ReadExecute {
            bail!("write to read-only region at 0x{addr:x}");
        }
        let r = self
            .regions
            .iter_mut()
            .find(|r| r.base == base)
            .expect("region found above");
        let off = (addr - r.base) as usize;
        r.data[off..off + bytes].copy_from_slice(&value.to_le_bytes()[..bytes]);
        Ok(())
    }

    /// Copy out a slice of guest memory (used by tooling and tests).
    pub fn slice(&self, addr: u64, len: usize) -> Result<&[u8]> {
        let r = self.region_for(addr, len as u64)?;
        let off = (addr - r.base) as usize;
        Ok(&r.data[off..off + len])
    }

    pub fn region_names(&self) -> Vec<String> {
        self.regions.iter().map(|r| r.name.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> GuestMemory {
        let mut m = GuestMemory::new();
        m.map("text", 0, 0x100, Perm::ReadExecute).unwrap();
        m.map("data", 0x1000, 0x100, Perm::ReadWrite).unwrap();
        m
    }

    #[test]
    fn reads_and_writes_within_region() {
        let mut m = mem();
        m.write(0x1000, 0xdead_beef, 4).unwrap();
        assert_eq!(m.read(0x1000, 4).unwrap(), 0xdead_beef);
        assert_eq!(m.read(0x1002, 2).unwrap(), 0xdead);
    }

    #[test]
    fn rejects_out_of_bounds_access() {
        let m = mem();
        assert!(m.read(0x10FE, 8).is_err(), "straddles the end of the region");
        assert!(m.read(0x2000, 4).is_err());
        assert!(m.read(u64::MAX - 1, 8).is_err());
    }

    #[test]
    fn text_is_not_writable() {
        let mut m = mem();
        let err = m.write(0x10, 0, 4).unwrap_err();
        assert!(err.to_string().contains("read-only"));
    }

    #[test]
    fn rejects_overlapping_mappings() {
        let mut m = mem();
        assert!(m.map("clash", 0x80, 0x100, Perm::ReadWrite).is_err());
        assert!(m.map("fine", 0x2000, 0x10, Perm::ReadWrite).is_ok());
    }

    #[test]
    fn copies_initial_contents() {
        let mut m = GuestMemory::new();
        m.map_with("rodata", 0x100, b"MOD0", Perm::Read).unwrap();
        assert_eq!(m.slice(0x100, 4).unwrap(), b"MOD0");
    }
}
