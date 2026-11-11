//! `ew-inspect` — evidence collector for an extracted Fire Emblem: Fortune's
//! Weave dump.
//!
//! It never writes to the game data. It parses the ExeFS modules, decompresses
//! every segment it can, inventories the romfs, probes asset containers, and
//! reports anything that blocks progress as an explicit blocker rather than
//! silently skipping it.

use anyhow::{bail, Context, Result};
use ew_format::{Blocker, Compression, ExeFs, GameImage, Inventory, Nso, SegmentKind};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
struct SegmentReport {
    kind: String,
    compression: String,
    stored_bytes: u64,
    declared_bytes: u32,
    decoded_bytes: Option<u64>,
    fingerprint: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ModuleReport {
    file_name: String,
    module_name: String,
    module_id: String,
    flags: u32,
    file_size: u64,
    image_size: u64,
    segments: Vec<SegmentReport>,
    has_mod0: Option<bool>,
    blocked_by: Option<String>,
}

#[derive(Debug, Serialize)]
struct Report {
    generated_by: String,
    exefs_root: Option<String>,
    romfs_root: Option<String>,
    modules: Vec<ModuleReport>,
    inventory: Option<Inventory>,
    probes: Vec<ew_format::ProbeResult>,
    blockers: Vec<BlockerLine>,
    notes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct BlockerLine {
    id: String,
    summary: String,
    affects: Vec<String>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("ew-inspect: {e:#}");
        std::process::exit(1);
    }
}

struct Args {
    exefs: Option<PathBuf>,
    romfs: Option<PathBuf>,
    json: Option<PathBuf>,
    probe: usize,
    inventory: bool,
}

fn parse_args() -> Result<Args> {
    let mut args = Args { exefs: None, romfs: None, json: None, probe: 2, inventory: true };
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--exefs" => args.exefs = Some(PathBuf::from(next(&mut it, "--exefs")?)),
            "--romfs" => args.romfs = Some(PathBuf::from(next(&mut it, "--romfs")?)),
            "--json" => args.json = Some(PathBuf::from(next(&mut it, "--json")?)),
            "--probe" => {
                args.probe = next(&mut it, "--probe")?
                    .parse()
                    .context("--probe expects a number")?
            }
            "--no-inventory" => args.inventory = false,
            "-h" | "--help" => {
                println!(
                    "ew-inspect — inspect an extracted Fire Emblem: Fortune's Weave dump\n\
                     \n\
                     USAGE:\n  ew-inspect [--exefs DIR] [--romfs DIR] [--json OUT] [--probe N]\n\
                     \n\
                     With no arguments, the current directory is searched for `exefs/`\n\
                     and `romfs/` subdirectories."
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument `{other}` (try --help)"),
        }
    }
    Ok(args)
}

fn next(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    it.next().with_context(|| format!("{flag} needs a value"))
}

fn run() -> Result<()> {
    let mut args = parse_args()?;
    if args.exefs.is_none() && args.romfs.is_none() {
        // Discover a conventional layout: <root>/exefs and <root>/romfs.
        if Path::new("exefs").is_dir() {
            args.exefs = Some(PathBuf::from("exefs"));
        }
        if Path::new("romfs").is_dir() {
            args.romfs = Some(PathBuf::from("romfs"));
        }
    }
    if args.exefs.is_none() && args.romfs.is_none() {
        bail!("nothing to inspect: pass --exefs DIR and/or --romfs DIR (try --help)");
    }

    let mut report = Report {
        generated_by: format!("ew-inspect {}", env!("CARGO_PKG_VERSION")),
        exefs_root: args.exefs.as_ref().map(|p| p.display().to_string()),
        romfs_root: args.romfs.as_ref().map(|p| p.display().to_string()),
        modules: Vec::new(),
        inventory: None,
        probes: Vec::new(),
        blockers: Vec::new(),
        notes: Vec::new(),
    };

    if let Some(exefs_dir) = &args.exefs {
        let exefs = ExeFs::scan_dir(exefs_dir)?;
        if let Some(npdm) = exefs.read_npdm()? {
            report
                .notes
                .push(format!("NPDM: category={}, name={}", npdm.process_category, npdm.name));
        }
        println!("== ExeFS: {} ==", exefs_dir.display());
        println!(
            "{:<10} {:<10} {:>10} {:>11} {:>7} {:<12} {}",
            "file", "module", "file size", "image size", "flags", "compress", "status"
        );

        let mut blocked_modules = Vec::new();
        for m in &exefs.modules {
            let mut segments = Vec::new();
            let mut decoded_image: Option<GameImage> = None;
            let mut module_error: Option<String> = None;

            if m.is_zbic() {
                blocked_modules.push(m.file_name.clone());
            } else {
                let bytes = std::fs::read(&m.path)?;
                let nso = Nso::parse(&bytes)?;
                let mut parts = Vec::new();
                for kind in SegmentKind::ALL {
                    let seg = nso.header.segment(kind);
                    let comp = nso.header.compression(kind);
                    let stored = stored_len(&nso, kind);
                    match nso.decompress_segment(kind) {
                        Ok(data) => parts.push(SegmentReport {
                            kind: kind.as_str().to_string(),
                            compression: comp_name(comp).to_string(),
                            stored_bytes: stored,
                            declared_bytes: seg.size,
                            decoded_bytes: Some(data.len() as u64),
                            fingerprint: Some(fnv_hex(&data)),
                            error: None,
                        }),
                        Err(e) => {
                            parts.push(SegmentReport {
                                kind: kind.as_str().to_string(),
                                compression: comp_name(comp).to_string(),
                                stored_bytes: stored,
                                declared_bytes: seg.size,
                                decoded_bytes: None,
                                fingerprint: None,
                                error: Some(format!("{e:#}")),
                            });
                            module_error = Some(format!("{}: {e:#}", kind.as_str()));
                        }
                    }
                }
                match nso.to_image() {
                    Ok(img) => decoded_image = Some(img),
                    Err(e) if module_error.is_none() => module_error = Some(format!("{e:#}")),
                    Err(_) => {}
                }
                segments = parts;
            }

            let has_mod0 = decoded_image.as_ref().map(|i| i.find_mod0().is_some());

            if m.is_zbic() {
                // Report the real stored ranges even though the payload cannot
                // be decoded yet, so the cost of the blocker is visible.
                let bytes = std::fs::read(&m.path)?;
                let nso = Nso::parse(&bytes)?;
                segments = SegmentKind::ALL
                    .iter()
                    .map(|kind| {
                        let seg = nso.header.segment(*kind);
                        SegmentReport {
                            kind: kind.as_str().to_string(),
                            compression: "zbic".to_string(),
                            stored_bytes: stored_len(&nso, *kind),
                            declared_bytes: seg.size,
                            decoded_bytes: None,
                            fingerprint: None,
                            error: Some("blocked: ZBIC compression not implemented".to_string()),
                        }
                    })
                    .collect();
            }

            println!(
                "{:<10} {:<10} {:>10} {:>11} {:>7} {:<12} {}",
                m.file_name,
                m.module_name,
                human(m.file_size),
                human(m.image_size()),
                format!("{:02x}", m.flags),
                m.compressions.join("+"),
                if m.is_zbic() {
                    "blocked (ZBIC)"
                } else if module_error.is_some() {
                    "decode error"
                } else {
                    "decoded + verified"
                }
            );

            report.modules.push(ModuleReport {
                file_name: m.file_name.clone(),
                module_name: m.module_name.clone(),
                module_id: m.module_id.clone(),
                flags: m.flags,
                file_size: m.file_size,
                image_size: m.image_size(),
                segments,
                has_mod0,
                blocked_by: if m.is_zbic() { Some(Blocker::ZbicCompression.id().to_string()) } else { None },
            });
        }

        if !blocked_modules.is_empty() {
            report.blockers.push(BlockerLine {
                id: Blocker::ZbicCompression.id().to_string(),
                summary: Blocker::ZbicCompression.summary().to_string(),
                affects: blocked_modules,
            });
        }
        println!();
    }

    if let Some(romfs_dir) = &args.romfs {
        if args.inventory {
            eprintln!("scanning romfs (metadata only)…");
            let inv = Inventory::scan(romfs_dir)?;
            println!("== romfs: {} ==", romfs_dir.display());
            println!(
                "{} files, {} total",
                inv.total_files,
                human(inv.total_bytes)
            );
            println!("\ntop-level directories (by size):");
            for d in inv.dirs.iter().take(12) {
                println!("  {:<14} {:>7} files  {:>10}", d.name, d.files, human(d.bytes));
            }
            println!("\nextensions (by count):");
            for e in inv.extensions.iter().take(12) {
                println!("  .{:<12} {:>7} files  {:>10}", e.ext, e.files, human(e.bytes));
            }
            report.inventory = Some(inv);
        }

        if args.probe > 0 {
            eprintln!("probing asset containers…");
            let probes = ew_format::probe_samples(romfs_dir, args.probe)?;
            println!("\ninner formats (sample of {} per extension):", args.probe);
            for p in &probes {
                println!("  {:<58} {:<12} {}", truncate(&p.relative, 58), p.format.id, p.format.description);
            }
            report.probes = probes;
        }
    }

    if let Some(out) = &args.json {
        let json = serde_json::to_string_pretty(&report)?;
        std::fs::write(out, json).with_context(|| format!("writing {}", out.display()))?;
        println!("\nwrote {}", out.display());
    }

    if !report.blockers.is_empty() {
        println!("\n== blockers ==");
        for b in &report.blockers {
            println!("  {} affects {:?}", b.id, b.affects);
            println!("    {}", b.summary);
        }
    }
    Ok(())
}

fn stored_len(nso: &Nso<'_>, kind: SegmentKind) -> u64 {
    let start = nso.header.segment(kind).file_offset as u64;
    let end = match kind {
        SegmentKind::Text => nso.header.segment(SegmentKind::Rodata).file_offset as u64,
        SegmentKind::Rodata => nso.header.segment(SegmentKind::Data).file_offset as u64,
        SegmentKind::Data => nso.bytes_len() as u64,
    };
    end.saturating_sub(start)
}

fn comp_name(c: Compression) -> &'static str {
    match c {
        Compression::None => "none",
        Compression::Lz4 => "lz4",
        Compression::Zbic => "zbic",
    }
}

/// FNV-1a 64-bit: a cheap content fingerprint for the evidence report, so a
/// decoded segment can be compared across runs and machines.
fn fnv_hex(data: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    format!("{h:016x}")
}

fn human(n: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
