//! Background work: data detection, module decoding, the recompilation
//! pipeline and boot runs.
//!
//! Each job runs on its own thread and reports through a shared [`Progress`]:
//! named stages with their status and timing, a current message, a log, and a
//! result. The interface reads that state every frame and never waits on a
//! job. Cancellation is a flag the job checks between units of work (and, for
//! a boot, inside the dispatch loop), so it stops at a safe point.

use crate::config::Config;
use crate::gamedata::{self, GameData};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageStatus {
    Pending,
    Active,
    Done,
    Skipped,
    Failed,
}

#[derive(Debug, Clone)]
pub struct Stage {
    pub name: &'static str,
    pub status: StageStatus,
    pub detail: String,
    pub started: Option<Instant>,
    pub elapsed: Option<Duration>,
}

/// What a finished pipeline measured, for the overview and diagnostics.
#[derive(Debug, Clone, Default)]
pub struct PipelineSummary {
    pub report: Option<ew_lift::LiftReport>,
    pub output: Option<PathBuf>,
    pub compiled: Option<bool>,
    pub verified: Option<(usize, usize)>,
}

#[derive(Debug, Clone)]
pub enum JobResult {
    Detected(Box<GameData>),
    Decoded(PathBuf),
    Pipeline(Box<PipelineSummary>),
    Boot(Box<ew_boot::BootReport>, u128),
    /// The engine built the game; the run folder holding fefw.ini.
    Engine(PathBuf),
}

#[derive(Debug, Default)]
pub struct Progress {
    pub stages: Vec<Stage>,
    pub message: String,
    pub log: Vec<String>,
    pub finished: bool,
    pub failed: Option<String>,
    pub result: Option<JobResult>,
    /// The file this job's log is written to, when one could be opened.
    pub log_file: Option<PathBuf>,
    /// Bytes copied and bytes to copy, while a package is being unpacked.
    pub copied: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Detect,
    Decode,
    Pipeline,
    Boot,
    Engine,
}

impl JobKind {
    pub fn title(self) -> &'static str {
        match self {
            JobKind::Detect => "Checking game data",
            JobKind::Decode => "Decoding the main module",
            JobKind::Pipeline => "Recompilation pipeline",
            JobKind::Boot => "Running the game",
            JobKind::Engine => "Building the game",
        }
    }

    fn slug(self) -> &'static str {
        match self {
            JobKind::Detect => "detect",
            JobKind::Decode => "decode",
            JobKind::Pipeline => "pipeline",
            JobKind::Boot => "boot",
            JobKind::Engine => "engine",
        }
    }
}

/// `%APPDATA%\EmblemWeave\logs`, where every task writes what it did.
pub fn log_dir() -> Option<PathBuf> {
    Config::dir().map(|d| d.join("logs"))
}

/// A UTC stamp for the file name: `2026-11-14 18.42.07`.
fn stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86400, secs % 86400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // Civil date from days since the epoch.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 };
    format!("{y:04}-{mo:02}-{d:02} {h:02}.{m:02}.{s:02}")
}

/// Open a fresh log file for one job, with what a bug report needs first.
fn open_log(dir: &Path, kind: JobKind) -> std::io::Result<(PathBuf, std::fs::File)> {
    std::fs::create_dir_all(dir)?;
    let mut path = dir.join(format!("{}-{}.log", kind.slug(), stamp()));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{}-{} ({}).log", kind.slug(), stamp(), n));
        n += 1;
    }
    let mut f = std::fs::File::create(&path)?;
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    let cwd = std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    writeln!(f, "EmblemWeave {} ({})", env!("CARGO_PKG_VERSION"), env!("EW_GIT_REV"))?;
    writeln!(f, "{}: {} UTC", kind.title(), stamp())?;
    writeln!(f, "os: {} {}", std::env::consts::OS, std::env::consts::ARCH)?;
    writeln!(f, "exe: {exe}")?;
    writeln!(f, "cwd: {cwd}")?;
    writeln!(f, "args: {}", args.join(" "))?;
    writeln!(f)?;
    Ok((path, f))
}

pub struct Job {
    pub kind: JobKind,
    pub started: Instant,
    pub progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
    /// Guest instructions executed, for a boot job.
    pub steps: Arc<AtomicU64>,
    pub handle: Option<std::thread::JoinHandle<()>>,
}

impl Job {
    pub fn snapshot<R>(&self, f: impl FnOnce(&Progress) -> R) -> R {
        let p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        f(&p)
    }

    pub fn finished(&self) -> bool {
        self.snapshot(|p| p.finished)
    }

    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// The job's side of the shared state.
#[derive(Clone)]
pub struct Ctx {
    progress: Arc<Mutex<Progress>>,
    pub cancel: Arc<AtomicBool>,
    pub steps: Arc<AtomicU64>,
    /// The job's log file, when one could be created.
    file: Arc<Mutex<Option<std::fs::File>>>,
}

impl Ctx {
    fn with<R>(&self, f: impl FnOnce(&mut Progress) -> R) -> R {
        let mut p = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut p)
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    /// Append one line to the job's log file. The file is the record that
    /// survives the window, so it is written before the interface copies.
    fn write(&self, line: &str) {
        if let Ok(mut f) = self.file.lock() {
            if let Some(file) = f.as_mut() {
                let _ = writeln!(file, "{line}");
                let _ = file.flush();
            }
        }
    }
    pub fn log(&self, line: impl Into<String>) {
        let line = line.into();
        self.write(&line);
        self.with(|p| {
            p.message = line.clone();
            p.log.push(line);
        });
    }
    /// Report a byte count without a log line: unpacking copies a quarter of
    /// a million files, and the log records stages, not every block.
    pub fn bytes(&self, done: u64, total: u64) {
        self.with(|p| {
            p.copied = done;
            p.total = total;
            p.message =
                format!("Unpacking the package: {} of {} copied", crate::fmt_size(done), crate::fmt_size(total));
        });
    }
    pub fn start(&self, i: usize) {
        self.with(|p| {
            if let Some(s) = p.stages.get_mut(i) {
                s.status = StageStatus::Active;
                s.started = Some(Instant::now());
                p.message = format!("{}...", s.name);
            }
        });
    }
    pub fn end(&self, i: usize, status: StageStatus, detail: impl Into<String>) {
        let detail = detail.into();
        self.write(&format!("({status:?}) {detail}"));
        self.with(|p| {
            if let Some(s) = p.stages.get_mut(i) {
                s.status = status;
                s.elapsed = s.started.map(|t| t.elapsed());
                s.detail = detail.clone();
            }
            p.log.push(format!("{:?}: {}", status, detail));
        });
    }
    fn finish(&self, result: Result<JobResult, String>) {
        if let Err(e) = &result {
            self.write(&format!("failed: {e}"));
        }
        self.with(|p| {
            for s in p.stages.iter_mut().filter(|s| s.status == StageStatus::Pending) {
                s.status = StageStatus::Skipped;
            }
            match result {
                Ok(r) => p.result = Some(r),
                Err(e) => {
                    p.log.push(format!("failed: {e}"));
                    p.failed = Some(e);
                }
            }
            p.finished = true;
        });
    }
}

pub(crate) fn spawn(kind: JobKind, stages: &[&'static str], work: impl FnOnce(&Ctx) -> Result<JobResult, String> + Send + 'static) -> Job {
    let progress = Arc::new(Mutex::new(Progress {
        stages: stages
            .iter()
            .map(|&name| Stage { name, status: StageStatus::Pending, detail: String::new(), started: None, elapsed: None })
            .collect(),
        ..Default::default()
    }));
    let (log_path, log) = match log_dir().map(|d| open_log(&d, kind)) {
        Some(Ok((path, file))) => (Some(path), Some(file)),
        _ => (None, None),
    };
    progress.lock().unwrap_or_else(|e| e.into_inner()).log_file = log_path;
    let ctx = Ctx {
        progress: progress.clone(),
        cancel: Arc::new(AtomicBool::new(false)),
        steps: Arc::new(AtomicU64::new(0)),
        file: Arc::new(Mutex::new(log)),
    };
    let job = Job {
        kind,
        started: Instant::now(),
        progress,
        cancel: ctx.cancel.clone(),
        steps: ctx.steps.clone(),
        handle: None,
    };
    let thread_ctx = ctx.clone();
    let handle = std::thread::Builder::new()
        .name(format!("ew-{kind:?}"))
        // Recovery and the dispatcher recurse; give them room.
        .stack_size(64 << 20)
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&thread_ctx)));
            let r = r.unwrap_or_else(|panic| {
                let what = panic
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "unknown panic".into());
                Err(format!("internal error: {what}"))
            });
            thread_ctx.finish(r);
        })
        .ok();
    Job { handle, ..job }
}

// ---- detection ---------------------------------------------------------------

pub fn detect(config: &Config) -> Job {
    let paths = config.paths.clone();
    spawn(JobKind::Detect, &["Locate and validate game data"], move |ctx| {
        ctx.start(0);
        let c = ctx.clone();
        let g = gamedata::detect_with(&paths, &mut |done, total| c.bytes(done, total));
        ctx.end(0, StageStatus::Done, format!("{} checks", g.checks.len()));
        Ok(JobResult::Detected(Box::new(g)))
    })
}

// ---- decoding ------------------------------------------------------------------

/// The ZBIC decoder is a development bridge (`tools/zbic_extract.py`, run out
/// of process). Look for it next to the executable and in the working tree.
fn find_bridge() -> Option<(PathBuf, PathBuf)> {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    let mut roots: Vec<PathBuf> = vec![std::env::current_dir().ok()?];
    if let Some(d) = exe_dir {
        roots.extend(d.ancestors().take(5).map(Path::to_path_buf));
    }
    for root in roots {
        let script = root.join("tools").join("zbic_extract.py");
        if script.is_file() {
            let venv = root.join(".venv").join("Scripts").join("python.exe");
            let python = if venv.is_file() { venv } else { PathBuf::from("python") };
            return Some((python, script));
        }
    }
    None
}

pub fn decode(exefs: PathBuf) -> Job {
    spawn(JobKind::Decode, &["Find the ZBIC bridge", "Decode text, rodata and data", "Check sizes"], move |ctx| {
        ctx.start(0);
        let (python, script) = find_bridge().ok_or_else(|| {
            "tools/zbic_extract.py was not found next to the launcher or in the working folder".to_string()
        })?;
        ctx.end(0, StageStatus::Done, format!("{} with {}", script.display(), python.display()));
        let out = Config::work_dir().ok_or("APPDATA is not set")?.join("decoded");
        std::fs::create_dir_all(&out).map_err(|e| format!("creating {}: {e}", out.display()))?;
        ctx.start(1);
        let output = std::process::Command::new(&python)
            .arg("-I")
            .arg(&script)
            .arg(&exefs)
            .arg(&out)
            .arg(gamedata::MAIN_MODULE)
            .output()
            .map_err(|e| format!("starting {}: {e}", python.display()))?;
        let text = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
        for line in text.lines() {
            ctx.log(line.to_string());
        }
        if !output.status.success() {
            ctx.end(1, StageStatus::Failed, "the bridge reported an error (see the log)");
            return Err("decoding failed; if the log mentions `zbic`, install it with `pip install zbic`".into());
        }
        ctx.end(1, StageStatus::Done, "segments written");
        ctx.start(2);
        let mismatch = text.lines().any(|l| l.trim_start().starts_with("text") && !l.ends_with("OK"));
        if mismatch {
            ctx.end(2, StageStatus::Failed, "a decoded size differs from the module header");
            return Err("decoded sizes do not match the module header".into());
        }
        ctx.end(2, StageStatus::Done, "every segment matches its declared size");
        Ok(JobResult::Decoded(out.join(gamedata::MAIN_MODULE)))
    })
}

// ---- recompilation pipeline -----------------------------------------------------

pub const PIPELINE_STAGES: [&str; 9] = [
    "Parse executable",
    "Segment decompression",
    "Instruction decoding",
    "Function recovery",
    "Control-flow analysis",
    "C code generation",
    "Native compilation",
    "Verification",
    "Runtime preparation",
];

/// A C compiler for the emitted code: zig's bundled clang through the Python
/// wheel, or clang on the PATH.
fn find_cc() -> Option<Vec<String>> {
    if let Some((python, _)) = find_bridge() {
        let ok = std::process::Command::new(&python)
            .args(["-m", "ziglang", "version"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            return Some(vec![python.to_string_lossy().into(), "-m".into(), "ziglang".into(), "cc".into()]);
        }
    }
    let clang = std::process::Command::new("clang").arg("--version").output().map(|o| o.status.success()).unwrap_or(false);
    clang.then(|| vec!["clang".to_string()])
}

pub fn pipeline(game: GameData, limit: usize) -> Job {
    spawn(JobKind::Pipeline, &PIPELINE_STAGES, move |ctx| {
        let mut summary = PipelineSummary::default();
        let check = |ctx: &Ctx| if ctx.cancelled() { Err("cancelled by the user".to_string()) } else { Ok(()) };

        ctx.start(0);
        let main = game.modules.as_ref().and_then(|e| e.find(gamedata::MAIN_MODULE)).cloned();
        let Some(main) = main else {
            ctx.end(0, StageStatus::Failed, "no ExeFS with a `main` module");
            return Err("set the game folder on Game Setup first".into());
        };
        ctx.end(0, StageStatus::Done, format!("`{}`: text {} bytes, rodata {}, data {}", main.module_name, main.text_size, main.rodata_size, main.data_size));
        check(ctx)?;

        ctx.start(1);
        let Some(dir) = game.decoded.clone().filter(|_| game.segments.iter().all(|s| s.ok())) else {
            ctx.end(1, StageStatus::Failed, "decode the module on Game Setup first");
            return Err("the main module is not decoded".into());
        };
        let module = ew_boot::Module::from_dir(&dir).map_err(|e| format!("{e:#}"))?;
        ctx.end(1, StageStatus::Done, format!("{} verified against the header", dir.display()));
        check(ctx)?;

        ctx.start(2);
        let segments: Vec<(u64, &[u8])> = vec![(module.rodata_addr, &module.rodata), (module.data_addr, &module.data)];
        let opts = ew_lift::LiftOptions {
            module: "app".into(),
            max_functions: limit,
            max_insns: 4096,
            ..Default::default()
        };
        let (report, funcs) = ew_lift::lift_text_in(&module.text, &segments, &opts);
        ctx.end(
            2,
            StageStatus::Done,
            format!("{:.2}% of {} instruction words decode ({} unsupported)", report.decode_coverage * 100.0, report.text_words - report.padding_words, report.unsupported_words),
        );
        ctx.start(3);
        ctx.end(3, StageStatus::Done, format!("{} functions recovered, {} complete", report.lifted_functions, report.complete_functions));
        ctx.start(4);
        ctx.end(
            4,
            StageStatus::Done,
            format!("{} blocks, {} jump tables ({} targets), {} indirect edges left for run time", report.blocks_lifted, report.jump_tables, report.jump_table_targets, report.unresolved_edges),
        );
        check(ctx)?;

        ctx.start(5);
        let out = Config::work_dir().ok_or("APPDATA is not set")?.join("lift");
        std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        // Shards of a few hundred functions compile in parallel and fail
        // independently, where one translation unit of the whole lift takes
        // minutes on a single core.
        for old in std::fs::read_dir(&out).map_err(|e| e.to_string())?.flatten() {
            let name = old.file_name().to_string_lossy().into_owned();
            if name.starts_with("app_lift") && (name.ends_with(".c") || name.ends_with(".o")) {
                let _ = std::fs::remove_file(old.path());
            }
        }
        std::fs::write(out.join("ew_aarch64.h"), ew_lift::RUNTIME_HEADER).map_err(|e| e.to_string())?;
        std::fs::write(out.join("ew_dispatch.h"), ew_lift::dispatch_header()).map_err(|e| e.to_string())?;
        let mut shards = Vec::new();
        let mut bytes = 0usize;
        for (i, chunk) in balanced_shards(&funcs).into_iter().enumerate() {
            let c = ew_lift::emit_module(&format!("app_{i:03}"), chunk);
            bytes += c.len();
            let path = out.join(format!("app_lift_{i:03}.c"));
            std::fs::write(&path, &c).map_err(|e| e.to_string())?;
            shards.push(path);
            check(ctx)?;
        }
        ctx.end(5, StageStatus::Done, format!("{} MiB of C in {} shards, {}", bytes >> 20, shards.len(), out.display()));
        summary.output = Some(out.clone());
        check(ctx)?;

        ctx.start(6);
        let cc = find_cc();
        match &cc {
            None => ctx.end(6, StageStatus::Skipped, "no C compiler found (install zig with `pip install ziglang`, or clang)"),
            Some(cc) => {
                if let Err(e) = check_compiler(ctx, cc, &out) {
                    ctx.end(6, StageStatus::Failed, e.clone());
                    return Err(e);
                }
                let (built, failed, threads, first) = compile_shards(ctx, cc, &out, &shards)?;
                summary.compiled = Some(failed == 0);
                if failed == 0 {
                    ctx.end(6, StageStatus::Done, format!("{built} native objects built on {threads} threads"));
                } else {
                    // The compiler's own words, not just a count: that is what a
                    // reader needs to tell a missing tool from a bad shard.
                    let why = first.unwrap_or_else(|| "no output".into());
                    ctx.end(6, StageStatus::Failed, format!("{failed} of {} shards were rejected by the C compiler: {why}", shards.len()));
                    return Err(format!(
                        "the C compiler rejected {failed} of {} shards ({why}). The whole output is in {}",
                        shards.len(),
                        log_dir().map(|d| d.display().to_string()).unwrap_or_else(|| "the log".into())
                    ));
                }
            }
        }
        check(ctx)?;

        ctx.start(7);
        match &cc {
            Some(cc) if summary.compiled == Some(true) => {
                let (ok, total) = verify(ctx, cc, &funcs, &out)?;
                summary.verified = Some((ok, total));
                let status = if ok == total { StageStatus::Done } else { StageStatus::Failed };
                ctx.end(7, status, format!("{ok} of {total} functions identical to the reference interpreter"));
            }
            _ => ctx.end(7, StageStatus::Skipped, "needs the native build"),
        }

        ctx.start(8);
        let services = ew_boot::hle::HostServices::defaults();
        let seq = module.startup_sequence();
        ctx.end(
            8,
            StageStatus::Done,
            format!("{} imports named, {} host services, startup: {}", module.imports.jumps.len(), services.len(), seq.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(" -> ")),
        );
        summary.report = Some(report);
        Ok(JobResult::Pipeline(Box::new(summary)))
    })
}

/// Translation units per compiler thread. Several per thread keep every core
/// busy to the end even when one unit holds an unusually large function.
const SHARDS_PER_THREAD: usize = 4;

/// Split the lifted functions into contiguous groups of roughly equal
/// instruction count, which is what compile time follows.
fn balanced_shards(funcs: &[ew_lift::LiftedFunction]) -> Vec<&[ew_lift::LiftedFunction]> {
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    let total: usize = funcs.iter().map(|f| f.insn_count()).sum();
    let target = (total / (threads * SHARDS_PER_THREAD).max(1)).max(1);
    let mut out = Vec::new();
    let (mut start, mut acc) = (0usize, 0usize);
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

/// Compile every shard with one compiler process per core. Cancelling kills
/// the processes still running. Returns (built, failed, threads).
/// Build one short file first: a compiler that cannot run at all should say so
/// once, in its own words, instead of failing every shard in turn.
fn check_compiler(ctx: &Ctx, cc: &[String], out: &Path) -> Result<(), String> {
    let probe = out.join("ew_cc_probe.c");
    std::fs::write(&probe, "int ew_cc_probe(void) { return 0; }
").map_err(|e| format!("writing {}: {e}", probe.display()))?;
    let output = std::process::Command::new(&cc[0])
        .args(&cc[1..])
        .args(["-std=c99", "-c", "-o"])
        .arg(probe.with_extension("o"))
        .arg(&probe)
        .output()
        .map_err(|e| {
            format!("running `{}`: {e}. Install a C compiler: `pip install ziglang` (zig cc), or LLVM's clang", cc.join(" "))
        })?;
    if output.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&output.stderr).lines().take(4).collect::<Vec<_>>().join(" | ");
    ctx.log(format!("the compiler cannot build a one-line file: {err}"));
    Err(format!("`{}` cannot build C: {err}", cc.join(" ")))
}

fn compile_shards(ctx: &Ctx, cc: &[String], out: &Path, shards: &[PathBuf]) -> Result<(usize, usize, usize, Option<String>), String> {
    // What will be run, and which version of it: a failing build is usually one
    // of these two lines.
    let version = std::process::Command::new(&cc[0])
        .args(&cc[1..])
        .arg("--version")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or_default().to_string())
        .unwrap_or_else(|e| format!("could not run {}: {e}", cc[0]));
    ctx.log(format!("compiler: {}", cc.join(" ")));
    ctx.log(format!("version: {version}"));
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(1, 32);
    let mut pending: std::collections::VecDeque<&PathBuf> = shards.iter().collect();
    let mut running: Vec<(PathBuf, std::process::Child)> = Vec::new();
    let (mut built, mut failed) = (0usize, 0usize);
    let mut first_error: Option<String> = None;
    loop {
        if ctx.cancelled() {
            for (_, mut child) in running {
                let _ = child.kill();
            }
            return Err("cancelled by the user".into());
        }
        while running.len() < threads {
            let Some(src) = pending.pop_front() else { break };
            let child = std::process::Command::new(&cc[0])
                .args(&cc[1..])
                // -O0: this stage proves the translation builds natively. The
                // largest lifted functions take many minutes at -O1, which is
                // recorded as open work in docs/STATUS.md.
                .args(["-std=c99", "-O0", "-c", "-w", "-I"])
                .arg(out)
                .arg(src)
                .arg("-o")
                .arg(src.with_extension("o"))
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| format!("starting the C compiler ({}): {e}", cc.join(" ")))?;
            running.push((src.clone(), child));
        }
        if running.is_empty() {
            break;
        }
        let mut still = Vec::new();
        for (src, mut child) in running {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        built += 1;
                    } else {
                        failed += 1;
                        let mut err = String::new();
                        if let Some(mut e) = child.stderr.take() {
                            use std::io::Read;
                            let _ = e.read_to_string(&mut err);
                        }
                        let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("no output").trim().to_string();
                        if failed == 1 {
                            first_error = Some(format!("{}: {first}", src.file_name().unwrap_or_default().to_string_lossy()));
                        }
                        // The whole complaint goes to the file: the first line is
                        // usually the file and line, the reason is under it.
                        ctx.log(format!("{}: {}", src.display(), first));
                        for line in err.lines().take(200).skip(1) {
                            ctx.write(&format!("  {line}"));
                        }
                    }
                    ctx.log(format!("compiled {} of {} shards", built + failed, shards.len()));
                }
                Ok(None) => still.push((src, child)),
                Err(e) => return Err(format!("waiting for the C compiler: {e}")),
            }
        }
        running = still;
        std::thread::sleep(Duration::from_millis(50));
    }
    if failed > 0 {
        ctx.log(format!("{failed} of {} shards failed with `{}`", shards.len(), cc.join(" ")));
    }
    Ok((built, failed, threads, first_error))
}

/// Run up to 100 leaf functions through the reference interpreter and the
/// compiled C, and count the ones whose final registers agree.
fn verify(ctx: &Ctx, cc: &[String], funcs: &[ew_lift::LiftedFunction], out: &Path) -> Result<(usize, usize), String> {
    let mut chosen = Vec::new();
    let mut expected = Vec::new();
    for f in funcs.iter().filter(|f| f.complete && !f.has_calls) {
        if chosen.len() >= 100 {
            break;
        }
        let mut st = ew_lift::State::patterned();
        if ew_lift::interpreter::run_function(&mut st, f, 5000).is_ok() {
            let mut line = format!("fn=0x{:x} halted={} pc={:x} nzcv={:x}", f.addr, st.halted as u32, st.pc, st.nzcv);
            for i in 0..31 {
                line.push_str(&format!(" x{i}={:x}", st.x[i]));
            }
            line.push_str(&format!(" sp={:x}", st.sp));
            for i in 0..32 {
                line.push_str(&format!(" v{i}={:x}:{:x}", st.v[i][1], st.v[i][0]));
            }
            expected.push(line);
            chosen.push(f.clone());
        }
    }
    let src = out.join("campaign.c");
    std::fs::write(&src, ew_lift::emit_campaign("app", &chosen)).map_err(|e| e.to_string())?;
    let exe = out.join("campaign.exe");
    let built = std::process::Command::new(&cc[0])
        .args(&cc[1..])
        .args(["-std=c99", "-O1", "-w", "-I"])
        .arg(out)
        .arg(&src)
        .arg("-o")
        .arg(&exe)
        .output()
        .map_err(|e| e.to_string())?;
    if !built.status.success() {
        ctx.log(String::from_utf8_lossy(&built.stderr).into_owned());
        return Err("the verification program did not compile".into());
    }
    let ran = std::process::Command::new(&exe).output().map_err(|e| e.to_string())?;
    let native = String::from_utf8_lossy(&ran.stdout).replace('\r', "");
    let native: Vec<&str> = native.lines().filter(|l| l.starts_with("fn=")).collect();
    let ok = expected.iter().zip(&native).filter(|(a, b)| a.as_str() == **b).count();
    Ok((ok, expected.len()))
}

// ---- boot ---------------------------------------------------------------------------

pub fn boot(config: &Config, game: &GameData) -> Job {
    let decoded = game.decoded.clone();
    let romfs = game.romfs.clone();
    let exefs = game.exefs.clone();
    let title = game.title.clone();
    let cfg = config.clone();
    spawn(JobKind::Boot, &["Map the modules", "Execute the game"], move |ctx| {
        ctx.start(0);
        let dir = decoded.ok_or("the main module is not decoded")?;
        let module = ew_boot::Module::from_dir(&dir).map_err(|e| format!("{e:#}"))?;
        let mut sequence = module.startup_sequence();
        let (entry_name, entry) = sequence.remove(0);
        // The title's sdk module supplies the C++ standard library and other
        // pure library code. It is LZ4 compressed, so it is read straight from
        // the dump and never written anywhere.
        let mut libraries = Vec::new();
        if let Some(path) = exefs.map(|e| e.join("sdk")).filter(|p| p.is_file()) {
            let bytes = std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
            let nso = ew_format::Nso::parse(&bytes).map_err(|e| format!("{e:#}"))?;
            let seg = |k| nso.decompress_segment(k).map_err(|e| format!("{e:#}"));
            let (t, r, d) = (seg(ew_format::SegmentKind::Text)?, seg(ew_format::SegmentKind::Rodata)?, seg(ew_format::SegmentKind::Data)?);
            libraries.push(ew_boot::Module::from_segments("sdk", t, r, d).map_err(|e| format!("{e:#}"))?);
            ctx.log(format!("sdk loaded from {}", path.display()));
        } else {
            ctx.log("no sdk module next to main; library calls will stop the run".to_string());
        }
        ctx.end(
            0,
            StageStatus::Done,
            format!("{} imports, {} library module(s), startup {} then {} more", module.imports.jumps.len(), libraries.len(), entry_name, sequence.len()),
        );

        let mut settings = std::collections::BTreeMap::new();
        settings.insert("language".to_string(), cfg.system.language.clone());
        settings.insert("user_name".to_string(), cfg.system.user_name.clone());
        if let Some((_, version)) = title {
            settings.insert("display_version".to_string(), version);
        }
        let opts = ew_boot::BootOptions {
            entry,
            entry_name,
            then: sequence,
            max_steps: cfg.runtime.max_steps,
            heap_bytes: cfg.runtime.heap_mib.clamp(64, 8192) << 20,
            romfs,
            save_dir: Config::save_dir(),
            settings,
            cancel: Some(ctx.cancel.clone()),
            progress: Some(ctx.steps.clone()),
            libraries,
            ..Default::default()
        };
        ctx.start(1);
        let started = Instant::now();
        let report = ew_boot::boot(&module, &opts).map_err(|e| format!("{e:#}"))?;
        let ms = started.elapsed().as_millis();
        ctx.end(1, StageStatus::Done, report.headline());
        if let Some(path) = Config::dir().map(|d| d.join("last-boot.json")) {
            if let Ok(json) = serde_json::to_string_pretty(&report) {
                let _ = std::fs::write(path, json);
            }
        }
        Ok(JobResult::Boot(Box::new(report), ms))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_log_starts_with_what_a_report_needs() {
        let dir = std::env::temp_dir().join(format!("ew-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (path, mut file) = open_log(&dir, JobKind::Pipeline).unwrap();
        writeln!(file, "compiled 3 of 3 shards").unwrap();
        drop(file);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("EmblemWeave "), "{text}");
        assert!(text.contains("Recompilation pipeline"), "{text}");
        assert!(text.contains("exe: "), "{text}");
        assert!(text.contains("compiled 3 of 3 shards"), "{text}");
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("pipeline-"), "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
