//! Hook sites from mods.
//!
//! A code mod (a DLL in a mod's `exefs/`, the engine's `runtime/include/nxrt/mod_api.h`) can only hook an instruction the game
//! was lifted with as a site. A mod that needs sites of its own lists them in `exefs/<any name>.sites`, in the syntax of the
//! engine's `recomp/games/fefw/main.sites`, with only the kinds that add a call into the runtime (`inline`, `midhook`,
//! `entryhook`) and one `@nsobid <main build ID>` line, so sites for another version of the game are left out:
//!
//! ```text
//! @nsobid DB93B0125F4850601D61C3EDDE012F8EB2ECFE77
//! midhook 0x7100e1d03c      # comment
//! ```
//!
//! Before the game code is compiled, the build merges `main.sites` with the installed mods' sites (switched-off mods included:
//! a site with no hook attached only passes through, so switching a mod needs no rebuild) and hands the list to the engine
//! (`FEFW_SITES`). The list is rewritten only when it changes, and the engine lifts `main` again exactly then. The game build
//! puts the list it was lifted with next to `fefw.exe` as `fefw.sites`, which is how [`stale`] tells that installed mods
//! need the game built again, and what the runtime checks hooks against.

use std::path::{Path, PathBuf};

const MAIN_BASE: u64 = 0x71_0000_0000;
const NEXT_MODULE: u64 = 0x75_0000_0000;
const KINDS: [&str; 3] = ["inline", "midhook", "entryhook"];

/// One site a mod adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    /// `<mod>/exefs/<file>:<line>`, for messages.
    pub from: String,
    pub kind: String,
    pub addr: u64,
}

/// The engine's own site list.
pub fn base_list(root: &Path) -> PathBuf {
    root.join("recomp").join("games").join("fefw").join("main.sites")
}

/// Where the merged list goes (a fixed path, so the engine's cache keeps pointing at it).
pub fn merged_path(root: &Path) -> PathBuf {
    root.join("work").join("build").join("fefw-mod-sites").join("main.sites")
}

/// The list the built game was lifted with (`fefw.sites` next to `fefw.exe`).
pub fn built_list(root: &Path) -> PathBuf {
    crate::engine::game_exe(root).with_file_name("fefw.sites")
}

/// main's build ID from the prepared ExeFS (the NSO header's 32 bytes at 0x40), as upper-case hex without trailing zeros.
pub fn main_build_id(root: &Path) -> Option<String> {
    let data = std::fs::read(root.join("game").join("dump").join("exefs").join("main")).ok()?;
    let id = data.get(0x40..0x60)?;
    let hex: String = id.iter().map(|b| format!("{b:02X}")).collect();
    Some(hex.trim_end_matches('0').to_string()).filter(|h| !h.is_empty())
}

/// An `@nsobid` against main's build ID: a prefix of it, the rest zeros (as Atmosphere and the `.pchtxt` format match them).
pub fn bid_matches(text: &str, build_id: &str) -> bool {
    let t = text.trim().to_ascii_uppercase();
    let mut full = build_id.to_ascii_uppercase();
    while full.len() < 64 {
        full.push('0');
    }
    if t.len() < 16 || t.len() > 64 || !t.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    full.starts_with(&t) && full[t.len()..].chars().all(|c| c == '0')
}

/// Parses one `.sites` file: its `@nsobid` and its sites, checked for kind and address.
pub fn parse(text: &str, from: &str) -> Result<(Option<String>, Vec<Site>), String> {
    let mut bid = None;
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let ln = n + 1;
        let t: Vec<&str> = line.split('#').next().unwrap_or("").split_whitespace().collect();
        if t.is_empty() {
            continue;
        }
        if t[0].eq_ignore_ascii_case("@nsobid") {
            if t.len() != 2 {
                return Err(format!("{from}:{ln}: @nsobid takes one build ID"));
            }
            bid = Some(t[1].to_string());
            continue;
        }
        if !KINDS.contains(&t[0]) {
            return Err(format!("{from}:{ln}: \"{}\" is not a site a mod can add ({})", t[0], KINDS.join(", ")));
        }
        let addr = (t.len() == 2)
            .then(|| u64::from_str_radix(t[1].trim_start_matches("0x").trim_start_matches("0X"), 16).ok())
            .flatten()
            .ok_or_else(|| format!("{from}:{ln}: expected \"<kind> <guest address in hex>\""))?;
        if addr % 4 != 0 || !(MAIN_BASE..NEXT_MODULE).contains(&addr) {
            return Err(format!("{from}:{ln}: 0x{addr:x} is not an instruction address in main (main is at 0x{MAIN_BASE:x})"));
        }
        out.push(Site { from: format!("{from}:{ln}"), kind: t[0].to_string(), addr });
    }
    Ok((bid, out))
}

/// The sites every mod in `mods` adds for main build `build_id`, switched-off mods included. A mod's `exefs` may sit up to
/// three folders deep, as the Mods page finds it.
pub fn collect(mods: &Path, build_id: &str) -> Result<Vec<Site>, String> {
    let mut out = Vec::new();
    for m in crate::mods::list(mods) {
        for exefs in crate::mods::exefs_dirs(&m.path) {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&exefs)
                .map(|d| d.flatten().map(|e| e.path()).filter(|p| has_ext(p, "sites")).collect())
                .unwrap_or_default();
            files.sort();
            for f in files {
                let from = format!("{}/exefs/{}", m.name, f.file_name().unwrap_or_default().to_string_lossy());
                let text = std::fs::read_to_string(&f).map_err(|e| format!("{from}: {e}"))?;
                let (bid, sites) = parse(&text, &from)?;
                let bid = bid.ok_or_else(|| format!("{from} has no @nsobid line (the build ID of the main its sites are for)"))?;
                if bid_matches(&bid, build_id) {
                    out.extend(sites);
                }
            }
        }
    }
    Ok(out)
}

fn has_ext(p: &Path, ext: &str) -> bool {
    p.is_file() && p.extension().is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case(ext))
}

/// main.sites, plus the mods' sites that are not in it already (each checked to lie in main's code, from the prepared
/// ExeFS's NSO header when it is there). With no mod sites this is main.sites unchanged.
pub fn merged(root: &Path, sites: &[Site]) -> Result<String, String> {
    let base = std::fs::read_to_string(base_list(root)).map_err(|e| format!("{}: {e}", base_list(root).display()))?;
    if sites.is_empty() {
        return Ok(base);
    }
    let text = std::fs::read(root.join("game").join("dump").join("exefs").join("main")).ok().and_then(|d| {
        let u = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as u64);
        Some((u(0x14)?, u(0x18)?))
    });
    let mut have: std::collections::BTreeSet<(String, u64)> = base
        .lines()
        .filter_map(|l| {
            let t: Vec<&str> = l.split('#').next().unwrap_or("").split_whitespace().collect();
            (t.len() >= 2).then(|| u64::from_str_radix(t[1].trim_start_matches("0x"), 16).ok().map(|a| (t[0].to_string(), a))).flatten()
        })
        .collect();
    let mut lines = Vec::new();
    for s in sites {
        if let Some((off, size)) = text {
            let o = s.addr - MAIN_BASE;
            if o < off || o >= off + size {
                return Err(format!("{}: 0x{:x} is outside main's code", s.from, s.addr));
            }
        }
        if have.insert((s.kind.clone(), s.addr)) {
            lines.push(format!("{} 0x{:x}   # {}", s.kind, s.addr, s.from));
        }
    }
    Ok(format!("{}\n\n# ---- added by mods (exefs/*.sites)\n{}\n", base.trim_end_matches(['\r', '\n']), lines.join("\n")))
}

/// The site list for this build and whether mods add to it: writes the merged list (only when it changed, so the engine
/// lifts again only then) and returns the path to hand to the engine as `FEFW_SITES`. With no mod sites that is the
/// engine's own main.sites, so a build without code mods is the same as before.
pub fn prepare(root: &Path, mods: &Path) -> Result<(PathBuf, Vec<Site>), String> {
    let bid = main_build_id(root).ok_or("main's build ID cannot be read from game\\dump\\exefs\\main")?;
    let sites = collect(mods, &bid)?;
    if sites.is_empty() {
        return Ok((base_list(root), sites));
    }
    let text = merged(root, &sites)?;
    let path = merged_path(root);
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
        std::fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).map_err(|e| e.to_string())?;
        std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok((path, sites))
}

/// Whether the built game lacks hook sites the installed mods need: `Some(true)` when the game should be built again,
/// `Some(false)` when it has them all, `None` when that cannot be told (no game built yet, or built before fefw.sites).
pub fn stale(root: &Path, mods: &Path) -> Option<bool> {
    let built = std::fs::read_to_string(built_list(root)).ok()?;
    let bid = main_build_id(root)?;
    let sites = collect(mods, &bid).ok()?;
    let want = merged(root, &sites).ok()?;
    let norm = |s: &str| s.replace("\r\n", "\n").trim_end().to_string();
    if norm(&want) == norm(&built) {
        return Some(false);
    }
    // the built list may hold more (a mod since removed): only missing sites need a build
    let have: std::collections::BTreeSet<(String, u64)> = built
        .lines()
        .filter_map(|l| {
            let t: Vec<&str> = l.split('#').next().unwrap_or("").split_whitespace().collect();
            (t.len() >= 2).then(|| u64::from_str_radix(t[1].trim_start_matches("0x"), 16).ok().map(|a| (t[0].to_string(), a))).flatten()
        })
        .collect();
    Some(sites.iter().any(|s| !have.contains(&(s.kind.clone(), s.addr))))
}

/// Whether one mod's sites are missing from the built game (for the Mods page).
pub fn mod_missing(root: &Path, mod_path: &Path) -> bool {
    let (Some(built), Some(bid)) = (std::fs::read_to_string(built_list(root)).ok(), main_build_id(root)) else { return false };
    let have: std::collections::BTreeSet<(String, u64)> = built
        .lines()
        .filter_map(|l| {
            let t: Vec<&str> = l.split('#').next().unwrap_or("").split_whitespace().collect();
            (t.len() >= 2).then(|| u64::from_str_radix(t[1].trim_start_matches("0x"), 16).ok().map(|a| (t[0].to_string(), a))).flatten()
        })
        .collect();
    for exefs in crate::mods::exefs_dirs(mod_path) {
        for f in std::fs::read_dir(&exefs).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| has_ext(p, "sites")) {
            let Ok(text) = std::fs::read_to_string(&f) else { continue };
            let Ok((Some(b), sites)) = parse(&text, "") else { continue };
            if bid_matches(&b, &bid) && sites.iter().any(|s| !have.contains(&(s.kind.clone(), s.addr))) {
                return true;
            }
        }
    }
    false
}

/// Whether installed code mods bring sites the built game lacks, for pages drawn every frame (Overview's Play): the answer is
/// kept for two seconds, so the mods folder is not read on every frame.
pub fn stale_installed() -> bool {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut c = CACHE.lock().unwrap();
    if let Some((at, v)) = *c {
        if at.elapsed() < Duration::from_secs(2) {
            return v;
        }
    }
    let v = match (crate::engine::root(), crate::mods::dir()) {
        (Some(root), Some(dir)) => crate::mods::list(&dir).iter().any(|m| m.has_code) && stale(&root, &dir) == Some(true),
        _ => false,
    };
    *c = Some((Instant::now(), v));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    const BID: &str = "DB93B0125F4850601D61C3EDDE012F8EB2ECFE77";

    #[test]
    fn build_ids_match_like_atmosphere() {
        assert!(bid_matches(BID, BID));
        assert!(bid_matches(&BID.to_lowercase(), BID));
        assert!(!bid_matches(&BID[..16], BID)); // a shorter ID matches only when the rest is zeros
        assert!(bid_matches(&format!("{BID}000000"), BID));
        assert!(!bid_matches("0123456789ABCDEF", BID));
    }

    #[test]
    fn parses_and_checks_sites() {
        let (bid, s) = parse(&format!("# x\n@nsobid {BID}\nmidhook 0x7100e1d03c # gate\nentryhook 7100DB9700\n"), "m/exefs/a.sites").unwrap();
        assert_eq!(bid.as_deref(), Some(BID));
        assert_eq!(s.len(), 2);
        assert_eq!((s[0].kind.as_str(), s[0].addr, s[0].from.as_str()), ("midhook", 0x7100e1d03c, "m/exefs/a.sites:3"));
        assert_eq!(s[1].addr, 0x7100db9700);
        for (bad, why) in [
            ("patch 0x7100e1d03c 1 2", "not a site"),
            ("midhook 0x7100e1d03e", "not an instruction"),
            ("midhook 0x7500000000", "not an instruction"),
            ("midhook zz", "expected"),
        ] {
            let e = parse(bad, "f").unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }
}
