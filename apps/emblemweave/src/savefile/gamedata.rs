//! Game-data names for the save editor.
//!
//! The save stores hashes of the game's Japanese person, class, item and skill
//! ids.  The display text lives beside the game in `Msg/<language>/*.msbt.zst`;
//! the Japanese and English files use the same labels, so pairing the two gives
//! the official English name for an id without shipping a copy of the game.
//!
//! The generic unit ids are assembled from words (`PID_SCXX_02_アーチャー`,
//! `DN_アトラスベア_力強化_冥界`).  Those are translated part by part using the
//! same message files and the small glossary that the community editor uses.
//! The generic-id rules follow Antiadvocate/fefw-editor's `build_unit_names.py`.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};

const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// All translated names read from a RomFS.  The four maps are separate because
/// the same Japanese word can be a person in one context and a class in another.
#[derive(Debug, Default, Clone)]
pub struct Texts {
    pub person: HashMap<String, String>,
    pub job: HashMap<String, String>,
    pub item: HashMap<String, String>,
    pub skill: HashMap<String, String>,
    /// Every short message pair, with Person and Job first. Generic unit ids
    /// are made from words that also appear outside the four name tables.
    pub names: HashMap<String, String>,
}

impl Texts {
    pub fn load(romfs: &Path) -> Texts {
        let jp_dir = romfs.join("Msg").join("JPja");
        let en_dir = ["EUen", "USen"]
            .iter()
            .map(|n| romfs.join("Msg").join(n))
            .find(|p| p.is_dir());
        let Some(en_dir) = en_dir else {
            return Texts::default();
        };
        let mut stems: Vec<String> = Vec::new();
        if let Ok(dir) = std::fs::read_dir(&jp_dir) {
            for e in dir.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if let Some(stem) = name.split(".msbt").next() {
                    if !stem.is_empty() && stem != name {
                        stems.push(stem.to_string());
                    }
                }
            }
        }
        stems.sort_by_key(|s| stem_priority(s));
        stems.dedup();
        let mut jp = Vec::new();
        let mut en = Vec::new();
        for stem in stems {
            if let Some(texts) = message_file(&jp_dir, &stem).and_then(|p| read_msbt(&p).ok()) {
                jp.push((stem.clone(), texts));
            }
            if let Some(texts) = message_file(&en_dir, &stem).and_then(|p| read_msbt(&p).ok()) {
                en.push((stem, texts));
            }
        }
        Texts::from_maps(jp, en)
    }

    /// Build the maps from message tables that have already been parsed.
    /// A RomFS folder and a `.nup` package both feed this, so the label pairing
    /// and the priority rules are the same for either source.
    pub fn from_maps(
        jp: Vec<(String, HashMap<String, String>)>,
        en: Vec<(String, HashMap<String, String>)>,
    ) -> Texts {
        let mut en_by_stem: HashMap<String, HashMap<String, String>> = HashMap::new();
        for (stem, texts) in en {
            en_by_stem.entry(stem).or_insert(texts);
        }
        let mut jp = jp;
        jp.sort_by(|a, b| {
            stem_priority(&a.0)
                .cmp(&stem_priority(&b.0))
                .then(a.0.cmp(&b.0))
        });
        jp.dedup_by(|a, b| a.0 == b.0);
        let mut out = Texts::default();
        for (stem, jp) in jp {
            let Some(en) = en_by_stem.remove(&stem) else { continue };
            let pairs = pair_maps(jp, en);
            match stem.as_str() {
                "Person" => out.person = pairs.clone(),
                "Job" => out.job = pairs.clone(),
                "Item" => out.item = pairs.clone(),
                "Skill" => out.skill = pairs.clone(),
                _ => {}
            }
            for (jp, en) in pairs {
                if jp.chars().count() <= 16 && en.chars().count() <= 40 {
                    out.names.entry(jp).or_insert(en);
                }
            }
        }
        out
    }

    pub fn loaded(&self) -> bool {
        !self.names.is_empty()
    }

    fn lookup(&self, text: &str) -> Option<&str> {
        self.person
            .get(text)
            .or_else(|| self.job.get(text))
            .or_else(|| self.names.get(text))
            .map(String::as_str)
    }
}

/// Official English name for a stored person id (the part after `PID_`).
pub fn person_name(raw: &str, texts: &Texts) -> String {
    let cleaned = strip_id_prefix(raw);
    if let Some(name) = english(&cleaned, texts) {
        return name;
    }
    if !has_cjk(&cleaned) {
        return pretty_ascii(&cleaned);
    }
    cleaned
}

/// Official English name for a stored class id (the part after `JID_`).
pub fn class_name(raw: &str, texts: &Texts) -> String {
    let s = raw.trim();
    let s = s.strip_prefix("H_").unwrap_or(s);
    if let Some(name) = english(s, texts) {
        return name;
    }
    if let Some(v) = texts.job.get(s) {
        return first_line(v);
    }
    if let Some(v) = texts.names.get(s) {
        return first_line(v);
    }
    if let Some(v) = glossary(s) {
        return v.to_string();
    }
    if !has_cjk(s) {
        return pretty_ascii(s);
    }
    s.to_string()
}

/// Official English name for a stored item id (the part after `IID_`).
pub fn item_name(raw: &str, texts: &Texts) -> String {
    let s = raw.trim();
    if let Some(v) = texts.item.get(s) {
        return first_line(v);
    }
    if let Some(v) = texts.names.get(s) {
        return first_line(v);
    }
    let base = strip_chapter_suffix(s);
    let base = base.strip_prefix("デモ_").unwrap_or(base);
    let base = base.strip_suffix("_戦技なし").unwrap_or(base);
    let base = base.strip_suffix("_チュートリアル").unwrap_or(base);
    if let Some(v) = texts.item.get(base) {
        return first_line(v);
    }
    if let Some(v) = texts.names.get(base) {
        return first_line(v);
    }
    let (base, plus) = split_forge(base);
    if let Some(v) = texts.item.get(base) {
        let v = first_line(v);
        return match plus {
            Some(n) if !v.ends_with(&format!("+{n}")) => format!("{v} +{n}"),
            _ => v,
        };
    }
    if let Some(v) = texts.names.get(base) {
        let v = first_line(v);
        return match plus {
            Some(n) if !v.ends_with(&format!("+{n}")) => format!("{v} +{n}"),
            _ => v,
        };
    }
    if !has_cjk(s) {
        return pretty_ascii(s);
    }
    s.to_string()
}

/// Official English name for a stored skill id (the part after `SID_`).
pub fn skill_name(raw: &str, texts: &Texts) -> String {
    let s = raw.trim();
    if let Some(v) = texts.skill.get(s) {
        return first_line(v);
    }
    if let Some(v) = texts.names.get(s) {
        return first_line(v);
    }
    if !has_cjk(s) {
        return pretty_ascii(s);
    }
    s.to_string()
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

/// The save stores only the part after the id prefix.  Be liberal so the same
/// helper also works if a caller passes the full game id.
fn strip_id_prefix(raw: &str) -> String {
    let s = raw.trim();
    let s = s
        .strip_prefix("PID_")
        .or_else(|| s.strip_prefix("JID_"))
        .or_else(|| s.strip_prefix("IID_"))
        .or_else(|| s.strip_prefix("SID_"))
        .unwrap_or(s);
    s.to_string()
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        matches!(c,
            '\u{2e80}'..='\u{2eff}'
            | '\u{3040}'..='\u{30ff}'
            | '\u{3400}'..='\u{9fff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{ff00}'..='\u{ffef}')
    })
}

fn pretty_ascii(s: &str) -> String {
    let s = s.strip_prefix("H_").unwrap_or(s);
    let mut out = String::new();
    for token in s.split('_') {
        if token.is_empty() {
            continue;
        }
        let mut prev: Option<char> = None;
        for c in token.chars() {
            if let Some(p) = prev {
                if c.is_ascii_uppercase()
                    && (p.is_ascii_lowercase() || p.is_ascii_digit())
                    && !out.ends_with(' ')
                {
                    out.push(' ');
                }
            }
            out.push(c);
            prev = Some(c);
        }
        out.push(' ');
    }
    out.trim().to_string()
}

/// `…_SC18_13`, `…_SCXX_02` and the other per-chapter copies use the base item.
fn strip_chapter_suffix(s: &str) -> &str {
    if let Some(i) = s.rfind("_SC") {
        let tail = &s[i + 3..];
        if !tail.is_empty()
            && tail
                .split('_')
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit() || c == 'X'))
        {
            return &s[..i];
        }
    }
    if let Some(i) = s.rfind("_BD") {
        let tail = &s[i + 3..];
        if !tail.is_empty()
            && tail
                .split('_')
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit() || c == 'X'))
        {
            return &s[..i];
        }
    }
    s
}

/// Split `木製の剣＋２` into its base and forge level.
fn split_forge(s: &str) -> (&str, Option<u32>) {
    let cut = s
        .char_indices()
        .rev()
        .find(|(_, c)| *c == '＋' || *c == '+')
        .map(|(i, _)| i);
    let Some(cut) = cut else { return (s, None) };
    let digits: String = s[cut..]
        .chars()
        .skip(1)
        .filter(|c| c.is_ascii_digit() || ('０'..='９').contains(c))
        .map(|c| {
            if ('０'..='９').contains(&c) {
                char::from_u32(c as u32 - '０' as u32 + '0' as u32).unwrap_or(c)
            } else {
                c
            }
        })
        .collect();
    match digits.parse::<u32>() {
        Ok(n) => (&s[..cut], Some(n)),
        Err(_) => (s, None),
    }
}

// ---------------------------------------------------------------------------
// MSBT

fn message_file(dir: &Path, stem: &str) -> Option<PathBuf> {
    for ext in ["msbt.zst", "msbt"] {
        let p = dir.join(format!("{stem}.{ext}"));
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn stem_priority(stem: &str) -> u8 {
    match stem {
        "Person" => 0,
        "Job" => 1,
        "Item" => 2,
        "Skill" => 3,
        _ => 4,
    }
}

fn pair_maps(
    jp: HashMap<String, String>,
    en: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (label, jp_text) in jp {
        if label.contains("_H_") {
            continue;
        }
        let Some(en_text) = en.get(&label) else {
            continue;
        };
        let jp_text = jp_text.trim();
        let en_text = first_line(en_text);
        if !jp_text.is_empty() && !en_text.is_empty() {
            out.entry(jp_text.to_string()).or_insert(en_text);
        }
    }
    out
}

fn read_bytes(path: &Path) -> Result<Vec<u8>, String> {
    let raw = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    if path.extension().and_then(|x| x.to_str()) == Some("zst") || raw.starts_with(&ZSTD_MAGIC) {
        let mut out = Vec::new();
        ruzstd::StreamingDecoder::new(std::io::Cursor::new(&raw))
            .map_err(|e| e.to_string())?
            .read_to_end(&mut out)
            .map_err(|e| format!("decompressing {}: {e}", path.display()))?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn read_msbt(path: &Path) -> Result<HashMap<String, String>, String> {
    read_msbt_bytes(&read_bytes(path)?)
}

/// The text table of one message file, from the bytes as they sit in the game
/// data: a plain `.msbt`, or a `.msbt.zst` frame. A `.nup` hands its entries
/// over this way, so a package and an extracted folder share one parser.
pub fn message_text(raw: &[u8]) -> HashMap<String, String> {
    let owned = if raw.starts_with(&ZSTD_MAGIC) {
        let mut out = Vec::new();
        let Ok(mut decoder) = ruzstd::StreamingDecoder::new(std::io::Cursor::new(raw)) else {
            return HashMap::new();
        };
        if decoder.read_to_end(&mut out).is_err() {
            return HashMap::new();
        }
        Some(out)
    } else {
        None
    };
    read_msbt_bytes(owned.as_deref().unwrap_or(raw)).unwrap_or_default()
}

fn read_msbt_bytes(raw: &[u8]) -> Result<HashMap<String, String>, String> {
    let utf16 = raw.get(0x0C..0x0E) == Some(&[0xFF, 0xFE]);
    let (Some(lbl), Some(txt)) = (find(&raw, b"LBL1"), find(&raw, b"TXT2")) else {
        return Ok(HashMap::new());
    };

    let body = lbl + 16;
    let buckets = u32_at(&raw, body).ok_or("LBL1 has no bucket count")? as usize;
    if buckets > 100_000 {
        return Err("LBL1 bucket count is not sane".into());
    }
    let mut labels: HashMap<String, usize> = HashMap::new();
    for k in 0..buckets {
        let head = body + 4 + 8 * k;
        let (Some(count), Some(off)) = (u32_at(&raw, head), u32_at(&raw, head + 4)) else {
            break;
        };
        let mut at = body + off as usize;
        for _ in 0..count {
            let Some(&len) = raw.get(at) else { break };
            let len = len as usize;
            if at + 1 + len + 4 > raw.len() {
                break;
            }
            let name = String::from_utf8_lossy(&raw[at + 1..at + 1 + len]).into_owned();
            if let Some(index) = u32_at(&raw, at + 1 + len) {
                labels.insert(name, index as usize);
            }
            at += 1 + len + 4;
        }
    }

    let tbody = txt + 16;
    let count = u32_at(&raw, tbody).ok_or("TXT2 has no text count")? as usize;
    if count > 100_000 {
        return Err("TXT2 text count is not sane".into());
    }
    let mut offsets = Vec::with_capacity(count);
    for k in 0..count {
        let Some(off) = u32_at(&raw, tbody + 4 + 4 * k) else {
            break;
        };
        offsets.push(off as usize);
    }
    let section_size = u32_at(&raw, txt + 4).unwrap_or(0) as usize;
    let section_end = (txt + 4 + section_size).min(raw.len());

    let mut texts = Vec::with_capacity(offsets.len());
    for (k, off) in offsets.iter().enumerate() {
        let start = tbody.saturating_add(*off);
        let end = offsets
            .get(k + 1)
            .map(|n| tbody.saturating_add(*n))
            .unwrap_or(section_end)
            .min(raw.len());
        let Some(slice) = raw.get(start..end) else {
            texts.push(String::new());
            continue;
        };
        let mut s = if utf16 {
            let mut units = Vec::with_capacity(slice.len() / 2);
            for c in slice.chunks_exact(2) {
                units.push(u16::from_le_bytes([c[0], c[1]]));
            }
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(slice).into_owned()
        };
        s = s.split('\0').next().unwrap_or("").to_string();
        texts.push(strip_tags(&s).trim().to_string());
    }

    Ok(labels
        .into_iter()
        .filter_map(|(name, index)| texts.get(index).map(|t| (name, t.clone())))
        .collect())
}

/// Control tags are `\x0e group type size-in-bytes parameters…` and
/// `\x0f group type`.  Furigana is a tag too, so the kanji after it remains.
fn strip_tags(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\u{e}' {
            // A full tag carries a group, type and byte length. A short tag
            // (`\x0e\x01`) has no length and can sit at the end of a name.
            i += if i + 3 < chars.len() {
                4 + (chars[i + 3] as usize / 2)
            } else {
                2.min(chars.len() - i)
            };
            continue;
        }
        if chars[i] == '\u{f}' {
            i += 3.min(chars.len() - i);
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Generic unit ids

const GLOSSARY: &[(&str, &str)] = &[
    ("男", "male"),
    ("女", "female"),
    ("標準", ""),
    ("汎用", ""),
    ("冥界", "Underworld"),
    ("冥", "Underworld"),
    ("強", "strong"),
    ("弱", "weak"),
    ("中", "medium"),
    ("最強", "strongest"),
    ("ボス", "boss"),
    ("右", "right"),
    ("左", "left"),
    ("攻撃", "attack"),
    ("防御", "defense"),
    ("魔法", "magic"),
    ("物理", "physical"),
    ("力強化", "Str+"),
    ("技強化", "Dex+"),
    ("速さ強化", "Spd+"),
    ("守備強化", "Def+"),
    ("魔力強化", "Mag+"),
    ("魔防強化", "Res+"),
    ("幸運強化", "Lck+"),
    ("HP強化", "HP+"),
    ("守備劣化", "Def-"),
    ("速さ劣化", "Spd-"),
    ("魔防劣化", "Res-"),
    ("力劣化", "Str-"),
    ("必殺無効", "crit-immune"),
    ("回避", "evasive"),
    ("計略", "schemer"),
    ("アサシン", "assassin"),
    ("アサシン型", "assassin"),
    ("外套", "cloaked"),
    ("外套あり", "cloaked"),
    ("霊体", "spirit"),
    ("護衛", "escort"),
    ("0部", "Prologue"),
    ("1部", "Part 1"),
    ("2部", "Part 2"),
    ("3部", "Part 3"),
    ("第1部", "Part 1"),
    ("第2部", "Part 2"),
    ("第3部", "Part 3"),
    ("腰の武器なし", "no sheathed weapon"),
    ("OP", "opening"),
    ("大型", "large"),
    ("小型", "small"),
    ("召喚", "summoned"),
    ("受付", "clerk"),
    ("市場受付", "market clerk"),
    ("武器屋受付", "weapon shop clerk"),
    ("道具屋受付", "item shop clerk"),
    ("訓練所受付", "training ground clerk"),
    ("モブ男性", "Man"),
    ("モブ女性", "Woman"),
    ("モブ少年", "Boy"),
    ("モブ少女", "Girl"),
    ("村の男性", "Villager (male)"),
    ("村の女性", "Villager (female)"),
    ("街の男性", "Townsman"),
    ("街の女性", "Townswoman"),
    ("裕福な男性", "Wealthy Man"),
    ("裕福な女性", "Wealthy Woman"),
    ("男性神官", "Priest (male)"),
    ("女性神官", "Priestess"),
    ("試練兵", "Trial Soldier"),
    ("近衛兵", "Royal Guard"),
    ("ツワモノ", "Veteran"),
    ("無職", "No Class"),
    ("歩行兵士", "Foot Soldier"),
    ("興行師", "Showman"),
    ("歩兵", "Infantry"),
    ("騎馬", "Cavalry"),
    ("騎兵", "Cavalry"),
    ("重装歩兵", "Heavy Infantry"),
    ("重装騎兵", "Heavy Cavalry"),
    ("僧兵", "Warrior Monk"),
    ("ペガサスナイト", "Pegasus Knight"),
    ("ファルコンナイト", "Falcon Knight"),
    ("ダークペガサス", "Dark Pegasus"),
    ("ヘヴィーアーマー", "Heavy Armor"),
    ("マミー", "Mummy"),
    ("亡者兵士", "Undead Soldier"),
    ("駝鳥", "Ornius Rider"),
    ("サンドヴィーグル", "Sand Vigr"),
    ("ノブタ", "Wild Boar"),
    ("ドラゴンナイト", "Dragon Knight"),
    ("巨鳥ルフ", "Giant Roc"),
];

const GROUPS: &[(&str, &str)] = &[
    ("刃鳴り", "Answerer's Cry"),
    ("モブ", "extra"),
    ("第1部", "Part 1"),
    ("第2部", "Part 2"),
    ("第3部", "Part 3"),
    ("0部", "Prologue"),
    ("1部", "Part 1"),
    ("2部", "Part 2"),
    ("3部", "Part 3"),
];

const WEAPON: &[(&str, &str)] = &[
    ("手斧", "Hand Axe"),
    ("手槍", "Javelin"),
    ("素手", "Unarmed"),
    ("籠手", "Gauntlet"),
    ("剣", "Sword"),
    ("槍", "Lance"),
    ("斧", "Axe"),
    ("弓", "Bow"),
    ("炎", "Fire"),
    ("氷", "Ice"),
    ("雷", "Thunder"),
    ("風", "Wind"),
    ("強", "Strong"),
    ("毒", "Poison"),
    ("スケルトン", "Skeleton"),
];

const SUFFIX: &[(&str, &str)] = &[
    ("替", "stand-in"),
    ("代理", "stand-in"),
    ("小", "small"),
    ("大", "large"),
    ("弱", "weak"),
    ("金", "gold"),
    ("銀", "silver"),
    ("下馬", "dismounted"),
];

fn glossary(tok: &str) -> Option<&'static str> {
    GLOSSARY
        .iter()
        .find(|(jp, _)| *jp == tok)
        .map(|(_, en)| *en)
        .filter(|s| !s.is_empty())
}

fn group(tok: &str) -> Option<&'static str> {
    GROUPS.iter().find(|(jp, _)| *jp == tok).map(|(_, en)| *en)
}

fn english(raw: &str, texts: &Texts) -> Option<String> {
    let mut s = strip_template_prefix(raw);
    let mut lead = Vec::new();
    let test = s.starts_with("テスト");
    if test {
        s = s.strip_prefix("テスト").unwrap_or(&s).to_string();
        s = s
            .strip_prefix("DN_")
            .or_else(|| s.strip_prefix("クイックバトル_"))
            .unwrap_or(&s)
            .to_string();
    }
    s = s
        .strip_prefix("動物_")
        .or_else(|| s.strip_prefix("拠点_"))
        .or_else(|| s.strip_prefix("兵種_"))
        .unwrap_or(&s)
        .to_string();
    let mut parts: Vec<String> = s
        .split('_')
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    while parts.len() > 1 {
        let Some(g) = group(&parts[0]) else { break };
        lead.push(g.to_string());
        parts.remove(0);
    }
    if let Some(first) = parts.first_mut() {
        if first.starts_with("冥界") && first.chars().count() > 2 {
            *first = first["冥界".len()..].to_string();
            lead.push("Underworld".into());
        }
    }
    for n in (1..=parts.len()).rev() {
        let Some((main, mut tags)) = lookup(&parts[..n].join("_"), texts) else {
            continue;
        };
        for p in &parts[n..] {
            let t = glossary(p)
                .map(str::to_string)
                .or_else(|| {
                    lookup(p, texts).map(|(n, sub)| {
                        if sub.is_empty() {
                            n
                        } else {
                            format!("{n} {}", sub.join(" "))
                        }
                    })
                })
                .or_else(|| (!has_cjk(p)).then(|| p.to_string()));
            if let Some(t) = t {
                if !t.is_empty() && !tags.contains(&t) {
                    tags.push(t);
                }
            }
        }
        let mut main = main;
        for l in lead.iter() {
            if l != "extra" && !tags.contains(l) {
                tags.push(l.clone());
            }
        }
        if test {
            tags.insert(0, "test".into());
        }
        if let Some((base, inside)) = split_trailing_paren(&main) {
            if !tags.is_empty() {
                tags.insert(0, inside);
                main = base;
            }
        }
        return Some(if tags.is_empty() {
            main
        } else {
            format!("{main} ({})", tags.join(", "))
        });
    }
    if lead.iter().any(|s| s == "extra") {
        return Some("Extra".into());
    }
    None
}

fn split_trailing_paren(s: &str) -> Option<(String, String)> {
    if !s.ends_with(')') {
        return None;
    }
    let at = s.rfind(" (")?;
    Some((s[..at].to_string(), s[at + 2..s.len() - 1].to_string()))
}

fn strip_template_prefix(raw: &str) -> String {
    let s = strip_id_prefix(raw);
    let mut parts: Vec<&str> = s.split('_').collect();
    while parts.len() >= 2 {
        let first = parts[0];
        let code = (first.len() == 2 && first.chars().all(|c| c.is_ascii_uppercase()))
            || (first.len() >= 3
                && (first.starts_with("SC") || first.starts_with("BD"))
                && first
                    .chars()
                    .skip(2)
                    .all(|c| c.is_ascii_digit() || c == 'X'));
        if !code {
            break;
        }
        parts.remove(0);
        if let Some(first) = parts.first().copied() {
            let digits = first
                .char_indices()
                .find(|(_, c)| !c.is_ascii_digit())
                .map(|(i, _)| i)
                .unwrap_or(first.len());
            if digits == first.len() {
                parts.remove(0);
            } else if digits > 0 {
                parts[0] = &first[digits..];
            }
        }
    }
    parts.join("_")
}

fn lookup(tok: &str, texts: &Texts) -> Option<(String, Vec<String>)> {
    lookup_depth(tok, texts, 0)
}

fn lookup_depth(tok: &str, texts: &Texts, depth: usize) -> Option<(String, Vec<String>)> {
    if depth > 4 || tok.is_empty() {
        return None;
    }
    if let Some(name) = texts.lookup(tok) {
        return Some((first_line(name), Vec::new()));
    }
    if let Some(name) = glossary(tok) {
        return Some((name.to_string(), Vec::new()));
    }
    if let Some(rest) = tok.strip_prefix("モブ") {
        if rest.chars().all(|c| c.is_ascii_digit()) {
            return Some((
                if rest.is_empty() {
                    "Extra".to_string()
                } else {
                    format!("Extra {rest}")
                },
                Vec::new(),
            ));
        }
    }
    if let Some(at) = tok.find('[') {
        if tok.ends_with(']') {
            let (base, tag) = (&tok[..at], &tok[at + 1..tok.len() - 1]);
            if let Some((name, mut tags)) = lookup_depth(base, texts, depth + 1) {
                if !tag.is_empty() {
                    tags.push(tag.to_string());
                }
                return Some((name, tags));
            }
        }
    }
    if let Some(last) = tok.chars().last() {
        if last.is_ascii_digit() || last.is_ascii_uppercase() {
            let base = &tok[..tok.len() - last.len_utf8()];
            if let Some((name, mut tags)) = lookup_depth(base, texts, depth + 1) {
                tags.push(last.to_string());
                return Some((name, tags));
            }
        }
    }
    for (suffix, tag) in SUFFIX {
        if let Some(base) = tok.strip_suffix(suffix) {
            if !base.is_empty() {
                if let Some((name, mut tags)) = lookup_depth(base, texts, depth + 1) {
                    tags.push((*tag).to_string());
                    return Some((name, tags));
                }
            }
        }
    }
    if let Some(rest) = tok.strip_prefix('女') {
        if !rest.is_empty() {
            if let Some((name, mut tags)) = lookup_depth(rest, texts, depth + 1) {
                tags.push("female".to_string());
                return Some((name, tags));
            }
        }
    }
    for (prefix, word) in WEAPON {
        if let Some(rest) = tok.strip_prefix(prefix) {
            if !rest.is_empty() {
                if let Some((name, tags)) = lookup_depth(rest, texts, depth + 1) {
                    return Some((format!("{word} {name}"), tags));
                }
            }
        }
    }
    let chars: Vec<char> = tok.chars().collect();
    for i in (2..chars.len()).rev() {
        let head: String = chars[..i].iter().collect();
        let tail: String = chars[i..].iter().collect();
        if texts.lookup(&head).is_none() && glossary(&head).is_none() {
            continue;
        }
        if let Some((name, tags)) = lookup_depth(&tail, texts, depth + 1) {
            let head_name = texts
                .lookup(&head)
                .map(first_line)
                .or_else(|| glossary(&head).map(str::to_string))
                .unwrap_or(head);
            return Some((format!("{head_name} {name}"), tags));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts() -> Texts {
        let mut t = Texts::default();
        t.person.insert("イシュマール".into(), "Eshmel".into());
        t.person.insert("カイ".into(), "Cai".into());
        t.person.insert("盗賊".into(), "Thief".into());
        t.job.insert("ウォーリアー".into(), "Warrior".into());
        t.job.insert("剣士".into(), "Myrmidon".into());
        t.item.insert("鉄の剣".into(), "Iron Sword".into());
        t.item.insert("無銘の槍".into(), "The Nameless".into());
        t
    }

    #[test]
    fn generic_template_ids_use_the_game_words() {
        let t = texts();
        assert_eq!(person_name("SC00_03_イシュマール", &t), "Eshmel");
        assert_eq!(person_name("SC00_03_ウォーリアー", &t), "Warrior");
        assert_eq!(person_name("PID_カイ", &t), "Cai");
        assert_eq!(person_name("EN_女斧盗賊2", &t), "Axe Thief (female, 2)");
        assert_eq!(class_name("ガーディアン", &t), "ガーディアン");
        assert_eq!(class_name("ウォーリアー", &t), "Warrior");
        assert_eq!(class_name("無職", &t), "No Class");
        assert_eq!(class_name("H_ファルコンナイト", &t), "Falcon Knight");
        assert_eq!(
            class_name("ペガサスナイト下馬", &t),
            "Pegasus Knight (dismounted)"
        );
    }

    #[test]
    fn items_take_the_base_name_and_keep_the_forge_level() {
        let t = texts();
        assert_eq!(item_name("鉄の剣", &t), "Iron Sword");
        assert_eq!(item_name("鉄の剣＋２", &t), "Iron Sword +2");
        assert_eq!(item_name("鉄の剣_SC18_13", &t), "Iron Sword");
        assert_eq!(item_name("無銘の槍_戦技なし", &t), "The Nameless");
        assert_eq!(item_name("鉄の剣_チュートリアル", &t), "Iron Sword");
    }

    #[test]
    fn tags_are_removed_without_losing_the_text_around_them() {
        assert_eq!(strip_tags("攻\u{e}\u{0}\u{0}\u{4}\u{0}\u{0}撃"), "攻撃");
        assert_eq!(strip_tags("前\u{f}\u{0}\u{0}後"), "前後");
        assert_eq!(strip_tags("青銅の槍\u{e}\u{1}"), "青銅の槍");
    }
}
