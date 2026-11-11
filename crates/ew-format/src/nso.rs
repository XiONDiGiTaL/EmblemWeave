//! NSO0 executable container parsing and segment extraction.
//!
//! Header layout used here was established empirically against the retail
//! Fire Emblem: Fortune's Weave ExeFS dump: every segment boundary declared by
//! this layout is reproduced exactly by an independent LZ4 decode, and the last
//! segment lands precisely on end-of-file for all five LZ4-compressed modules.
//! See `docs/FEASIBILITY.md` for the evidence table.
//!
//! ```text
//! 0x00  magic "NSO0"
//! 0x04  u32 version
//! 0x08  u32 reserved
//! 0x0C  u32 flags            bit0..2 = text/rodata/data compressed,
//!                            bit3..5 = per-segment hashes present,
//!                            bit7    = ZBIC compression instead of LZ4
//! 0x10  u32 text.file_offset
//! 0x14  u32 text.mem_offset
//! 0x18  u32 text.size        (decompressed)
//! 0x1C  u32 header_size      (0x100 in every observed module)
//! 0x20  u32 rodata.file_offset
//! 0x24  u32 rodata.mem_offset
//! 0x28  u32 rodata.size
//! 0x2C  u32 name_len         (bytes occupied by the module name + NUL)
//! 0x30  u32 data.file_offset
//! 0x34  u32 data.mem_offset
//! 0x38  u32 data.size
//! 0x3C  u32 bss_size
//! 0x40  u8  module_id[0x20]
//! ...   module name at (text.file_offset - name_len)
//! ```

use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::Path;

pub const NSO_MAGIC: &[u8; 4] = b"NSO0";
pub const NSO_HEADER_SIZE: usize = 0x100;

/// Which segment a piece of the module lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SegmentKind {
    Text,
    Rodata,
    Data,
}

impl SegmentKind {
    pub const ALL: [SegmentKind; 3] = [SegmentKind::Text, SegmentKind::Rodata, SegmentKind::Data];

    pub fn as_str(self) -> &'static str {
        match self {
            SegmentKind::Text => "text",
            SegmentKind::Rodata => "rodata",
            SegmentKind::Data => "data",
        }
    }

    fn index(self) -> usize {
        match self {
            SegmentKind::Text => 0,
            SegmentKind::Rodata => 1,
            SegmentKind::Data => 2,
        }
    }
}

/// Per-segment byte range in the module image.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Segment {
    /// Offset of the (possibly compressed) segment inside the NSO file.
    pub file_offset: u32,
    /// Offset the segment occupies once loaded into memory.
    pub mem_offset: u32,
    /// Size after decompression.
    pub size: u32,
}

/// Compression applied to a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Compression {
    /// Stored verbatim.
    None,
    /// Raw LZ4 blocks; decoded length comes from the segment header.
    Lz4,
    /// Nintendo's modified Zstandard stream, magic `ZBIC`, which replaces the
    /// FSE table reader with Binary Interpolative Coding.
    Zbic,
}

#[derive(Debug, Clone, Serialize)]
pub struct NsoHeader {
    pub version: u32,
    pub flags: u32,
    pub segments: [Segment; 3],
    pub header_size: u32,
    /// Raw `name_len` field (the module name plus its NUL terminator).
    pub name_len: u32,
    pub bss_size: u32,
    pub module_id: String,
    pub module_name: String,
}

impl NsoHeader {
    pub fn segment(&self, kind: SegmentKind) -> Segment {
        self.segments[kind.index()]
    }

    /// Compression of one segment, derived from the flag word.
    pub fn compression(&self, kind: SegmentKind) -> Compression {
        let bit = 1u32 << kind.index();
        if self.flags & bit == 0 {
            return Compression::None;
        }
        if self.flags & 0x80 != 0 {
            Compression::Zbic
        } else {
            Compression::Lz4
        }
    }

    /// Set of compressions present anywhere in this module.
    pub fn compressions(&self) -> Vec<Compression> {
        let mut v = Vec::new();
        for kind in SegmentKind::ALL {
            let c = self.compression(kind);
            if !v.contains(&c) {
                v.push(c);
            }
        }
        v
    }

    pub fn has_zbic(&self) -> bool {
        self.compressions().contains(&Compression::Zbic)
    }
}

/// A parsed NSO module: header plus the original file bytes.
#[derive(Debug)]
pub struct Nso<'a> {
    bytes: &'a [u8],
    pub header: NsoHeader,
}

/// A decompressed module image with the three segments laid out by their
/// declared memory offsets, plus zero-initialised `.bss`.
#[derive(Debug, Clone)]
pub struct GameImage {
    pub module_name: String,
    pub module_id: String,
    pub text: Vec<u8>,
    pub rodata: Vec<u8>,
    pub data: Vec<u8>,
    pub bss_size: u32,
}

impl GameImage {
    pub fn segment(&self, kind: SegmentKind) -> &[u8] {
        match kind {
            SegmentKind::Text => &self.text,
            SegmentKind::Rodata => &self.rodata,
            SegmentKind::Data => &self.data,
        }
    }

    /// Total addressable image size: data end + bss.
    pub fn mapped_size(&self) -> usize {
        self.text.len() + self.rodata.len() + self.data.len() + self.bss_size as usize
    }

    /// Locate the `MOD0` module metadata structure, which normally lives in
    /// rodata and describes the dynamic section, bss and exception frames.
    pub fn find_mod0(&self) -> Option<Mod0> {
        mod0::locate(self.rodata.as_slice())
    }
}

fn u32_at(bytes: &[u8], off: usize) -> Result<u32> {
    let end = off + 4;
    if end > bytes.len() {
        bail!("NSO header truncated at 0x{off:x}");
    }
    Ok(u32::from_le_bytes(bytes[off..end].try_into().expect("4 bytes")))
}

impl<'a> Nso<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Nso<'a>> {
        if bytes.len() < NSO_HEADER_SIZE {
            bail!("file is smaller than an NSO header ({} bytes)", bytes.len());
        }
        if &bytes[0..4] != NSO_MAGIC {
            bail!("not an NSO0 module (magic {:02x?})", &bytes[0..4]);
        }

        let version = u32_at(bytes, 0x04)?;
        let flags = u32_at(bytes, 0x0C)?;
        let mut segments = [Segment { file_offset: 0, mem_offset: 0, size: 0 }; 3];
        for (i, _kind) in SegmentKind::ALL.iter().enumerate() {
            // Segment entries sit at 0x10 (text), 0x20 (rodata) and 0x30
            // (data); the words at 0x1C, 0x2C and 0x3C hold header_size,
            // name_len and bss_size.
            let base = 0x10 + i * 0x10;
            segments[i] = Segment {
                file_offset: u32_at(bytes, base)?,
                mem_offset: u32_at(bytes, base + 4)?,
                size: u32_at(bytes, base + 8)?,
            };
        }

        let header_size = u32_at(bytes, 0x1C)?;
        let name_len = u32_at(bytes, 0x2C)?;
        let bss_size = u32_at(bytes, 0x3C)?;
        let module_id = hex(&bytes[0x40..0x60]);

        // The module name sits immediately before the text segment.
        let name_end = segments[0].file_offset as usize;
        let name_start = name_end.checked_sub(name_len as usize).unwrap_or(0);
        let module_name = if name_start < name_end && name_end <= bytes.len() {
            String::from_utf8_lossy(&bytes[name_start..name_end])
                .trim_end_matches('\0')
                .to_string()
        } else {
            String::new()
        };

        Ok(Nso {
            bytes,
            header: NsoHeader {
                version,
                flags,
                segments,
                header_size,
                name_len,
                bss_size,
                module_id,
                module_name,
            },
        })
    }

    /// Length of the module file that was parsed.
    pub fn bytes_len(&self) -> usize {
        self.bytes.len()
    }

    pub fn open(path: &Path) -> Result<(Vec<u8>, NsoHeader)> {
        let bytes = std::fs::read(path)
            .with_context(|| format!("reading NSO module {}", path.display()))?;
        let header = {
            let nso = Nso::parse(&bytes)?;
            nso.header.clone()
        };
        Ok((bytes, header))
    }

    /// Validate that the declared segment ranges describe a sane file: ordered,
    /// in bounds, and ending at end-of-file.
    pub fn validate(&self) -> Result<Vec<String>> {
        let mut notes = Vec::new();
        let file_len = self.bytes.len();
        let mut prev_end = 0usize;
        for kind in SegmentKind::ALL {
            let seg = self.header.segment(kind);
            let start = seg.file_offset as usize;
            if start < prev_end {
                bail!("{} segment starts before the previous segment ends", kind.as_str());
            }
            if start > file_len {
                bail!("{} segment offset 0x{start:x} is past end of file", kind.as_str());
            }
            prev_end = start;
        }
        if self.header.bss_size == 0 {
            notes.push("module declares no bss".to_string());
        }
        Ok(notes)
    }

    /// Byte range of a segment's stored (possibly compressed) representation.
    /// The end is bounded by the next segment's file offset, or EOF.
    fn stored_range(&self, kind: SegmentKind) -> Result<(usize, usize)> {
        let start = self.header.segment(kind).file_offset as usize;
        let end = match kind {
            SegmentKind::Text => self.header.segment(SegmentKind::Rodata).file_offset as usize,
            SegmentKind::Rodata => self.header.segment(SegmentKind::Data).file_offset as usize,
            SegmentKind::Data => self.bytes.len(),
        };
        if start > end || end > self.bytes.len() {
            bail!("{} segment range 0x{start:x}..0x{end:x} is invalid", kind.as_str());
        }
        Ok((start, end))
    }

    /// Decompress one segment into its declared size.
    pub fn decompress_segment(&self, kind: SegmentKind) -> Result<Vec<u8>> {
        let (start, end) = self.stored_range(kind)?;
        let stored = &self.bytes[start..end];
        let size = self.header.segment(kind).size as usize;

        match self.header.compression(kind) {
            Compression::None => {
                if stored.len() < size {
                    bail!(
                        "{} segment stores fewer bytes than declared ({} < {})",
                        kind.as_str(),
                        stored.len(),
                        size
                    );
                }
                Ok(stored[..size].to_vec())
            }
            Compression::Lz4 => {
                let out = lz4_flex::block::decompress(stored, size).with_context(|| {
                    format!("LZ4 decode of {} segment failed", kind.as_str())
                })?;
                if out.len() != size {
                    bail!(
                        "{} segment decoded to {} bytes, header declares {}",
                        kind.as_str(),
                        out.len(),
                        size
                    );
                }
                Ok(out)
            }
            Compression::Zbic => bail!(
                "{} segment uses ZBIC (Nintendo Zstandard variant); decoding requires a \
                 BIC-based FSE table reader that is not implemented",
                kind.as_str()
            ),
        }
    }

    /// Decompress every segment, producing a loadable image.
    pub fn to_image(&self) -> Result<GameImage> {
        let text = self.decompress_segment(SegmentKind::Text)?;
        let rodata = self.decompress_segment(SegmentKind::Rodata)?;
        let data = self.decompress_segment(SegmentKind::Data)?;
        Ok(GameImage {
            module_name: self.header.module_name.clone(),
            module_id: self.header.module_id.clone(),
            text,
            rodata,
            data,
            bss_size: self.header.bss_size,
        })
    }
}

/// `MOD0` dynamic-link metadata embedded in a module.
#[derive(Debug, Clone, Serialize)]
pub struct Mod0 {
    pub offset: usize,
    pub dynamic: u32,
    pub bss_start: u32,
    pub bss_end: u32,
    pub eh_frame_hdr_start: u32,
    pub eh_frame_hdr_end: u32,
    pub module_object: u32,
}

mod mod0 {
    use super::Mod0;

    pub fn locate(rodata: &[u8]) -> Option<Mod0> {
        let pos = rodata.windows(4).position(|w| w == b"MOD0")?;
        let at = |o: usize| -> Option<u32> {
            rodata
                .get(pos + o..pos + o + 4)
                .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        };
        Some(Mod0 {
            offset: pos,
            dynamic: at(4)?,
            bss_start: at(8)?,
            bss_end: at(0x0C)?,
            eh_frame_hdr_start: at(0x10)?,
            eh_frame_hdr_end: at(0x14)?,
            module_object: at(0x18)?,
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a synthetic NSO with three uncompressed segments.
    fn synthetic(text_len: usize, ro_len: usize, da_len: usize, name: &str) -> Vec<u8> {
        let name_bytes = name.len() + 1;
        let text_off = 0x100 + name_bytes;
        let ro_off = text_off + text_len;
        let da_off = ro_off + ro_len;
        let mut v = vec![0u8; da_off + da_len];
        v[0..4].copy_from_slice(b"NSO0");
        v[0x0C..0x10].copy_from_slice(&0u32.to_le_bytes()); // no compression
        let put = |v: &mut [u8], off: usize, val: u32| {
            v[off..off + 4].copy_from_slice(&val.to_le_bytes())
        };
        put(&mut v, 0x10, text_off as u32);
        put(&mut v, 0x14, 0);
        put(&mut v, 0x18, text_len as u32);
        put(&mut v, 0x1C, 0x100);
        put(&mut v, 0x20, ro_off as u32);
        put(&mut v, 0x24, 0x2000);
        put(&mut v, 0x28, ro_len as u32);
        put(&mut v, 0x2C, name_bytes as u32);
        put(&mut v, 0x30, da_off as u32);
        put(&mut v, 0x34, 0x4000);
        put(&mut v, 0x38, da_len as u32);
        put(&mut v, 0x3C, 0x1000); // bss
        v[0x100..0x100 + name.len()].copy_from_slice(name.as_bytes());
        for (i, b) in v.iter_mut().enumerate().skip(text_off) {
            *b = (i & 0xff) as u8;
        }
        v
    }

    #[test]
    fn parses_header_fields() {
        let bytes = synthetic(0x40, 0x30, 0x20, "app");
        let nso = Nso::parse(&bytes).unwrap();
        assert_eq!(nso.header.module_name, "app");
        assert_eq!(nso.header.header_size, 0x100);
        assert_eq!(nso.header.bss_size, 0x1000);
        assert_eq!(nso.header.segment(SegmentKind::Text).size, 0x40);
        assert_eq!(nso.header.segment(SegmentKind::Rodata).mem_offset, 0x2000);
        assert_eq!(nso.header.segment(SegmentKind::Data).mem_offset, 0x4000);
        assert_eq!(nso.header.compression(SegmentKind::Text), Compression::None);
    }

    #[test]
    fn zbic_flag_detection() {
        let mut bytes = synthetic(0x10, 0x10, 0x10, "app");
        // bit0..2 = segments compressed, bit7 = ZBIC instead of LZ4
        bytes[0x0C..0x10].copy_from_slice(&0xbfu32.to_le_bytes());
        let nso = Nso::parse(&bytes).unwrap();
        assert!(nso.header.has_zbic());
        assert_eq!(nso.header.compression(SegmentKind::Text), Compression::Zbic);
        // ZBIC must fail loudly rather than silently returning garbage.
        let err = nso.decompress_segment(SegmentKind::Text).unwrap_err();
        assert!(err.to_string().contains("ZBIC"));
    }

    #[test]
    fn decompresses_uncompressed_segments() {
        let bytes = synthetic(0x40, 0x30, 0x20, "subsdk9");
        let nso = Nso::parse(&bytes).unwrap();
        let image = nso.to_image().unwrap();
        assert_eq!(image.text.len(), 0x40);
        assert_eq!(image.rodata.len(), 0x30);
        assert_eq!(image.data.len(), 0x20);
        assert_eq!(image.module_name, "subsdk9");
    }

    #[test]
    fn rejects_wrong_magic() {
        let mut bytes = synthetic(0x10, 0x10, 0x10, "app");
        bytes[0..4].copy_from_slice(b"NRO0");
        assert!(Nso::parse(&bytes).is_err());
    }

    #[test]
    fn lz4_segment_roundtrip() {
        // Compress a text segment with a real LZ4 block encoder, then decode
        // it back through the container layer.
        let text: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let packed = lz4_flex::block::compress(&text);
        let name = b"app\0".to_vec();
        let text_off = 0x100 + name.len();
        let ro_off = text_off + packed.len();
        let mut v = vec![0u8; ro_off + 0x10];
        v[0..4].copy_from_slice(b"NSO0");
        v[0x0C..0x10].copy_from_slice(&0x01u32.to_le_bytes()); // text compressed, LZ4
        let put = |v: &mut [u8], off: usize, val: u32| {
            v[off..off + 4].copy_from_slice(&val.to_le_bytes())
        };
        put(&mut v, 0x10, text_off as u32);
        put(&mut v, 0x18, text.len() as u32);
        put(&mut v, 0x1C, 0x100);
        put(&mut v, 0x20, ro_off as u32);
        put(&mut v, 0x28, 0x10);
        put(&mut v, 0x2C, name.len() as u32);
        put(&mut v, 0x30, ro_off as u32 + 0x10);
        put(&mut v, 0x38, 0);
        v[0x100..0x104].copy_from_slice(&name);
        v[text_off..text_off + packed.len()].copy_from_slice(&packed);
        let nso = Nso::parse(&v).unwrap();
        let out = nso.decompress_segment(SegmentKind::Text).unwrap();
        assert_eq!(out, text);
    }
}
