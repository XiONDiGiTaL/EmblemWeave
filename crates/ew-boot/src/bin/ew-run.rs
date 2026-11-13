//! `ew-run` — run the recovered module and report where it stops.
//!
//! This is the closest thing the project has to starting the game. It maps the
//! module image, enters at the module's text start, dispatches recovered
//! functions until control leaves the translated code, and prints a typed reason
//! for every stop with the address that caused it.
//!
//! It is a diagnostic runner, not the product runtime: it executes the reference
//! interpreter so the report cannot disagree with the lifter about what a block
//! does.

use anyhow::{Context, Result};
use ew_boot::{boot, BootOptions, Module, Stop};
use std::path::PathBuf;

struct Args {
    module_dir: Option<PathBuf>,
    text: Option<PathBuf>,
    rodata: Option<PathBuf>,
    data: Option<PathBuf>,
    entry: Option<u64>,
    romfs: Option<PathBuf>,
    libs: Vec<PathBuf>,
    save: Option<PathBuf>,
    nacp: Option<PathBuf>,
    language: Option<String>,
    snapshot_at: Option<u64>,
    snapshot_dir: Option<PathBuf>,
    snapshot_steps: usize,
    max_steps: u64,
    max_seconds: Option<u64>,
    watch: Vec<u64>,
    trace: usize,
    x0: u64,
    json: Option<PathBuf>,
    verbose: bool,
    all_calls: bool,
    presses: Vec<ew_boot::hid::Press>,
    dump: Option<(u64, usize)>,
    /// Print the module's named imports whose mangled name contains this text.
    imports: Option<String>,
    /// `--native <ew_native.dll>`: run the game's code from the library built
    /// by `ew-native` wherever it has it.
    native: Option<PathBuf>,
    /// `--window [title]`: show the game's frames in a window.
    window: Option<String>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("ew-run: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut args = Args {
        module_dir: None,
        text: None,
        rodata: None,
        data: None,
        entry: None,
        romfs: None,
        libs: Vec::new(),
        save: None,
        nacp: None,
        language: None,
        snapshot_at: None,
        snapshot_dir: None,
        snapshot_steps: 20_000,
        max_steps: 20_000_000,
        max_seconds: None,
        watch: Vec::new(),
        trace: 64,
        x0: 0,
        native: None,
        window: None,
        json: None,
        verbose: false,
        all_calls: false,
        presses: Vec::new(),
        dump: None,
        imports: None,
    };

    let mut it = std::env::args().skip(1).peekable();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--module" => args.module_dir = Some(PathBuf::from(value(&mut it, "--module")?)),
            "--text" => args.text = Some(PathBuf::from(value(&mut it, "--text")?)),
            "--rodata" => args.rodata = Some(PathBuf::from(value(&mut it, "--rodata")?)),
            "--data" => args.data = Some(PathBuf::from(value(&mut it, "--data")?)),
            "--entry" => args.entry = Some(hex(&value(&mut it, "--entry")?)?),
            "--max-steps" => args.max_steps = value(&mut it, "--max-steps")?.parse()?,
            "--max-seconds" => args.max_seconds = Some(value(&mut it, "--max-seconds")?.parse()?),
            "--watch" => args.watch.push(u64::from_str_radix(value(&mut it, "--watch")?.trim_start_matches("0x"), 16)?),
            "--trace" => args.trace = value(&mut it, "--trace")?.parse()?,
            "--x0" => args.x0 = hex(&value(&mut it, "--x0")?)?,
            "--json" => args.json = Some(PathBuf::from(value(&mut it, "--json")?)),
            "--imports" => args.imports = Some(value(&mut it, "--imports")?),
            "--verbose" => args.verbose = true,
            "--all-calls" => args.all_calls = true,
            "--press" => {
                let v = value(&mut it, "--press")?;
                args.presses.push(ew_boot::hid::parse_press(&v).map_err(anyhow::Error::msg)?);
            }
            "--romfs" => args.romfs = Some(PathBuf::from(value(&mut it, "--romfs")?)),
            "--native" => args.native = Some(PathBuf::from(value(&mut it, "--native")?)),
            "--window" => {
                // An optional title follows; anything else is the next flag.
                let title = match it.peek() {
                    Some(next) if !next.starts_with("--") => it.next().unwrap_or_default(),
                    _ => "EmblemWeave".to_string(),
                };
                args.window = Some(title);
            }
            "--lib" => args.libs.push(PathBuf::from(value(&mut it, "--lib")?)),
            "--save" => args.save = Some(PathBuf::from(value(&mut it, "--save")?)),
            "--nacp" => args.nacp = Some(PathBuf::from(value(&mut it, "--nacp")?)),
            "--language" => args.language = Some(value(&mut it, "--language")?),
            "--snapshot-at" => args.snapshot_at = Some(hex(&value(&mut it, "--snapshot-at")?)?),
            "--snapshot-dir" => args.snapshot_dir = Some(PathBuf::from(value(&mut it, "--snapshot-dir")?)),
            "--snapshot-steps" => args.snapshot_steps = value(&mut it, "--snapshot-steps")?.parse()?,
            "--dump" => {
                let v = value(&mut it, "--dump")?;
                let (addr, count) = v
                    .split_once(':')
                    .context("--dump expects ADDR:COUNT")?;
                args.dump = Some((hex(addr)?, count.parse().context("--dump count must be a number")?));
            }
            "-h" | "--help" => {
                println!(
                    "ew-run — execute a decoded module and report where it stops\n\
                     \n\
                     USAGE:\n  ew-run --module <decoded-module-dir> [options]\n\
                     \x20 ew-run --text text.bin [--rodata rodata.bin] [--data data.bin] [options]\n\
                     \n\
                     OPTIONS:\n\
                     \x20 --module DIR     directory holding text.bin, rodata.bin, data.bin\n\
                     \x20 --entry ADDR     start address in hex (default 0, the text start)\n\
                     \x20 --max-steps N    instruction budget for the run (default 20000000)\n\
                     \x20 --trace N        keep N transfer lines for the report (default 64)\n\
                     \x20 --x0 VALUE       initial x0 in hex (default 0)\n\
                     \x20 --json PATH      also write the report as JSON\n\
                     \x20 --imports TEXT   list named imports containing TEXT and exit\n\
                     \x20 --verbose        print the transfer trace\n\
                     \x20 --all-calls      list every import reached, not the first 12\n\
                     \x20 --press B@F[+N]  hold button B (A, B, PLUS, UP, A+B, ...) from presented\n\
                     \x20                  frame F for N frames (default 10); repeatable\n\
                     \x20 --max-seconds N  stop after N seconds of wall-clock time\n\
                     \x20 --watch ADDR     log entry and return of the function at hex ADDR\n\
                     \x20                  (EW_PEEK=addr,... shows memory at each hit)\n\
                     \x20 --romfs DIR      your extracted romfs, mounted read-only for nn::fs\n\
                     \x20 --save DIR       writable save directory (default build/save)\n\
                     \x20 --nacp FILE      the title's control.nacp, for its display version\n\
                     \x20 --language CODE  system language reported to the game (default en-US)\n\
                     \x20 --dump A:N       decode N instructions at hex address A and exit\n\
                     \x20 --window [TITLE] show the game's frames in a window (opens on the\n\
                     \x20                  first present)\n"
                );
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument `{other}` (try --help)"),
        }
    }

    let module = match (&args.module_dir, &args.text) {
        (Some(dir), None) => Module::from_dir(dir)?,
        (None, Some(text)) => {
            let read = |p: &Option<PathBuf>| -> Result<Vec<u8>> {
                match p {
                    Some(path) => std::fs::read(path)
                        .with_context(|| format!("reading {}", path.display())),
                    None => Ok(Vec::new()),
                }
            };
            Module::new(
                "module",
                std::fs::read(text).with_context(|| format!("reading {}", text.display()))?,
                read(&args.rodata)?,
                read(&args.data)?,
            )
        }
        (Some(_), Some(_)) => anyhow::bail!("use either --module or --text, not both"),
        (None, None) => anyhow::bail!("--module or --text is required (try --help)"),
    };

    println!("module:            {}", module.name);
    println!(
        "image:             text 0x0 ({} bytes), rodata 0x{:x} ({} bytes), data 0x{:x} ({} bytes)",
        module.text.len(),
        module.rodata_addr,
        module.rodata.len(),
        module.data_addr,
        module.data.len()
    );

    if let Some(filter) = &args.imports {
        for export in &module.imports.exports {
            if export.name.contains(filter.as_str()) {
                println!("export 0x{:08x}  {:6}  {}", export.addr, export.size, export.name);
            }
        }
        let mut shown = 0usize;
        for import in &module.imports.jumps {
            if import.name.contains(filter.as_str()) {
                println!("0x{:08x}  {}", import.slot, import.name);
                shown += 1;
            }
        }
        println!("{shown} of {} named imports contain `{filter}`", module.imports.jumps.len());
        return Ok(());
    }

    if let Some((at, count)) = args.dump {
        for i in 0..count {
            let addr = at + (i as u64) * 4;
            let Some(word) = ew_lift::scan::word_at(&module.text, addr) else {
                println!("0x{addr:x}: outside the text segment");
                break;
            };
            let op = ew_lift::decode(word, addr);
            println!("0x{addr:08x}: {word:08x}  {op:?}");
        }
        return Ok(());
    }

    // Without an explicit entry, follow nnSdk's startup order through every
    // entry point the module exports.
    let mut sequence = match args.entry {
        Some(e) => vec![("entry".to_string(), e)],
        None => module.startup_sequence(),
    };
    let (entry_name, entry) = sequence.remove(0);
    println!("entry:             0x{entry:x} ({entry_name})");
    for (name, addr) in &sequence {
        println!("  then:            0x{addr:x} ({name})");
    }
    println!("imports:           {} named jump slots", module.imports.jumps.len());
    println!("relocations:       {} relative, {} relr", module.imports.relatives.len(), module.imports.relr.len());

    let mut settings = std::collections::BTreeMap::new();
    if let Some(path) = &args.nacp {
        match ew_boot::read_nacp(path) {
            Some((title, version)) => {
                println!("title:             {title} {version} (control.nacp)");
                settings.insert("display_version".to_string(), version);
            }
            None => println!("title:             {} is not a control.nacp", path.display()),
        }
    }
    if let Some(lang) = &args.language {
        settings.insert("language".to_string(), lang.clone());
    }
    let opts = BootOptions {
        window: args.window.clone(),
        settings,
        snapshot_at: args.snapshot_at,
        snapshot_dir: args.snapshot_at.map(|_| args.snapshot_dir.clone().unwrap_or_else(|| PathBuf::from("build/snapshot"))),
        snapshot_steps: args.snapshot_steps,
        romfs: args.romfs.clone(),
        save_dir: Some(args.save.clone().unwrap_or_else(|| PathBuf::from("build/save"))),
        entry,
        max_steps: args.max_steps,
        watch: args.watch.clone(),
        // A wall-clock limit: once the game paces itself by the host clock,
        // an instruction budget alone can take arbitrarily long to reach.
        cancel: args.max_seconds.map(|secs| {
            let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let f = flag.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(secs));
                f.store(true, std::sync::atomic::Ordering::Relaxed);
            });
            flag
        }),
        trace: args.trace,
        x0: args.x0,
        then: sequence,
        entry_name,
        libraries: args
            .libs
            .iter()
            .map(|d| ew_boot::Module::from_dir(d))
            .collect::<Result<Vec<_>>>()?,
        native: match &args.native {
            Some(p) => {
                let n = ew_boot::native::Native::load(p).map_err(|e| anyhow::anyhow!(e))?;
                println!("native:            {} ({} functions, {} blocks)", p.display(), n.functions, n.blocks());
                Some(std::sync::Arc::new(n))
            }
            None => None,
        },
        ..Default::default()
    };
    for lib in &opts.libraries {
        println!(
            "library:           {} ({} exports, {} imports, init {:?}, init_array {:?})",
            lib.name,
            lib.imports.exports.len(),
            lib.imports.jumps.len(),
            lib.imports.init,
            lib.imports.init_array
        );
    }
    ew_boot::hid::set_script(args.presses.clone());
    let report = boot(&module, &opts)?;

    println!();
    println!("steps:             {}", report.steps);
    println!("function entries:  {}", report.dispatches);
    if args.native.is_some() {
        println!("native entries:    {} ({} handed to the interpreter)", report.native_dispatches, report.native_halts);
    }
    println!("distinct funcs:    {}", report.distinct_functions);
    println!(
        "calls / returns:   {} / {} (max depth {})",
        report.calls, report.returns, report.max_depth
    );
    println!("branches:          {}", report.branches);
    println!(
        "host calls:        {} ({} imports implemented)",
        report.host_calls, report.host_implemented
    );
    println!("guest memory:      {} bytes mapped", report.memory_bytes);
    println!(
        "stack:             sp 0x{:x}, low water 0x{:x}",
        report.final_sp, report.stack_low
    );
    println!();
    for line in &report.thread_states {
        println!("{line}");
    }
    for line in &report.host_log {
        println!("host: {line}");
    }
    if !report.resolved.is_empty() {
        println!("resolved at run time: {}", report.resolved.len());
        for name in &report.resolved {
            println!("  {name}");
        }
    }
    if let Ok(path) = std::env::var("EW_PROC_SLOTS") {
        let text: String = report.proc_slots.iter().map(|(a, n)| format!("{a:x} {n}
")).collect();
        let _ = std::fs::write(path, text);
    }
    if !report.recent.is_empty() {
        println!("last events before the stop:");
        for line in report.recent.iter().rev().take(40).collect::<Vec<_>>().into_iter().rev() {
            println!("  {line}");
        }
    }
    println!("phases completed:  {}", if report.phases.is_empty() { "none".to_string() } else { report.phases.join(", ") });
    println!("relocated slots:   {} changed by the loader", report.relocations_changed);
    println!("stopped:           {} (during {})", report.stop.describe(), report.phase);
    if let Stop::Unsupported { pc, word } = report.stop {
        println!("  blocked at       0x{pc:x} (word 0x{word:08x})");
    }

    if !report.import_calls.is_empty() {
        println!();
        println!("imports reached:");
        for i in report.import_calls.iter().take(if args.all_calls { usize::MAX } else { 12 }) {
            println!("  {:<64} {:>6} call(s) from 0x{:x}", i.name, i.calls, i.first_from);
        }
    }

    if !report.hot.is_empty() {
        println!();
        println!("hottest functions:");
        for h in &report.hot {
            println!(
                "  0x{:<10x} {:>8} entries  {:>10} instructions",
                h.addr, h.entries, h.instructions
            );
        }
    }

    if !report.externals.is_empty() {
        println!();
        println!("transfers outside the text segment:");
        for e in report.externals.iter().take(16) {
            println!("  {} 0x{:x} -> 0x{:x}", e.kind, e.from, e.to);
        }
    }

    if args.verbose && !report.trace.is_empty() {
        println!();
        println!("trace:");
        for line in &report.trace {
            println!("  {line}");
        }
    }

    if let Some(path) = &args.json {
        std::fs::write(path, serde_json::to_string_pretty(&report)?)
            .with_context(|| format!("writing {}", path.display()))?;
        println!();
        println!("report:            {}", path.display());
    }

    println!();
    println!("summary: {}", report.headline());
    Ok(())
}

fn value(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String> {
    it.next().with_context(|| format!("{flag} needs a value"))
}

fn hex(v: &str) -> Result<u64> {
    u64::from_str_radix(v.trim_start_matches("0x"), 16).context("expected a hex address")
}
