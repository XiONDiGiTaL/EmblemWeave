//! `ew-native`: lift a whole decoded module, emit it as C, compile it in
//! parallel and link one native library (`ew_native.dll`) that `ew-run
//! --native` loads in place of the reference interpreter.
//!
//! ```text
//! ew-native --module work/decoded/main --out build/native [--opt 1] [--limit N] [--cc clang]
//! ```
//!
//! The library exports `ew_native_shards()` and `ew_native_shard(i, &count)`,
//! which hand the runtime each shard's block table: every block start of every
//! lifted function, with the function to call there.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Args {
    module: PathBuf,
    out: PathBuf,
    opt: String,
    limit: usize,
    cc: PathBuf,
}

fn parse() -> Result<Args, String> {
    let mut a = Args {
        module: PathBuf::from("work/decoded/main"),
        out: PathBuf::from("build/native"),
        opt: "1".into(),
        limit: usize::MAX,
        cc: default_cc(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(k) = it.next() {
        let mut v = || it.next().ok_or(format!("{k} needs a value"));
        match k.as_str() {
            "--module" => a.module = PathBuf::from(v()?),
            "--out" => a.out = PathBuf::from(v()?),
            "--opt" => a.opt = v()?,
            "--limit" => a.limit = v()?.parse().map_err(|e| format!("--limit: {e}"))?,
            "--cc" => a.cc = PathBuf::from(v()?),
            "-h" | "--help" => {
                println!("ew-native --module <decoded main> --out <dir> [--opt 0|1|2] [--limit functions] [--cc clang]");
                std::process::exit(0);
            }
            _ => return Err(format!("unknown argument {k}")),
        }
    }
    Ok(a)
}

fn default_cc() -> PathBuf {
    let llvm = PathBuf::from(r"C:\Program Files\LLVM\bin\clang.exe");
    if llvm.is_file() { llvm } else { PathBuf::from("clang") }
}

/// Contiguous groups of functions of roughly equal instruction count.
fn shards(funcs: &[ew_lift::LiftedFunction], n: usize) -> Vec<&[ew_lift::LiftedFunction]> {
    let total: usize = funcs.iter().map(|f| f.insn_count()).sum();
    let target = (total / n.max(1)).max(1);
    let mut out = Vec::new();
    let (mut start, mut acc) = (0, 0);
    for (i, f) in funcs.iter().enumerate() {
        acc += f.insn_count();
        if acc >= target {
            out.push(&funcs[start..=i]);
            start = i + 1;
            acc = 0;
        }
    }
    if start < funcs.len() {
        out.push(&funcs[start..]);
    }
    out
}

/// Compiler processes to run at once: half the cores, and no more than one
/// per 3 GiB of physical memory, so the machine stays usable.
fn jobs() -> usize {
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let mem_gib = physical_memory_gib().unwrap_or(16);
    (cores / 2).min((mem_gib / 3) as usize).max(1)
}

#[cfg(windows)]
fn physical_memory_gib() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    (unsafe { GlobalMemoryStatusEx(&mut m) } != 0).then_some(m.ullTotalPhys >> 30)
}

#[cfg(not(windows))]
fn physical_memory_gib() -> Option<u64> {
    None
}

/// Give every block to one function: the one with the highest entry at or
/// below it (else the lowest). The lifter follows control flow from every
/// entry, so the same code turns up in many functions; a branch into code
/// another function owns now leaves the native function, and the run loop
/// enters the owner at that block.
fn own_blocks(funcs: &mut Vec<ew_lift::LiftedFunction>) {
    funcs.sort_by_key(|f| f.addr);
    funcs.dedup_by_key(|f| f.addr);
    let mut owner: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
    for f in funcs.iter() {
        for b in &f.blocks {
            let e = owner.entry(b.addr).or_insert(f.addr);
            let cur = *e;
            let better = match (f.addr <= b.addr, cur <= b.addr) {
                (true, true) => f.addr > cur,
                (true, false) => true,
                (false, true) => false,
                (false, false) => f.addr < cur,
            };
            if better {
                *e = f.addr;
            }
        }
    }
    for f in funcs.iter_mut() {
        let a = f.addr;
        f.blocks.retain(|b| owner.get(&b.addr) == Some(&a));
    }
    funcs.retain(|f| !f.blocks.is_empty());
}

fn compile_all(cc: &Path, opt: &str, out: &Path, sources: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let threads = jobs();
    eprintln!("compiling {} files, {threads} at a time", sources.len());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed = std::sync::Mutex::new(Vec::<String>::new());
    let done = std::sync::atomic::AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(src) = sources.get(i) else { return };
                let obj = src.with_extension("o");
                let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
                if matches!((mtime(&obj), mtime(src)), (Some(o), Some(c)) if o >= c) {
                    done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    continue;
                }
                let mut cmd = Command::new(cc);
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    // Below normal priority, so the desktop stays responsive.
                    cmd.creation_flags(0x0000_4000);
                }
                let r = cmd
                    .args(["-std=c11", &format!("-O{opt}"), "-c", "-w", "-fno-strict-aliasing", "-I"])
                    .arg(out)
                    .arg(src)
                    .arg("-o")
                    .arg(&obj)
                    .output();
                match r {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => failed.lock().unwrap().push(format!("{}: {}", src.display(), String::from_utf8_lossy(&o.stderr).lines().take(4).collect::<Vec<_>>().join(" | "))),
                    Err(e) => failed.lock().unwrap().push(format!("{}: {e}", src.display())),
                }
                let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if n % 16 == 0 || n == sources.len() {
                    eprintln!("compiled {n} of {}", sources.len());
                }
            });
        }
    });
    let failed = failed.into_inner().unwrap();
    if !failed.is_empty() {
        return Err(format!("{} shards failed:\n{}", failed.len(), failed.join("\n")));
    }
    Ok(sources.iter().map(|s| s.with_extension("o")).collect())
}

/// Write `text` to `path` unless it already holds exactly that. Returns
/// whether it wrote.
fn write_if_changed(path: &Path, text: &str) -> Result<bool, String> {
    if std::fs::read(path).ok().as_deref() == Some(text.as_bytes()) {
        return Ok(false);
    }
    std::fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(true)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("ew-native: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let a = parse()?;
    let t0 = std::time::Instant::now();
    let module = ew_boot::Module::from_dir(&a.module).map_err(|e| format!("{e:#}"))?;
    let segments: Vec<(u64, &[u8])> = vec![(module.rodata_addr, &module.rodata), (module.data_addr, &module.data)];
    let opts = ew_lift::LiftOptions { module: "app".into(), max_functions: a.limit, max_insns: 4096, own_blocks: true, ..Default::default() };
    let (report, mut funcs) = ew_lift::lift_text_in(&module.text, &segments, &opts);
    let lifted: usize = funcs.iter().map(|f| f.insn_count()).sum();
    own_blocks(&mut funcs);
    let insns: usize = funcs.iter().map(|f| f.insn_count()).sum();
    eprintln!(
        "lifted {lifted} instructions; kept {} functions ({} complete), {} blocks, {} instructions in {:.1?}",
        funcs.len(),
        report.complete_functions,
        funcs.iter().map(|f| f.blocks.len()).sum::<usize>(),
        insns,
        t0.elapsed()
    );

    std::fs::create_dir_all(&a.out).map_err(|e| e.to_string())?;
    // Incremental: a file is only rewritten when its text changes, and only
    // compiled when it is newer than its object. A changed header rebuilds all.
    let header_changed = write_if_changed(&a.out.join("ew_aarch64.h"), ew_lift::RUNTIME_HEADER)?;
    write_if_changed(&a.out.join("ew_dispatch.h"), ew_lift::dispatch_header())?;
    if header_changed {
        for old in std::fs::read_dir(&a.out).map_err(|e| e.to_string())?.flatten() {
            if old.file_name().to_string_lossy().ends_with(".o") {
                let _ = std::fs::remove_file(old.path());
            }
        }
    }
    // Shards of about 20,000 instructions: a few MiB of C each, which clang
    // compiles in bounded memory.
    let parts = shards(&funcs, (insns / 20_000).max(1));
    let mut sources = Vec::new();
    let mut table = String::from("#include \"ew_dispatch.h\"\n\n");
    for (i, chunk) in parts.iter().enumerate() {
        let name = format!("app_{i:04}");
        let path = a.out.join(format!("{name}.c"));
        write_if_changed(&path, &ew_lift::emit_module(&name, chunk))?;
        sources.push(path);
        table.push_str(&format!("extern const ew_func_entry {name}_blocks[];\nextern const unsigned {name}_block_count;\n"));
    }
    table.push_str(&format!("\n__declspec(dllexport) unsigned ew_native_shards(void) {{ return {}u; }}\n\n", parts.len()));
    table.push_str("__declspec(dllexport) const ew_func_entry *ew_native_shard(unsigned i, unsigned *count)\n{\n    switch (i) {\n");
    for i in 0..parts.len() {
        table.push_str(&format!("    case {i}u: *count = app_{i:04}_block_count; return app_{i:04}_blocks;\n"));
    }
    table.push_str("    default: *count = 0; return 0;\n    }\n}\n");
    let table_path = a.out.join("app_table.c");
    write_if_changed(&table_path, &table)?;
    sources.push(table_path);
    // Shards left over from a larger build.
    for old in std::fs::read_dir(&a.out).map_err(|e| e.to_string())?.flatten() {
        let p = old.path();
        let keep = sources.iter().any(|s| *s == p || s.with_extension("o") == p);
        let n = old.file_name().to_string_lossy().into_owned();
        if n.starts_with("app_") && !keep && (n.ends_with(".c") || n.ends_with(".o")) {
            let _ = std::fs::remove_file(p);
        }
    }
    eprintln!("emitted {} C files in {:.1?}", sources.len(), t0.elapsed());

    let objects = compile_all(&a.cc, &a.opt, &a.out, &sources)?;
    eprintln!("compiled in {:.1?}", t0.elapsed());
    let dll = a.out.join("ew_native.dll");
    let o = Command::new(&a.cc)
        .args(["-shared", "-fuse-ld=lld", "-o"])
        .arg(&dll)
        .args(&objects)
        .output()
        .map_err(|e| format!("linking: {e}"))?;
    if !o.status.success() {
        return Err(format!("linking failed: {}", String::from_utf8_lossy(&o.stderr).lines().take(10).collect::<Vec<_>>().join("\n")));
    }
    eprintln!("linked {} in {:.1?}", dll.display(), t0.elapsed());
    Ok(())
}
