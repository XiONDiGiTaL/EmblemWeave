//! Cheats: Atmosphere cheat codes and Cheat Engine tables, applied to the
//! running game.
//!
//! The engine maps the game's memory at the addresses the game itself uses
//! (main module at 0x71_0000_0000, heap at 0x78_0000_0000), so EmblemWeave
//! reads and writes it from outside the process with ReadProcessMemory and
//! WriteProcessMemory, the way Cheat Engine does.
//!
//! * Atmosphere codes (`[name]` followed by hex words, the dmnt cheat VM) run
//!   about 60 times a second while switched on. Code patches to the main
//!   module's instructions have no effect, because the game's code is compiled
//!   ahead of time; data writes, pointer chains and conditionals work.
//! * Cheat Engine tables (`.CT`): `aobscanregion` scripts find their pattern in
//!   the game's memory (the range in the table belongs to the emulator it was
//!   made for, so the game's own memory is scanned instead), and the entries
//!   below show live values that can be edited or frozen.

use std::collections::HashMap;

pub const MAIN_BASE: u64 = 0x71_0000_0000;
pub const HEAP_BASE: u64 = 0x78_0000_0000;
pub const ALIAS_BASE: u64 = 0x60_0000_0000;
/// The game's memory, scanned for patterns: alias views, the main module,
/// the heap; stacks start at 0x7C_0000_0000.
pub const GUEST_RANGE: (u64, u64) = (0x60_0000_0000, 0x7C_0000_0000);

/// The engine's process name.
pub const GAME_EXE: &str = "fefw.exe";

// ------------------------------------------------------------------ process

pub trait Memory {
    fn read(&self, addr: u64, buf: &mut [u8]) -> bool;
    fn write(&self, addr: u64, data: &[u8]) -> bool;
    /// Committed, readable and writable regions within `[lo, hi)`.
    fn regions(&self, lo: u64, hi: u64) -> Vec<(u64, u64)>;
}

#[cfg(windows)]
pub mod process {
    use super::Memory;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::Memory::{VirtualQueryEx, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_READWRITE, PAGE_EXECUTE_READWRITE};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
    };

    pub struct Process {
        handle: HANDLE,
        pub pid: u32,
    }

    unsafe impl Send for Process {}
    unsafe impl Sync for Process {}

    impl Drop for Process {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.handle) };
        }
    }

    pub fn find(exe: &str) -> Option<u32> {
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap as isize == -1 {
                return None;
            }
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut found = None;
            let mut ok = Process32FirstW(snap, &mut e);
            while ok != 0 {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                if String::from_utf16_lossy(&e.szExeFile[..len]).eq_ignore_ascii_case(exe) {
                    found = Some(e.th32ProcessID);
                    break;
                }
                ok = Process32NextW(snap, &mut e);
            }
            CloseHandle(snap);
            found
        }
    }

    impl Process {
        pub fn open(pid: u32) -> Option<Process> {
            let h = unsafe { OpenProcess(PROCESS_VM_READ | PROCESS_VM_WRITE | PROCESS_VM_OPERATION | PROCESS_QUERY_INFORMATION, 0, pid) };
            (!h.is_null()).then_some(Process { handle: h, pid })
        }

        pub fn alive(&self) -> bool {
            let mut code = 0u32;
            unsafe { GetExitCodeProcess(self.handle, &mut code) != 0 && code == 259 }
        }
    }

    impl Memory for Process {
        fn read(&self, addr: u64, buf: &mut [u8]) -> bool {
            let mut n = 0usize;
            unsafe { ReadProcessMemory(self.handle, addr as _, buf.as_mut_ptr() as _, buf.len(), &mut n) != 0 && n == buf.len() }
        }

        fn write(&self, addr: u64, data: &[u8]) -> bool {
            let mut n = 0usize;
            unsafe { WriteProcessMemory(self.handle, addr as _, data.as_ptr() as _, data.len(), &mut n) != 0 && n == data.len() }
        }

        fn regions(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
            let mut out = Vec::new();
            let mut a = lo;
            while a < hi {
                let mut m: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
                let n = unsafe { VirtualQueryEx(self.handle, a as _, &mut m, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) };
                if n == 0 {
                    break;
                }
                let (base, size) = (m.BaseAddress as u64, m.RegionSize as u64);
                if size == 0 {
                    break;
                }
                let rw = m.Protect & (PAGE_READWRITE | PAGE_EXECUTE_READWRITE) != 0 && m.Protect & PAGE_GUARD == 0;
                if m.State == MEM_COMMIT && rw {
                    out.push((base.max(lo), (base + size).min(hi)));
                }
                a = base + size;
            }
            out
        }
    }
}

/// Find `pattern` (None = wildcard) in readable memory within `[lo, hi)`.
pub fn aob_scan(mem: &dyn Memory, lo: u64, hi: u64, pattern: &[Option<u8>]) -> Option<u64> {
    if pattern.is_empty() {
        return None;
    }
    const CHUNK: u64 = 4 << 20;
    let first = pattern.iter().position(|b| b.is_some())?;
    let key = pattern[first].unwrap();
    let mut buf = Vec::new();
    for (s, e) in mem.regions(lo, hi) {
        let mut a = s;
        while a < e {
            let len = (e - a).min(CHUNK + pattern.len() as u64 - 1);
            buf.resize(len as usize, 0);
            if mem.read(a, &mut buf) {
                let mut i = 0;
                while i + pattern.len() <= buf.len() {
                    match buf[i + first..].iter().position(|&b| b == key) {
                        None => break,
                        Some(k) => i += k,
                    }
                    if i + pattern.len() > buf.len() {
                        break;
                    }
                    if pattern.iter().zip(&buf[i..]).all(|(p, b)| p.is_none_or(|p| p == *b)) {
                        return Some(a + i as u64);
                    }
                    i += 1;
                }
            }
            a += CHUNK;
        }
    }
    None
}

/// `aob_scan` split across the CPU's cores, returning the lowest match.
pub fn aob_scan_parallel(mem: &(dyn Memory + Sync), lo: u64, hi: u64, pattern: &[Option<u8>]) -> Option<u64> {
    const PIECE: u64 = 64 << 20;
    let mut pieces = Vec::new();
    for (s, e) in mem.regions(lo, hi) {
        let mut a = s;
        while a < e {
            let b = (a + PIECE).min(e);
            // Overlap so a match across the boundary is still seen.
            pieces.push((a, (b + pattern.len() as u64).min(e)));
            a = b;
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let best = std::sync::Mutex::new(None::<u64>);
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(&(a, b)) = pieces.get(i) else { return };
                if best.lock().unwrap().is_some_and(|x| x < a) {
                    continue;
                }
                if let Some(found) = aob_scan(&Window(mem, a, b), a, b, pattern) {
                    let mut g = best.lock().unwrap();
                    if g.is_none_or(|x| found < x) {
                        *g = Some(found);
                    }
                }
            });
        }
    });
    best.into_inner().unwrap()
}

// ------------------------------------------------------------ unit records

/// Offsets in a unit's in-memory record, as Cheat Engine tables for this
/// game use them: the record starts with five growth-rate bytes and three
/// zero bytes, then max HP and the eight stats follow as single bytes.
#[allow(dead_code)]
pub mod unit {
    /// The five growth-rate bytes that start every record, then three zeros.
    pub const GROWTH: u64 = 0x00;
    /// Max HP: one byte, the first of the nine stat bytes.
    pub const MAX_HP: u64 = 0x08;
    /// The eight other stats, one byte each, two bytes apart: Str, Mag, Dex,
    /// Spd, Lck, Def, Res, Cha. The community tables read them as bytes at
    /// +A, +C, +E, +10, +12, +14, +16 and +18.
    pub const STAT_OFFSETS: [u64; 8] = [0x0A, 0x0C, 0x0E, 0x10, 0x12, 0x14, 0x16, 0x18];
    /// Current HP, a u16.
    pub const HP: u64 = 0x20;
    /// Level, one byte.
    pub const LEVEL: u64 = 0x26;
    /// Experience, a u32: a running total in the thousands, not a 0-99 bar.
    pub const EXP: u64 = 0x2C;
    pub const LEN: usize = 0x30;
    /// The level the game stops raising at.
    pub const MAX_LEVEL: u8 = 40;
    /// The value the community tables use for a maxed stat (the game shows
    /// 200 without complaint), not the 99 of the older tables' small numbers.
    pub const MAX_STAT: u8 = 200;
}

/// Whether `b` (at least `unit::LEN` bytes) looks like a unit record.
///
/// The layout is the one the community Cheat Engine tables for this game use:
/// five growth bytes, three zeros, then max HP and eight stats as single
/// bytes, current HP as a u16, the level, and experience as a u32.
pub fn looks_like_unit(b: &[u8]) -> bool {
    if b.len() < unit::LEN || b[5..8] != [0, 0, 0] {
        return false;
    }
    // The five signature bytes are non-zero multiples of 5 in every record
    // the community tables name (50 50 20 90 80, 5 5 90 35 90, ...).
    if b[0..5].iter().any(|&g| g == 0 || g > 100 || g % 5 != 0) {
        return false;
    }
    // Max HP and the stats are single bytes, so 200 is an ordinary value, not
    // an outlier; a unit at 0 HP is still a unit, and the one that most needs
    // infinite HP.
    let max_hp = b[unit::MAX_HP as usize];
    let hp = u16::from_le_bytes([b[unit::HP as usize], b[unit::HP as usize + 1]]);
    let level = b[unit::LEVEL as usize];
    let exp = u32::from_le_bytes(b[0x2C..0x30].try_into().unwrap());
    max_hp > 0 && hp <= u16::from(max_hp) && (1..=60).contains(&level) && exp < 10_000_000
}

/// Every unit record in the game's memory, lowest address first.
///
/// The guest range is searched first. When it holds nothing — the engine may
/// map the guest somewhere other than the addresses these constants assume —
/// every committed read-write region of the process is searched instead.
pub fn scan_units(mem: &(dyn Memory + Sync)) -> Vec<u64> {
    let found = scan_units_in(mem, GUEST_RANGE);
    if found.is_empty() {
        scan_units_in(mem, (0x1_0000, 0x7FFF_FFFF_FFFF))
    } else {
        found
    }
}

fn scan_units_in(mem: &(dyn Memory + Sync), range: (u64, u64)) -> Vec<u64> {
    const PIECE: u64 = 64 << 20;
    let mut pieces = Vec::new();
    for (s, e) in mem.regions(range.0, range.1) {
        let mut a = s;
        while a < e {
            pieces.push((a, (a + PIECE + unit::LEN as u64).min(e)));
            a += PIECE;
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let found = std::sync::Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(16);
    std::thread::scope(|sc| {
        for _ in 0..threads {
            sc.spawn(|| {
                let mut buf = Vec::new();
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    let Some(&(a, b)) = pieces.get(i) else { return };
                    buf.resize((b - a) as usize, 0);
                    if !mem.read(a, &mut buf) {
                        continue;
                    }
                    let mut local = Vec::new();
                    let mut o = 0usize;
                    while o + unit::LEN <= buf.len() {
                        if buf[o + 5] == 0 && buf[o + 6] == 0 && buf[o + 7] == 0 && looks_like_unit(&buf[o..o + unit::LEN]) {
                            local.push(a + o as u64);
                        }
                        o += 4;
                    }
                    found.lock().unwrap().extend(local);
                }
            });
        }
    });
    let mut v = found.into_inner().unwrap();
    v.sort_unstable();
    v.dedup();
    v
}

/// One piece of a memory, as its only region.
struct Window<'a>(&'a (dyn Memory + Sync), u64, u64);

impl Memory for Window<'_> {
    fn read(&self, a: u64, b: &mut [u8]) -> bool {
        self.0.read(a, b)
    }
    fn write(&self, a: u64, d: &[u8]) -> bool {
        self.0.write(a, d)
    }
    fn regions(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
        let (s, e) = (self.1.max(lo), self.2.min(hi));
        if s < e { vec![(s, e)] } else { vec![] }
    }
}

pub fn parse_aob(s: &str) -> Option<Vec<Option<u8>>> {
    let compact: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let toks: Vec<String> = if compact.contains(' ') {
        compact.split(' ').map(String::from).collect()
    } else {
        compact.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).into_owned()).collect()
    };
    toks.iter()
        .map(|t| if t.chars().all(|c| c == '?' || c == '*') { Some(None) } else { u8::from_str_radix(t, 16).ok().map(Some) })
        .collect()
}

// -------------------------------------------------------- Cheat Engine tables

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarType {
    Byte,
    Bytes2,
    Bytes4,
    Bytes8,
    Float,
    Double,
}

impl VarType {
    pub fn parse(s: &str) -> Option<VarType> {
        Some(match s.trim() {
            "Byte" => VarType::Byte,
            "2 Bytes" => VarType::Bytes2,
            "4 Bytes" => VarType::Bytes4,
            "8 Bytes" => VarType::Bytes8,
            "Float" => VarType::Float,
            "Double" => VarType::Double,
            _ => return None,
        })
    }

    pub fn size(self) -> usize {
        match self {
            VarType::Byte => 1,
            VarType::Bytes2 => 2,
            VarType::Bytes4 | VarType::Float => 4,
            VarType::Bytes8 | VarType::Double => 8,
        }
    }

    pub fn format(self, b: &[u8], signed: bool) -> String {
        let mut w = [0u8; 8];
        w[..self.size()].copy_from_slice(&b[..self.size()]);
        let u = u64::from_le_bytes(w);
        match self {
            VarType::Float => format!("{}", f32::from_bits(u as u32)),
            VarType::Double => format!("{}", f64::from_bits(u)),
            _ if signed => {
                let bits = self.size() * 8;
                format!("{}", ((u << (64 - bits)) as i64) >> (64 - bits))
            }
            _ => format!("{u}"),
        }
    }

    pub fn encode(self, text: &str) -> Option<Vec<u8>> {
        let t = text.trim();
        let b = match self {
            VarType::Float => t.parse::<f32>().ok()?.to_le_bytes().to_vec(),
            VarType::Double => t.parse::<f64>().ok()?.to_le_bytes().to_vec(),
            _ => {
                let v: i128 = if let Some(h) = t.strip_prefix("0x") { i128::from_str_radix(h, 16).ok()? } else { t.parse().ok()? };
                (v as u64).to_le_bytes()[..self.size()].to_vec()
            }
        };
        Some(b)
    }
}

/// An `aobscanregion(name, from, to, bytes)` from an entry's script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AobScan {
    pub symbol: String,
    pub pattern: Vec<Option<u8>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: String,
    pub description: String,
    pub var: Option<VarType>,
    pub address: String,
    pub signed: bool,
    pub group: bool,
    pub scans: Vec<AobScan>,
    /// The script uses something other than `aobscan*`/`registersymbol`
    /// (code injection), which EmblemWeave cannot run.
    pub unsupported_script: bool,
    pub children: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub name: String,
    pub entries: Vec<Entry>,
}

fn parse_scans(script: &str) -> (Vec<AobScan>, bool) {
    let enable = script.split("[DISABLE]").next().unwrap_or(script);
    let mut scans = Vec::new();
    let mut unsupported = false;
    for line in enable.lines() {
        let l = line.split("//").next().unwrap_or("").trim();
        let lower = l.to_ascii_lowercase();
        if l.is_empty() || lower.starts_with("[enable]") || lower.starts_with("registersymbol") || lower.starts_with("{") {
            continue;
        }
        if let Some(args) = lower.strip_prefix("aobscanregion(").or_else(|| lower.strip_prefix("aobscan(")).or_else(|| lower.strip_prefix("aobscanmodule(")) {
            let region = lower.starts_with("aobscanregion") || lower.starts_with("aobscanmodule");
            let original = &l[l.len() - args.len()..];
            let inner = original.trim_end_matches(')');
            let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
            let (sym, bytes) = match (region, parts.as_slice()) {
                (true, [s, _, _, b, ..]) if lower.starts_with("aobscanregion") => (s, b),
                (true, [s, _, b, ..]) => (s, b),
                (false, [s, b, ..]) => (s, b),
                _ => {
                    unsupported = true;
                    continue;
                }
            };
            match parse_aob(bytes) {
                Some(pattern) => scans.push(AobScan { symbol: sym.to_string(), pattern }),
                None => unsupported = true,
            }
        } else {
            unsupported = true;
        }
    }
    (scans, unsupported)
}

fn parse_entry(n: roxmltree::Node) -> Entry {
    let text = |tag: &str| n.children().find(|c| c.has_tag_name(tag)).and_then(|c| c.text()).unwrap_or("").to_string();
    let vt = text("VariableType");
    let (scans, unsupported) = if vt == "Auto Assembler Script" { parse_scans(&text("AssemblerScript")) } else { (Vec::new(), false) };
    let children = n
        .children()
        .find(|c| c.has_tag_name("CheatEntries"))
        .map(|c| c.children().filter(|e| e.has_tag_name("CheatEntry")).map(parse_entry).collect())
        .unwrap_or_default();
    Entry {
        id: text("ID"),
        description: text("Description").trim().trim_matches('"').to_string(),
        var: VarType::parse(&vt),
        address: text("Address").trim().to_string(),
        signed: text("ShowAsSigned").trim() == "1",
        group: text("GroupHeader").trim() == "1",
        scans,
        unsupported_script: unsupported,
        children,
    }
}

pub fn parse_table(name: &str, xml: &str) -> Result<Table, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|e| format!("not a Cheat Engine table: {e}"))?;
    let root = doc.root_element();
    if !root.has_tag_name("CheatTable") {
        return Err("not a Cheat Engine table (no <CheatTable>)".into());
    }
    let entries = root
        .children()
        .find(|c| c.has_tag_name("CheatEntries"))
        .map(|c| c.children().filter(|e| e.has_tag_name("CheatEntry")).map(parse_entry).collect())
        .unwrap_or_default();
    Ok(Table { name: name.to_string(), entries })
}

/// Resolve a CE address: `symbol`, `symbol+44`, `+26+6` (relative to the
/// parent), `7800001000`. Terms are hex; symbols are matched ignoring case.
pub fn resolve(expr: &str, parent: Option<u64>, symbols: &HashMap<String, u64>) -> Option<u64> {
    let e: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
    if e.is_empty() {
        return None;
    }
    let relative = e.starts_with('+') || e.starts_with('-');
    let mut total: i128 = if relative { parent? as i128 } else { 0 };
    let mut sign = 1i128;
    let mut term = String::new();
    let flush = |term: &mut String, sign: i128, total: &mut i128| -> Option<()> {
        if term.is_empty() {
            return Some(());
        }
        let v = match u64::from_str_radix(term.trim_start_matches("0x"), 16) {
            Ok(v) => v,
            Err(_) => *symbols.get(&term.to_ascii_lowercase())?,
        };
        *total += sign * v as i128;
        term.clear();
        Some(())
    };
    for c in e.chars() {
        match c {
            '+' | '-' => {
                flush(&mut term, sign, &mut total)?;
                sign = if c == '+' { 1 } else { -1 };
            }
            _ => term.push(c),
        }
    }
    flush(&mut term, sign, &mut total)?;
    (total >= 0).then_some(total as u64)
}

// ------------------------------------------------------ Atmosphere cheat VM

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    pub name: String,
    pub master: bool,
    pub words: Vec<u32>,
}

/// Parse an Atmosphere cheat file: `[name]` or `{master}` headers, each
/// followed by hex words.
pub fn parse_codes(text: &str) -> Result<Vec<Code>, String> {
    let mut out: Vec<Code> = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let l = line.trim();
        if l.is_empty() || l.starts_with("//") || l.starts_with('#') {
            continue;
        }
        if let Some(name) = l.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            out.push(Code { name: name.trim().to_string(), master: false, words: Vec::new() });
            continue;
        }
        if let Some(name) = l.strip_prefix('{').and_then(|r| r.strip_suffix('}')) {
            out.push(Code { name: name.trim().to_string(), master: true, words: Vec::new() });
            continue;
        }
        let Some(cur) = out.last_mut() else {
            return Err(format!("line {}: code before any [name]", n + 1));
        };
        for w in l.split_whitespace() {
            cur.words.push(u32::from_str_radix(w, 16).map_err(|_| format!("line {}: {w:?} is not a hex word", n + 1))?);
        }
    }
    Ok(out.into_iter().filter(|c| !c.words.is_empty() || c.master).collect())
}

/// The game's Switch buttons in the bit order cheats use.
pub mod keys {
    pub const A: u32 = 1 << 0;
    pub const B: u32 = 1 << 1;
    pub const X: u32 = 1 << 2;
    pub const Y: u32 = 1 << 3;
    pub const LSTICK: u32 = 1 << 4;
    pub const RSTICK: u32 = 1 << 5;
    pub const L: u32 = 1 << 6;
    pub const R: u32 = 1 << 7;
    pub const ZL: u32 = 1 << 8;
    pub const ZR: u32 = 1 << 9;
    pub const PLUS: u32 = 1 << 10;
    pub const MINUS: u32 = 1 << 11;
    pub const LEFT: u32 = 1 << 12;
    pub const UP: u32 = 1 << 13;
    pub const RIGHT: u32 = 1 << 14;
    pub const DOWN: u32 = 1 << 15;
}

/// Buttons held on the first Xbox-style controller, as Switch buttons by
/// position (the east face button is A).
#[cfg(windows)]
pub fn held_keys() -> u32 {
    use windows_sys::Win32::UI::Input::XboxController::*;
    let mut st: XINPUT_STATE = unsafe { std::mem::zeroed() };
    if unsafe { XInputGetState(0, &mut st) } != 0 {
        return 0;
    }
    let g = st.Gamepad;
    let b = g.wButtons;
    let map = [
        (XINPUT_GAMEPAD_B, keys::A),
        (XINPUT_GAMEPAD_A, keys::B),
        (XINPUT_GAMEPAD_Y, keys::X),
        (XINPUT_GAMEPAD_X, keys::Y),
        (XINPUT_GAMEPAD_LEFT_THUMB, keys::LSTICK),
        (XINPUT_GAMEPAD_RIGHT_THUMB, keys::RSTICK),
        (XINPUT_GAMEPAD_LEFT_SHOULDER, keys::L),
        (XINPUT_GAMEPAD_RIGHT_SHOULDER, keys::R),
        (XINPUT_GAMEPAD_START, keys::PLUS),
        (XINPUT_GAMEPAD_BACK, keys::MINUS),
        (XINPUT_GAMEPAD_DPAD_LEFT, keys::LEFT),
        (XINPUT_GAMEPAD_DPAD_UP, keys::UP),
        (XINPUT_GAMEPAD_DPAD_RIGHT, keys::RIGHT),
        (XINPUT_GAMEPAD_DPAD_DOWN, keys::DOWN),
    ];
    let mut k = map.iter().filter(|(x, _)| b & *x != 0).fold(0, |a, (_, s)| a | s);
    if g.bLeftTrigger > 128 {
        k |= keys::ZL;
    }
    if g.bRightTrigger > 128 {
        k |= keys::ZR;
    }
    k
}

/// Registers that persist between runs, as in Atmosphere.
#[derive(Debug, Clone)]
pub struct VmState {
    pub regs: [u64; 16],
    pub saved: [u64; 16],
    pub statics: [u64; 128],
}

impl Default for VmState {
    fn default() -> Self {
        VmState { regs: [0; 16], saved: [0; 16], statics: [0; 128] }
    }
}

fn mem_base(m: u32) -> u64 {
    match m {
        0 => MAIN_BASE,
        1 => HEAP_BASE,
        2 => ALIAS_BASE,
        _ => MAIN_BASE,
    }
}

fn mask(width: u32) -> u64 {
    if width >= 8 { u64::MAX } else { (1u64 << (width * 8)) - 1 }
}

fn rd(mem: &dyn Memory, a: u64, w: u32) -> u64 {
    let mut b = [0u8; 8];
    let w = (w as usize).clamp(1, 8);
    if !mem.read(a, &mut b[..w]) {
        return 0;
    }
    u64::from_le_bytes(b)
}

fn wr(mem: &dyn Memory, a: u64, w: u32, v: u64) {
    let w = (w as usize).clamp(1, 8);
    mem.write(a, &v.to_le_bytes()[..w]);
}

fn cmp(c: u32, a: u64, b: u64) -> bool {
    match c {
        1 => a > b,
        2 => a >= b,
        3 => a < b,
        4 => a <= b,
        5 => a == b,
        6 => a != b,
        _ => false,
    }
}

fn nib(w: u32, i: u32) -> u32 {
    (w >> (28 - 4 * i)) & 0xF
}

/// Decoded opcode with its length in words.
fn op_len(words: &[u32], i: usize) -> usize {
    let w = words[i];
    let t = nib(w, 0);
    let width = nib(w, 1);
    match t {
        0 | 1 => if width == 8 { 4 } else { 3 },
        2 => 1,
        3 => if nib(w, 1) == 0 { 2 } else { 1 },
        4 => 3,
        5 => 2,
        6 => 3,
        7 => 2,
        8 => 1,
        9 => {
            if nib(w, 5) == 1 { if width == 8 { 3 } else { 2 } } else { 1 }
        }
        0xA => {
            let o = nib(w, 5);
            if o == 2 || o == 4 || o == 5 { 2 } else { 1 }
        }
        0xC => match nib(w, 1) {
            0 => match nib(w, 5) {
                0 | 2 => 2,
                4 => if nib(w, 2) == 8 { 3 } else { 2 },
                _ => 1,
            },
            _ => 1,
        },
        _ => 1,
    }
}

/// Skip from just after a false conditional at `i` to the instruction after
/// its matching end (or just after its else). Returns the new index.
fn skip_block(words: &[u32], mut i: usize, stop_at_else: bool) -> usize {
    let mut depth = 0;
    while i < words.len() {
        let w = words[i];
        let t = nib(w, 0);
        let n = op_len(words, i);
        if t == 1 || t == 8 || (t == 0xC && nib(w, 1) == 0) {
            depth += 1;
        } else if t == 2 {
            if depth == 0 {
                if nib(w, 1) == 0 || stop_at_else {
                    return i + 1;
                }
            } else if nib(w, 1) == 0 {
                depth -= 1;
            }
        }
        i += n;
    }
    i
}

/// Run one cheat's opcodes once.
pub fn run(words: &[u32], st: &mut VmState, mem: &dyn Memory, keys_held: u32) {
    let mut i = 0;
    let mut loops: Vec<(usize, usize)> = Vec::new(); // (start index, register)
    let mut guard = 0u32;
    let w = |i: usize| words.get(i).copied().unwrap_or(0);
    let addr40 = |hi: u32, lo: u32| ((hi as u64 & 0xFF) << 32) | lo as u64;
    while i < words.len() {
        guard += 1;
        if guard > 1_000_000 {
            return;
        }
        let op = words[i];
        let t = nib(op, 0);
        let width = nib(op, 1);
        let n = op_len(words, i);
        match t {
            0 => {
                let (m, r) = (nib(op, 2), nib(op, 3) as usize);
                let a = mem_base(m).wrapping_add(st.regs[r]).wrapping_add(addr40(op, w(i + 1)));
                let v = if width == 8 { ((w(i + 3) as u64) << 32) | w(i + 2) as u64 } else { w(i + 2) as u64 };
                wr(mem, a, width, v);
            }
            1 => {
                let (m, c) = (nib(op, 2), nib(op, 3));
                let a = mem_base(m).wrapping_add(addr40(op, w(i + 1)));
                let v = if width == 8 { ((w(i + 3) as u64) << 32) | w(i + 2) as u64 } else { w(i + 2) as u64 };
                if !cmp(c, rd(mem, a, width) & mask(width), v) {
                    i = skip_block(words, i + n, true);
                    continue;
                }
            }
            2 => {
                if nib(op, 1) == 1 {
                    // Reached an else while the branch ran: skip to the end.
                    i = skip_block(words, i + 1, false);
                    continue;
                }
            }
            3 => {
                let r = nib(op, 3) as usize;
                if nib(op, 1) == 0 {
                    st.regs[r] = w(i + 1) as u64;
                    loops.push((i + n, r));
                } else if let Some(&(start, lr)) = loops.last() {
                    if st.regs[lr] > 1 {
                        st.regs[lr] -= 1;
                        i = start;
                        continue;
                    }
                    st.regs[lr] = 0;
                    loops.pop();
                }
            }
            4 => {
                st.regs[nib(op, 3) as usize] = ((w(i + 1) as u64) << 32) | w(i + 2) as u64;
            }
            5 => {
                let (m, r, mode) = (nib(op, 2), nib(op, 3) as usize, nib(op, 4));
                let off = addr40(op, w(i + 1));
                let a = match mode {
                    1 => st.regs[r].wrapping_add(off),
                    2 => mem_base(m).wrapping_add(st.regs[r]).wrapping_add(off),
                    _ => mem_base(m).wrapping_add(off),
                };
                st.regs[r] = rd(mem, a, width) & mask(width);
            }
            6 => {
                let (r, inc, use_off, ro) = (nib(op, 3) as usize, nib(op, 4), nib(op, 5), nib(op, 6) as usize);
                let v = ((w(i + 1) as u64) << 32) | w(i + 2) as u64;
                let a = st.regs[r].wrapping_add(if use_off != 0 { st.regs[ro] } else { 0 });
                wr(mem, a, width, v);
                if inc != 0 {
                    st.regs[r] = st.regs[r].wrapping_add(width as u64);
                }
            }
            7 => {
                let (r, c) = (nib(op, 3) as usize, nib(op, 4));
                let v = w(i + 1) as u64;
                let x = st.regs[r];
                st.regs[r] = match c {
                    0 => x.wrapping_add(v),
                    1 => x.wrapping_sub(v),
                    2 => x.wrapping_mul(v),
                    3 => x.wrapping_shl(v as u32),
                    4 => x.wrapping_shr(v as u32),
                    _ => x,
                } & mask(width);
            }
            8 => {
                let k = op & 0x0FFF_FFFF;
                if keys_held & k != k {
                    i = skip_block(words, i + n, true);
                    continue;
                }
            }
            9 => {
                let (c, d, s1, imm) = (nib(op, 2), nib(op, 3) as usize, nib(op, 4) as usize, nib(op, 5));
                let a = st.regs[s1];
                let b = if imm == 1 {
                    if width == 8 { ((w(i + 1) as u64) << 32) | w(i + 2) as u64 } else { w(i + 1) as u64 }
                } else {
                    st.regs[nib(op, 6) as usize]
                };
                let r = match c {
                    0 => a.wrapping_add(b),
                    1 => a.wrapping_sub(b),
                    2 => a.wrapping_mul(b),
                    3 => a.wrapping_shl(b as u32),
                    4 => a.wrapping_shr(b as u32),
                    5 => a & b,
                    6 => a | b,
                    7 => !a,
                    8 => a ^ b,
                    _ => a,
                };
                st.regs[d] = r & mask(width);
            }
            0xA => {
                let (s, r, inc, o, x) = (nib(op, 2) as usize, nib(op, 3) as usize, nib(op, 4), nib(op, 5), nib(op, 6));
                let a40 = ((op as u64 & 0xF) << 32) | w(i + 1) as u64;
                let a = match o {
                    1 => st.regs[r].wrapping_add(st.regs[x as usize]),
                    2 => st.regs[r].wrapping_add(a40),
                    3 => mem_base(x).wrapping_add(st.regs[r]),
                    4 => mem_base(x).wrapping_add(a40),
                    5 => mem_base(x).wrapping_add(st.regs[r]).wrapping_add(a40),
                    _ => st.regs[r],
                };
                wr(mem, a, width, st.regs[s]);
                if inc != 0 {
                    st.regs[r] = st.regs[r].wrapping_add(width as u64);
                }
            }
            0xC => match nib(op, 1) {
                0 => {
                    let (width, c, s, x) = (nib(op, 2), nib(op, 3), nib(op, 4) as usize, nib(op, 5));
                    let a = st.regs[s] & mask(width);
                    let p6 = nib(op, 6);
                    let p7 = nib(op, 7);
                    let a40 = ((p7 as u64) << 32) | w(i + 1) as u64;
                    let b = match x {
                        0 => rd(mem, mem_base(p6).wrapping_add(a40), width),
                        1 => rd(mem, mem_base(p6).wrapping_add(st.regs[p7 as usize]), width),
                        2 => rd(mem, st.regs[p6 as usize].wrapping_add(a40), width),
                        3 => rd(mem, st.regs[p6 as usize].wrapping_add(st.regs[p7 as usize]), width),
                        4 => if width == 8 { ((w(i + 1) as u64) << 32) | w(i + 2) as u64 } else { w(i + 1) as u64 },
                        _ => st.regs[p6 as usize],
                    } & mask(width);
                    if !cmp(c, a, b) {
                        i = skip_block(words, i + n, true);
                        continue;
                    }
                }
                1 => {
                    let (d, s, x) = (nib(op, 3) as usize, nib(op, 5) as usize, nib(op, 6));
                    match x {
                        0 => st.regs[d] = st.saved[s],
                        1 => st.saved[d] = st.regs[s],
                        2 => st.saved[d] = 0,
                        3 => st.regs[d] = 0,
                        _ => {}
                    }
                }
                2 => {
                    let (x, m) = (nib(op, 2), op & 0xFFFF);
                    for k in 0..16 {
                        if m & (1 << k) != 0 {
                            match x {
                                0 => st.regs[k] = st.saved[k],
                                1 => st.saved[k] = st.regs[k],
                                2 => st.saved[k] = 0,
                                3 => st.regs[k] = 0,
                                _ => {}
                            }
                        }
                    }
                }
                3 => {
                    let idx = ((op >> 4) & 0xFF) as usize;
                    let r = (op & 0xF) as usize;
                    if idx < 0x80 {
                        st.regs[r] = st.statics[idx];
                    } else {
                        st.statics[idx & 0x7F] = st.regs[r];
                    }
                }
                _ => {}
            },
            _ => {}
        }
        i += n;
    }
}

/// Hold one unit's switched-on quick cheats. Runs every tick from the cheat
/// thread, so it keeps applying while the in-game menu is closed and to a
/// unit whose HP has just reached 0.
fn apply_unit(mem: &dyn Memory, a: u64, c: UnitCheats, level_due: bool, rejected: &mut u64) {
    if c.max_stats {
        for off in std::iter::once(unit::MAX_HP).chain(unit::STAT_OFFSETS) {
            if !mem.write(a + off, &[unit::MAX_STAT]) {
                *rejected += 1;
            }
        }
    }
    if c.infinite_hp {
        let max_hp = if c.max_stats {
            unit::MAX_STAT
        } else {
            let mut b = [0u8; 1];
            if mem.read(a + unit::MAX_HP, &mut b) { b[0] } else { 0 }
        };
        if max_hp > 0 && !mem.write(a + unit::HP, &u16::from(max_hp).to_le_bytes()) {
            *rejected += 1;
        }
    }
    if c.quick_level && level_due {
        let mut b = [0u8; 1];
        if !mem.read(a + unit::LEVEL, &mut b) {
            *rejected += 1;
        } else if b[0] < unit::MAX_LEVEL && !mem.write(a + unit::LEVEL, &[b[0] + 1]) {
            *rejected += 1;
        }
    }
}

// ------------------------------------------------------------------ runner

/// The quick cheats switched on for one unit.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnitCheats {
    pub infinite_hp: bool,
    pub quick_level: bool,
    pub max_stats: bool,
}

/// What the cheat thread applies, shared with the launcher's UI.
#[derive(Default)]
pub struct Shared {
    /// Atmosphere codes switched on (master codes first).
    pub codes: Vec<Code>,
    /// Frozen values: address and the bytes kept there.
    pub freezes: Vec<(u64, Vec<u8>)>,
    /// Units with a quick cheat switched on: the record's address and the
    /// cheats to hold there, applied by the cheat thread every tick.
    pub units: Vec<(u64, UnitCheats)>,
    /// Writes the game refused since the units were found.
    pub rejected: u64,
    /// The game's process id while attached.
    pub attached: Option<u32>,
    pub stop: bool,
}

pub type Handle = std::sync::Arc<std::sync::Mutex<Shared>>;

#[cfg(windows)]
thread_local! {
    static OPEN: std::cell::RefCell<Option<std::sync::Arc<process::Process>>> = const { std::cell::RefCell::new(None) };
}

/// The game's process, opened for reading and writing, while it runs.
#[cfg(windows)]
pub fn game() -> Option<std::sync::Arc<process::Process>> {
    OPEN.with(|o| {
        let mut o = o.borrow_mut();
        if o.as_ref().is_some_and(|p| !p.alive()) {
            *o = None;
        }
        if o.is_none() {
            *o = process::find(GAME_EXE).and_then(process::Process::open).map(std::sync::Arc::new);
        }
        o.clone()
    })
}

/// Start the thread that applies codes and freezes about 60 times a second
/// while the game runs.
#[cfg(windows)]
pub fn start() -> Handle {
    let h: Handle = Default::default();
    let shared = h.clone();
    std::thread::Builder::new()
        .name("cheats".into())
        .spawn(move || {
            let mut vm = VmState::default();
            let mut idle = 0u32;
            let mut seen: Option<u32> = None;
            let mut leveled = std::time::Instant::now();
            loop {
                let proc = game();
                let pid = proc.as_ref().map(|p| p.pid);
                let (codes, freezes, units) = {
                    let mut s = shared.lock().unwrap();
                    if s.stop {
                        return;
                    }
                    if pid != seen {
                        // Another run of the game (or none): addresses and
                        // records from the old one mean nothing now.
                        s.units.clear();
                        s.freezes.clear();
                        s.rejected = 0;
                        seen = pid;
                    }
                    s.attached = pid;
                    (s.codes.clone(), s.freezes.clone(), s.units.clone())
                };
                if codes.is_empty() && freezes.is_empty() && units.is_empty() {
                    // Only look for the game now and then when there is nothing to apply.
                    idle += 1;
                    if idle % 60 == 0 {
                        shared.lock().unwrap().attached = pid;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(16));
                    continue;
                }
                if let Some(p) = proc {
                    let k = held_keys();
                    for c in &codes {
                        run(&c.words, &mut vm, p.as_ref(), k);
                    }
                    for (a, b) in &freezes {
                        p.write(*a, b);
                    }
                    let due = leveled.elapsed() >= std::time::Duration::from_millis(250);
                    let mut rejected = 0u64;
                    for (a, c) in &units {
                        apply_unit(p.as_ref(), *a, *c, due, &mut rejected);
                    }
                    if due && units.iter().any(|(_, c)| c.quick_level) {
                        leveled = std::time::Instant::now();
                    }
                    if rejected > 0 {
                        shared.lock().unwrap().rejected += rejected;
                    }
                } else {
                    vm = VmState::default();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        })
        .expect("starting the cheat thread");
    h
}

/// Where cheats are kept: Atmosphere `.txt` files and Cheat Engine `.CT` tables.
pub fn dir() -> Option<std::path::PathBuf> {
    crate::engine::run_dir().map(|d| d.join("cheats"))
}

/// The names of the codes switched on, per file, kept in `cheats/enabled.json`.
pub fn load_enabled(dir: &std::path::Path) -> HashMap<String, Vec<String>> {
    std::fs::read_to_string(dir.join("enabled.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save_enabled(dir: &std::path::Path, e: &HashMap<String, Vec<String>>) {
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(dir.join("enabled.json"), serde_json::to_string_pretty(e).unwrap_or_default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A flat memory for tests: the heap's first 64 KiB.
    struct Fake(Mutex<Vec<u8>>);
    impl Memory for Fake {
        fn read(&self, a: u64, b: &mut [u8]) -> bool {
            let m = self.0.lock().unwrap();
            let Some(o) = a.checked_sub(HEAP_BASE).map(|o| o as usize) else { return false };
            if o + b.len() > m.len() {
                return false;
            }
            b.copy_from_slice(&m[o..o + b.len()]);
            true
        }
        fn write(&self, a: u64, d: &[u8]) -> bool {
            let mut m = self.0.lock().unwrap();
            let Some(o) = a.checked_sub(HEAP_BASE).map(|o| o as usize) else { return false };
            if o + d.len() > m.len() {
                return false;
            }
            m[o..o + d.len()].copy_from_slice(d);
            true
        }
        fn regions(&self, lo: u64, hi: u64) -> Vec<(u64, u64)> {
            let (s, e) = (HEAP_BASE, HEAP_BASE + self.0.lock().unwrap().len() as u64);
            if e <= lo || s >= hi { vec![] } else { vec![(s.max(lo), e.min(hi))] }
        }
    }

    fn fake() -> Fake {
        Fake(Mutex::new(vec![0; 0x10000]))
    }

    #[test]
    fn static_store_and_pointer_chain() {
        let m = fake();
        // [heap+0x100] = heap+0x2000 (a pointer); then write 999 at [ptr]+0x10.
        m.write(HEAP_BASE + 0x100, &(HEAP_BASE + 0x2000).to_le_bytes());
        let codes = parse_codes(
            "[Gold]\n04100000 00000040 0000270F\n[Chain]\n58100000 00000100\n78000000 00000010\n64000000 00000000 000003E7\n",
        )
        .unwrap();
        assert_eq!(codes.len(), 2);
        let mut st = VmState::default();
        for c in &codes {
            run(&c.words, &mut st, &m, 0);
        }
        assert_eq!(rd(&m, HEAP_BASE + 0x40, 4), 9999);
        assert_eq!(rd(&m, HEAP_BASE + 0x2010, 4), 999);
    }

    #[test]
    fn conditionals_and_keys() {
        let m = fake();
        m.write(HEAP_BASE + 0x10, &5u32.to_le_bytes());
        // if [heap+0x10] == 5 { [heap+0x20] = 1 } else { [heap+0x20] = 2 }
        let c = parse_codes("[c]\n14150000 00000010 00000005\n04100000 00000020 00000001\n21000000\n04100000 00000020 00000002\n20000000\n").unwrap();
        run(&c[0].words, &mut VmState::default(), &m, 0);
        assert_eq!(rd(&m, HEAP_BASE + 0x20, 4), 1);
        m.write(HEAP_BASE + 0x10, &6u32.to_le_bytes());
        run(&c[0].words, &mut VmState::default(), &m, 0);
        assert_eq!(rd(&m, HEAP_BASE + 0x20, 4), 2);
        // while A is held, [heap+0x30] = 7
        let k = parse_codes("[k]\n80000001\n04100000 00000030 00000007\n20000000\n").unwrap();
        run(&k[0].words, &mut VmState::default(), &m, 0);
        assert_eq!(rd(&m, HEAP_BASE + 0x30, 4), 0);
        run(&k[0].words, &mut VmState::default(), &m, keys::A);
        assert_eq!(rd(&m, HEAP_BASE + 0x30, 4), 7);
    }

    #[test]
    fn loops_write_runs() {
        let m = fake();
        // r1 = heap+0x400; loop 4 { [r1] = 0x63 (1 byte), r1 += 1 }
        let c = parse_codes("[l]\n40010000 00000078 00000400\n30000000 00000004\n61011000 00000000 00000063\n31000000\n").unwrap();
        run(&c[0].words, &mut VmState::default(), &m, 0);
        let mut b = [0u8; 5];
        m.read(HEAP_BASE + 0x400, &mut b);
        assert_eq!(b, [0x63, 0x63, 0x63, 0x63, 0]);
    }

    #[test]
    fn ce_tables_resolve_and_scan() {
        let xml = r#"<?xml version="1.0"?><CheatTable><CheatEntries>
          <CheatEntry><ID>37</ID><Description>"Dietrich Stats"</Description><VariableType>Auto Assembler Script</VariableType>
            <AssemblerScript>[ENABLE]
aobscanregion(diet, 20000000000, 30000000000, 32 32 14 5A 50 00 00 00)
registersymbol(diet)
[DISABLE]
unregistersymbol(diet)
</AssemblerScript>
            <CheatEntries><CheatEntry><ID>93</ID><Description>"Base"</Description><VariableType>4 Bytes</VariableType><Address>diet</Address>
              <CheatEntries><CheatEntry><ID>45</ID><Description>"XP"</Description><VariableType>4 Bytes</VariableType><Address>+26+6</Address></CheatEntry>
              <CheatEntry><ID>51</ID><Description>"Str"</Description><VariableType>Byte</VariableType><Address>+A</Address></CheatEntry></CheatEntries>
            </CheatEntry></CheatEntries></CheatEntry>
        </CheatEntries></CheatTable>"#;
        let t = parse_table("t", xml).unwrap();
        let e = &t.entries[0];
        assert_eq!(e.description, "Dietrich Stats");
        assert_eq!(e.scans.len(), 1);
        assert!(!e.unsupported_script);
        let base = &e.children[0];
        assert_eq!(base.children[0].var, Some(VarType::Bytes4));

        let m = fake();
        m.write(HEAP_BASE + 0x1234, &[0x32, 0x32, 0x14, 0x5A, 0x50, 0, 0, 0]);
        let at = aob_scan(&m, GUEST_RANGE.0, GUEST_RANGE.1, &e.scans[0].pattern).unwrap();
        assert_eq!(at, HEAP_BASE + 0x1234);
        let mut syms = HashMap::new();
        syms.insert("diet".to_string(), at);
        let b = resolve(&base.address, None, &syms).unwrap();
        assert_eq!(resolve("+26+6", Some(b), &syms), Some(at + 0x2C));
        assert_eq!(resolve("DIET +44", None, &syms), Some(at + 0x44));
        assert_eq!(resolve("+A", None, &syms), None);
        assert_eq!(VarType::Bytes2.format(&[0xFF, 0xFF], true), "-1");
        assert_eq!(VarType::Byte.encode("200"), Some(vec![200]));
    }

    /// A record as the community tables read it (Cai's in the supplied table):
    /// growth 23 14 05 05 14, max HP and stats as bytes, HP u16, level, XP u32.
    fn cai_record() -> Vec<u8> {
        let mut r = vec![0u8; unit::LEN];
        r[..5].copy_from_slice(&[35, 20, 5, 5, 20]);
        r[unit::MAX_HP as usize] = 200;
        for off in unit::STAT_OFFSETS {
            r[off as usize] = 200;
        }
        r[unit::HP as usize..unit::HP as usize + 2].copy_from_slice(&99u16.to_le_bytes());
        r[unit::LEVEL as usize] = 17;
        r[0x2C..0x30].copy_from_slice(&1626u32.to_le_bytes());
        r
    }

    #[test]
    fn finds_unit_records() {
        let m = fake();
        // The tables' record for Cai sits at 0x14633BEB1CC: 4-byte aligned,
        // not 8, which the old every-8 scan missed.
        m.write(HEAP_BASE + 0x804, &cai_record());
        assert_eq!(scan_units(&m), vec![HEAP_BASE + 0x804]);
    }

    #[test]
    fn unit_cheats_hold_and_revive() {
        let m = fake();
        let at = HEAP_BASE + 0x900;
        let mut dead = cai_record();
        dead[unit::HP as usize..unit::HP as usize + 2].copy_from_slice(&0u16.to_le_bytes());
        m.write(at, &dead);
        assert!(looks_like_unit(&dead), "a unit at 0 HP is still a unit");
        assert_eq!(scan_units(&m), vec![at]);

        let mut rejected = 0;
        let c = UnitCheats { infinite_hp: true, quick_level: true, max_stats: true };
        apply_unit(&m, at, c, true, &mut rejected);
        assert_eq!(rd(&m, at + unit::MAX_HP, 1), 200);
        assert_eq!(rd(&m, at + unit::HP, 2), 200);
        assert_eq!(rd(&m, at + unit::LEVEL, 1), 18);
        assert_eq!(rejected, 0, "the fake accepts every write");

        // Maxed out, the level stops at the cap rather than running away.
        m.write(at + unit::LEVEL, &[unit::MAX_LEVEL]);
        apply_unit(&m, at, c, true, &mut rejected);
        assert_eq!(rd(&m, at + unit::LEVEL, 1), u64::from(unit::MAX_LEVEL));
    }

    #[test]
    fn aob_patterns() {
        assert_eq!(parse_aob("19 00 ?? 4E"), Some(vec![Some(0x19), Some(0), None, Some(0x4E)]));
        assert_eq!(parse_aob("19004E"), Some(vec![Some(0x19), Some(0), Some(0x4E)]));
        assert_eq!(parse_aob("zz"), None);
    }

    /// Attaches to a running game: its memory is where the cheats expect it.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn live_game_memory() {
        let pid = process::find(GAME_EXE).expect("start the game first");
        let p = process::Process::open(pid).expect("open the game");
        let regions = p.regions(GUEST_RANGE.0, GUEST_RANGE.1);
        let total: u64 = regions.iter().map(|(a, b)| b - a).sum();
        println!("{} regions, {} MiB in the guest range", regions.len(), total >> 20);
        let heap: u64 = regions.iter().filter(|(a, _)| *a >= HEAP_BASE && *a < HEAP_BASE + (1 << 36)).map(|(a, b)| b - a).sum();
        let main: u64 = regions.iter().filter(|(a, _)| *a >= MAIN_BASE && *a < HEAP_BASE).map(|(a, b)| b - a).sum();
        println!("heap {} MiB, main module data {} KiB", heap >> 20, main >> 10);
        assert!(heap > 0);
        let (a, _) = regions.iter().find(|(a, _)| *a >= HEAP_BASE).copied().unwrap();
        let mut b = [0u8; 16];
        assert!(p.read(a, &mut b));
        assert!(p.write(a, &b), "write back the same bytes");
        let t = std::time::Instant::now();
        let none = aob_scan_parallel(&p, GUEST_RANGE.0, GUEST_RANGE.1, &parse_aob("DE AD BE EF 13 37 C0 DE 55 AA").unwrap());
        println!("parallel scan {:?} in {:?}", none, t.elapsed());
        p.write(a, &[0xDE, 0xAD, 0xBE, 0xEF, 0x13, 0x37, 0xC0, 0xDE, 0x55, 0xAA]);
        let found = aob_scan_parallel(&p, GUEST_RANGE.0, GUEST_RANGE.1, &parse_aob("DE AD BE EF 13 37 C0 DE 55 AA").unwrap());
        p.write(a, &b);
        assert_eq!(found, Some(a));
    }

    /// Lists the unit records found in a running game.
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn live_units() {
        let pid = process::find(GAME_EXE).expect("start the game first");
        let p = process::Process::open(pid).unwrap();
        let t = std::time::Instant::now();
        let units = scan_units(&p);
        println!("{} unit records in {:?}", units.len(), t.elapsed());
        for a in units.iter().take(40) {
            let mut b = [0u8; unit::LEN];
            p.read(*a, &mut b);
            let u16s: Vec<u16> = (0..9).map(|i| u16::from_le_bytes([b[8 + 2 * i], b[9 + 2 * i]])).collect();
            println!("{a:#x} growth {:?} stats {:?} hp {} lv {} exp {}", &b[..5], u16s, u16::from_le_bytes([b[0x20], b[0x21]]), b[0x26], b[0x2C]);
        }
    }
}
