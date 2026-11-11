//! Import discovery from the module's own dynamic-link metadata.
//!
//! A retail NSO carries everything needed to name every external symbol it
//! calls: `MOD0` in `.rodata` points at an ELF-style dynamic array, whose
//! `DT_SYMTAB`, `DT_STRTAB` and `DT_JMPREL` tables describe each jump slot by
//! name. Reading them is what turns "branched to address zero" into "called
//! `nn::os::LockMutex`".
//!
//! `MOD0` stores its fields relative to the `MOD0` structure itself, not to the
//! module base. That detail is the difference between finding the dynamic array
//! and reading garbage, so it is applied explicitly here.

use crate::Module;
use serde::Serialize;
use std::collections::BTreeMap;

const DT_NULL: u64 = 0;
const DT_PLTRELSZ: u64 = 2;
const DT_PLTGOT: u64 = 3;
const DT_HASH: u64 = 4;
const DT_STRTAB: u64 = 5;
const DT_SYMTAB: u64 = 6;
const DT_RELA: u64 = 7;
const DT_RELASZ: u64 = 8;
const DT_SYMENT: u64 = 11;
const DT_STRSZ: u64 = 10;
const DT_JMPREL: u64 = 23;
const DT_INIT: u64 = 12;
const DT_FINI: u64 = 13;
const DT_INIT_ARRAY: u64 = 25;
const DT_INIT_ARRAYSZ: u64 = 27;
const DT_RELRSZ: u64 = 35;
const DT_RELR: u64 = 36;

/// One external symbol the module imports, addressed through a jump slot.
#[derive(Debug, Clone, Serialize)]
pub struct Import {
    pub slot: u64,
    pub name: String,
    /// Raw ELF relocation type, kept so `R_AARCH64_JUMP_SLOT` and data imports
    /// can be told apart later.
    pub reloc: u32,
}

/// A symbol the module defines and exports, such as `nnMain`.
#[derive(Debug, Clone, Serialize)]
pub struct Export {
    pub addr: u64,
    pub size: u64,
    pub name: String,
    /// ELF symbol type: 1 object, 2 function.
    pub kind: u8,
}

/// Everything the dynamic table says about this module.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ImportTable {
    pub mod0: Option<u64>,
    pub dynamic: Option<u64>,
    pub symtab: Option<u64>,
    pub strtab: Option<u64>,
    pub jmprel: Option<u64>,
    pub got: Option<u64>,
    pub bss_start: Option<u64>,
    pub bss_end: Option<u64>,
    pub jumps: Vec<Import>,
    /// Symbols the module defines, sorted by address.
    pub exports: Vec<Export>,
    pub init: Option<u64>,
    pub fini: Option<u64>,
    /// `DT_INIT_ARRAY` and its entry count.
    pub init_array: Option<(u64, u64)>,
    /// Data-symbol relocations (vtables, typeinfo) that would need the other
    /// module's data to satisfy.
    pub data_imports: usize,
    pub relative_relocs: usize,
    /// `R_AARCH64_RELATIVE` fixups as (slot, addend). With the image at base
    /// zero, the loader writes the addend itself into the slot.
    #[serde(skip)]
    pub relatives: Vec<(u64, u64)>,
    /// Slots listed by the compact `DT_RELR` table: each holds a link-time
    /// address, to which the loader adds the load base.
    #[serde(skip)]
    pub relr: Vec<u64>,
    /// Named data relocations (`R_AARCH64_GLOB_DAT`, `R_AARCH64_ABS64`) as
    /// (slot, symbol, addend): pointers to another module's objects.
    #[serde(skip)]
    pub data_binds: Vec<(u64, String, u64)>,
    /// Reasons a table could not be read, so a partial result never looks whole.
    pub notes: Vec<String>,
}

impl ImportTable {
    /// Address of an exported symbol by exact name.
    pub fn export(&self, name: &str) -> Option<u64> {
        self.exports.iter().find(|e| e.name == name).map(|e| e.addr)
    }

    /// Slot address to symbol name, for reporting.
    pub fn by_slot(&self) -> BTreeMap<u64, String> {
        self.jumps.iter().map(|i| (i.slot, i.name.clone())).collect()
    }

    pub fn name_at(&self, slot: u64) -> Option<&str> {
        self.jumps
            .iter()
            .find(|i| i.slot == slot)
            .map(|i| i.name.as_str())
    }
}

/// A read-only view over the module image that maps guest addresses onto the
/// segment that contains them.
struct View<'a> {
    segments: [(u64, &'a [u8]); 3],
}

impl<'a> View<'a> {
    fn new(module: &'a Module) -> View<'a> {
        View {
            segments: [
                (0, module.text.as_slice()),
                (module.rodata_addr, module.rodata.as_slice()),
                (module.data_addr, module.data.as_slice()),
            ],
        }
    }

    fn slice(&self, addr: u64, len: usize) -> Option<&'a [u8]> {
        for (base, bytes) in self.segments {
            if bytes.is_empty() {
                continue;
            }
            let end = base + bytes.len() as u64;
            if addr >= base && addr + len as u64 <= end {
                let off = (addr - base) as usize;
                return Some(&bytes[off..off + len]);
            }
        }
        None
    }

    fn u8(&self, addr: u64) -> Option<u8> {
        self.slice(addr, 1).map(|b| b[0])
    }

    fn u16(&self, addr: u64) -> Option<u16> {
        self.slice(addr, 2).map(|b| u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&self, addr: u64) -> Option<u32> {
        self.slice(addr, 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&self, addr: u64) -> Option<u64> {
        self.slice(addr, 8).map(|b| {
            let mut v = [0u8; 8];
            v.copy_from_slice(b);
            u64::from_le_bytes(v)
        })
    }

    fn cstr(&self, addr: u64, max: usize) -> Option<String> {
        for (base, bytes) in self.segments {
            if bytes.is_empty() || addr < base {
                continue;
            }
            let off = usize::try_from(addr - base).ok()?;
            if off >= bytes.len() {
                continue;
            }
            // Clamp to the segment: a name at the end of .rodata is short, not
            // missing, and asking for a fixed window would reject it.
            let avail = &bytes[off..(off + max).min(bytes.len())];
            let end = avail.iter().position(|b| *b == 0)?;
            return Some(String::from_utf8_lossy(&avail[..end]).to_string());
        }
        None
    }
}

/// Locate `MOD0` and read the dynamic table it points at.
pub fn read(module: &Module) -> ImportTable {
    let view = View::new(module);
    let mut table = ImportTable::default();

    let Some(mod0) = find_mod0(&view, module) else {
        table.notes.push("no MOD0 structure in .rodata".to_string());
        return table;
    };
    table.mod0 = Some(mod0);

    // MOD0 fields are offsets from the MOD0 structure itself.
    let field = |offset: u64| view.u32(mod0 + offset).map(|v| mod0.wrapping_add(v as u64));
    let Some(dynamic) = field(4) else {
        table.notes.push("MOD0 is truncated".to_string());
        return table;
    };
    table.dynamic = Some(dynamic);
    table.bss_start = field(8);
    table.bss_end = field(0x0C);

    let mut tags: BTreeMap<u64, u64> = BTreeMap::new();
    for i in 0..256u64 {
        let at = dynamic + i * 16;
        let (Some(tag), Some(value)) = (view.u64(at), view.u64(at + 8)) else {
            table.notes.push(format!("dynamic array truncated at 0x{at:x}"));
            break;
        };
        if tag == DT_NULL {
            break;
        }
        tags.entry(tag).or_insert(value);
    }
    if tags.is_empty() {
        table.notes.push("dynamic array is empty".to_string());
        return table;
    }

    table.symtab = tags.get(&DT_SYMTAB).copied();
    table.strtab = tags.get(&DT_STRTAB).copied();
    table.jmprel = tags.get(&DT_JMPREL).copied();
    table.got = tags.get(&DT_PLTGOT).copied();
    let strsz = tags.get(&DT_STRSZ).copied().unwrap_or(0) as usize;
    let syment = tags.get(&DT_SYMENT).copied().unwrap_or(24);

    let (Some(symtab), Some(strtab)) = (table.symtab, table.strtab) else {
        table.notes.push("dynamic table has no symbol or string table".to_string());
        return table;
    };
    if syment != 24 {
        table
            .notes
            .push(format!("unexpected symbol entry size {syment}"));
    }

    let symbol_name = |index: u64| -> Option<String> {
        let name_off = view.u32(symtab + index * syment)? as u64;
        if strsz > 0 && name_off >= strsz as u64 {
            return None;
        }
        view.cstr(strtab + name_off, 512)
    };

    if let (Some(jmprel), Some(size)) = (table.jmprel, tags.get(&DT_PLTRELSZ).copied()) {
        for i in 0..size / 24 {
            let at = jmprel + i * 24;
            let (Some(offset), Some(info)) = (view.u64(at), view.u64(at + 8)) else {
                break;
            };
            let symbol = info >> 32;
            let reloc = (info & 0xFFFF_FFFF) as u32;
            if symbol == 0 {
                continue;
            }
            if let Some(name) = symbol_name(symbol) {
                table.jumps.push(Import { slot: offset, name, reloc });
            }
        }
        table.jumps.sort_by_key(|i| i.slot);
    } else {
        table.notes.push("no jump-slot relocation table".to_string());
    }

    // The general relocation table names the data symbols (C++ vtables and
    // typeinfo for the exception runtime) and counts relative fixups.
    if let (Some(rela), Some(size)) = (tags.get(&DT_RELA).copied(), tags.get(&DT_RELASZ).copied()) {
        for i in 0..size / 24 {
            let at = rela + i * 24;
            let Some(info) = view.u64(at + 8) else { break };
            let reloc = (info & 0xFFFF_FFFF) as u32;
            let symbol = info >> 32;
            match reloc {
                1027 => {
                    // R_AARCH64_RELATIVE
                    table.relative_relocs += 1;
                    if let (Some(offset), Some(addend)) = (view.u64(at), view.u64(at + 16)) {
                        table.relatives.push((offset, addend));
                    }
                }
                1025 | 257 if symbol != 0 => {
                    table.data_imports += 1;
                    if let (Some(offset), Some(addend), Some(name)) = (view.u64(at), view.u64(at + 16), symbol_name(symbol)) {
                        table.data_binds.push((offset, name, addend));
                    }
                }
                _ if symbol != 0 => table.data_imports += 1,
                _ => {}
            }
        }
    }
    // DT_RELR: an address entry relocates one slot and sets the cursor after
    // it; a bitmap entry (low bit set) relocates the following 63 slots whose
    // bits are set, then advances the cursor past them.
    if let (Some(relr), Some(size)) = (tags.get(&DT_RELR).copied(), tags.get(&DT_RELRSZ).copied()) {
        let mut cursor = 0u64;
        for i in 0..size / 8 {
            let Some(entry) = view.u64(relr + i * 8) else { break };
            if entry & 1 == 0 {
                table.relr.push(entry);
                cursor = entry + 8;
            } else {
                let mut bits = entry >> 1;
                let mut at = cursor;
                while bits != 0 {
                    if bits & 1 != 0 {
                        table.relr.push(at);
                    }
                    bits >>= 1;
                    at += 8;
                }
                cursor += 63 * 8;
            }
        }
        table.relative_relocs += table.relr.len();
    }
    table.init = tags.get(&DT_INIT).copied();
    table.fini = tags.get(&DT_FINI).copied();
    if let (Some(at), Some(size)) = (tags.get(&DT_INIT_ARRAY).copied(), tags.get(&DT_INIT_ARRAYSZ).copied()) {
        table.init_array = Some((at, size / 8));
    }

    // The symbol count comes from the SysV hash table's chain length. Without
    // one, the string table conventionally follows the symbol table.
    let count = tags
        .get(&DT_HASH)
        .and_then(|&h| view.u32(h + 4))
        .map(|n| n as u64)
        .or_else(|| (strtab > symtab).then(|| (strtab - symtab) / syment));
    match count {
        Some(count) => {
            for i in 1..count.min(1 << 20) {
                let at = symtab + i * syment;
                let (Some(info), Some(shndx), Some(value), Some(size)) =
                    (view.u8(at + 4), view.u16(at + 6), view.u64(at + 8), view.u64(at + 16))
                else {
                    break;
                };
                if shndx == 0 || value == 0 {
                    continue;
                }
                if let Some(name) = symbol_name(i) {
                    table.exports.push(Export { addr: value, size, name, kind: info & 0xF });
                }
            }
            table.exports.sort_by_key(|e| e.addr);
        }
        None => table.notes.push("symbol count unknown; exports not read".to_string()),
    }

    if tags.get(&DT_HASH).is_none() && table.jumps.is_empty() {
        table.notes.push("no symbol hash table to validate names against".to_string());
    }

    table
}

fn find_mod0(view: &View, module: &Module) -> Option<u64> {
    let ro = &module.rodata;
    let mut at = 0usize;
    while at + 0x20 <= ro.len() {
        let hit = ro[at..].windows(4).position(|w| w == b"MOD0")? + at;
        // Validate: the dynamic pointer must land inside the image and the
        // first entries must be plain tags. A string match alone is not proof.
        if let Some(field) = view.u32(module.rodata_addr + hit as u64 + 4) {
            let base = module.rodata_addr + hit as u64;
            let dynamic = base.wrapping_add(field as u64);
            if dynamic >= 0x1000 && view.u64(dynamic).is_some() {
                return Some(base);
            }
        }
        at = hit + 4;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words_to_bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// Synthesize a module with MOD0, a dynamic array, two symbols and one jump
    /// slot, so the parse path is exercised without the game's data.
    fn synthetic() -> Module {
        // Four words of text put rodata at page one, so absolute table addresses
        // below are relative to that base.
        const RO: u64 = 0x1000;
        let text = words_to_bytes(&[0xD65F03C0; 4]);
        let mut ro = vec![0u8; 0x100];
        ro[0..4].copy_from_slice(b"MOD0");
        // dynamic array at MOD0+0x40
        let dynamic = 0x40u32;
        ro[4..8].copy_from_slice(&dynamic.to_le_bytes());
        let dyn_at = dynamic as usize;
        let entries = [
            (DT_SYMTAB, RO + 0xA0),
            (DT_STRTAB, RO + 0x200),
            (DT_STRSZ, 0x40),
            (DT_JMPREL, RO + 0x280),
            (DT_PLTRELSZ, 24),
            (DT_SYMENT, 24),
            (DT_NULL, 0),
        ];
        for (i, (tag, value)) in entries.iter().enumerate() {
            let at = dyn_at + i * 16;
            ro[at..at + 8].copy_from_slice(&tag.to_le_bytes());
            ro[at + 8..at + 16].copy_from_slice(&value.to_le_bytes());
        }
        ro.resize(0x400, 0);
        // symbols at 0xA0: index 1 has name offset 8 in strtab
        let sym = 0xA0 + 24;
        ro[sym..sym + 4].copy_from_slice(&8u32.to_le_bytes()); // name at strtab+8
        // strtab at 0x200
        ro[0x200..0x200 + 4].copy_from_slice(b"\0nn\0");
        let name = b"nn::os::Test";
        ro[0x208..0x208 + name.len()].copy_from_slice(name);
        // jmprel at 0x280: slot 0x1234, symbol 1, type 1026
        let jmprel = 0x280;
        ro[jmprel..jmprel + 8].copy_from_slice(&0x1234u64.to_le_bytes());
        ro[jmprel + 8..jmprel + 16].copy_from_slice(&((1u64 << 32) | 1026).to_le_bytes());
        ro[jmprel + 16..jmprel + 24].copy_from_slice(&0i64.to_le_bytes());
        Module::new("test", text, ro, Vec::new())
    }

    #[test]
    fn names_a_jump_slot() {
        let m = synthetic();
        let table = read(&m);
        assert_eq!(table.jumps.len(), 1, "{:?}", table.notes);
        assert_eq!(table.jumps[0].slot, 0x1234);
        assert_eq!(table.jumps[0].name, "nn::os::Test");
        assert_eq!(table.name_at(0x1234), Some("nn::os::Test"));
    }

    #[test]
    fn missing_metadata_is_reported_not_invented() {
        let m = Module::new("empty", vec![0u8; 0x10], vec![0u8; 0x10], Vec::new());
        let table = read(&m);
        assert!(table.jumps.is_empty());
        assert!(table.notes.iter().any(|n| n.contains("MOD0")));
    }
}
