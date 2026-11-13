//! Fortune's Weave save slots: reading, editing and writing one slot file
//! (`Auto`, `Chapter_Ism_0_3`, ...).
//!
//! The container, as the game writes it:
//!
//! ```text
//! 0x000 u32 0xCDEFBCDD
//! 0x004 u32 length of the header checksum window (0xE0)
//! 0x008 u32 FNV-1a 32 over file[0x1F .. 0x1F + window]
//! 0x100 u32 file length - 0x104
//! 0x104 u32 0xCDEFBCDD
//! 0x108 u32 length of the zstd frame
//! 0x10C u32 FNV-1 32 over the decompressed payload
//! 0x110 u32 3 (zstd)
//! 0x124     zstd frame to the end of the file
//! ```
//!
//! The payload is the game's object tree. Its first u32 points (pointers are
//! `(v & 0xFFFFFF) - 32`) at a table of `(FNV-1 32 of the block name, pointer
//! to the block)` pairs; each block starts with a pointer to its end.
//! Game ids (people, classes, items) are stored as the low 32 bits of the
//! FNV-1 64 hash of their id string, such as `PID_カイ`. The strings come from
//! the player's own game data, so nothing of the game is shipped here.
//!
//! When EmblemWeave writes a slot it stores the payload in raw (uncompressed)
//! zstd blocks: a valid zstd frame that any decoder reads, so no compressor
//! is needed. The first time a slot is written, the untouched original is kept
//! beside it as `<slot>.orig`.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

const MAGIC: u32 = 0xCDEF_BCDD;
const HDR_FNV_START: usize = 0x1F;
const HDR_FNV_LEN: usize = 0xE0;
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

pub fn fnv1a32(data: &[u8]) -> u32 {
    data.iter().fold(0x811C_9DC5u32, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

pub fn fnv1_32(data: &[u8]) -> u32 {
    data.iter().fold(0x811C_9DC5u32, |h, &b| h.wrapping_mul(0x0100_0193) ^ b as u32)
}

pub fn fnv1_64(data: &[u8]) -> u64 {
    data.iter().fold(0xCBF2_9CE4_8422_2325u64, |h, &b| h.wrapping_mul(0x0000_0100_0000_01B3) ^ b as u64)
}

/// How the save stores a game id string.
pub fn id_hash(id: &str) -> u32 {
    fnv1_64(id.as_bytes()) as u32
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn ptr(v: u32) -> usize {
    ((v & 0x00FF_FFFF) as usize).wrapping_sub(32)
}

/// Named blocks of the payload, by the FNV-1 32 of their name.
pub const BLOCK_NAMES: [&str; 20] = [
    "Header", "GameSound", "GameSchedule", "GameUserData", "HubData", "GmapSpotMgr", "GmapTroopMgr", "GameGmapData",
    "GameRestartData", "SubquestData", "SideStoryData", "Calendar", "RecommendData", "Random", "UnitPool", "RidePool",
    "LegionPool", "Transporter", "HubSeq", "Data",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub name: String,
    pub start: usize,
    pub end: usize,
}

/// One decoded slot.
#[derive(Debug, Clone)]
pub struct Slot {
    pub path: PathBuf,
    raw: Vec<u8>,
    zstd_at: usize,
    pub payload: Vec<u8>,
}

impl Slot {
    pub fn open(path: &Path) -> Result<Slot, String> {
        let raw = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
        Self::from_bytes(path, raw)
    }

    pub fn from_bytes(path: &Path, raw: Vec<u8>) -> Result<Slot, String> {
        if raw.len() < 0x124 || u32_at(&raw, 0) != Some(MAGIC) {
            return Err("not a Fortune's Weave save slot".into());
        }
        let zstd_at = raw.windows(4).position(|w| w == ZSTD_MAGIC).ok_or("the slot holds no zstd frame")?;
        let mut payload = Vec::new();
        ruzstd::StreamingDecoder::new(std::io::Cursor::new(&raw[zstd_at..]))
            .map_err(|e| e.to_string())?
            .read_to_end(&mut payload)
            .map_err(|e| format!("decompressing: {e}"))?;
        Ok(Slot { path: path.to_path_buf(), raw, zstd_at, payload })
    }

    /// Problems with the slot's checksums, empty when it is intact.
    pub fn verify(&self) -> Vec<String> {
        let mut out = Vec::new();
        let window = u32_at(&self.raw, 4).map(|w| w as usize).filter(|&w| w > 0).unwrap_or(HDR_FNV_LEN);
        if self.raw.get(HDR_FNV_START..HDR_FNV_START + window).map(fnv1a32) != u32_at(&self.raw, 8) {
            out.push("header checksum mismatch".into());
        }
        if u32_at(&self.raw, 0x10C) != Some(fnv1_32(&self.payload)) {
            out.push("payload checksum mismatch".into());
        }
        out
    }

    /// The slot file with the current payload and fresh checksums.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.raw[..self.zstd_at].to_vec();
        out.extend(raw_zstd_frame(&self.payload));
        let n = out.len();
        out[0x100..0x104].copy_from_slice(&((n - 0x104) as u32).to_le_bytes());
        out[0x104..0x108].copy_from_slice(&MAGIC.to_le_bytes());
        out[0x108..0x10C].copy_from_slice(&((n - self.zstd_at) as u32).to_le_bytes());
        out[0x10C..0x110].copy_from_slice(&fnv1_32(&self.payload).to_le_bytes());
        out[0x110..0x114].copy_from_slice(&3u32.to_le_bytes());
        let window = u32_at(&out, 4).map(|w| w as usize).filter(|&w| w > 0).unwrap_or(HDR_FNV_LEN);
        let sum = fnv1a32(&out[HDR_FNV_START..HDR_FNV_START + window]);
        out[8..12].copy_from_slice(&sum.to_le_bytes());
        out
    }

    /// Write the slot back. The original file is kept once as `<slot>.orig`;
    /// the new file is written beside the slot and swapped in.
    pub fn save(&mut self) -> Result<(), String> {
        let orig = with_suffix(&self.path, ".orig");
        if self.path.is_file() && !orig.exists() {
            std::fs::copy(&self.path, &orig).map_err(|e| format!("keeping the original: {e}"))?;
        }
        let bytes = self.to_bytes();
        let tmp = with_suffix(&self.path, ".tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("replacing {}: {e}", self.path.display()))?;
        self.zstd_at = self.zstd_at.min(bytes.len());
        self.raw = bytes;
        Ok(())
    }

    pub fn blocks(&self) -> Vec<Block> {
        let p = &self.payload;
        let Some(table) = u32_at(p, 0).map(ptr) else { return Vec::new() };
        if table == 0 || table >= p.len() {
            return Vec::new();
        }
        let names: HashMap<u32, &str> = BLOCK_NAMES.iter().map(|n| (fnv1_32(n.as_bytes()), *n)).collect();
        let mut out = Vec::new();
        let mut i = table;
        while i + 8 <= p.len() {
            let (h, s) = (u32_at(p, i).unwrap(), ptr(u32_at(p, i + 4).unwrap()));
            if (4..p.len().saturating_sub(4)).contains(&s) {
                let e = ptr(u32_at(p, s).unwrap());
                if s < e && e <= p.len() {
                    out.push(Block { name: names.get(&h).map(|n| n.to_string()).unwrap_or(format!("#{h:08X}")), start: s, end: e });
                }
            }
            i += 8;
        }
        out.sort_by_key(|b| b.start);
        out
    }

    pub fn block(&self, name: &str) -> Option<Block> {
        self.blocks().into_iter().find(|b| b.name == name)
    }

    /// Offset of the player data (gold and the like), when the slot has it.
    fn user_data(&self) -> Option<usize> {
        let b = self.block("GameUserData")?;
        (self.payload.get(b.start + 4) == Some(&23) && b.start + 4 + 19 <= self.payload.len()).then_some(b.start + 4)
    }

    pub fn player(&self) -> Option<Player> {
        let at = self.user_data()?;
        Some(Player { gold: u32_at(&self.payload, at + 6)?, renown: u32_at(&self.payload, at + 10)? })
    }

    pub fn set_player(&mut self, p: &Player) -> bool {
        let Some(at) = self.user_data() else { return false };
        self.payload[at + 6..at + 10].copy_from_slice(&p.gold.to_le_bytes());
        self.payload[at + 10..at + 14].copy_from_slice(&p.renown.to_le_bytes());
        true
    }

    /// The units recorded in the slot: every place a known person id is
    /// followed by a known class id and a stat block. A unit can appear more
    /// than once (the party, a restart copy); each copy is listed.
    pub fn units(&self, ids: &Ids) -> Vec<Unit> {
        let p = &self.payload;
        let mut out = Vec::new();
        if ids.people.is_empty() {
            return out;
        }
        for o in 0..p.len().saturating_sub(80) {
            let person = u32_at(p, o).unwrap();
            let Some(name) = ids.people.get(&person) else { continue };
            let class = u32_at(p, o + 4).unwrap();
            if !ids.classes.is_empty() && !ids.classes.contains_key(&class) {
                continue;
            }
            if let Some(mut u) = parse_unit(p, o, name, class) {
                if let Some((table, m)) = find_masteries(p, u.level_at + 5) {
                    let after = table + 1 + 3 * m.len();
                    u.masteries = m;
                    if let Some((items, spells)) = find_items(p, after, &ids.items) {
                        u.items = items;
                        u.spells = spells;
                    }
                }
                out.push(u);
            }
        }
        out
    }

    pub fn write_unit(&mut self, u: &Unit) {
        let p = &mut self.payload;
        for (i, v) in u.stats.iter().enumerate() {
            p[u.stats_at + 2 * i..u.stats_at + 2 * i + 2].copy_from_slice(&v.to_le_bytes());
        }
        p[u.level_at] = u.level;
        for &(o, _, v) in &u.masteries {
            p[o..o + 2].copy_from_slice(&v.to_le_bytes());
        }
        for &(o, id) in u.items.iter().chain(&u.spells) {
            p[o..o + 4].copy_from_slice(&id.to_le_bytes());
        }
        let total = (u.level.max(1) as u16 - 1) * 100 + u.exp.min(99) as u16;
        p[u.level_at + 1..u.level_at + 3].copy_from_slice(&total.to_le_bytes());
        match u.hp_width {
            4 => p[u.level_at + 5..u.level_at + 9].copy_from_slice(&(u.max_hp as u32).to_le_bytes()),
            2 => p[u.level_at + 5..u.level_at + 7].copy_from_slice(&u.max_hp.to_le_bytes()),
            _ => p[u.level_at + 5] = u.max_hp.min(255) as u8,
        }
    }
}

fn with_suffix(p: &Path, s: &str) -> PathBuf {
    let mut o = p.as_os_str().to_owned();
    o.push(s);
    PathBuf::from(o)
}

/// A zstd frame holding `data` in raw blocks.
pub fn raw_zstd_frame(data: &[u8]) -> Vec<u8> {
    let mut out = ZSTD_MAGIC.to_vec();
    // Single segment, 4-byte content size, no checksum, no dictionary.
    out.push(0b1010_0000);
    out.extend((data.len() as u32).to_le_bytes());
    const MAX: usize = 128 * 1024;
    let mut chunks = data.chunks(MAX).peekable();
    if data.is_empty() {
        out.extend([1, 0, 0]);
    }
    while let Some(c) = chunks.next() {
        let last = chunks.peek().is_none() as u32;
        let h = last | ((c.len() as u32) << 3);
        out.extend(&h.to_le_bytes()[..3]);
        out.extend(c);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Player {
    pub gold: u32,
    pub renown: u32,
}

pub const STAT_NAMES: [&str; 9] = ["HP", "Str", "Mag", "Dex", "Spd", "Lck", "Def", "Res", "Cha"];

/// One copy of a unit's record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    pub at: usize,
    pub name: String,
    pub class: u32,
    pub stats_at: usize,
    /// What each stat has gained from level-ups (the game adds the class and
    /// personal bases on top), current HP first.
    pub stats: [u16; 9],
    pub level_at: usize,
    pub level: u8,
    /// Experience into the current level (0-99).
    pub exp: u8,
    pub max_hp: u16,
    pub hp_width: u8,
    /// Weapon and movement masteries: (offset of the value, key, cumulative
    /// experience). Keys are [`MASTERY_NAMES`].
    pub masteries: Vec<(usize, u8, u16)>,
    /// Carried items, then known spells: (offset of the item id, item id).
    pub items: Vec<(usize, u32)>,
    pub spells: Vec<(usize, u32)>,
}

/// Mastery keys, in the order the game stores them.
pub const MASTERY_NAMES: [&str; 13] = [
    "Sword", "Lance", "Axe", "Bow", "Fist", "Black magic", "White magic", "Other (7)", "Infantry", "Horse", "Armor", "Wing", "Flag",
];

/// Experience at which each mastery grade starts.
pub const GRADES: [(&str, u16); 12] =
    [("E", 0), ("E+", 20), ("D", 35), ("D+", 80), ("C", 220), ("C+", 440), ("B", 690), ("B+", 910), ("A", 1215), ("A+", 1510), ("S", 1960), ("S+", 2560)];

pub fn grade(exp: u16) -> &'static str {
    GRADES.iter().rev().find(|(_, t)| exp >= *t).map(|(g, _)| *g).unwrap_or("E")
}

/// The mastery table after a record's HP: a count byte, then (u8 key, u16
/// cumulative experience) entries with rising keys. The gap varies between
/// copies, so the longest table found in a short window wins.
fn find_masteries(p: &[u8], hp_at: usize) -> Option<(usize, Vec<(usize, u8, u16)>)> {
    let mut best: Option<(usize, Vec<(usize, u8, u16)>)> = None;
    for off in hp_at + 4..hp_at + 24 {
        let Some(&n) = p.get(off) else { break };
        if !(1..=13).contains(&n) || off + 1 + 3 * n as usize > p.len() {
            continue;
        }
        let mut v = Vec::new();
        let mut last: i32 = -1;
        for i in 0..n as usize {
            let o = off + 1 + 3 * i;
            let (k, val) = (p[o], u16::from_le_bytes([p[o + 1], p[o + 2]]));
            if k > 12 || (k as i32) <= last || val > 9999 {
                v.clear();
                break;
            }
            last = k as i32;
            v.push((o + 1, k, val));
        }
        if !v.is_empty() && best.as_ref().is_none_or(|(_, b)| v.len() > b.len()) {
            best = Some((off, v));
        }
    }
    best
}

/// The carried items and spells that follow the mastery table: a u32 marker,
/// a zero byte, the item and spell counts, then entries tagged 0x41 (an item
/// id) or 0x61 (an item id and 8 bytes of state, such as uses).
fn find_items(p: &[u8], from: usize, ids: &HashMap<u32, String>) -> Option<(Vec<(usize, u32)>, Vec<(usize, u32)>)> {
    for h in from..(from + 0x80).min(p.len().saturating_sub(7)) {
        let marker = u32_at(p, h)?;
        if !(1..0x0100_0000).contains(&marker) || p[h + 4] != 0 {
            continue;
        }
        let (ni, ns) = (p[h + 5] as usize, p[h + 6] as usize);
        if ni + ns == 0 || ni > 32 || ns > 64 {
            continue;
        }
        let mut o = h + 7;
        let mut all = Vec::new();
        let mut ok = true;
        for _ in 0..ni + ns {
            let size = match p.get(o) {
                Some(0x41) => 5,
                Some(0x61) => 13,
                _ => {
                    ok = false;
                    break;
                }
            };
            match u32_at(p, o + 1) {
                Some(id) if ids.contains_key(&id) => all.push((o + 1, id)),
                _ => {
                    ok = false;
                    break;
                }
            }
            o += size;
        }
        if ok {
            let spells = all.split_off(ni);
            return Some((all, spells));
        }
    }
    None
}

/// The level byte of a record: level 1-99, a zero byte at +4, then max HP as
/// a u32, a u16 or a byte. Returns HP and its width.
fn level_sig(p: &[u8], b: usize) -> Option<(u16, u8)> {
    if b + 9 > p.len() || !(1..=99).contains(&p[b]) || p[b + 4] != 0 {
        return None;
    }
    let hp32 = u32_at(p, b + 5)?;
    if (5..=999).contains(&hp32) {
        return Some((hp32 as u16, 4));
    }
    let hp16 = u16::from_le_bytes([p[b + 5], p[b + 6]]);
    if (5..=999).contains(&hp16) && p[b + 7] == 0 && p[b + 8] == 0 {
        return Some((hp16, 2));
    }
    (p[b + 5] >= 5 && p[b + 6] >= 5).then_some((p[b + 5] as u16, 1))
}

fn parse_unit(p: &[u8], o: usize, name: &str, class: u32) -> Option<Unit> {
    // The nine u16 stats, then up to four zero bytes, then the level byte, a
    // little past the person and class ids.
    for s in o + 16..o + 56 {
        let stats: Vec<u16> = (0..9).map(|i| u16::from_le_bytes([p[s + 2 * i], p[s + 2 * i + 1]])).collect();
        if stats.iter().any(|&v| v > 255) {
            continue;
        }
        for gap in 0..=4 {
            let b = s + 18 + gap;
            if p[s + 18..b].iter().any(|&z| z != 0) {
                break;
            }
            if let Some((hp, w)) = level_sig(p, b) {
                let level = p[b];
                let raw = u16::from_le_bytes([p[b + 1], p[b + 2]]);
                let into = raw as i32 - (level as i32 - 1) * 100;
                let exp = if (0..=99).contains(&into) { into as u8 } else { (raw % 100) as u8 };
                return Some(Unit {
                    at: o,
                    name: name.to_string(),
                    class,
                    stats_at: s,
                    stats: stats.try_into().ok()?,
                    level_at: b,
                    level,
                    exp,
                    max_hp: hp,
                    hp_width: w,
                    masteries: Vec::new(),
                    items: Vec::new(),
                    spells: Vec::new(),
                });
            }
        }
    }
    None
}

/// Game ids by hash, read from the player's RomFS `GameData`.
#[derive(Debug, Default, Clone)]
pub struct Ids {
    pub people: HashMap<u32, String>,
    pub classes: HashMap<u32, String>,
    pub items: HashMap<u32, String>,
}

impl Ids {
    /// Collect `PID_`, `JID_` and `IID_` strings from `GameData/*.bin`.
    pub fn from_romfs(romfs: &Path) -> Ids {
        let mut ids = Ids::default();
        let Ok(dir) = std::fs::read_dir(romfs.join("GameData")) else { return ids };
        for e in dir.flatten() {
            let path = e.path();
            if path.extension().and_then(|x| x.to_str()) != Some("bin") {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            ids.harvest(&bytes);
        }
        ids
    }

    pub fn harvest(&mut self, bytes: &[u8]) {
        for (prefix, which) in [(&b"PID_"[..], 0), (b"JID_", 1), (b"IID_", 2)] {
            let mut i = 0;
            while let Some(k) = bytes[i..].windows(4).position(|w| w == prefix) {
                let start = i + k;
                let end = bytes[start..].iter().position(|&c| c == 0 || c < 0x20).map_or(bytes.len(), |n| start + n);
                if let Ok(s) = std::str::from_utf8(&bytes[start..end]) {
                    if s.len() > 4 && s.len() < 96 {
                        let map = match which {
                            0 => &mut self.people,
                            1 => &mut self.classes,
                            _ => &mut self.items,
                        };
                        map.entry(id_hash(s)).or_insert_with(|| s[4..].to_string());
                    }
                }
                i = end.max(start + 4);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_frames_decode() {
        for n in [0usize, 1, 1000, 128 * 1024, 300_000] {
            let data: Vec<u8> = (0..n).map(|i| (i * 7 % 251) as u8).collect();
            let frame = raw_zstd_frame(&data);
            let mut out = Vec::new();
            ruzstd::StreamingDecoder::new(std::io::Cursor::new(&frame[..])).unwrap().read_to_end(&mut out).unwrap();
            assert_eq!(out, data, "{n} bytes");
        }
    }

    #[test]
    fn hashes_match_the_format() {
        assert_eq!(fnv1_32(b""), 0x811C_9DC5);
        assert_eq!(fnv1a32(b"a"), 0xE40C_292C);
        assert_eq!(fnv1_64(b"a"), 0xAF63_BD4C_8601_B7BE);
    }

    #[test]
    fn harvests_ids() {
        let mut ids = Ids::default();
        ids.harvest("..PID_カイ\0JID_剣士\0IID_鉄の剣\0".as_bytes());
        assert_eq!(ids.people.get(&id_hash("PID_カイ")).map(String::as_str), Some("カイ"));
        assert_eq!(ids.classes.len(), 1);
        assert_eq!(ids.items.len(), 1);
    }

    /// Reads the real slots when they are on this PC: they decode, verify,
    /// and survive a write unchanged apart from the frame.
    #[test]
    fn real_slots_round_trip() {
        let Some(dir) = crate::config::Config::save_dir() else { return };
        let dir = crate::saves::account_dir(&dir).join("000");
        let Ok(entries) = std::fs::read_dir(&dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().is_some() {
                continue;
            }
            let Ok(slot) = Slot::open(&p) else { continue };
            assert!(slot.verify().is_empty(), "{}: {:?}", p.display(), slot.verify());
            let again = Slot::from_bytes(&p, slot.to_bytes()).unwrap();
            assert_eq!(again.payload, slot.payload);
            assert!(again.verify().is_empty());
        }
    }
}


#[cfg(test)]
mod edits {
    /// Edits a copy of the real Auto slot, when it is here: a mastery, an
    /// item and gold survive a save and a reopen, with valid checksums.
    #[test]
    fn edits_survive_a_save() {
        let romfs = std::path::Path::new("E:/FEFW/romfs");
        let Some(save) = crate::config::Config::save_dir() else { return };
        let auto = crate::saves::account_dir(&save).join("000").join("Auto");
        if !auto.is_file() || !romfs.is_dir() {
            return;
        }
        let ids = super::Ids::from_romfs(romfs);
        let dir = std::env::temp_dir().join(format!("ew-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let copy = dir.join("Auto");
        std::fs::copy(&auto, &copy).unwrap();
        let mut s = super::Slot::open(&copy).unwrap();
        let mut u = s.units(&ids).into_iter().find(|u| !u.masteries.is_empty() && u.items.len() >= 2).expect("a unit with masteries and two items");
        let other = u.items[1].1;
        u.masteries[0].2 = 2560;
        u.items[0].1 = other;
        s.write_unit(&u);
        let mut pl = s.player().unwrap();
        pl.gold = 123_456;
        s.set_player(&pl);
        s.save().unwrap();
        assert!(dir.join("Auto.orig").is_file());
        let again = super::Slot::open(&copy).unwrap();
        assert!(again.verify().is_empty(), "{:?}", again.verify());
        assert_eq!(again.player().unwrap().gold, 123_456);
        let v = again.units(&ids).into_iter().find(|x| x.at == u.at).unwrap();
        assert_eq!(v.masteries[0].2, 2560);
        assert_eq!(v.items[0].1, other);
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod probe {
    /// Prints what the editor finds in the real Auto slot, when it is here.
    #[test]
    #[ignore]
    fn print_units() {
        let ids = super::Ids::from_romfs(std::path::Path::new("E:/FEFW/romfs"));
        let dir = crate::saves::account_dir(&crate::config::Config::save_dir().unwrap()).join("000");
        let s = super::Slot::open(&dir.join("Auto")).unwrap();
        for u in s.units(&ids) {
            println!("{} class {} lv{} hp{}", u.name, ids.classes.get(&u.class).map(String::as_str).unwrap_or("?"), u.level, u.max_hp);
            println!("  masteries {:?}", u.masteries.iter().map(|(_, k, v)| format!("{}={v}({})", super::MASTERY_NAMES[*k as usize], super::grade(*v))).collect::<Vec<_>>());
            println!("  items {:?}", u.items.iter().map(|(_, i)| ids.items.get(i).cloned().unwrap_or_default()).collect::<Vec<_>>());
            println!("  spells {:?}", u.spells.iter().map(|(_, i)| ids.items.get(i).cloned().unwrap_or_default()).collect::<Vec<_>>());
        }
    }
}
