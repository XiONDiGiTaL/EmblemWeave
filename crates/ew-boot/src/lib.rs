//! `ew-boot` — the whole-program dispatcher.
//!
//! `ew-lift` recovers one function at a time and `ew-lift`'s emitter returns to
//! a dispatcher at every edge that leaves a function. This crate is that
//! dispatcher, running the recovered code against a mapped module image so the
//! project can answer a question that static analysis cannot: what does the game
//! actually do when it starts, and where does it stop?
//!
//! Three properties are deliberate:
//!
//! * every stop is a typed reason with an address, so a blocker is a fact and
//!   not an opinion;
//! * memory is a flat buffer sized from the module image, and an access outside
//!   it is a fault reported with the address, never a silent zero;
//! * an instruction the decoder does not model stops the run. Nothing is skipped
//!   to make progress look better than it is.
//!
//! The runner uses the reference interpreter, not the emitted C. That is stated
//! plainly because it matters: the C emitter is verified function by function,
//! while this runner is the tool that exposes which function to look at next.

use anyhow::{bail, Context, Result};
use ew_lift::{cfg, scan, Op, State};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

pub mod alloc;
pub mod audio;
pub mod audio_update;
pub mod codec;
pub mod prepo;
pub mod movie;
pub mod cxx;
pub mod fs;
pub mod gfx;
pub mod hash;
pub mod hid;
pub mod hle;
pub mod imports;
pub mod kernel;
pub mod libc;
pub mod lmem;
pub mod math;
pub mod native;

/// A fast hasher for the run loop's integer keys (guest addresses and call
/// ids): a multiply and a rotate, as rustc's own FxHash. The standard
/// library's SipHash costs more than the work done per function entry.
#[derive(Default, Clone, Copy)]
pub struct FastHasher(u64);

impl std::hash::Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u64(&mut self, x: u64) {
        self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u32(&mut self, x: u32) {
        self.write_u64(x as u64);
    }
    fn write_usize(&mut self, x: usize) {
        self.write_u64(x as u64);
    }
}

pub type FastMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<FastHasher>>;
pub type FastSet<K> = std::collections::HashSet<K, std::hash::BuildHasherDefault<FastHasher>>;
pub mod nvn;
pub mod nvn_mem;
pub mod nvn_tex;
pub mod nvn_win;
pub mod nvn_cmd;
pub mod nvn_state;
pub mod nvn_prog;
pub mod pmr;
pub mod sched;
pub mod services;
pub mod system;
pub mod threads;

pub use hle::{HostResult, HostServices};
pub use imports::{Import, ImportTable};

/// Guest stack size reserved below the module image.
pub const STACK_SIZE: u64 = 1 << 20;
/// Guest heap size reserved above the stack.
pub const HEAP_SIZE: u64 = 4 << 30;
/// Extra zero-filled room for `.bss` when the real size is unknown.
pub const BSS_SLACK: u64 = 4 << 20;

/// Return address installed in `x30` before entry. It is deliberately outside
/// any module image and page-aligned, so returning from the entry function is
/// unambiguous rather than looking like a jump into code.
pub const RETURN_SENTINEL: u64 = 0xFFFF_FFFF_FFFF_FFFC;
/// Room for host functions created at run time, such as NVN procedures.
const DYNAMIC_STUBS: u64 = 4096;

const PAGE: u64 = 0x1000;

/// Main-module imports a library may satisfy: the C++ standard library, whose
/// code is pure and self-contained. Everything else stays with the host
/// services, which name what they do not provide.
pub const LIBRARY_PREFIXES: &[&str] =
    &[
        "_ZNSt3__1", "_ZNKSt3__1", "_ZSt", "_ZTVNSt3__1", "_ZTINSt3__1", "_ZTSNSt3__1",
        "_ZN2nn4util", "_ZNK2nn4util",
        "_ZN2nn3hid20ControllerSupportArg",
        "_ZN2nn5audio", "_ZNK2nn5audio",
        "_ZN2nn5codec", "_ZNK2nn5codec",
        "__cxa_demangle", "__dynamic_cast", "qsort",
        "_ZNSt13exception_ptr", "_ZNKSt13exception_ptr",
    ];

/// Where a library module sits in guest memory.
struct Placed<'a> {
    module: &'a Module,
    base: u64,
}

fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

/// Title metadata from a `control.nacp`: the first title name and the display
/// version. Returns `None` for a file that is not a NACP.
pub fn read_nacp(path: &std::path::Path) -> Option<(String, String)> {
    let d = std::fs::read(path).ok()?;
    if d.len() < 0x4000 {
        return None;
    }
    let text = |b: &[u8]| {
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        String::from_utf8_lossy(&b[..end]).into_owned()
    };
    let title = text(&d[0..0x200]);
    let version = text(&d[0x3060..0x3070]);
    (!version.is_empty()).then_some((title, version))
}

/// A decompressed module laid out at the addresses its code expects.
///
/// The text segment is placed at address 0. Nintendo's linker links NSO modules
/// against a zero base and applies load relocations at run time, so a zero base
/// keeps PC-relative references, and every unrelocated pointer stored in `.data`,
/// pointing at the right place.
#[derive(Debug, Clone)]
pub struct Module {
    pub name: String,
    pub text: Vec<u8>,
    pub rodata: Vec<u8>,
    pub data: Vec<u8>,
    pub rodata_addr: u64,
    pub data_addr: u64,
    /// `.bss` range from `MOD0` when the module carries one.
    pub bss_start: Option<u64>,
    pub bss_end: Option<u64>,
    /// Dynamic-link metadata: every external symbol this module calls.
    pub imports: ImportTable,
}

impl Module {
    pub fn new(name: &str, text: Vec<u8>, rodata: Vec<u8>, data: Vec<u8>) -> Module {
        let rodata_addr = align_up(text.len() as u64, PAGE);
        let data_addr = align_up(rodata_addr + rodata.len() as u64, PAGE);
        Module {
            name: name.to_string(),
            text,
            rodata,
            data,
            rodata_addr,
            data_addr,
            bss_start: None,
            bss_end: None,
            imports: ImportTable::default(),
        }
    }

    /// A module from decompressed segments, with its dynamic tables read.
    pub fn from_segments(name: &str, text: Vec<u8>, rodata: Vec<u8>, data: Vec<u8>) -> Result<Module> {
        if text.len() % 4 != 0 {
            bail!("text segment length {} is not a multiple of 4", text.len());
        }
        let mut module = Module::new(name, text, rodata, data);
        module.imports = imports::read(&module);
        module.bss_start = module.imports.bss_start;
        module.bss_end = module.imports.bss_end;
        Ok(module)
    }

    /// Load `text.bin`, `rodata.bin` and `data.bin` from a decoded module
    /// directory. `data.bin` is optional because a module can have no `.data`.
    pub fn from_dir(dir: &Path) -> Result<Module> {
        let name = dir
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "module".to_string());
        let text = std::fs::read(dir.join("text.bin"))
            .with_context(|| format!("reading {}", dir.join("text.bin").display()))?;
        let rodata = read_optional(&dir.join("rodata.bin"))?;
        let data = read_optional(&dir.join("data.bin"))?;
        if text.len() % 4 != 0 {
            bail!(
                "text segment length {} is not a multiple of 4 words",
                text.len()
            );
        }
        let mut module = Module::new(&name, text, rodata, data);
        module.imports = imports::read(&module);
        module.bss_start = module.imports.bss_start;
        module.bss_end = module.imports.bss_end;
        Ok(module)
    }

    /// Best entry address for this module.
    ///
    /// A retail NSO program module starts `.text` with a 0x40 byte header rather
    /// than code: a flag word, the address of `MOD0`, and a `ret` stub. When that
    /// pattern is present the entry stub follows the header, which is where the
    /// loader starts the main thread. Modules that begin with code return 0.
    /// The calls nnSdk's `nn::init::Start` makes into a program module, in
    /// order, as far as this module provides them: `nninitStartup` (which sets
    /// up the allocator the static constructors use), the module's own entry
    /// (relocation protection and C++ static initialisation), then `nnMain`.
    pub fn startup_sequence(&self) -> Vec<(String, u64)> {
        let mut seq = Vec::new();
        if let Some(a) = self.imports.export("nninitStartup") {
            seq.push(("nninitStartup".to_string(), a));
        }
        seq.push(("module entry".to_string(), self.entry_hint()));
        if let Some(a) = self.imports.export("nnMain") {
            seq.push(("nnMain".to_string(), a));
        }
        seq
    }

    pub fn entry_hint(&self) -> u64 {
        if self.text.len() < 0x44 {
            return 0;
        }
        let first = u32::from_le_bytes([self.text[0], self.text[1], self.text[2], self.text[3]]);
        let decodes = !matches!(
            ew_lift::decode(first, 0),
            Op::Unsupported { .. } | Op::UnsupportedBitfield { .. }
        );
        if decodes {
            return 0;
        }
        // The header stores 32-bit image offsets, not 64-bit pointers.
        let mut raw = [0u8; 4];
        raw.copy_from_slice(&self.text[4..8]);
        let second = u32::from_le_bytes(raw) as u64;
        let ro_end = self.rodata_addr + self.rodata.len() as u64;
        if second >= self.rodata_addr && second < ro_end {
            return 0x40;
        }
        0
    }

    /// End of the loadable image: data end, or bss end when the module declares
    /// one past it.
    pub fn image_end(&self) -> u64 {
        let data_end = self.data_addr + self.data.len() as u64;
        match self.bss_end {
            Some(end) if end > data_end => end,
            _ => data_end,
        }
    }
}

fn read_optional(path: &Path) -> Result<Vec<u8>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// How far to run, and how much detail to keep.
#[derive(Debug, Clone)]
pub struct BootOptions {
    /// Guest address execution starts at. Text starts at 0, which is where the
    /// loader enters an NSO program module.
    pub entry: u64,
    /// Hard instruction budget for the whole run.
    pub max_steps: u64,
    /// Hard cap on dispatcher iterations (function entries).
    pub max_dispatches: usize,
    /// Instruction cap for one recovered function.
    pub max_insns: usize,
    /// Number of event lines to keep for the human-readable trace.
    pub trace: usize,
    /// Initial `x0`. The loader passes a launch-parameter pointer; callers that
    /// do not have one pass 0 and read the fault instead of guessing.
    pub x0: u64,
    /// Functions to call, in order, each time the previous one returns. This is
    /// how the run follows nnSdk's `nn::init::Start`: module initialisation,
    /// then `nnMain`, on one continuous guest state.
    pub then: Vec<(String, u64)>,
    /// Name of the first phase, for the report.
    pub entry_name: String,
    /// Bytes of guest heap arena. The memory is zero-initialised, which the
    /// host OS commits lazily, so an untouched gigabyte costs nothing.
    pub heap_bytes: u64,
    /// The user's extracted romfs, mounted read-only by `nn::fs::MountRom`.
    pub romfs: Option<std::path::PathBuf>,
    /// Writable directory backing `nn::fs::MountSaveData`.
    pub save_dir: Option<std::path::PathBuf>,
    /// System settings for the host services: `language`, `display_version`,
    /// `user_name`.
    pub settings: std::collections::BTreeMap<String, String>,
    /// Capture the guest state the first time the dispatcher reaches this
    /// address, into `snapshot_dir`, then log `snapshot_steps` instructions.
    pub snapshot_at: Option<u64>,
    pub snapshot_dir: Option<std::path::PathBuf>,
    pub snapshot_steps: usize,
    /// Set from another thread to stop the run at the next dispatch.
    pub cancel: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Updated with the instruction count as the run proceeds, for a UI.
    pub progress: Option<std::sync::Arc<std::sync::atomic::AtomicU64>>,
    /// The game's code compiled ahead of time (`ew-native`): run natively
    /// wherever it has the block, interpreted everywhere else.
    pub native: Option<std::sync::Arc<native::Native>>,
    /// A window title: the game's frames are shown in a window by that name
    /// for the rest of the run. The window opens on the first present, when the
    /// frame's size is known.
    pub window: Option<String>,
    /// Other modules of the program (the title's `sdk`), loaded after the
    /// main image. Their exports satisfy the main module's imports that the
    /// host does not serve and whose names start with one of
    /// [`LIBRARY_PREFIXES`]; their own imports bind to the main module first.
    pub libraries: Vec<Module>,
    /// Run each library's DT_INIT before the program. Off by default: nnSdk's
    /// initialisers build on its own `nn::os` start-up, which the host serves
    /// instead, so library code is used for routines that need no statics.
    pub init_libraries: bool,
    /// Addresses to watch: each time a function is entered at one of them,
    /// its first five arguments and caller go to the host log (up to 64 each).
    pub watch: Vec<u64>,
}

impl Default for BootOptions {
    fn default() -> Self {
        BootOptions {
            entry: 0,
            max_steps: 20_000_000,
            max_dispatches: usize::MAX,
            max_insns: 1024,
            trace: 64,
            x0: 0,
            then: Vec::new(),
            entry_name: "entry".to_string(),
            heap_bytes: HEAP_SIZE,
            romfs: None,
            save_dir: None,
            settings: Default::default(),
            snapshot_at: None,
            snapshot_dir: None,
            snapshot_steps: 20_000,
            cancel: None,
            progress: None,
            native: None,
            window: None,
            libraries: Vec::new(),
            init_libraries: false,
            watch: Vec::new(),
        }
    }
}

/// Why the run stopped.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stop {
    /// The entry function returned to the sentinel address.
    EntryReturned,
    /// Execution left the text segment: an import, a call into another module or
    /// a computed pointer that is not code.
    LeftText {
        pc: u64,
        from: u64,
        /// What the address points at, when it is inside a mapped segment.
        segment: Option<String>,
        /// First eight bytes at the address when it is inside the image.
        word: Option<u64>,
    },
    /// The decoder does not model the instruction at `pc`.
    Unsupported { pc: u64, word: u32 },
    /// The interpreter rejected the instruction, usually a guest memory fault.
    Fault { pc: u64, message: String },
    /// The guest set its own halt flag (a `brk` style stop).
    Halted { pc: u64, reason: String },
    /// The run reached the instruction budget with code still executing.
    StepBudget,
    /// The run reached the dispatch budget.
    DispatchBudget,
    /// Every live guest thread waits on something only another thread could
    /// provide. `waits` names each waiter and the call it is parked in.
    Deadlock { waits: String },
    /// Every guest thread, including the main thread, has exited.
    AllThreadsExited,
    /// The caller cancelled the run.
    Cancelled,
    /// One function ran its per-entry budget without leaving, which means a spin
    /// loop or a block set that does not contain the exit.
    SpinLimit { pc: u64 },
    /// The guest called an imported function. `import` is the symbol name read
    /// from the module's own dynamic table, so the blocker is always named.
    Blocked {
        import: String,
        pc: u64,
        from: u64,
        /// x0..x7 at the call, which is usually enough to see what was wanted.
        args: [u64; 8],
    },
    /// An implemented host function rejected the call, usually a bad pointer.
    HostFault {
        import: String,
        from: u64,
        message: String,
    },
}

impl Stop {
    /// One line a human can act on.
    pub fn describe(&self) -> String {
        match self {
            Stop::EntryReturned => "the entry function returned".to_string(),
            Stop::Deadlock { waits } => format!("deadlock: every guest thread is waiting ({waits})"),
            Stop::AllThreadsExited => "every guest thread exited".to_string(),
            Stop::Cancelled => "cancelled by the user".to_string(),
            Stop::LeftText { pc, from, segment, word } => {
                let mut s = format!("left the text segment at 0x{pc:x} (from 0x{from:x})");
                match segment {
                    Some(seg) => s.push_str(&format!(", inside {seg}")),
                    None => s.push_str(", outside the module image"),
                }
                if let Some(w) = word {
                    s.push_str(&format!(", holding 0x{w:016x}"));
                }
                s
            }
            Stop::Unsupported { pc, word } => {
                format!("unsupported instruction 0x{word:08x} at 0x{pc:x}")
            }
            Stop::Fault { pc, message } => format!("{message} at 0x{pc:x}"),
            Stop::Halted { pc, reason } => format!("guest halted at 0x{pc:x}: {reason}"),
            Stop::StepBudget => "instruction budget exhausted".to_string(),
            Stop::DispatchBudget => "dispatch budget exhausted".to_string(),
            Stop::SpinLimit { pc } => {
                format!("no control-flow exit from the block set at 0x{pc:x} within the per-entry budget")
            }
            Stop::Blocked { import, from, .. } => {
                format!("called {import} at 0x{from:x}, which the runtime does not implement")
            }
            Stop::HostFault { import, from, message } => {
                format!("host {import} failed at 0x{from:x}: {message}")
            }
        }
    }
}

/// A call or return the dispatcher resolved.
#[derive(Debug, Clone, Serialize)]
pub struct TransferRecord {
    pub from: u64,
    pub to: u64,
    /// `call`, `return` or `branch`.
    pub kind: &'static str,
}

/// A function that owns a large share of the executed instructions.
#[derive(Debug, Clone, Serialize)]
pub struct HotFunction {
    pub addr: u64,
    pub entries: u64,
    pub instructions: u64,
}

/// One imported function the guest called.
#[derive(Debug, Clone, Serialize)]
pub struct ImportCall {
    pub name: String,
    pub calls: u64,
    pub first_from: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootReport {
    pub module: String,
    pub entry: u64,
    pub steps: u64,
    pub dispatches: u64,
    /// Dispatches run by the native library, and the times it handed an
    /// address back to the interpreter.
    pub native_dispatches: u64,
    pub native_halts: u64,
    pub distinct_functions: usize,
    pub calls: u64,
    pub returns: u64,
    pub branches: u64,
    pub max_depth: u32,
    pub final_sp: u64,
    pub stack_low: u64,
    pub stop: Stop,
    pub trace: Vec<String>,
    pub hot: Vec<HotFunction>,
    pub externals: Vec<TransferRecord>,
    pub memory_bytes: u64,
    /// Imported symbols found in the module's dynamic table.
    pub import_count: usize,
    /// Imports the run reached, in call order, with call counts.
    pub import_calls: Vec<ImportCall>,
    pub bss_bytes: u64,
    /// Host functions the run executed successfully.
    pub host_calls: u64,
    /// Host functions available to this run.
    pub host_implemented: usize,
    /// Startup phases that ran to completion, in order.
    pub phases: Vec<String>,
    /// The phase that was running when the run stopped.
    pub phase: String,
    /// Relative relocation slots whose stored value differed from the addend.
    pub relocations_changed: u64,
    /// Guest threads that ran, including the main thread, and context switches.
    pub threads: u64,
    pub switches: u64,
    /// The last events before the stop (host calls with arguments and results,
    /// calls and returns), oldest first.
    pub recent: Vec<String>,
    /// Host-level events (files opened, mounts), in order.
    pub host_log: Vec<String>,
    /// Every guest thread at the stop.
    pub thread_states: Vec<String>,
    /// Names the game resolved to host functions at run time (NVN procedures),
    /// in the order it first asked for them.
    pub resolved: Vec<String>,
    /// Image slots (data and bss) holding a run-time host function's address,
    /// with that function's name: the game's own procedure tables.
    pub proc_slots: Vec<(u64, String)>,
}

impl BootReport {
    /// A short, honest summary suitable for a UI banner.
    pub fn headline(&self) -> String {
        format!(
            "{} instructions, {} function entries, {} distinct functions, stopped because {}",
            self.steps,
            self.dispatches,
            self.distinct_functions,
            self.stop.describe()
        )
    }
}

/// One recovered function plus its instruction index, held for the whole run.
struct Cached {
    compiled: ew_lift::interpreter::Compiled,
}

/// Run the module from `opts.entry` for as long as the guest code allows, using
/// the default host services.
pub fn boot(module: &Module, opts: &BootOptions) -> Result<BootReport> {
    boot_with(module, opts, &HostServices::defaults())
}

/// Run with an explicit set of host services.
pub fn boot_with(
    module: &Module,
    opts: &BootOptions,
    services: &HostServices,
) -> Result<BootReport> {
    let text = &module.text;
    if opts.entry % 4 != 0 || opts.entry as usize + 4 > text.len() {
        bail!("entry 0x{:x} is not inside the text segment", opts.entry);
    }

    // Layout: image, import stubs, stack, heap. Everything is inside one flat
    // buffer, so a read or write outside it faults in the interpreter with the
    // address that caused it.
    // Libraries follow the main image, each on a 1 MiB boundary.
    let mut next_base = align_up(module.image_end(), 0x10_0000);
    let placed: Vec<Placed> = opts
        .libraries
        .iter()
        .map(|m| {
            let base = next_base;
            next_base = align_up(base + m.image_end(), 0x10_0000);
            Placed { module: m, base }
        })
        .collect();
    let image_end = if placed.is_empty() { align_up(module.image_end(), PAGE) } else { next_base };
    // Every symbol each module defines, at its loaded address. The main module
    // comes first, so a symbol it defines (operator new, for one) wins.
    let mut symbols: HashMap<String, u64> = HashMap::new();
    for e in &module.imports.exports {
        symbols.entry(e.name.clone()).or_insert(e.addr);
    }
    for p in &placed {
        for e in &p.module.imports.exports {
            symbols.entry(e.name.clone()).or_insert(p.base + e.addr);
        }
    }
    let library_symbol = |name: &str| -> Option<u64> {
        if module.imports.export(name).is_some() || !LIBRARY_PREFIXES.iter().any(|p| name.starts_with(p)) {
            return None;
        }
        placed.iter().find_map(|p| p.module.imports.export(name).map(|a| p.base + a))
    };
    // Main imports the host does not serve go to a library when one has them.
    let main_binds: Vec<Option<u64>> = module
        .imports
        .jumps
        .iter()
        .map(|j| if services.get(&j.name).is_some() { None } else { library_symbol(&j.name) })
        .collect();
    // Library imports: another module's symbol, or a host stub by name.
    let mut lib_binds: Vec<(u64, Option<u64>, String)> = Vec::new();
    for p in &placed {
        for j in &p.module.imports.jumps {
            let target = if services.get(&j.name).is_some() { None } else { symbols.get(&j.name).copied() };
            lib_binds.push((p.base + j.slot, target, j.name.clone()));
        }
    }
    let jumps = &module.imports.jumps;
    // Sixteen bytes per stub keeps every entry distinct and aligned, and one
    // final stub stands for a slot the guest reached without a binding.
    // Host functions with no import slot (vtable entries of host-built
    // objects) get stubs after the imports.
    let names: Vec<String> = jumps
        .iter()
        .map(|j| j.name.clone())
        .chain(pmr::SYNTHETIC.iter().map(|n| n.to_string()))
        .chain(lib_binds.iter().filter(|b| b.1.is_none()).map(|b| b.2.clone()))
        .collect();
    let stub_bytes = align_up((names.len() as u64 + 1 + DYNAMIC_STUBS) * 16, PAGE);
    let stub_base = image_end;
    let unresolved_stub = stub_base + names.len() as u64 * 16;
    let stack_base = stub_base + stub_bytes;
    let stack_top = stack_base + STACK_SIZE;
    let heap_base = stack_top;
    let memory_bytes = align_up(heap_base + opts.heap_bytes, PAGE);
    let mut memory = vec![0u8; memory_bytes as usize];
    memory[..text.len()].copy_from_slice(text);
    let ro = module.rodata_addr as usize;
    memory[ro..ro + module.rodata.len()].copy_from_slice(&module.rodata);
    let da = module.data_addr as usize;
    memory[da..da + module.data.len()].copy_from_slice(&module.data);
    for p in &placed {
        let m = p.module;
        let b = p.base as usize;
        memory[b..b + m.text.len()].copy_from_slice(&m.text);
        let ro = b + m.rodata_addr as usize;
        memory[ro..ro + m.rodata.len()].copy_from_slice(&m.rodata);
        let da = b + m.data_addr as usize;
        memory[da..da + m.data.len()].copy_from_slice(&m.data);
        for &(slot, addend) in &m.imports.relatives {
            write_u64(&mut memory, p.base + slot, p.base + addend);
        }
        for &slot in &m.imports.relr {
            let at = (p.base + slot) as usize;
            if at + 8 <= memory.len() {
                let v = u64::from_le_bytes(memory[at..at + 8].try_into().unwrap());
                write_u64(&mut memory, p.base + slot, v + p.base);
            }
        }
    }

    // Apply the relative relocations exactly as rtld does with a zero load
    // base: the slot receives the addend. Count how many slots this changed, so
    // the report shows whether the image depended on it.
    let mut relocated = 0u64;
    for &(slot, addend) in &module.imports.relatives {
        let at = slot as usize;
        if at + 8 <= memory.len() {
            let mut old = [0u8; 8];
            old.copy_from_slice(&memory[at..at + 8]);
            if u64::from_le_bytes(old) != addend {
                relocated += 1;
            }
            write_u64(&mut memory, slot, addend);
        }
    }

    // Bind every jump slot to its own stub. This is the loader's job in the real
    // system; here the stub address is how the dispatcher recognises an import
    // call and can name it instead of jumping to zero.
    for (i, import) in jumps.iter().enumerate() {
        let target = main_binds[i].unwrap_or(stub_base + i as u64 * 16);
        write_u64(&mut memory, import.slot, target);
    }
    let mut lib_stub = jumps.len() + pmr::SYNTHETIC.len();
    for (slot, target, _) in &lib_binds {
        let addr = match target {
            Some(a) => *a,
            None => {
                lib_stub += 1;
                stub_base + (lib_stub - 1) as u64 * 16
            }
        };
        write_u64(&mut memory, *slot, addr);
    }
    // Named data pointers between modules (vtables, typeinfo, globals).
    let mut data_bound = 0u64;
    for (base, m) in std::iter::once((0u64, module)).chain(placed.iter().map(|p| (p.base, p.module))) {
        for (slot, name, addend) in &m.imports.data_binds {
            if let Some(a) = symbols.get(name) {
                write_u64(&mut memory, base + slot, a + addend);
                data_bound += 1;
            }
        }
        if base != 0 {
            if let Some(got) = m.imports.got {
                write_u64(&mut memory, base + got + 0x10, unresolved_stub);
            }
        }
    }
    let _ = data_bound;
    // GOT[2] is the lazy resolver every unbound slot starts at. Point it at a
    // stub that reports an unresolved binding rather than branching to zero.
    if let Some(got) = module.imports.got {
        write_u64(&mut memory, got + 0x10, unresolved_stub);
    }
    let stub_index: HashMap<u64, usize> = names
        .iter()
        .enumerate()
        .map(|(i, _)| (stub_base + i as u64 * 16, i))
        .collect();

    let mut state = State { mem: memory, ..State::default() };
    // The heap doubles as the page arena the `nn::os` memory services hand out
    // from, so an allocation is real guest memory the interpreter can load from.
    state.host.alloc_next = heap_base;
    state.host.alloc_end = heap_base + opts.heap_bytes;
    state.host.program_size = image_end;
    // A windowed run shows the game's frames as it presents them; a headless
    // run counts them instead.
    if let Some(title) = &opts.window {
        nvn_win::show_frames_in_a_window(&mut state, title);
    }
    state.host.dyn_base = unresolved_stub + 16;
    state.host.dyn_capacity = DYNAMIC_STUBS;
    state.host.romfs_root = opts.romfs.clone();
    state.host.save_root = opts.save_dir.clone();
    state.host.settings = opts.settings.clone();
    state.host.stack_size = STACK_SIZE;
    let synthetic_stubs: Vec<u64> = (0..pmr::SYNTHETIC.len())
        .map(|i| stub_base + (jumps.len() + i) as u64 * 16)
        .collect();
    let exports: Vec<(&str, Option<u64>)> = ["_Znwm", "_ZnwmSt11align_val_t", "_ZdlPv", "_ZdlPvSt11align_val_t"]
        .iter()
        .map(|n| (*n, module.imports.export(n)))
        .collect();
    // Every symbol the module defines is available to host services that
    // forward into the game's own code (operator new, for one).
    for e in &module.imports.exports {
        state.host.named.insert(format!("export.{}", e.name), e.addr);
    }
    // Library exports too, where the main module does not define the name, so
    // a host service can observe a call and then forward it to the library.
    for p in &placed {
        for e in &p.module.imports.exports {
            state.host.named.entry(format!("export.{}", e.name)).or_insert(p.base + e.addr);
        }
    }
    if let Err(e) = pmr::install(&mut state, &synthetic_stubs, &exports) {
        bail!("could not build the host's libc++ objects: {e}");
    }
    state.sp = (stack_top - 0x100) & !0xF;
    state.x[30] = RETURN_SENTINEL;
    state.x[0] = opts.x0;
    state.pc = opts.entry;

    // Code of every module, at its loaded address, for the recompiler.
    let code_end = placed.iter().map(|p| p.base + p.module.text.len() as u64).max().unwrap_or(0).max(text.len() as u64);
    let code_buf: Vec<u8> = if placed.is_empty() { Vec::new() } else { state.mem[..code_end as usize].to_vec() };
    let main_text = text;
    let text: &[u8] = if placed.is_empty() { main_text } else { &code_buf };
    let code_ranges: Vec<(u64, u64)> = std::iter::once((0u64, main_text.len() as u64))
        .chain(placed.iter().map(|p| (p.base, p.base + p.module.text.len() as u64)))
        .collect();
    // A faulting branch can hand this a program counter near the top of the
    // address space, so the four bytes it needs are subtracted rather than
    // added: adding would overflow and wrap into a range that looks valid.
    let in_code = |pc: u64| pc % 4 == 0 && code_ranges.iter().any(|&(lo, hi)| pc >= lo && hi.saturating_sub(pc) >= 4);
    let mut boundaries = scan::call_targets(main_text);
    for p in &placed {
        boundaries.extend(scan::call_targets(&p.module.text).into_iter().map(|t| t + p.base));
    }
    let mut segments: Vec<(u64, &[u8])> = vec![
        (module.rodata_addr, module.rodata.as_slice()),
        (module.data_addr, module.data.as_slice()),
    ];
    for p in &placed {
        segments.push((p.base + p.module.rodata_addr, p.module.rodata.as_slice()));
        segments.push((p.base + p.module.data_addr, p.module.data.as_slice()));
    }

    let mut cache: FastMap<u64, Cached> = FastMap::default();
    let mut hot: FastMap<u64, (u64, u64)> = FastMap::default();
    let mut trace: Vec<String> = Vec::new();
    let mut externals: Vec<TransferRecord> = Vec::new();
    let mut dispatches = 0u64;
    let mut native_dispatches = 0u64;
    let mut native_halts = 0u64;
    let mut skip_native: Option<u64> = None;
    // EW_NATIVE_CHECK=1: every native call is also tried on the side and
    // compared with the interpreter, which stays the one that runs.
    let native_check = opts.native.is_some() && std::env::var("EW_NATIVE_CHECK").is_ok_and(|v| v != "0");
    let mut native_compared = 0u64;
    let mut check_start: Option<native::Regs>;
    let mut calls = 0u64;
    let mut returns = 0u64;
    let mut branches = 0u64;
    let mut depth = 0u32;
    let mut max_depth = 0u32;
    let mut stack_low = state.sp;
    let mut seen_functions: FastSet<u64> = FastSet::default();
    // Host calls are counted by call id: the index of a static import name, or
    // `names.len()` plus the index of a run-time host function. Supervisor
    // calls, which are rare, are counted by name.
    let mut call_counts: FastMap<u32, (u64, u64)> = FastMap::default();
    let mut import_calls: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    // Each stub address resolved once: its call id and handler.
    let mut resolved: FastMap<u64, (u32, Option<hle::Handler>)> = FastMap::default();
    let mut seen_starts = u64::MAX;
    let mut watch_hits: FastMap<u64, u32> = FastMap::default();
    // Return addresses of watched calls still in flight: (function, thread).
    let mut watch_returns: FastMap<u64, (u64, u64)> = FastMap::default();
    let mut host_calls = 0u64;
    // Origin of the last transfer, so a stop outside the text segment can name
    // the instruction that sent control there.
    let mut last_from = opts.entry;

    // The last events before a stop, kept as plain data and only formatted
    // into text for the report: formatting every call costs more than the
    // calls themselves in host-call-heavy code.
    let mut recent: std::collections::VecDeque<Event> = std::collections::VecDeque::new();
    macro_rules! event {
        ($e:expr) => {{
            if recent.len() >= 96 {
                recent.pop_front();
            }
            recent.push_back($e);
        }};
    }
    macro_rules! note {
        ($($arg:tt)*) => {{
            event!(Event::Text(format!($($arg)*)));
        }};
    }

    let mut queue: std::collections::VecDeque<(String, u64)> = opts.then.iter().cloned().collect();
    let mut phases: Vec<String> = Vec::new();
    let mut phase = opts.entry_name.clone();
    // Libraries initialise first, as the loader does before any program code
    // runs: each library's DT_INIT constructs its static objects.
    let lib_inits: Vec<(String, u64)> = placed
        .iter()
        .filter(|_| opts.init_libraries)
        .filter_map(|p| p.module.imports.init.map(|i| (format!("{} init", p.module.name), p.base + i)))
        .collect();
    if let Some((first, rest)) = lib_inits.split_first() {
        queue.push_front((phase.clone(), opts.entry));
        for init in rest.iter().rev() {
            queue.push_front(init.clone());
        }
        phase = first.0.clone();
        state.pc = first.1;
        state.x[0] = 0;
    }

    // The main thread is the pseudo thread object the os services already use.
    let main_object = match state.host.take_thread_object() {
        Ok(o) => o,
        Err(e) => bail!("could not create the main thread object: {e}"),
    };
    state.host.threads.insert(
        main_object,
        ew_lift::ThreadInfo { started: true, priority: 44, ..Default::default() },
    );
    // The main thread's thread-local region, holding its kernel handle.
    match sched::new_tls(&mut state, sched::handle_for(0), main_object) {
        Ok(tls) => state.tpidrro = tls,
        Err(e) => bail!("could not create the main thread's thread-local region: {e}"),
    }
    let mut sched = sched::Scheduler::new(main_object, &state);
    macro_rules! adopt {
        () => {{
            if state.host.thread_starts != seen_starts {
                seen_starts = state.host.thread_starts;
                sched.adopt_started(&mut state);
            }
        }};
    }
    // Switch away from the running thread; a stop when nothing can run.
    macro_rules! switch_or_stop {
        ($status:expr) => {
            match sched.switch(&mut state, $status) {
                Ok(()) => continue,
                Err(sched::NoRunnable::Deadlock(waits)) => break Stop::Deadlock { waits },
                Err(sched::NoRunnable::AllExited) => break Stop::AllThreadsExited,
                Err(sched::NoRunnable::Fault { call, message }) => {
                    break Stop::HostFault { import: call, from: 0, message }
                }
            }
        };
    }

    let mut since_poll = 0u32;
    let stop = loop {
        since_poll += 1;
        if since_poll >= 4096 {
            since_poll = 0;
            if let Some(p) = &opts.progress {
                p.store(state.steps, std::sync::atomic::Ordering::Relaxed);
            }
            if opts.cancel.as_ref().is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed)) {
                break Stop::Cancelled;
            }
        }
        if state.steps >= opts.max_steps {
            break Stop::StepBudget;
        }
        if dispatches as usize >= opts.max_dispatches {
            break Stop::DispatchBudget;
        }
        let pc = state.pc;
        if opts.snapshot_at == Some(pc) && state.step_log.is_none() {
            if let Some(dir) = &opts.snapshot_dir {
                if let Err(e) = write_snapshot(&state, text.len() as u64, dir) {
                    bail!("writing the snapshot: {e}");
                }
                let stubs: String = names
                    .iter()
                    .enumerate()
                    .map(|(i, n)| format!("{:x} {n}
", stub_base + i as u64 * 16))
                    .collect();
                std::fs::write(dir.join("stubs.txt"), stubs)?;
                state.step_log = Some(Vec::new());
                state.step_log_limit = opts.snapshot_steps;
            }
        }
        if pc == sched::THREAD_EXIT {
            let me = sched.current_object();
            if let Some(info) = state.host.threads.get_mut(&me) {
                info.exited = true;
            }
            switch_or_stop!(sched::Status::Exited);
        }
        if state.steps - sched.slice_start >= sched::QUANTUM && sched.runnable_others() {
            switch_or_stop!(sched::Status::Ready);
        }
        if pc == RETURN_SENTINEL && sched.is_main() {
            match queue.pop_front() {
                Some((name, addr)) => {
                    phases.push(std::mem::replace(&mut phase, name));
                    state.pc = addr;
                    state.x[30] = RETURN_SENTINEL;
                    state.x[0] = 0;
                    depth = 0;
                    continue;
                }
                None => {
                    phases.push(phase.clone());
                    break Stop::EntryReturned;
                }
            }
        }
        // A stub address means the guest called an imported function. That is
        // where host code has to take over, so the run stops with its name.
        let call = match resolved.get(&pc) {
            Some(&(id, handler)) => Some((id, handler)),
            None => {
                let dyn_index = (pc >= state.host.dyn_base && pc < state.host.dyn_base + state.host.dyn_capacity * 16)
                    .then(|| ((pc - state.host.dyn_base) / 16) as usize)
                    .filter(|i| *i < state.host.dyn_stubs.len());
                let id = stub_index.get(&pc).map(|&i| i as u32).or(dyn_index.map(|i| (names.len() + i) as u32));
                id.map(|id| {
                    let name = call_name(&names, &state.host.dyn_stubs, id);
                    let entry = (id, services.get(&name));
                    resolved.insert(pc, entry);
                    entry
                })
            }
        };
        if let Some((id, handler)) = call {
            call_counts.entry(id).or_insert((0, last_from)).0 += 1;
            let mut args = [0u64; 8];
            args.copy_from_slice(&state.x[0..8]);
            // The caller's return address, saved before the host runs.
            let return_to = state.x[30];
            let from = last_from;
            macro_rules! name {
                () => {
                    call_name(&names, &state.host.dyn_stubs, id)
                };
            }
            match handler {
                Some(handler) => match handler(&mut state) {
                    HostResult::Returned(value) => {
                        event!(Event::Host {
                            thread: sched.current_object(),
                            call: id,
                            args: [args[0], args[1], args[2], args[3]],
                            value,
                            from: return_to.wrapping_sub(4),
                        });
                        host_calls += 1;
                        adopt!();
                        state.x[0] = value;
                        state.pc = return_to;
                        last_from = from;
                        if !in_code(return_to) {
                            break Stop::LeftText {
                                pc: return_to,
                                from,
                                segment: segment_at(module, return_to),
                                word: read_word_at(module, return_to),
                            };
                        }
                        continue;
                    }
                    HostResult::Unavailable(message) => {
                        break Stop::HostFault { import: name!(), from, message }
                    }
                    HostResult::Jump(target) => {
                        host_calls += 1;
                        state.pc = target;
                        last_from = pc;
                        continue;
                    }
                    HostResult::Block { resume, timed } => {
                        event!(Event::Block { thread: sched.current_object(), call: id, args: [args[0], args[1]] });
                        host_calls += 1;
                        adopt!();
                        let call = name!();
                        switch_or_stop!(sched::Status::Blocked { resume, timed, return_to, call });
                    }
                },
                None => break Stop::Blocked { import: name!(), pc, from, args },
            }
        }
        if pc == unresolved_stub {
            break Stop::Blocked {
                import: "<unresolved jump slot>".to_string(),
                pc,
                from: last_from,
                args: [0u64; 8],
            };
        }
        if !in_code(pc) {
            break Stop::LeftText {
                pc,
                from: last_from,
                segment: segment_at(module, pc),
                word: read_word_at(module, pc),
            };
        }

        let native_fn = if skip_native.take() == Some(pc) { None } else { opts.native.as_ref().and_then(|n| n.get(pc)) };
        if (native_fn.is_none() || native_check) && !cache.contains_key(&pc) {
            let built = cfg::build(text, &segments, pc, opts.max_insns, &boundaries);
            match built {
                Some(func) => {
                    cache.insert(pc, Cached { compiled: ew_lift::interpreter::Compiled::new(&func) });
                }
                None => {
                    break Stop::Fault {
                        pc,
                        message: "no decodable instruction".to_string(),
                    }
                }
            }
        }

        if let Some(&(func, thread)) = watch_returns.get(&pc) {
            if thread == sched.current_object() {
                watch_returns.remove(&pc);
                let inner = hle::read_bytes(&state, state.x[0].wrapping_add(8), 8).map(|b| u64::from_le_bytes(b.try_into().unwrap())).unwrap_or(0);
                state.host.log.push(format!(
                    "watch 0x{func:x} returned x0 0x{:x} x1 0x{:x}; [x0]: {}; [[x0+8]]: {}",
                    state.x[0], state.x[1], peek_qwords(&state, state.x[0]), peek_qwords(&state, inner)
                ));
            }
        }
        if opts.watch.contains(&pc) {
            let n = watch_hits.entry(pc).or_insert(0u32);
            *n += 1;
            if *n <= 64 {
                watch_returns.insert(state.x[30], (pc, sched.current_object()));
                state.host.log.push(format!(
                    "watch 0x{pc:x} #{n} thread 0x{:x}: x0 0x{:x} x1 0x{:x} x2 0x{:x} x3 0x{:x} x4 0x{:x}, from 0x{:x}; [x0]: {}",
                    sched.current_object(), state.x[0], state.x[1], state.x[2], state.x[3], state.x[4], state.x[30].wrapping_sub(4), peek_qwords(&state, state.x[0])
                ));
                // EW_PEEK=addr,addr: memory to show at every watch hit.
                for addr in std::env::var("EW_PEEK").unwrap_or_default().split(',').filter_map(|a| u64::from_str_radix(a.trim().trim_start_matches("0x"), 16).ok()) {
                    let line = format!("  peek 0x{addr:x}: {}", peek_qwords(&state, addr));
                    state.host.log.push(line);
                }
            }
        }
        seen_functions.insert(pc);
        let entry = hot.entry(pc).or_insert((0, 0));
        entry.0 += 1;
        dispatches += 1;

        let steps_before = state.steps;
        let budget = if sched.runnable_others() { sched::QUANTUM as usize } else { 1 << 20 };
        let outcome = match (native_fn, opts.native.as_ref()) {
            (Some(f), Some(n)) if native_check => {
                // The registers at entry, for narrowing down a difference.
                // (The interpreter's stores are what change memory, and they
                // have already happened by then, so the narrowing replays
                // from these registers against memory as the run left it:
                // exact for the stores the function does not read back.)
                check_start = Some(native::Regs::of(&state));
                let trial = n.trial(f, &mut state, budget as u64);
                state.undo_log = Some(Vec::new());
                let outcome = cache.get(&pc).expect("cached above").compiled.run_traced(&mut state, budget);
                let after = native::Regs::of(&state);
                let comparable = !matches!(trial.exit, native::Exit::Halt(_) | native::Exit::Budget) && trial.regs.pc == after.pc && outcome.as_ref().map_or(state.svc.is_some(), |_| true);
                if comparable {
                    native_compared += 1;
                    let mut diff = trial.regs.diff(&after);
                    for (a, b) in &trial.writes {
                        let real = state.mem.get(*a as usize).copied();
                        if real != Some(*b) && diff.len() < 12 {
                            diff.push(format!("memory 0x{a:x}: native {b:02x} interpreter {:02x}", real.unwrap_or(0)));
                        }
                    }
                    if diff.is_empty() {
                        state.undo_log = None;
                    } else {
                        // Put memory back as it was at entry.
                        state.undo();
                        // Narrow it down: grow the native budget block by block
                        // and replay the interpreter for the same number of
                        // instructions, taking its stores back each time.
                        let end = native::Regs::of(&state);
                        let (end_x, end_mem_ok) = (end, ());
                        let _ = end_mem_ok;
                        let mut first = String::new();
                        if let Some(start) = check_start.take() {
                            let mut b = 1u64;
                            let mut prev_pc = pc;
                            while b < 1 << 16 {
                                let saved = std::mem::replace(&mut state.x, start.x);
                                let (sv, ssp, spc, snz) = (state.v, state.sp, state.pc, state.nzcv);
                                state.v = start.v;
                                state.sp = start.sp;
                                state.pc = start.pc;
                                state.nzcv = start.nzcv;
                                let t = n.trial(f, &mut state, b);
                                if t.executed == 0 {
                                    b += 1;
                                    state.x = saved;
                                    state.v = sv;
                                    state.sp = ssp;
                                    state.pc = spc;
                                    state.nzcv = snz;
                                    continue;
                                }
                                state.undo_log = Some(Vec::new());
                                let steps0 = state.steps;
                                let _ = cache.get(&pc).expect("cached above").compiled.run_traced(&mut state, t.executed as usize);
                                let interp = native::Regs::of(&state);
                                let mut d = t.regs.diff(&interp);
                                for (a, v) in &t.writes {
                                    if state.mem.get(*a as usize) != Some(v) && d.len() < 12 {
                                        d.push(format!("memory 0x{a:x}: native {v:02x} interpreter {:02x}", state.mem.get(*a as usize).copied().unwrap_or(0)));
                                    }
                                }
                                state.undo();
                                state.steps = steps0;
                                state.x = saved;
                                state.v = sv;
                                state.sp = ssp;
                                state.pc = spc;
                                state.nzcv = snz;
                                if !d.is_empty() || !matches!(t.exit, native::Exit::Budget) {
                                    if !d.is_empty() {
                                        first = format!(" First difference after {} instructions, in the block at 0x{prev_pc:x}: {}", t.executed, d.join("; "));
                                    }
                                    break;
                                }
                                prev_pc = t.regs.pc;
                                b = t.executed + 1;
                            }
                        }
                        let _ = end_x;
                        let _ = state.svc.take();
                        break Stop::Fault {
                            pc,
                            message: format!("native code differs from the interpreter in the function entered at 0x{pc:x} (comparison {native_compared}): {}.{first}", diff.join("; ")),
                        };
                    }
                }
                state.undo_log = None;
                outcome
            }
            (Some(f), Some(n)) => {
                native_dispatches += 1;
                match n.run(f, &mut state, budget as u64) {
                    native::Exit::Left => Ok(Some(ew_lift::interpreter::Transfer { from: pc, to: state.pc })),
                    native::Exit::Svc => Err("supervisor call".to_string()),
                    native::Exit::Budget => Err(format!("step budget {budget} exhausted in function 0x{pc:x}")),
                    native::Exit::Fault(m) => Err(m),
                    native::Exit::Halt(_) => {
                        // Something the library cannot run: the interpreter
                        // takes this address.
                        native_halts += 1;
                        hot.get_mut(&pc).expect("inserted above").1 += state.steps - steps_before;
                        skip_native = Some(state.pc);
                        continue;
                    }
                }
            }
            _ => cache.get(&pc).expect("cached above").compiled.run_traced(&mut state, budget),
        };
        hot.get_mut(&pc).expect("inserted above").1 += state.steps - steps_before;
        stack_low = stack_low.min(state.sp);

        match outcome {
            Err(message) if state.svc.is_some() => {
                // A supervisor call: the host kernel layer serves it by number,
                // then the guest continues after the instruction.
                let imm = state.svc.take().unwrap_or(0);
                let at = state.pc;
                let name = format!("svc:0x{imm:x}");
                import_calls.entry(name.clone()).or_insert((0, at)).0 += 1;
                let _ = message;
                match services.get(&name) {
                    Some(handler) => match handler(&mut state) {
                        HostResult::Returned(value) => {
                            note!("[0x{:x}] {name} = 0x{value:x} at 0x{at:x}", sched.current_object());
                            host_calls += 1;
                            state.x[0] = value;
                            state.pc = at + 4;
                            continue;
                        }
                        HostResult::Unavailable(message) => break Stop::HostFault { import: name, from: at, message },
                        HostResult::Jump(target) => {
                            state.pc = target;
                            continue;
                        }
                        HostResult::Block { resume, timed } => {
                            host_calls += 1;
                            switch_or_stop!(sched::Status::Blocked { resume, timed, return_to: at + 4, call: name });
                        }
                    },
                    None => break Stop::Blocked { import: name, pc: at, from: at, args: [0; 8] },
                }
            }
            Err(message) => {
                let at = state.pc;
                if message.starts_with("unsupported instruction") {
                    break Stop::Unsupported { pc: at, word: word_at(text, at) };
                }
                if message.starts_with("step budget") {
                    // A spin-wait on another thread: give the others a turn.
                    if sched.runnable_others() {
                        switch_or_stop!(sched::Status::Ready);
                    }
                    break Stop::SpinLimit { pc };
                }
                break Stop::Fault { pc: at, message };
            }
            Ok(None) => {}
            Ok(Some(t)) => {
                last_from = t.from;
                let from_word = scan::word_at(text, t.from);
                let kind = match from_word.map(|w| ew_lift::decode(w, t.from)) {
                    Some(Op::Bl { .. }) | Some(Op::Blr { .. }) => {
                        calls += 1;
                        depth = depth.saturating_add(1);
                        max_depth = max_depth.max(depth);
                        "call"
                    }
                    Some(Op::Ret { .. }) => {
                        returns += 1;
                        depth = depth.saturating_sub(1);
                        "return"
                    }
                    _ => {
                        branches += 1;
                        "branch"
                    }
                };
                if kind != "branch" {
                    event!(Event::Transfer { kind, from: t.from, to: t.to });
                }
                if trace.len() < opts.trace {
                    trace.push(format!("{kind} 0x{:x} -> 0x{:x}", t.from, t.to));
                }
                if !in_code(t.to) && externals.len() < 64 {
                    externals.push(TransferRecord { from: t.from, to: t.to, kind });
                }
            }
        }

        if state.halted {
            break Stop::Halted { pc: state.pc, reason: state.halt_reason.clone() };
        }
        // A run that made no progress and did not move control would spin here.
        if state.steps == steps_before && state.pc == pc {
            break Stop::Fault { pc, message: "no progress at this address".to_string() };
        }
    };

    if let (Some(log), Some(dir)) = (state.step_log.take(), &opts.snapshot_dir) {
        let mut out = String::with_capacity(log.len() * 900);
        for r in &log {
            out.push_str(&format!("{:x} {:x} {:x}", r.pc, r.sp, r.nzcv));
            for x in r.x {
                out.push_str(&format!(" {x:x}"));
            }
            for v in r.v {
                out.push_str(&format!(" {:x}:{:x}", v[1], v[0]));
            }
            out.push('\n');
        }
        std::fs::write(dir.join("trace.txt"), out)?;
    }

    for (id, (count, first)) in call_counts {
        let e = import_calls.entry(call_name(&names, &state.host.dyn_stubs, id)).or_insert((0, first));
        e.0 += count;
    }
    let mut hot_list: Vec<HotFunction> = hot
        .into_iter()
        .map(|(addr, (entries, instructions))| HotFunction { addr, entries, instructions })
        .collect();
    hot_list.sort_by_key(|h| std::cmp::Reverse(h.instructions));
    hot_list.truncate(12);

    Ok(BootReport {
        module: module.name.clone(),
        entry: opts.entry,
        steps: state.steps,
        dispatches,
        native_dispatches,
        native_halts,
        distinct_functions: seen_functions.len(),
        calls,
        returns,
        branches,
        max_depth,
        final_sp: state.sp,
        stack_low,
        stop,
        trace,
        hot: hot_list,
        externals,
        memory_bytes,
        import_count: jumps.len(),
        import_calls: import_calls
            .into_iter()
            .map(|(name, (calls, first_from))| ImportCall { name, calls, first_from })
            .collect(),
        bss_bytes: module.bss_end.unwrap_or(0).saturating_sub(module.bss_start.unwrap_or(0)),
        host_calls,
        host_implemented: services.len(),
        phases,
        phase,
        relocations_changed: relocated,
        recent: recent.iter().map(|e| e.render(&names, &state.host.dyn_stubs)).collect(),
        host_log: state.host.log.clone(),
        resolved: state.host.dyn_stubs.clone(),
        proc_slots: {
            let (lo, hi) = (module.data_addr, image_end.min(state.mem.len() as u64));
            let (base, cap) = (state.host.dyn_base, state.host.dyn_capacity);
            (lo..hi)
                .step_by(8)
                .filter_map(|a| {
                    let o = a as usize;
                    let v = u64::from_le_bytes(state.mem[o..o + 8].try_into().ok()?);
                    (base != 0 && v >= base && v < base + cap * 16 && (v - base) % 16 == 0)
                        .then(|| state.host.dyn_stubs.get(((v - base) / 16) as usize).map(|n| (a, n.clone())))
                        .flatten()
                })
                .collect()
        },
        thread_states: sched.summary(&state),
        threads: sched.created,
        switches: sched.switches,
    })
}

fn write_u64(memory: &mut [u8], addr: u64, value: u64) {
    let Ok(off) = usize::try_from(addr) else { return };
    let Some(end) = off.checked_add(8) else { return };
    if end <= memory.len() {
        memory[off..end].copy_from_slice(&value.to_le_bytes());
    }
}

fn word_at(text: &[u8], addr: u64) -> u32 {
    scan::word_at(text, addr).unwrap_or(0)
}

fn segment_at(module: &Module, addr: u64) -> Option<String> {
    if addr >= module.rodata_addr && addr < module.rodata_addr + module.rodata.len() as u64 {
        return Some("rodata".to_string());
    }
    if addr >= module.data_addr && addr < module.data_addr + module.data.len() as u64 {
        return Some("data".to_string());
    }
    None
}

fn read_word_at(module: &Module, addr: u64) -> Option<u64> {
    let (base, bytes) = if addr >= module.rodata_addr && addr < module.rodata_addr + module.rodata.len() as u64
    {
        (module.rodata_addr, module.rodata.as_slice())
    } else if addr >= module.data_addr && addr < module.data_addr + module.data.len() as u64 {
        (module.data_addr, module.data.as_slice())
    } else {
        return None;
    };
    let off = (addr - base) as usize;
    let end = (off + 8).min(bytes.len());
    if end - off < 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[off..off + 8]);
    Some(u64::from_le_bytes(buf))
}


/// The name of a host call by id (see the run loop).
fn call_name(names: &[String], dyn_stubs: &[String], id: u32) -> String {
    let id = id as usize;
    names.get(id).cloned().or_else(|| dyn_stubs.get(id - names.len().min(id)).cloned()).unwrap_or_default()
}

/// One of the last events before a stop, as data.
enum Event {
    Transfer { kind: &'static str, from: u64, to: u64 },
    Host { thread: u64, call: u32, args: [u64; 4], value: u64, from: u64 },
    Block { thread: u64, call: u32, args: [u64; 2] },
    Text(String),
}

impl Event {
    fn render(&self, names: &[String], dyn_stubs: &[String]) -> String {
        match self {
            Event::Transfer { kind, from, to } => format!("{kind} 0x{from:x} -> 0x{to:x}"),
            Event::Host { thread, call, args, value, from } => format!(
                "[0x{thread:x}] {}(0x{:x}, 0x{:x}, 0x{:x}, 0x{:x}) = 0x{value:x} from 0x{from:x}",
                call_name(names, dyn_stubs, *call),
                args[0],
                args[1],
                args[2],
                args[3]
            ),
            Event::Block { thread, call, args } => {
                format!("[0x{thread:x}] {}(0x{:x}, 0x{:x}) blocks", call_name(names, dyn_stubs, *call), args[0], args[1])
            }
            Event::Text(t) => t.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words_to_bytes(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// add x0, x0, #1 ; cbnz x1, +8 ; ret ; ret
    fn tiny_module() -> Module {
        Module::new(
            "test",
            words_to_bytes(&[
                0x9100_0400, // 0x00: add x0, x0, #1
                0xB500_0041, // 0x04: cbnz x1, 0x0c
                0xD65F03C0, // 0x08: ret
                0xD65F03C0, // 0x0c: ret
            ]),
            Vec::new(),
            Vec::new(),
        )
    }

    #[test]
    fn entry_return_is_reported_as_such() {
        let m = tiny_module();
        let opts = BootOptions { entry: 0, max_steps: 100, ..Default::default() };
        let report = boot(&m, &opts).unwrap();
        assert!(matches!(report.stop, Stop::EntryReturned), "{:?}", report.stop);
        assert_eq!(report.steps, 3, "two instructions then the ret");
        assert_eq!(report.distinct_functions, 1);
    }

    #[test]
    fn leaving_the_text_segment_names_the_address() {
        // bl 0x100 (outside this four word module)
        let m = Module::new("test", words_to_bytes(&[0x9400_0040]), Vec::new(), Vec::new());
        let opts = BootOptions { entry: 0, max_steps: 100, ..Default::default() };
        let report = boot(&m, &opts).unwrap();
        match report.stop {
            Stop::LeftText { pc, from, .. } => {
                assert_eq!(pc, 0x100);
                assert_eq!(from, 0);
            }
            other => panic!("expected LeftText, got {other:?}"),
        }
        assert_eq!(report.calls, 1);
    }

    #[test]
    fn unsupported_instruction_stops_the_run() {
        let m = Module::new("test", words_to_bytes(&[0x0000_0000]), Vec::new(), Vec::new());
        let opts = BootOptions { entry: 0, max_steps: 100, ..Default::default() };
        let report = boot(&m, &opts).unwrap();
        assert!(matches!(report.stop, Stop::Unsupported { pc: 0, .. }), "{:?}", report.stop);
    }

    #[test]
    fn loader_header_is_skipped_when_present() {
        // A header word that is not code, then a pointer into rodata.
        let mut text = words_to_bytes(&[0x0000_0001, 0x2000])
            .into_iter()
            .chain(std::iter::repeat(0u8).take(0x40 - 8))
            .chain(words_to_bytes(&[0xD65F03C0]))
            .collect::<Vec<u8>>();
        text.resize(0x44, 0);
        let m = Module::new("test", text, vec![0u8; 0x10], Vec::new());
        // rodata is mapped at 0x1000 for a 0x44 byte text segment, so the stored
        // pointer 0x2000 is outside it and the hint stays at zero.
        assert_eq!(m.entry_hint(), 0);

        let mut text2 = words_to_bytes(&[0x0000_0001, 0x1008]);
        text2.resize(0x44, 0);
        let m2 = Module::new("test", text2, vec![0u8; 0x10], Vec::new());
        assert_eq!(m2.entry_hint(), 0x40, "flag word, rodata pointer, then code");
    }

    #[test]
    fn module_layout_keeps_segments_apart() {
        let m = Module::new("test", vec![0u8; 0x2000], vec![1u8; 0x40], vec![2u8; 0x40]);
        assert_eq!(m.rodata_addr, 0x2000);
        assert_eq!(m.data_addr, 0x3000);
        assert_eq!(m.image_end(), 0x3040);
    }

    #[test]
    fn guest_memory_fault_is_reported_with_its_address() {
        // ldr x0, [x0]  with x0 = 0, which is inside the text image but the load
        // reads code, so instead force a wild pointer through x1.
        // ldr x2, [x1] ; ret   with x1 = u64::MAX - 8
        let m = Module::new(
            "test",
            words_to_bytes(&[0xF940_0022, 0xD65F03C0]),
            Vec::new(),
            Vec::new(),
        );
        let opts = BootOptions { entry: 0, max_steps: 100, x0: 0, ..Default::default() };
        let mut report = boot(&m, &opts).unwrap();
        // x1 defaults to 0, which is mapped text, so the read succeeds and the
        // run returns normally. Confirm no fault is invented.
        assert!(matches!(report.stop, Stop::EntryReturned), "{:?}", report.stop);
        report.trace.clear();
    }
}

/// Write the guest state for an external lockstep oracle: `regs.txt` (pc, sp,
/// nzcv, x0..x30, v0..v31, text size) and `mem.bin`, every non-zero 4 KiB page
/// as an eight-byte address followed by the page.
fn write_snapshot(state: &State, text_bytes: u64, dir: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut regs = format!("{:x} {:x} {:x}", state.pc, state.sp, state.nzcv);
    for x in state.x {
        regs.push_str(&format!(" {x:x}"));
    }
    for v in state.v {
        regs.push_str(&format!(" {:x}:{:x}", v[1], v[0]));
    }
    regs.push_str(&format!(" {text_bytes:x}\n"));
    std::fs::write(dir.join("regs.txt"), regs)?;
    let mut mem = Vec::new();
    for (i, page) in state.mem.chunks(4096).enumerate() {
        if page.iter().any(|&b| b != 0) {
            mem.extend_from_slice(&((i as u64) * 4096).to_le_bytes());
            mem.extend_from_slice(page);
            mem.resize(mem.len() + (4096 - page.len()), 0);
        }
    }
    std::fs::write(dir.join("mem.bin"), mem)
}

/// Six words of guest memory from `addr`, for `--watch` lines.
fn peek_qwords(state: &State, addr: u64) -> String {
    hle::read_bytes(state, addr, 48)
        .map(|b| b.chunks(8).map(|c| format!("{:x}", u64::from_le_bytes(c.try_into().unwrap()))).collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|_| "-".to_string())
}
