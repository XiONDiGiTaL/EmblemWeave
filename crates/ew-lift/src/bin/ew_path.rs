//! `ew-path` — report how far execution can reach from a starting address.
//!
//! The probe follows call and branch edges through the recovered control flow
//! graph and stops where the translation cannot continue. Its job is to answer
//! one question with evidence rather than opinion: what is the first thing that
//! blocks the game, and how much code sits behind that point.
//!
//! Nothing here executes guest code. Every number is derived from the static
//! graph, so the report cannot drift away from the lifter's actual behaviour.

use anyhow::{bail, Context, Result};
use ew_lift::{scan, Op};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::PathBuf;

struct Args {
    text: PathBuf,
    entry: u64,
    max_functions: usize,
    max_insns: usize,
    segments: Vec<(u64, PathBuf)>,
    verbose: bool,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("ew-path: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = Args {
        text: PathBuf::new(),
        entry: 0,
        max_functions: 4096,
        max_insns: 512,
        segments: Vec::new(),
        verbose: false,
    };
    let mut text_given = false;

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--text" => {
                args.text = PathBuf::from(value(&mut it, "--text")?);
                text_given = true;
            }
            "--entry" => {
                let v = value(&mut it, "--entry")?;
                args.entry = u64::from_str_radix(v.trim_start_matches("0x"), 16)
                    .context("--entry expects a hex address")?;
            }
            "--max-functions" => args.max_functions = value(&mut it, "--max-functions")?.parse()?,
            "--max-insns" => args.max_insns = value(&mut it, "--max-insns")?.parse()?,
            "--verbose" => args.verbose = true,
            "--segment" => {
                let v = value(&mut it, "--segment")?;
                let (addr, path) = v.split_once(':').context("--segment expects ADDR:PATH")?;
                let addr = u64::from_str_radix(addr.trim_start_matches("0x"), 16)
                    .context("--segment address must be hex")?;
                args.segments.push((addr, PathBuf::from(path)));
            }
            "-h" | "--help" => {
                println!(
                    "ew-path — reachability probe for a lifted module\n\
                     \n\
                     USAGE:\n  ew-path --text <text.bin> [--entry ADDR] [options]\n\
                     \n\
                     OPTIONS:\n\
                     \x20 --entry ADDR          start address in hex (default 0)\n\
                     \x20 --max-functions N     stop after N reachable functions\n\
                     \x20 --max-insns N        instruction cap per function\n\
                     \x20 --segment A:P        map a module segment at hex address A\n"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument `{other}` (try --help)"),
        }
    }
    if !text_given {
        bail!("--text is required (try --help)");
    }

    let text = std::fs::read(&args.text)
        .with_context(|| format!("reading text segment {}", args.text.display()))?;
    if text.len() % 4 != 0 {
        bail!("text segment length {} is not a multiple of 4", text.len());
    }

    let mut segment_bytes = Vec::new();
    for (addr, path) in &args.segments {
        segment_bytes.push((*addr, std::fs::read(path).with_context(|| format!("reading {}", path.display()))?));
    }
    let segments: Vec<(u64, &[u8])> = segment_bytes
        .iter()
        .map(|(addr, bytes)| (*addr, bytes.as_slice()))
        .collect();

    let boundaries = scan::call_targets(&text);
    let mut seen: BTreeSet<u64> = BTreeSet::new();
    let mut queue: VecDeque<u64> = VecDeque::new();
    queue.push_back(args.entry);

    let mut functions = 0usize;
    let mut blocks = 0usize;
    let mut instructions = 0usize;
    let mut unsupported: BTreeMap<u64, u32> = BTreeMap::new();
    let mut call_edges = 0usize;
    let mut external_calls = 0usize;
    let mut unresolved_edges = 0usize;
    let mut truncated = false;

    while let Some(start) = queue.pop_front() {
        if !seen.insert(start) {
            continue;
        }
        if functions >= args.max_functions {
            truncated = true;
            break;
        }
        let func = match ew_lift::cfg::build(&text, &segments, start, args.max_insns, &boundaries) {
            Some(f) => f,
            None => continue,
        };
        functions += 1;
        blocks += func.blocks.len();
        instructions += func.insn_count();
        unresolved_edges += func.unresolved_edges;

        if args.verbose {
            println!("function 0x{start:x}: {} blocks, {} calls", func.blocks.len(), if func.has_calls { "yes" } else { "no" });
            for block in &func.blocks {
                let kinds: Vec<String> = block
                    .edges
                    .iter()
                    .map(|e| match e.target {
                        Some(t) => format!("{:?}->0x{t:x}", e.kind),
                        None => format!("{:?}", e.kind),
                    })
                    .collect();
                println!("  block 0x{:x}: {} insns, edges [{}]", block.addr, block.insns.len(), kinds.join(", "));
            }
        }

        for block in &func.blocks {
            for insn in &block.insns {
                if matches!(insn.op, Op::Unsupported { .. } | Op::UnsupportedBitfield { .. }) {
                    unsupported.insert(insn.addr, insn.word);
                }
            }
            for edge in &block.edges {
                if edge.kind == scan::EdgeKind::Call {
                    if let Some(target) = edge.target {
                        if start_of_code(&text, target) {
                            call_edges += 1;
                            queue.push_back(target);
                        } else {
                            // The callee is outside this module: an import or a
                            // jump slot, which is where the host runtime has to
                            // take over.
                            external_calls += 1;
                        }
                    }
                }
            }
        }
    }

    println!("entry:             0x{:x}", args.entry);
    println!("reachable funcs:   {functions}{}", if truncated { " (capped)" } else { "" });
    println!("reachable blocks:  {blocks}");
    println!("instructions:      {instructions}");
    println!("internal calls:    {call_edges}");
    println!("external calls:    {external_calls}");
    println!("unresolved edges:  {unresolved_edges}");
    println!("blocked insns:     {}", unsupported.len());

    // The earliest blocked instruction is the first thing the runtime will hit
    // when it walks the entry path in address order, which is why it is named
    // explicitly rather than left to a histogram.
    if let Some((addr, word)) = unsupported.iter().next() {
        println!();
        println!("first blocker:     0x{addr:x}  word 0x{word:08x}");
    }

    let mut families: BTreeMap<u32, usize> = BTreeMap::new();
    for word in unsupported.values() {
        *families.entry((word >> 24) & 0x7F).or_insert(0) += 1;
    }
    let mut rows: Vec<(u32, usize)> = families.into_iter().collect();
    rows.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    if !rows.is_empty() {
        println!("blocked by group (bits 30:24):");
        for (family, count) in rows.into_iter().take(10) {
            println!("  family 0x{family:02x}  {count:>8} instructions");
        }
    }

    if text.is_empty() || !start_of_code(&text, args.entry) {
        bail!("entry 0x{:x} is not inside the text segment", args.entry);
    }
    Ok(())
}

/// True when an address lands on an aligned word inside the text segment.
fn start_of_code(text: &[u8], addr: u64) -> bool {
    addr % 4 == 0 && (addr as usize) < text.len()
}

fn value(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    it.next().with_context(|| format!("{flag} needs a value"))
}
