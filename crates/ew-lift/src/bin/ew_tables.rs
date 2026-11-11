//! `ew-tables` — measure jump-table recovery over a whole text segment.
//!
//! Every `br` in the image is an indirect branch. This tool asks the same
//! detector the control-flow recovery uses whether that `br` can be resolved to
//! concrete successors, and prints the instruction window of the ones that fail
//! so a pattern the detector is missing shows up as evidence rather than a guess.
//!
//! ```text
//! ew-tables --text text.bin [--segment ADDR:rodata.bin] [--show 8] [--show-resolved 4]
//! ```

use anyhow::{bail, Context, Result};
use ew_lift::jump_table;
use ew_lift::scan::word_at;
use ew_lift::{decode, Op};
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprintln!("ew-tables: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut text_path = PathBuf::new();
    let mut segments: Vec<(u64, PathBuf)> = Vec::new();
    let mut show_fail = 0usize;
    let mut show_ok = 0usize;
    let mut window_words = 96usize;
    let mut reason_filter = String::new();

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--text" => text_path = PathBuf::from(next(&mut it, "--text")?),
            "--segment" => {
                let v = next(&mut it, "--segment")?;
                let (addr, path) = v.split_once(':').context("--segment expects ADDR:PATH")?;
                let addr = u64::from_str_radix(addr.trim_start_matches("0x"), 16)
                    .context("--segment address must be hex")?;
                segments.push((addr, PathBuf::from(path)));
            }
            "--show" => show_fail = next(&mut it, "--show")?.parse()?,
            "--reason" => reason_filter = next(&mut it, "--reason")?,
            "--window" => window_words = next(&mut it, "--window")?.parse()?,
            "--show-resolved" => show_ok = next(&mut it, "--show-resolved")?.parse()?,
            "-h" | "--help" => {
                println!(
                    "ew-tables --text <text.bin> [--segment ADDR:rodata.bin] [--window N] [--show N] [--show-resolved N]"
                );
                return Ok(());
            }
            other => bail!("unknown argument `{other}`"),
        }
    }
    if text_path.as_os_str().is_empty() {
        bail!("--text is required");
    }

    let text =
        std::fs::read(&text_path).with_context(|| format!("reading {}", text_path.display()))?;
    let segment_bytes: Vec<(u64, Vec<u8>)> = segments
        .iter()
        .map(|(addr, path)| {
            std::fs::read(path)
                .map(|bytes| (*addr, bytes))
                .with_context(|| format!("reading {}", path.display()))
        })
        .collect::<Result<_>>()?;
    let segs: Vec<(u64, &[u8])> = segment_bytes.iter().map(|(a, b)| (*a, b.as_slice())).collect();

    let mut brs = 0usize;
    let mut resolved = 0usize;
    let mut targets_total = 0usize;
    let mut shown_fail = 0usize;
    let mut shown_ok = 0usize;
    let mut failures: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();

    let mut addr = 0u64;
    while addr + 4 <= text.len() as u64 {
        let word = match word_at(&text, addr) {
            Some(w) => w,
            None => break,
        };
        if matches!(decode(word, addr), Op::Br { .. }) {
            brs += 1;
            let window = window_before(&text, addr, window_words);
            let known = jump_table::prologue_constants(&text, addr, 24);
            match jump_table::detect_with_reason(&text, &segs, &known, &window) {
                Ok(table) => {
                    resolved += 1;
                    targets_total += table.targets.len();
                    if shown_ok < show_ok {
                        println!(
                            "resolved br @0x{addr:x}: table 0x{:x} base 0x{:x}, {} entries, {} targets, {} byte entries, shift {}, signed {}",
                            table.table,
                            table.base,
                            table.entries,
                            table.targets.len(),
                            table.entry_bytes,
                            table.shift,
                            table.signed
                        );
                        shown_ok += 1;
                    }
                }
                Err(reason) => {
                    *failures.entry(reason.as_str()).or_insert(0) += 1;
                    if shown_fail < show_fail
                        && (reason_filter.is_empty() || reason.as_str().contains(reason_filter.as_str()))
                    {
                        println!("unresolved br @0x{addr:x}:");
                        let start = window.len().saturating_sub(10);
                        for insn in &window[start..] {
                            println!("  0x{:08x}  {:08x}  {:?}", insn.addr, insn.word, insn.op);
                        }
                        shown_fail += 1;
                    }
                }
            }
        }
        addr += 4;
    }

    println!(
        "br instructions: {brs}  resolved: {resolved} ({:.1}%)  table targets: {targets_total}",
        if brs == 0 { 0.0 } else { resolved as f64 / brs as f64 * 100.0 }
    );
    let mut reasons: Vec<_> = failures.into_iter().collect();
    reasons.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (reason, count) in reasons {
        println!("  {count:6}  {reason}");
    }
    Ok(())
}

/// Decode `words` instructions before `addr`, then the instruction at `addr`.
fn window_before(text: &[u8], addr: u64, words: usize) -> Vec<ew_lift::Insn> {
    let start = addr.saturating_sub(words as u64 * 4);
    let mut insns = Vec::with_capacity(words + 1);
    let mut at = start;
    while at <= addr {
        if let Some(word) = word_at(text, at) {
            insns.push(ew_lift::Insn { addr: at, word, op: decode(word, at) });
        }
        at += 4;
    }
    insns
}

fn next(it: &mut impl Iterator<Item = String>, name: &str) -> Result<String> {
    it.next().with_context(|| format!("{name} needs a value"))
}
