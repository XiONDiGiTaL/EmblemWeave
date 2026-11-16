//! ExeFS code patches: IPS, IPS32 and IPSwitch `.pchtxt`, applied to the
//! uncompressed `main` NSO before the lifter and compiler see it.
//!
//! Atmosphère's loader documents the contract this follows: an IPS/IPS32 patch
//! is made between the uncompressed NSO and the modified uncompressed NSO, and
//! its offsets count the 0x100-byte NSO header. IPSwitch files carry main
//! addresses and normally put `@flag offset_shift 0x100` before their records.
//! The launcher never applies these to the user's dump: it applies them to the
//! launcher-owned copy at `game/dump/exefs/main`, which is exactly the file the
//! lifter reads.
//!
//! Only `main` is supported. The other ExeFS modules are not written
//! uncompressed by `prepare_exefs.py`, so patching one would be applying a
//! format that expects uncompressed bytes to compressed storage. Such a patch
//! is refused by name instead of being silently skipped or misapplied.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const MODULES: [&str; 6] = ["main", "sdk", "subsdk0", "subsdk1", "subsdk2", "rtld"];
const MAX_PATCH_BYTES: u64 = 16 * 1024 * 1024;
const NSO_HEADER: usize = 0x100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Ips,
    Ips32,
    Pchtxt,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Ips => "IPS",
            Format::Ips32 => "IPS32",
            Format::Pchtxt => "pchtxt",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub offset: usize,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub mod_name: String,
    /// The file inside the mod, for the audit trail.
    pub file: String,
    pub format: Format,
    pub module: String,
    pub build_id: Option<String>,
    pub sha256: String,
    pub records: Vec<Record>,
    pub error: Option<String>,
    pub disabled: bool,
}

impl Patch {
    pub fn label(&self) -> String {
        format!("{} / {}", self.mod_name, self.file)
    }

    pub fn ranges(&self) -> Vec<String> {
        self.records.iter().map(|r| format!("0x{:X}-0x{:X}", r.offset, r.offset.saturating_add(r.bytes.len()))).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub left: String,
    pub right: String,
    pub start: usize,
    pub end: usize,
    /// False when the overlap is byte-identical: a duplicate write, not a
    /// disagreement. It is reported but does not refuse the build.
    pub differing: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Inspection {
    pub main_build: Option<String>,
    pub main_size: Option<u64>,
    pub patches: Vec<Patch>,
    pub conflicts: Vec<Conflict>,
}

impl Inspection {
    pub fn errors(&self) -> Vec<&Patch> {
        self.patches.iter().filter(|p| p.error.is_some()).collect()
    }

    pub fn has_active(&self) -> bool {
        self.patches.iter().any(|p| p.error.is_none() && !p.records.is_empty())
    }

    pub fn can_build(&self) -> bool {
        self.errors().is_empty() && !self.conflicts.iter().any(|c| c.differing)
    }

    pub fn refusal(&self) -> String {
        let mut lines = Vec::new();
        for p in self.errors() {
            lines.push(format!("{}: {}", p.label(), p.error.as_deref().unwrap_or("invalid patch")));
        }
        for c in self.conflicts.iter().filter(|c| c.differing) {
            lines.push(format!(
                "{} and {} both write 0x{:X}-0x{:X} with different bytes",
                c.left, c.right, c.start, c.end
            ));
        }
        lines.join("\n")
    }

    /// A deterministic identity for this active set. It contains every active
    /// patch's mod, relative path, content hash, target module and build ID, so
    /// moving, renaming, enabling, removing or editing one changes it. RomFS
    /// changes do not, so a RomFS-only mod does not force a recompile.
    pub fn fingerprint(&self) -> String {
        let mut entries: Vec<String> = self
            .patches
            .iter()
            .filter(|p| p.error.is_none() && !p.records.is_empty())
            .map(|p| {
                format!(
                    "{}|{}|{}|{}|{}|{}",
                    p.mod_name,
                    p.file,
                    p.format.label(),
                    p.module,
                    p.build_id.as_deref().unwrap_or("-"),
                    p.sha256
                )
            })
            .collect();
        entries.sort();
        let mut h = Sha256::new();
        h.update(b"emblemweave-exefs-patches-v1\n");
        h.update(self.main_build.as_deref().unwrap_or("unknown").as_bytes());
        h.update(b"\n");
        for e in entries {
            h.update(e.as_bytes());
            h.update(b"\n");
        }
        format!("{:x}", h.finalize())
    }

    pub fn for_mod(&self, name: &str) -> Vec<&Patch> {
        self.patches.iter().filter(|p| p.mod_name.eq_ignore_ascii_case(name)).collect()
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// The 40-hex build ID from an NSO header. It is the first 20 bytes of the
/// 32-byte build ID field, which is the length the recomp's `BUILDS` table and
/// IPSwitch files use; a 32-hex declaration is compared as its 16-byte prefix.
pub fn main_build_id(path: &Path) -> Option<String> {
    let mut head = [0u8; 0x60];
    let mut f = std::fs::File::open(path).ok()?;
    use std::io::Read;
    f.read_exact(&mut head).ok()?;
    if &head[..4] != b"NSO0" {
        return None;
    }
    Some(head[0x40..0x54].iter().map(|b| format!("{b:02X}")).collect())
}

fn hex_digits(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether a declared ID matches the current main build. Declarations of 32 or
/// 40 hex digits are accepted; a shorter declaration is ambiguous and refused.
fn build_matches(declared: &str, current: &str) -> bool {
    let d = declared.trim().to_ascii_uppercase();
    let c = current.trim().to_ascii_uppercase();
    (d.len() == 32 || d.len() == 40) && hex_digits(&d) && c.starts_with(&d)
}

fn parse_hex_byte(token: &str) -> Result<u8, String> {
    u8::from_str_radix(token, 16).map_err(|_| format!("'{token}' is not a two-digit hexadecimal byte"))
}

/// C-style string escapes from the IPSwitch format. Anything not a byte escape
/// is refused rather than guessed.
fn parse_quoted(text: &str) -> Result<Vec<u8>, String> {
    let mut chars = text.chars();
    if chars.next() != Some('"') {
        return Err("a string value must start with a quote".into());
    }
    let mut out = Vec::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                if chars.as_str().trim().is_empty() {
                    return Ok(out);
                }
                return Err("unexpected text after the closing quote".into());
            }
            '\\' => match chars.next().ok_or("a backslash at the end of a string")? {
                'n' => out.push(b'\n'),
                'r' => out.push(b'\r'),
                't' => out.push(b'\t'),
                '0' => out.push(0),
                '\\' => out.push(b'\\'),
                '"' => out.push(b'"'),
                'x' => {
                    let a = chars.next().ok_or("\\x needs two hex digits")?;
                    let b = chars.next().ok_or("\\x needs two hex digits")?;
                    out.push(parse_hex_byte(&format!("{a}{b}"))?);
                }
                other => return Err(format!("unsupported string escape \\{other}")),
            },
            c if c.is_ascii() => out.push(c as u8),
            c => return Err(format!("'{c}' is not an ASCII byte")),
        }
    }
    Err("the string value has no closing quote".into())
}

/// Parse an IPS or IPS32 file. Offsets are absolute in the uncompressed NSO,
/// including its 0x100-byte header. No final-size extension is supported: a
/// patch that tries to change the NSO's size is refused rather than truncating
/// an executable.
pub fn parse_ips(data: &[u8], main_size: Option<u64>) -> Result<(Format, Vec<Record>), String> {
    let (format, addr_len, end) = if data.starts_with(b"PATCH") {
        (Format::Ips, 3usize, 3usize)
    } else if data.starts_with(b"IPS32") {
        (Format::Ips32, 4usize, 4usize)
    } else {
        return Err("not an IPS or IPS32 file (no PATCH or IPS32 header)".into());
    };
    let end_magic: &[u8] = if format == Format::Ips { b"EOF" } else { b"EEOF" };
    let mut pos = 5usize;
    let mut records = Vec::new();
    loop {
        if pos + end > data.len() {
            return Err("the file ends before its EOF marker".into());
        }
        if data[pos..pos + end] == *end_magic {
            break;
        }
        let mut offset = 0usize;
        for b in &data[pos..pos + addr_len] {
            offset = offset.checked_mul(256).and_then(|v| v.checked_add(*b as usize)).ok_or("the record offset overflows")?;
        }
        pos += addr_len;
        if pos + 2 > data.len() {
            return Err("a record is missing its two-byte length".into());
        }
        let len = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
        pos += 2;
        let bytes = if len == 0 {
            if pos + 3 > data.len() {
                return Err("an RLE record is missing its run length or value".into());
            }
            let run = u16::from_be_bytes([data[pos], data[pos + 1]]) as usize;
            let value = data[pos + 2];
            pos += 3;
            if run == 0 {
                return Err("an RLE record has a zero run length".into());
            }
            vec![value; run]
        } else {
            if pos + len > data.len() {
                return Err(format!("a record at 0x{offset:X} says {len} bytes, but the file ends first"));
            }
            let bytes = data[pos..pos + len].to_vec();
            pos += len;
            bytes
        };
        if offset < NSO_HEADER {
            return Err(format!("a record at 0x{offset:X} writes into the NSO header; offsets start at 0x100"));
        }
        let end = offset.checked_add(bytes.len()).ok_or("the record range overflows")?;
        if let Some(size) = main_size {
            if end as u64 > size {
                return Err(format!("a record at 0x{offset:X}-0x{end:X} is past the end of main ({size} bytes)"));
            }
        }
        records.push(Record { offset, bytes });
    }
    if records.is_empty() {
        return Err("the patch contains no records".into());
    }
    check_within_patch(&records)?;
    Ok((format, records))
}

fn check_within_patch(records: &[Record]) -> Result<(), String> {
    let mut ranges: Vec<(usize, usize)> = records.iter().map(|r| (r.offset, r.offset + r.bytes.len())).collect();
    ranges.sort_unstable();
    for pair in ranges.windows(2) {
        if pair[1].0 < pair[0].1 {
            return Err(format!("records at 0x{:X} and 0x{:X} overlap inside one patch", pair[0].0, pair[1].0));
        }
    }
    Ok(())
}

fn parse_shift(text: &str) -> Result<i64, String> {
    let t = text.trim();
    let (neg, body) = if let Some(t) = t.strip_prefix('-') { (true, t) } else { (false, t) };
    let body = body.strip_prefix("0x").or_else(|| body.strip_prefix("0X")).unwrap_or(body);
    let n = i64::from_str_radix(body, 16).map_err(|_| format!("'{text}' is not a hexadecimal offset shift"))?;
    Ok(if neg { -n } else { n })
}

fn parse_offset(text: &str) -> Result<usize, String> {
    let body = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text);
    if !hex_digits(body) {
        return Err(format!("'{text}' is not a hexadecimal offset"));
    }
    usize::from_str_radix(body, 16).map_err(|_| format!("'{text}' is too large for an offset"))
}

/// Parse an IPSwitch `.pchtxt`. The build ID is mandatory and must name main.
/// `@enabled` / `@disabled` sections are honoured, `@stop` ends the file, and an
/// `@flag offset_shift` must come before the first record. A section disabled in
/// the file is a valid no-op, not an error.
pub fn parse_pchtxt(data: &[u8], current_build: Option<&str>, main_size: Option<u64>) -> Result<(String, Vec<Record>, bool), String> {
    let text = std::str::from_utf8(data).map_err(|_| "the pchtxt is not UTF-8 text".to_string())?;
    let mut declared = None;
    let mut enabled = true;
    let mut shift: i64 = 0;
    let mut records = Vec::new();
    let mut saw_header = false;
    let mut saw_record = false;
    let mut stopped = false;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@nsobid-") {
            if saw_header || saw_record {
                return Err(format!("line {}: @nsobid must be the first directive and appear once", n + 1));
            }
            let id = rest.trim().to_ascii_uppercase();
            if !(id.len() == 32 || id.len() == 40) || !hex_digits(&id) {
                return Err(format!("line {}: @nsobid needs 32 or 40 hexadecimal digits", n + 1));
            }
            saw_header = true;
            declared = Some(id);
            continue;
        }
        if let Some(flag) = line.strip_prefix("@flag") {
            if !saw_header {
                return Err(format!("line {}: the first directive must be @nsobid-<build id>", n + 1));
            }
            if saw_record {
                return Err(format!("line {}: @flag must come before the first patch record", n + 1));
            }
            let mut parts = flag.split_whitespace();
            match parts.next() {
                Some("offset_shift") => {
                    let value = parts.next().ok_or_else(|| format!("line {}: offset_shift needs a value", n + 1))?;
                    if parts.next().is_some() {
                        return Err(format!("line {}: offset_shift takes one value", n + 1));
                    }
                    shift = shift.checked_add(parse_shift(value)?).ok_or("the offset shift overflows")?;
                }
                Some("print_values") => {}
                Some(other) => return Err(format!("line {}: unsupported @flag {other}", n + 1)),
                None => return Err(format!("line {}: @flag needs a name", n + 1)),
            }
            continue;
        }
        if line == "@enabled" {
            if !saw_header {
                return Err(format!("line {}: the first directive must be @nsobid-<build id>", n + 1));
            }
            enabled = true;
            continue;
        }
        if line == "@disabled" {
            if !saw_header {
                return Err(format!("line {}: the first directive must be @nsobid-<build id>", n + 1));
            }
            enabled = false;
            continue;
        }
        if line == "@stop" {
            stopped = true;
            break;
        }
        if line.starts_with('@') {
            return Err(format!("line {}: unsupported directive {line}", n + 1));
        }
        if !saw_header {
            return Err(format!("line {}: the first directive must be @nsobid-<build id>", n + 1));
        }
        let (offset_text, rest) = line.split_once(char::is_whitespace).ok_or_else(|| format!("line {}: a patch line needs an offset and bytes", n + 1))?;
        let raw_offset = parse_offset(offset_text)?;
        let shifted = (raw_offset as i64).checked_add(shift).ok_or_else(|| format!("line {}: the offset shift overflows", n + 1))?;
        if shifted < 0 {
            return Err(format!("line {}: the offset shift makes the offset negative", n + 1));
        }
        let offset = shifted as usize;
        let rest = rest.trim();
        let bytes = if rest.starts_with('"') {
            parse_quoted(rest).map_err(|e| format!("line {}: {e}", n + 1))?
        } else {
            let mut out = Vec::new();
            for token in rest.split_whitespace() {
                if token.len() != 2 || !hex_digits(token) {
                    return Err(format!("line {}: '{token}' is not a two-digit hexadecimal byte", n + 1));
                }
                out.push(parse_hex_byte(token).map_err(|e| format!("line {}: {e}", n + 1))?);
            }
            out
        };
        if bytes.is_empty() {
            return Err(format!("line {}: the patch line has no bytes", n + 1));
        }
        saw_record = true;
        if !enabled {
            continue;
        }
        if offset < NSO_HEADER {
            return Err(format!("line {}: offset 0x{offset:X} writes into the NSO header; offsets start at 0x100", n + 1));
        }
        let end = offset.checked_add(bytes.len()).ok_or_else(|| format!("line {}: the range overflows", n + 1))?;
        if let Some(size) = main_size {
            if end as u64 > size {
                return Err(format!("line {}: 0x{offset:X}-0x{end:X} is past the end of main ({size} bytes)", n + 1));
            }
        }
        records.push(Record { offset, bytes });
    }
    if !saw_header {
        return Err("the first directive must be @nsobid-<build id>".into());
    }
    let declared = declared.expect("saw_header implies a build id");
    if !build_matches(&declared, current_build.ok_or("main's build ID is not available, so the patch cannot be checked")?) {
        return Err(format!(
            "the patch targets build {} but this main is {}",
            declared,
            current_build.unwrap_or("unknown")
        ));
    }
    if !saw_record {
        return Err("the pchtxt has no patch records".into());
    }
    if records.is_empty() && !stopped {
        // Every patch line was in a @disabled section. It is a valid no-op.
        return Ok((declared, records, false));
    }
    if records.is_empty() {
        return Ok((declared, records, false));
    }
    check_within_patch(&records)?;
    Ok((declared, records, true))
}

fn walk_files(root: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_files(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push((rel.display().to_string().replace('\\', "/"), p));
        }
    }
}

fn extension(p: &Path) -> Option<String> {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}

fn module_for_stem(stem: &str) -> Option<&'static str> {
    MODULES.iter().copied().find(|m| stem.eq_ignore_ascii_case(m))
}

fn target_module(stem: &str, format: Format, current: Option<&str>) -> Result<String, String> {
    if let Some(m) = module_for_stem(stem) {
        return Ok(m.to_string());
    }
    if format == Format::Pchtxt {
        return Ok("main".into());
    }
    let t = stem.trim().to_ascii_uppercase();
    if (t.len() == 32 || t.len() == 40) && hex_digits(&t) {
        let current = current.ok_or("main's build ID is not available")?;
        if build_matches(&t, current) {
            return Ok("main".into());
        }
        return Err(format!("the file name names build {t}, but this main is {current}"));
    }
    Err("the file name does not identify main's build ID or a known module".into())
}

fn inspect_file(m: &str, rel: &str, path: &Path, current: Option<&str>, main_size: Option<u64>) -> Patch {
    let ext = extension(path).unwrap_or_default();
    let format = match ext.as_str() {
        "ips" | "ips32" => {
            // The extension chooses the parser's preferred answer, but the
            // magic in the file decides; an .ips that says IPS32 is valid.
            if ext == "ips32" { Format::Ips32 } else { Format::Ips }
        }
        "pchtxt" => Format::Pchtxt,
        _ => unreachable!("the caller filters extensions"),
    };
    let mut patch = Patch {
        mod_name: m.to_string(),
        file: rel.to_string(),
        format,
        module: "main".into(),
        build_id: None,
        sha256: String::new(),
        records: Vec::new(),
        error: None,
        disabled: false,
    };
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) => {
            patch.error = Some(format!("cannot read the file: {e}"));
            return patch;
        }
    };
    if meta.len() > MAX_PATCH_BYTES {
        patch.error = Some(format!("the file is {} bytes; patches over 16 MiB are refused", meta.len()));
        return patch;
    }
    let data = match std::fs::read(path) {
        Ok(data) => data,
        Err(e) => {
            patch.error = Some(format!("cannot read the file: {e}"));
            return patch;
        }
    };
    patch.sha256 = hash(&data);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let module = match target_module(&stem, format, current) {
        Ok(m) => m,
        Err(e) => {
            patch.error = Some(e);
            return patch;
        }
    };
    patch.module = module;
    if patch.module != "main" {
        patch.error = Some(format!(
            "patches for {} are not supported: only main is written uncompressed before the lifter",
            patch.module
        ));
        return patch;
    }
    match format {
        Format::Ips | Format::Ips32 => {
            let named = stem.trim().to_ascii_uppercase();
            if named.len() == 32 || named.len() == 40 {
                if let Some(current) = current {
                    if !build_matches(&named, current) {
                        patch.error = Some(format!("the file name names build {named}, but this main is {current}"));
                        return patch;
                    }
                    patch.build_id = Some(named);
                } else {
                    patch.error = Some("main's build ID is not available, so the file name cannot be checked".into());
                    return patch;
                }
            } else {
                patch.error = Some("an IPS file must be named after main's build ID; a generic main.ips is not verifiable".into());
                return patch;
            }
            match parse_ips(&data, main_size) {
                Ok((actual, records)) => {
                    patch.format = actual;
                    patch.records = records;
                }
                Err(e) => patch.error = Some(e),
            }
        }
        Format::Pchtxt => match parse_pchtxt(&data, current, main_size) {
            Ok((declared, records, enabled)) => {
                patch.build_id = Some(declared);
                patch.records = records;
                patch.disabled = !enabled;
            }
            Err(e) => patch.error = Some(e),
        },
    }
    patch
}

/// Read every enabled mod's ExeFS folders and parse its patch files. Nothing is
/// written here, so the Mods page can call it safely.
pub fn inspect(mods: &Path, main_build: Option<String>, main_size: Option<u64>) -> Inspection {
    let mut out = Inspection { main_build, main_size, ..Inspection::default() };
    let list = crate::mods::list(mods);
    for m in list.iter().filter(|m| m.enabled) {
        let mut dirs = crate::mods::exefs_dirs(&m.path);
        dirs.sort();
        let mut files = Vec::new();
        for dir in dirs {
            walk_files(&dir, &dir, &mut files);
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        for (rel, path) in files {
            if !matches!(extension(&path).as_deref(), Some("ips" | "ips32" | "pchtxt")) {
                continue;
            }
            out.patches.push(inspect_file(&m.name, &rel, &path, out.main_build.as_deref(), out.main_size));
        }
    }
    out.patches.sort_by(|a, b| a.mod_name.to_lowercase().cmp(&b.mod_name.to_lowercase()).then_with(|| a.file.cmp(&b.file)));
    // Compare every pair of valid records. A patch is small; this is a few
    // hundred comparisons at most and never depends on filesystem order.
    let valid: Vec<&Patch> = out.patches.iter().filter(|p| p.error.is_none()).collect();
    for (i, a) in valid.iter().enumerate() {
        for b in valid.iter().skip(i + 1) {
            let mut reported = Vec::new();
            for ra in &a.records {
                for rb in &b.records {
                    let start = ra.offset.max(rb.offset);
                    let end = (ra.offset + ra.bytes.len()).min(rb.offset + rb.bytes.len());
                    if start >= end {
                        continue;
                    }
                    let differing = ra.bytes[start - ra.offset..end - ra.offset] != rb.bytes[start - rb.offset..end - rb.offset];
                    if reported.iter().any(|(s, e)| *s == start && *e == end) {
                        continue;
                    }
                    reported.push((start, end));
                    out.conflicts.push(Conflict { left: a.label(), right: b.label(), start, end, differing });
                }
            }
        }
    }
    out.conflicts.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.left.cmp(&b.left)).then_with(|| a.right.cmp(&b.right)));
    out
}

fn audit_path() -> Option<PathBuf> {
    crate::engine::run_dir().map(|d| d.join("patch-audit.json"))
}

fn state_path() -> Option<PathBuf> {
    crate::engine::run_dir().map(|d| d.join("patch-state.json"))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct AuditPatch {
    pub module: String,
    pub name: String,
    pub file: String,
    pub format: String,
    pub build_id: Option<String>,
    pub sha256: String,
    pub ranges: Vec<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Audit {
    pub fingerprint: String,
    pub build_id: Option<String>,
    pub original_sha256: String,
    pub patched_sha256: String,
    pub patches: Vec<AuditPatch>,
    pub conflicts: Vec<String>,
    pub applied_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    pub fingerprint: String,
    pub original_sha256: String,
    pub patched_sha256: String,
    pub count: usize,
}

fn audit_for(ins: &Inspection, original: &str, patched: &str) -> Audit {
    Audit {
        fingerprint: ins.fingerprint(),
        build_id: ins.main_build.clone(),
        original_sha256: original.to_string(),
        patched_sha256: patched.to_string(),
        patches: ins
            .patches
            .iter()
            .filter(|p| p.error.is_none())
            .map(|p| AuditPatch {
                module: p.module.clone(),
                name: p.mod_name.clone(),
                file: p.file.clone(),
                format: p.format.label().to_string(),
                build_id: p.build_id.clone(),
                sha256: p.sha256.clone(),
                ranges: p.ranges(),
                status: if p.records.is_empty() { "disabled or empty" } else { "applied" }.into(),
            })
            .collect(),
        conflicts: ins
            .conflicts
            .iter()
            .map(|c| {
                format!(
                    "{} + {} at 0x{:X}-0x{:X}: {}",
                    c.left,
                    c.right,
                    c.start,
                    c.end,
                    if c.differing { "different bytes" } else { "identical bytes" }
                )
            })
            .collect(),
        applied_at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    }
}

/// Apply the active plan to the launcher's uncompressed copy. The output is
/// written to a temporary file and renamed into place, so an interrupted write
/// cannot leave a half-patched executable. The audit is written whether or not
/// there are patches, so a support report can see exactly what the build used.
pub fn apply(ins: &Inspection, main: &Path) -> Result<Applied, String> {
    if !ins.can_build() {
        return Err(format!("the active code patches cannot be applied:\n{}", ins.refusal()));
    }
    let original = std::fs::read(main).map_err(|e| format!("reading {}: {e}", main.display()))?;
    let original_sha = hash(&original);
    let mut bytes = original.clone();
    let mut count = 0;
    for p in ins.patches.iter().filter(|p| p.error.is_none()) {
        for r in &p.records {
            let end = r.offset.checked_add(r.bytes.len()).ok_or_else(|| format!("{}: the range overflows", p.label()))?;
            if end > bytes.len() {
                return Err(format!("{}: 0x{:X}-0x{:X} is past the end of {}", p.label(), r.offset, end, main.display()));
            }
            bytes[r.offset..end].copy_from_slice(&r.bytes);
            count += 1;
        }
    }
    let patched_sha = hash(&bytes);
    if bytes != original {
        let tmp = main.with_extension("patched.tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, main).map_err(|e| format!("replacing {}: {e}", main.display()))?;
    }
    let audit = audit_for(ins, &original_sha, &patched_sha);
    if let Some(path) = audit_path() {
        if let Err(e) = std::fs::write(&path, serde_json::to_string_pretty(&audit).unwrap_or_default()) {
            eprintln!("writing {}: {e}", path.display());
        }
    }
    Ok(Applied { fingerprint: ins.fingerprint(), original_sha256: original_sha, patched_sha256: patched_sha, count })
}

/// Record the applied set after the build has succeeded. Until this is done,
/// Play treats the active set as newer than the executable.
pub fn record(ins: &Inspection, applied: &Applied) -> Result<(), String> {
    let path = state_path().ok_or(crate::platform::NO_DATA_DIR)?;
    let state = audit_for(ins, &applied.original_sha256, &applied.patched_sha256);
    std::fs::write(&path, serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?)
        .map_err(|e| format!("writing {}: {e}", path.display()))
}

pub fn read_state() -> Option<Audit> {
    let text = std::fs::read_to_string(state_path()?).ok()?;
    serde_json::from_str(&text).ok()
}

/// Whether the executable predates the current active patch set. No active
/// patches and no recorded state is a normal build; active patches with no
/// recorded state are always stale.
pub fn is_stale(ins: &Inspection, state: Option<&Audit>) -> bool {
    let Some(state) = state else { return ins.has_active() };
    state.fingerprint != ins.fingerprint() || state.build_id != ins.main_build
}

pub fn needs_build(ins: &Inspection) -> bool {
    is_stale(ins, read_state().as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ew-patches-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ips(offset: u32, data: &[u8], wide: bool) -> Vec<u8> {
        let mut v = if wide { b"IPS32".to_vec() } else { b"PATCH".to_vec() };
        if wide {
            v.extend_from_slice(&offset.to_be_bytes());
        } else {
            v.extend_from_slice(&offset.to_be_bytes()[1..]);
        }
        v.extend_from_slice(&(data.len() as u16).to_be_bytes());
        v.extend_from_slice(data);
        v.extend_from_slice(if wide { b"EEOF" } else { b"EOF" });
        v
    }

    #[test]
    fn reads_ips_and_ips32() {
        let (f, r) = parse_ips(&ips(0x120, &[1, 2, 3], false), Some(0x1000)).unwrap();
        assert_eq!(f, Format::Ips);
        assert_eq!(r[0].offset, 0x120);
        assert_eq!(r[0].bytes, vec![1, 2, 3]);
        let (f, r) = parse_ips(&ips(0x200, &[9, 8], true), Some(0x1000)).unwrap();
        assert_eq!(f, Format::Ips32);
        assert_eq!(r[0].offset, 0x200);
    }

    #[test]
    fn reads_rle_and_refuses_malformed_rle() {
        let mut v = b"PATCH".to_vec();
        v.extend_from_slice(&[0, 1, 0x20, 0, 0, 0, 4, 0xAB]);
        v.extend_from_slice(b"EOF");
        let (_, r) = parse_ips(&v, Some(0x1000)).unwrap();
        assert_eq!(r[0].bytes, vec![0xAB; 4]);
        let mut bad = b"PATCH".to_vec();
        bad.extend_from_slice(&[0, 1, 0x20, 0, 0, 0, 0, 0xAB]);
        bad.extend_from_slice(b"EOF");
        assert!(parse_ips(&bad, None).unwrap_err().contains("zero run"));
    }

    #[test]
    fn refuses_truncated_and_out_of_bounds() {
        assert!(parse_ips(b"PATCH\x00", None).unwrap_err().contains("EOF"));
        let mut v = ips(0x100, &[1, 2], false);
        v.truncate(v.len() - 4);
        assert!(parse_ips(&v, None).is_err());
        assert!(parse_ips(&ips(0x100, &[1; 4], false), Some(0x102)).unwrap_err().contains("past the end"));
        assert!(parse_ips(&ips(0x20, &[1], false), None).unwrap_err().contains("NSO header"));
    }

    #[test]
    fn refuses_overlapping_records_and_empty_files() {
        let mut v = b"PATCH".to_vec();
        v.extend_from_slice(&[0, 1, 0x10, 0, 2, 1, 2]);
        v.extend_from_slice(&[0, 1, 0x11, 0, 2, 3, 4]);
        v.extend_from_slice(b"EOF");
        assert!(parse_ips(&v, None).unwrap_err().contains("overlap"));
        assert!(parse_ips(b"PATCHEOF", None).unwrap_err().contains("no records"));
    }

    #[test]
    fn pchtxt_needs_a_matching_build_and_a_shift() {
        let d = temp("pch");
        let id = "0123456789ABCDEF0123456789ABCDEF01234567";
        let path = d.join("fps.pchtxt");
        std::fs::write(
            &path,
            format!(
                "@nsobid-{id}\n@flag print_values\n@flag offset_shift 0x100\n@enabled\n// comment\n0x1000 DE AD BE EF\n@stop\n"
            ),
        )
        .unwrap();
        let (declared, r, enabled) = parse_pchtxt(&std::fs::read(&path).unwrap(), Some(id), Some(0x2000)).unwrap();
        assert_eq!(declared, id);
        assert!(enabled);
        assert_eq!(r, vec![Record { offset: 0x1100, bytes: vec![0xDE, 0xAD, 0xBE, 0xEF] }]);
        let wrong = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        assert!(parse_pchtxt(&std::fs::read(&path).unwrap(), Some(wrong), Some(0x2000)).unwrap_err().contains("targets build"));
        let missing = "0x1000 DE AD\n";
        assert!(parse_pchtxt(missing.as_bytes(), Some(id), Some(0x2000)).unwrap_err().contains("first directive"));
    }

    #[test]
    fn pchtxt_handles_disabled_sections_crlf_and_strings() {
        let d = temp("pch-disabled");
        let id = "0123456789ABCDEF0123456789ABCDEF01234567";
        let path = d.join("x.pchtxt");
        std::fs::write(
            &path,
            format!("@nsobid-{id}\r\n@flag offset_shift 0x100\r\n@disabled\r\n1000 00\r\n@enabled\r\n1004 \"AB\\x43\"\r\n"),
        )
        .unwrap();
        let (_, r, enabled) = parse_pchtxt(&std::fs::read(&path).unwrap(), Some(id), Some(0x2000)).unwrap();
        assert!(enabled);
        assert_eq!(r, vec![Record { offset: 0x1104, bytes: b"ABC".to_vec() }]);
        let all_off = format!("@nsobid-{id}\n@flag offset_shift 0x100\n@disabled\n1000 00\n");
        let (_, r, on) = parse_pchtxt(all_off.as_bytes(), Some(id), Some(0x2000)).unwrap();
        assert!(r.is_empty() && !on);
    }

    #[test]
    fn pchtxt_rejects_unknown_syntax_and_bad_offsets() {
        let id = "0123456789ABCDEF0123456789ABCDEF01234567";
        let base = format!("@nsobid-{id}\n@flag offset_shift 0x100\n");
        assert!(parse_pchtxt(format!("{base}1000 GG\n").as_bytes(), Some(id), None).is_err());
        assert!(parse_pchtxt(format!("@nsobid-{id}\n20 00\n").as_bytes(), Some(id), None).unwrap_err().contains("header"));
        assert!(parse_pchtxt(format!("{base}FFFFFFFF 00\n").as_bytes(), Some(id), Some(0x100)).unwrap_err().contains("past the end"));
        assert!(parse_pchtxt(format!("{base}@what\n").as_bytes(), Some(id), None).unwrap_err().contains("unsupported directive"));
        // A file without offset_shift uses absolute uncompressed-NSO offsets,
        // which is also the IPSwitch format; it is valid, not an error.
        let (_, r, on) = parse_pchtxt(format!("@nsobid-{id}\n1000 00\n").as_bytes(), Some(id), None).unwrap();
        assert!(on);
        assert_eq!(r[0].offset, 0x1000);
    }

    #[test]
    fn inspection_finds_conflicts_and_identical_overlaps() {
        let d = temp("inspect");
        let id = "0123456789ABCDEF0123456789ABCDEF01234567";
        let mods = d.join("mods");
        let a = mods.join("Alpha/exefs");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(a.join(format!("{id}.ips")), ips(0x120, &[1, 2, 3], false)).unwrap();
        let b = mods.join("Beta/exefs");
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(b.join(format!("{id}.ips")), ips(0x122, &[4, 5], false)).unwrap();
        let i = inspect(&mods, Some(id.into()), Some(0x1000));
        assert_eq!(i.errors().len(), 0, "{:?}", i.errors());
        assert!(i.conflicts.iter().any(|c| c.start == 0x122 && c.end == 0x123 && c.differing));
        // Same bytes at the same place is an overlap, not a conflict.
        let d2 = temp("inspect-same");
        std::fs::create_dir_all(d2.join("mods/Alpha/exefs")).unwrap();
        std::fs::create_dir_all(d2.join("mods/Beta/exefs")).unwrap();
        std::fs::write(d2.join("mods/Alpha/exefs").join(format!("{id}.ips")), ips(0x120, &[1, 2], false)).unwrap();
        std::fs::write(d2.join("mods/Beta/exefs").join(format!("{id}.ips")), ips(0x120, &[1, 2], false)).unwrap();
        let i = inspect(&d2.join("mods"), Some(id.into()), Some(0x1000));
        assert!(i.can_build());
        assert!(i.conflicts.iter().any(|c| !c.differing));
    }

    #[test]
    fn a_patch_only_mod_is_installed_and_applied_to_the_copy() {
        let d = temp("end-to-end");
        let id = "0123456789ABCDEF0123456789ABCDEF01234567";
        let staged = d.join("staged");
        std::fs::create_dir_all(staged.join("exefs")).unwrap();
        std::fs::write(staged.join("exefs").join(format!("{id}.ips")), ips(0x120, &[0xAA, 0xBB], false)).unwrap();
        let mods = d.join("mods");
        let installed = crate::mods::install_from(&staged, &mods, &crate::mods::Info { title: "Patch only".into(), ..Default::default() }).unwrap();
        assert_eq!(installed.mods, vec!["Patch only".to_string()]);
        assert!(installed.needs_build);
        assert!(!installed.exefs_only);
        // The synthetic uncompressed main has an NSO header and enough room.
        let main = d.join("main");
        let dump = d.join("dump");
        let mut bytes = vec![0u8; 0x400];
        bytes[..4].copy_from_slice(b"NSO0");
        bytes[0x40..0x54].copy_from_slice(&(0..20).map(|i| i as u8).collect::<Vec<_>>());
        std::fs::write(&dump, &bytes).unwrap();
        std::fs::copy(&dump, &main).unwrap();
        let bid = main_build_id(&main).unwrap();
        // Rename the installed patch for this synthetic build ID.
        let source = mods.join("Patch only/exefs").join(format!("{id}.ips"));
        let target = mods.join("Patch only/exefs").join(format!("{bid}.ips"));
        std::fs::rename(&source, &target).unwrap();
        let ins = inspect(&mods, Some(bid.clone()), Some(0x400));
        assert!(ins.can_build(), "{:?}", ins.errors());
        let applied = apply(&ins, &main).unwrap();
        assert_eq!(applied.count, 1);
        assert_ne!(applied.original_sha256, applied.patched_sha256);
        let after = std::fs::read(&main).unwrap();
        assert_eq!(&after[0x120..0x122], &[0xAA, 0xBB]);
        assert_eq!(std::fs::read(&dump).unwrap(), bytes, "the dump was not modified");
        // A changed file changes the identity, and the build is stale.
        std::fs::write(&target, ips(0x124, &[0xCC], false)).unwrap();
        let changed = inspect(&mods, Some(bid), Some(0x400));
        assert_ne!(ins.fingerprint(), changed.fingerprint());
    }

    #[test]
    fn a_changed_patch_set_is_stale_and_an_empty_one_is_normal() {
        let d = temp("stale");
        let id = "0123456789ABCDEF0123456789ABCDEF01234567";
        let ex = d.join("mods/Foo/exefs");
        std::fs::create_dir_all(&ex).unwrap();
        let file = ex.join(format!("{id}.ips"));
        std::fs::write(&file, ips(0x120, &[1], false)).unwrap();
        let ins = inspect(&d.join("mods"), Some(id.into()), Some(0x1000));
        assert!(ins.has_active());
        assert!(is_stale(&ins, None), "active patches and no recorded build must ask for a build");
        let state = Audit { fingerprint: ins.fingerprint(), build_id: ins.main_build.clone(), ..Audit::default() };
        assert!(!is_stale(&ins, Some(&state)));
        std::fs::write(&file, ips(0x120, &[2], false)).unwrap();
        let changed = inspect(&d.join("mods"), Some(id.into()), Some(0x1000));
        assert!(is_stale(&changed, Some(&state)), "a changed patch must invalidate the build");
        // Removing the patch also needs one rebuild to take it out of the exe.
        std::fs::remove_file(&file).unwrap();
        let empty = inspect(&d.join("mods"), Some(state.build_id.clone().unwrap()), Some(0x1000));
        assert!(!empty.has_active());
        assert!(is_stale(&empty, Some(&state)), "removing the last patch must invalidate the build");
        let empty_state = Audit { fingerprint: empty.fingerprint(), build_id: empty.main_build.clone(), ..Audit::default() };
        assert!(!is_stale(&empty, Some(&empty_state)), "a build with no patches is current");
        assert!(!is_stale(&empty, None), "a build with no patches and no recorded state is normal");
    }

    #[test]
    fn disabled_mods_are_not_inspected() {
        let d = temp("disabled");
        let id = "0123456789ABCDEF0123456789ABCDEF01234567";
        let ex = d.join("mods/_Off/exefs");
        std::fs::create_dir_all(&ex).unwrap();
        std::fs::write(ex.join(format!("{id}.ips")), ips(0x120, &[1], false)).unwrap();
        let i = inspect(&d.join("mods"), Some(id.into()), Some(0x1000));
        assert!(i.patches.is_empty(), "a switched-off mod must not contribute patches");
    }

    #[test]
    fn mixed_romfs_and_patch_mods_install_together() {
        let d = temp("mixed");
        let id = "0123456789ABCDEF0123456789ABCDEF01234567";
        let staged = d.join("staged/Variant");
        std::fs::create_dir_all(staged.join("romfs/GameData")).unwrap();
        std::fs::create_dir_all(staged.join("exefs")).unwrap();
        std::fs::write(staged.join("romfs/GameData/Person.bin"), b"x").unwrap();
        std::fs::write(staged.join("exefs").join(format!("{id}.ips")), ips(0x120, &[1], false)).unwrap();
        let mods = d.join("mods");
        let installed = crate::mods::install_from(&staged, &mods, &crate::mods::Info { title: "Mixed".into(), ..Default::default() }).unwrap();
        assert_eq!(installed.mods, vec!["Mixed".to_string()]);
        assert!(installed.needs_build && !installed.patch_only);
        let list = crate::mods::list(&mods);
        assert_eq!(list.len(), 1);
        assert!(list[0].has_romfs && list[0].has_patches, "both halves must stay in one mod");
        assert!(mods.join("Mixed/romfs/GameData/Person.bin").is_file());
        assert!(mods.join("Mixed/exefs").join(format!("{id}.ips")).is_file());
    }

    #[test]
    fn wrong_build_id_and_non_main_are_refused() {
        let d = temp("wrong");
        let id = "0123456789ABCDEF0123456789ABCDEF01234567";
        let other = "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF";
        let ex = d.join("mods/Foo/exefs");
        std::fs::create_dir_all(&ex).unwrap();
        std::fs::write(ex.join(format!("{other}.ips")), ips(0x120, &[1], false)).unwrap();
        let i = inspect(&d.join("mods"), Some(id.into()), Some(0x1000));
        assert!(!i.can_build());
        assert!(i.errors()[0].error.as_deref().unwrap_or("").contains("this main is"));
        let d2 = temp("nonmain");
        let ex2 = d2.join("mods/Foo/exefs");
        std::fs::create_dir_all(&ex2).unwrap();
        std::fs::write(ex2.join("sdk.ips"), ips(0x120, &[1], false)).unwrap();
        let i = inspect(&d2.join("mods"), Some(id.into()), Some(0x1000));
        assert!(i.errors()[0].error.as_deref().unwrap_or("").contains("only main"));
    }
}
