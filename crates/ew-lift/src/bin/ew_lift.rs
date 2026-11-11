//! `ew-lift` — lift a decompressed text segment into C.
//!
//! Input is either a raw `text.bin` produced by `ew-inspect`/`zbic_extract.py`
//! or an NSO module that the pipeline can decompress itself.

use anyhow::{bail, Context, Result};
use ew_lift::{
    decode, dispatch_header, emit_campaign, emit_harness, emit_module, interpreter, lift_function,
    lift_text_in, Op, LiftOptions, RUNTIME_HEADER,
};
use std::collections::HashMap;
use std::path::PathBuf;

struct Args {
    text: PathBuf,
    module: String,
    out: PathBuf,
    opts: LiftOptions,
    harness: bool,
    interpret: bool,
    families: bool,
    segments: Vec<(u64, PathBuf)>,
    campaign: Option<usize>,
    simd_campaign: Option<usize>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("ew-lift: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = Args {
        text: PathBuf::new(),
        module: "app".to_string(),
        out: PathBuf::from("build/lift"),
        opts: LiftOptions::default(),
        harness: false,
        interpret: false,
        families: false,
        segments: Vec::new(),
        campaign: None,
        simd_campaign: None,
    };
    let mut text_given = false;

    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--text" => {
                args.text = PathBuf::from(value(&mut it, "--text")?);
                text_given = true;
            }
            "--module" => args.module = value(&mut it, "--module")?,
            "--out" => args.out = PathBuf::from(value(&mut it, "--out")?),
            "--limit" => args.opts.max_functions = value(&mut it, "--limit")?.parse()?,
            "--max-insns" => args.opts.max_insns = value(&mut it, "--max-insns")?.parse()?,
            "--entry" => {
                let v = value(&mut it, "--entry")?;
                args.opts.entry = u64::from_str_radix(v.trim_start_matches("0x"), 16)
                    .context("--entry expects a hex address")?;
            }
            "--pure-leaf" => args.opts.pure_leaf_only = true,
            "--harness" => args.harness = true,
            "--interpret" => args.interpret = true,
            "--families" => args.families = true,
            "--simd-campaign" => {
                args.simd_campaign = Some(value(&mut it, "--simd-campaign")?.parse()?);
            }
            "--campaign" => {
                args.campaign = Some(value(&mut it, "--campaign")?.parse()?);
            }
            "--segment" => {
                let v = value(&mut it, "--segment")?;
                let (addr, path) = v
                    .split_once(':')
                    .context("--segment expects ADDR:PATH")?;
                let addr = u64::from_str_radix(addr.trim_start_matches("0x"), 16)
                    .context("--segment address must be hex")?;
                args.segments.push((addr, PathBuf::from(path)));
            }
            "-h" | "--help" => {
                println!(
                    "ew-lift — lift an AArch64 text segment of Fire Emblem: Fortune's Weave to C\n\
                     \n\
                     USAGE:\n  ew-lift --text <text.bin> [--module NAME] [--out DIR] [options]\n\
                     \n\
                     OPTIONS:\n\
                     \x20 --limit N        lift at most N blocks (default 512)\n\
                     \x20 --max-insns N   instruction cap per block (default 256)\n\
                     \x20 --pure-leaf     keep only register-only blocks that end in ret/br\n\
                     \x20 --entry ADDR    module entry point in hex\n\
                     \x20 --harness       also emit a self-checking C harness\n\
                     \x20 --interpret     run the reference interpreter on the first pure block\n\
                     \x20 --families      report the largest unsupported instruction groups\n\
                     \x20 --segment A:P   map module segment at hex address A from file P\n\
                     \x20                (use for rodata so jump tables can be resolved)\n\
                     \x20 --campaign N    write a C program that runs N functions plus\n\
                     \x20                expected.txt from the reference interpreter\n"
                );
                std::process::exit(0);
            }
            other => bail!("unknown argument `{other}` (try --help)"),
        }
    }
    if !text_given {
        bail!("--text is required (try --help)");
    }
    args.opts.module = args.module.clone();

    let text = std::fs::read(&args.text)
        .with_context(|| format!("reading text segment {}", args.text.display()))?;
    if text.len() % 4 != 0 {
        bail!("text segment length {} is not a multiple of 4", text.len());
    }

    let mut segment_bytes = Vec::new();
    for (addr, path) in &args.segments {
        let bytes = std::fs::read(path)
            .with_context(|| format!("reading segment {}", path.display()))?;
        segment_bytes.push((*addr, bytes));
    }
    let segments: Vec<(u64, &[u8])> = segment_bytes
        .iter()
        .map(|(addr, bytes)| (*addr, bytes.as_slice()))
        .collect();

    let (report, funcs) = lift_text_in(&text, &segments, &args.opts);
    std::fs::create_dir_all(&args.out)?;

    let c_path = args.out.join(format!("{}_lift.c", args.module));
    std::fs::write(&c_path, emit_module(&args.module, &funcs))?;
    std::fs::write(args.out.join("ew_aarch64.h"), RUNTIME_HEADER)?;
    std::fs::write(args.out.join("ew_dispatch.h"), dispatch_header())?;
    std::fs::write(
        args.out.join("lift_report.json"),
        serde_json::to_string_pretty(&report)?,
    )?;

    println!("module:            {}", report.module);
    println!("text:              {} bytes ({} words)", report.text_bytes, report.text_words);
    println!(
        "decode coverage:   {:.2}% of {} real words ({} padding, {} unsupported)",
        report.decode_coverage * 100.0,
        report.text_words.saturating_sub(report.padding_words),
        report.padding_words,
        report.unsupported_words
    );
    println!("candidate starts:  {}", report.candidate_starts);
    println!(
        "lifted functions:  {} ({} complete, {} pure leaf)",
        report.lifted_functions, report.complete_functions, report.pure_leaf_functions
    );
    println!("basic blocks:      {}", report.blocks_lifted);
    println!("instructions:      {}", report.instructions_lifted);
    println!("jump tables:       {} ({} targets)", report.jump_tables, report.jump_table_targets);
    println!("unresolved edges:  {}", report.unresolved_edges);
    println!(
        "partial functions: {} budget, {} unterminated block, {} indirect",
        report.incomplete_budget, report.incomplete_unterminated, report.incomplete_indirect
    );
    println!("unsupported (in):  {}", report.unsupported_in_lifted);
    println!("wrote:             {}", c_path.display());

    if args.families {
        print_families(&text);
    }

    // Verification targets the largest register-only function, preferring one
    // with more than a single block: a one-instruction `ret` proves very little.
    let best = || {
        funcs
            .iter()
            .filter(|f| f.is_pure_leaf())
            .max_by_key(|f| (f.blocks.len(), f.insn_count()))
    };

    if args.harness {
        let target = best();
        if let Some(f) = target {
            let path = args.out.join("harness.c");
            std::fs::write(&path, emit_harness(&args.module, f))?;
            println!("harness:           {} (guest 0x{:x})", path.display(), f.addr);
        } else {
            println!("harness:           no pure-leaf block available");
        }
    }

    if args.interpret {
        let target = best();
        match target {
            None => println!("interpret:         no pure-leaf block available"),
            Some(f) => {
                let mut st = interpreter::State::patterned();
                interpreter::run_function(&mut st, f, 100_000).map_err(|e| {
                    anyhow::anyhow!("interpreting guest 0x{:x}: {e}", f.addr)
                })?;
                println!(
                    "interpret:         guest 0x{:x} ({} blocks, {} instructions)",
                    f.addr,
                    f.blocks.len(),
                    f.insn_count()
                );
                println!("  halted={} pc=0x{:x} nzcv=0x{:x}", st.halted, st.pc, st.nzcv);
                for i in 0..8 {
                    println!("  x{i}=0x{:x}", st.x[i]);
                }
            }
        }
    }

    if let Some(wanted) = args.simd_campaign {
        let (cases, expected, skipped) = build_simd_campaign(&text, wanted);
        let c_path = args.out.join("simd_campaign.c");
        std::fs::write(&c_path, ew_lift::emit_simd_campaign(&cases, SIMD_SEEDS_PER_CASE))?;
        std::fs::write(args.out.join("simd_expected.txt"), expected)?;
        println!(
            "simd campaign:     {} distinct encodings x {} states in {} ({} skipped: reference faulted)",
            cases.len(),
            SIMD_SEEDS_PER_CASE,
            c_path.display(),
            skipped
        );
    }

    if let Some(wanted) = args.campaign {
        let (chosen, expected, skipped) = build_campaign(&funcs, wanted);
        if chosen.is_empty() {
            bail!("no function completed a reference run; nothing to verify");
        }
        let c_path = args.out.join("campaign.c");
        std::fs::write(&c_path, emit_campaign(&args.module, &chosen))?;
        std::fs::write(args.out.join("expected.txt"), expected)?;
        println!(
            "campaign:          {} function(s) in {} ({} skipped by the interpreter)",
            chosen.len(),
            c_path.display(),
            skipped
        );
    }

    // Sanity: never emit an empty module silently.
    if funcs.is_empty() {
        bail!("no blocks were lifted; check --entry/--limit");
    }
    let _ = lift_function;
    Ok(())
}

const SIMD_SEEDS_PER_CASE: u32 = 6;

/// The seeded state shared with the C campaign's `ew_seed`.
fn simd_state(seed: u32) -> interpreter::State {
    let mut st = interpreter::State::patterned();
    for r in 0..32u32 {
        for h in 0..2u32 {
            st.v[r as usize][h as usize] = ew_lift::simd_seed_vector(seed, r, h);
        }
    }
    for x in st.x.iter_mut() {
        *x += seed as u64 * 0x100;
    }
    for r in 0..0x4000usize {
        st.mem[r] = (r as u32).wrapping_mul(37).wrapping_add(seed) as u8;
    }
    st
}

fn state_line(addr: u64, st: &interpreter::State) -> String {
    let mut line = format!("fn=0x{addr:x} halted={} pc={:x} nzcv={:x}", st.halted as u32, st.pc, st.nzcv);
    for i in 0..31 {
        line.push_str(&format!(" x{i}={:x}", st.x[i]));
    }
    line.push_str(&format!(" sp={:x}", st.sp));
    for i in 0..32 {
        line.push_str(&format!(" v{i}={:x}:{:x}", st.v[i][1], st.v[i][0]));
    }
    line.push('\n');
    line
}

/// Every distinct Advanced SIMD encoding in the text, up to `wanted`, with the
/// state the reference produces from each seeded register file.
fn build_simd_campaign(text: &[u8], wanted: usize) -> (Vec<(u64, ew_lift::Op)>, String, usize) {
    let mut seen = std::collections::HashSet::new();
    let mut cases = Vec::new();
    let mut expected = String::new();
    let mut skipped = 0;
    for (i, chunk) in text.chunks_exact(4).enumerate() {
        if cases.len() >= wanted {
            break;
        }
        let word = u32::from_le_bytes(chunk.try_into().unwrap());
        let addr = (i * 4) as u64;
        let op = decode(word, addr);
        if !matches!(op, ew_lift::Op::Simd(..) | ew_lift::Op::SimdMem(..)) || !seen.insert(word) {
            continue;
        }
        let mut lines = String::new();
        let mut ok = true;
        for seed in 0..SIMD_SEEDS_PER_CASE {
            let mut st = simd_state(seed);
            let r = match op {
                ew_lift::Op::Simd(ref s) => ew_lift::simd::exec(&mut st, s),
                ew_lift::Op::SimdMem(ref m) => ew_lift::simd::exec_mem(&mut st, m),
                _ => unreachable!(),
            };
            if r.is_err() {
                ok = false;
                break;
            }
            lines.push_str(&state_line(addr, &st));
        }
        if ok {
            expected.push_str(&lines);
            cases.push((addr, op));
        } else {
            skipped += 1;
        }
    }
    (cases, expected, skipped)
}

/// Choose functions that the reference interpreter can run to completion, and
/// record the exact state each one should produce.
fn build_campaign(
    funcs: &[ew_lift::LiftedFunction],
    wanted: usize,
) -> (Vec<ew_lift::LiftedFunction>, String, usize) {
    let mut candidates: Vec<&ew_lift::LiftedFunction> = funcs
        .iter()
        .filter(|f| f.complete && !f.has_calls)
        .collect();
    candidates.sort_by_key(|f| std::cmp::Reverse((f.insn_count(), f.blocks.len())));

    let mut chosen = Vec::new();
    let mut expected = String::new();
    let mut skipped = 0usize;
    for f in candidates {
        if chosen.len() >= wanted {
            break;
        }
        let mut st = interpreter::State::patterned();
        match interpreter::run_function(&mut st, f, 5000) {
            Ok(()) => {
                expected.push_str(&format!(
                    "fn=0x{:x} halted={} pc={:x} nzcv={:x}",
                    f.addr, st.halted as u32, st.pc, st.nzcv
                ));
                for i in 0..31 {
                    expected.push_str(&format!(" x{i}={:x}", st.x[i]));
                }
                expected.push_str(&format!(" sp={:x}", st.sp));
                for i in 0..32 {
                    expected.push_str(&format!(" v{i}={:x}:{:x}", st.v[i][1], st.v[i][0]));
                }
                expected.push('\n');
                chosen.push(f.clone());
            }
            Err(_) => skipped += 1,
        }
    }
    (chosen, expected, skipped)
}

/// Report the largest groups of words the decoder still rejects. Grouping by
/// bits 30:24 separates instruction families; the most common exact encoding in
/// each group is printed so the next decoder addition can be chosen from data.
fn print_families(text: &[u8]) {
    let mut families: HashMap<u32, (usize, HashMap<u32, usize>)> = HashMap::new();
    let mut words: HashMap<u32, usize> = HashMap::new();
    let mut total = 0usize;
    for (_, word) in ew_lift::scan::words(text) {
        if word == 0 {
            continue;
        }
        if matches!(decode(word, 0), Op::Unsupported { .. } | Op::UnsupportedBitfield { .. }) {
            total += 1;
            *words.entry(word).or_insert(0) += 1;
            let entry = families.entry((word >> 24) & 0x7F).or_default();
            entry.0 += 1;
            *entry.1.entry(word).or_insert(0) += 1;
        }
    }
    let mut rows: Vec<_> = families.into_iter().collect();
    rows.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
    println!("unsupported by group (bits 30:24):");
    for (family, (count, words)) in rows.into_iter().take(20) {
        let (word, seen) = words
            .into_iter()
            .max_by_key(|(_, n)| *n)
            .unwrap_or((0, 0));
        println!(
            "  family 0x{family:02x}  {count:>9} words ({:.1}%)  common 0x{word:08x} x{seen}",
            count as f64 * 100.0 / total.max(1) as f64
        );
    }
    println!("  total unsupported:  {total}");

    // Exact encodings, so the next decoder addition can be chosen from data and
    // cross-checked against an assembler rather than guessed from a bit pattern.
    let mut distinct: Vec<(u32, usize)> = words.into_iter().collect();
    distinct.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("most common unsupported encodings:");
    for (word, n) in distinct.into_iter().take(25) {
        println!("  0x{word:08x} x{n}");
    }
}

fn value(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    it.next().with_context(|| format!("{flag} needs a value"))
}
