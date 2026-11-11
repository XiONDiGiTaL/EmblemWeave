//! Native execution: the game's code compiled ahead of time by `ew-native`
//! into `ew_native.dll`, run in place of the reference interpreter.
//!
//! The library's functions take the C `Cpu` of `ew_aarch64.h`; [`Cpu`] mirrors
//! it field for field. A call copies the guest registers in, points the
//! library at guest memory (guest address = offset into `State::mem`), runs
//! one function until control leaves it, a supervisor call, a halt or the end
//! of its instruction budget, and copies the registers back.

use ew_lift::interpreter::State;
use std::ffi::c_void;
use std::path::Path;

pub type NativeFn = unsafe extern "C" fn(*mut Cpu) -> i32;

/// `Cpu` from `ew_aarch64.h`, in the same order.
#[repr(C)]
pub struct Cpu {
    pub x: [u64; 31],
    pub v: [[u64; 2]; 32],
    pub sp: u64,
    pub pc: u64,
    pub nzcv: u32,
    pub halted: i32,
    pub halt_reason: [u8; 128],
    pub tpidrro: u64,
    pub tpidr: u64,
    pub fpcr: u64,
    pub fpsr: u64,
    pub counter: Option<unsafe extern "C" fn() -> u64>,
    pub svc: i32,
    pub excl_addr: u64,
    pub excl_valid: i32,
    pub mem_read: unsafe extern "C" fn(*mut c_void, u64, u32) -> u64,
    pub mem_write: unsafe extern "C" fn(*mut c_void, u64, u64, u32),
    pub mem_ctx: *mut c_void,
    pub mem_base: *mut u8,
    pub mem_size: u64,
    pub budget: i64,
}

/// An access outside guest memory: the library calls back here, and the run
/// stops with the same message the interpreter gives.
struct Fault(Option<String>);

unsafe extern "C" fn mem_read(ctx: *mut c_void, addr: u64, bytes: u32) -> u64 {
    let f = unsafe { &mut *(ctx as *mut Fault) };
    f.0.get_or_insert_with(|| format!("guest read of {bytes} byte(s) at 0x{addr:x} is outside guest memory"));
    0
}

unsafe extern "C" fn mem_write(ctx: *mut c_void, addr: u64, _v: u64, bytes: u32) {
    let f = unsafe { &mut *(ctx as *mut Fault) };
    f.0.get_or_insert_with(|| format!("guest write of {bytes} byte(s) at 0x{addr:x} is outside guest memory"));
}

thread_local! {
    static CLOCK: std::cell::Cell<(Option<fn() -> u64>, u64)> = const { std::cell::Cell::new((None, 0)) };
}

/// The system counter, as the interpreter reads it: the host clock, or the
/// instruction count when there is none.
unsafe extern "C" fn counter() -> u64 {
    CLOCK.with(|c| {
        let (f, steps) = c.get();
        f.map(|f| f()).unwrap_or(steps)
    })
}

/// How a native call ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    /// Control left the function; `state.pc` is where it went.
    Left,
    /// A supervisor call: `state.svc` is set and `state.pc` is the `svc`.
    Svc,
    /// The instruction budget ran out; `state.pc` is a block to resume at.
    Budget,
    /// The library stopped at something it cannot run (`state.pc`); the
    /// interpreter takes over there.
    Halt(String),
    /// An access outside guest memory.
    Fault(String),
}

pub struct Native {
    _lib: Library,
    blocks: crate::FastMap<u64, NativeFn>,
    pub functions: usize,
    /// Run native functions back to back (`EW_NATIVE_CHAIN=0` turns it off,
    /// so the run loop sees every function entry).
    pub chain: bool,
}

impl std::fmt::Debug for Native {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Native {{ {} functions, {} blocks }}", self.functions, self.blocks.len())
    }
}

struct Library(*mut c_void);
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

#[cfg(windows)]
impl Library {
    fn open(path: &Path) -> Result<Library, String> {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let h = unsafe { windows_sys::Win32::System::LibraryLoader::LoadLibraryW(wide.as_ptr()) };
        if h.is_null() {
            return Err(format!("could not load {}: {}", path.display(), std::io::Error::last_os_error()));
        }
        Ok(Library(h as *mut c_void))
    }

    fn symbol(&self, name: &str) -> Option<*const c_void> {
        let c = std::ffi::CString::new(name).ok()?;
        let p = unsafe { windows_sys::Win32::System::LibraryLoader::GetProcAddress(self.0 as _, c.as_ptr() as _) }?;
        Some(p as *const c_void)
    }
}

#[repr(C)]
struct Entry {
    address: u64,
    f: Option<NativeFn>,
}

impl Native {
    #[cfg(windows)]
    pub fn load(path: &Path) -> Result<Native, String> {
        let lib = Library::open(path)?;
        let shards: unsafe extern "C" fn() -> u32 =
            unsafe { std::mem::transmute(lib.symbol("ew_native_shards").ok_or("ew_native_shards is missing: not an ew-native library")?) };
        let shard: unsafe extern "C" fn(u32, *mut u32) -> *const Entry =
            unsafe { std::mem::transmute(lib.symbol("ew_native_shard").ok_or("ew_native_shard is missing")?) };
        let mut blocks = crate::FastMap::default();
        let mut functions = std::collections::HashSet::new();
        for i in 0..unsafe { shards() } {
            let mut n = 0u32;
            let t = unsafe { shard(i, &mut n) };
            if t.is_null() {
                continue;
            }
            for e in unsafe { std::slice::from_raw_parts(t, n as usize) } {
                if let Some(f) = e.f {
                    blocks.insert(e.address, f);
                    functions.insert(f as usize);
                }
            }
        }
        let chain = std::env::var("EW_NATIVE_CHAIN").map_or(true, |v| v != "0");
        Ok(Native { _lib: lib, blocks, functions: functions.len(), chain })
    }

    pub fn blocks(&self) -> usize {
        self.blocks.len()
    }

    pub fn get(&self, pc: u64) -> Option<NativeFn> {
        self.blocks.get(&pc).copied()
    }

    /// Run `f` from `state.pc` for up to `budget` instructions.
    pub fn run(&self, f: NativeFn, state: &mut State, budget: u64) -> Exit {
        let mut fault = Fault(None);
        let mut cpu = Cpu {
            x: state.x,
            v: state.v,
            sp: state.sp,
            pc: state.pc,
            nzcv: state.nzcv,
            halted: 0,
            halt_reason: [0; 128],
            tpidrro: state.tpidrro,
            tpidr: state.tpidr,
            fpcr: state.fpcr,
            fpsr: state.fpsr,
            counter: Some(counter),
            svc: -1,
            excl_addr: state.exclusive.unwrap_or(0),
            excl_valid: state.exclusive.is_some() as i32,
            mem_read,
            mem_write,
            mem_ctx: &mut fault as *mut Fault as *mut c_void,
            mem_base: state.mem.as_mut_ptr(),
            mem_size: state.mem.len() as u64,
            budget: budget.min(i64::MAX as u64) as i64,
        };
        CLOCK.with(|c| c.set((state.host.clock, state.steps)));
        // Chain: when control leaves for another native function, enter it
        // here rather than going back to the run loop, which only needs to
        // see supervisor calls, host functions, halts and the end of the
        // budget.
        let mut f = f;
        let mut more;
        loop {
            more = unsafe { f(&mut cpu) };
            if more == 0 || cpu.halted != 0 || cpu.svc != -1 || fault.0.is_some() || !self.chain {
                break;
            }
            match self.blocks.get(&cpu.pc) {
                Some(&g) => f = g,
                None => break,
            }
        }
        state.x = cpu.x;
        state.v = cpu.v;
        state.sp = cpu.sp;
        state.pc = cpu.pc;
        state.nzcv = cpu.nzcv;
        state.tpidr = cpu.tpidr;
        state.fpcr = cpu.fpcr;
        state.fpsr = cpu.fpsr;
        state.exclusive = (cpu.excl_valid != 0).then_some(cpu.excl_addr);
        state.steps += budget.saturating_sub(cpu.budget.max(0) as u64);
        if let Some(m) = fault.0 {
            return Exit::Fault(m);
        }
        if cpu.halted != 0 {
            let n = cpu.halt_reason.iter().position(|&b| b == 0).unwrap_or(128);
            return Exit::Halt(String::from_utf8_lossy(&cpu.halt_reason[..n]).into_owned());
        }
        if more == 0 && cpu.svc >= 0 {
            state.svc = Some(cpu.svc as u16);
            state.pc = cpu.pc.wrapping_sub(4);
            return Exit::Svc;
        }
        if cpu.svc == -2 {
            return Exit::Budget;
        }
        Exit::Left
    }
}

// ------------------------------------------------------------- check mode

/// The registers a native call and the interpreter are compared on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Regs {
    pub x: [u64; 31],
    pub v: [[u64; 2]; 32],
    pub sp: u64,
    pub pc: u64,
    pub nzcv: u32,
}

impl Regs {
    pub fn of(s: &State) -> Regs {
        Regs { x: s.x, v: s.v, sp: s.sp, pc: s.pc, nzcv: s.nzcv }
    }

    /// The first differences from `other`, as text.
    pub fn diff(&self, other: &Regs) -> Vec<String> {
        let mut d = Vec::new();
        for i in 0..31 {
            if self.x[i] != other.x[i] {
                d.push(format!("x{i} native 0x{:x} interpreter 0x{:x}", self.x[i], other.x[i]));
            }
        }
        for i in 0..32 {
            if self.v[i] != other.v[i] {
                d.push(format!("v{i} native {:x}:{:x} interpreter {:x}:{:x}", self.v[i][1], self.v[i][0], other.v[i][1], other.v[i][0]));
            }
        }
        if self.sp != other.sp {
            d.push(format!("sp native 0x{:x} interpreter 0x{:x}", self.sp, other.sp));
        }
        if self.pc != other.pc {
            d.push(format!("pc native 0x{:x} interpreter 0x{:x}", self.pc, other.pc));
        }
        if self.nzcv != other.nzcv {
            d.push(format!("nzcv native {:x} interpreter {:x}", self.nzcv, other.nzcv));
        }
        d
    }
}

/// Guest memory seen through an overlay: reads come from the run's memory
/// unless the native call wrote the byte, writes go to the overlay only.
struct Overlay {
    mem: *const Vec<u8>,
    writes: std::collections::BTreeMap<u64, u8>,
    fault: Option<String>,
}

unsafe extern "C" fn overlay_read(ctx: *mut c_void, addr: u64, bytes: u32) -> u64 {
    let o = unsafe { &mut *(ctx as *mut Overlay) };
    let mem = unsafe { &*o.mem };
    let mut v = 0u64;
    for i in 0..bytes as u64 {
        let a = addr.wrapping_add(i);
        let b = match o.writes.get(&a) {
            Some(b) => *b,
            None => match mem.get(a as usize) {
                Some(b) => *b,
                None => {
                    o.fault.get_or_insert_with(|| format!("guest read of {bytes} byte(s) at 0x{addr:x} is outside guest memory"));
                    0
                }
            },
        };
        v |= (b as u64) << (8 * i);
    }
    v
}

unsafe extern "C" fn overlay_write(ctx: *mut c_void, addr: u64, value: u64, bytes: u32) {
    let o = unsafe { &mut *(ctx as *mut Overlay) };
    for i in 0..bytes as u64 {
        o.writes.insert(addr.wrapping_add(i), (value >> (8 * i)) as u8);
    }
}

/// What a native call would do, without changing the run: its exit, final
/// registers and the bytes it wrote. `state` is left as it was.
pub struct Trial {
    pub exit: Exit,
    pub regs: Regs,
    pub writes: std::collections::BTreeMap<u64, u8>,
    /// Instructions the call ran.
    pub executed: u64,
}

impl Native {
    pub fn trial(&self, f: NativeFn, state: &mut State, budget: u64) -> Trial {
        let before = Regs::of(state);
        let (excl, tpidr, fpcr, fpsr, steps) = (state.exclusive, state.tpidr, state.fpcr, state.fpsr, state.steps);
        let mut o = Overlay { mem: &state.mem, writes: Default::default(), fault: None };
        let mut cpu = Cpu {
            x: state.x,
            v: state.v,
            sp: state.sp,
            pc: state.pc,
            nzcv: state.nzcv,
            halted: 0,
            halt_reason: [0; 128],
            tpidrro: state.tpidrro,
            tpidr: state.tpidr,
            fpcr: state.fpcr,
            fpsr: state.fpsr,
            counter: Some(counter),
            svc: -1,
            excl_addr: state.exclusive.unwrap_or(0),
            excl_valid: state.exclusive.is_some() as i32,
            mem_read: overlay_read,
            mem_write: overlay_write,
            mem_ctx: &mut o as *mut Overlay as *mut c_void,
            mem_base: std::ptr::null_mut(),
            mem_size: 0,
            budget: budget.min(i64::MAX as u64) as i64,
        };
        CLOCK.with(|c| c.set((state.host.clock, state.steps)));
        let more = unsafe { f(&mut cpu) };
        let mut regs = Regs { x: cpu.x, v: cpu.v, sp: cpu.sp, pc: cpu.pc, nzcv: cpu.nzcv };
        let exit = if let Some(m) = o.fault.take() {
            Exit::Fault(m)
        } else if cpu.halted != 0 {
            Exit::Halt(String::new())
        } else if more == 0 && cpu.svc >= 0 {
            regs.pc = cpu.pc.wrapping_sub(4);
            Exit::Svc
        } else if cpu.svc == -2 {
            Exit::Budget
        } else {
            Exit::Left
        };
        // Leave the run exactly as it was.
        state.x = before.x;
        state.v = before.v;
        state.sp = before.sp;
        state.pc = before.pc;
        state.nzcv = before.nzcv;
        state.exclusive = excl;
        state.tpidr = tpidr;
        state.fpcr = fpcr;
        state.fpsr = fpsr;
        state.steps = steps;
        let executed = budget.saturating_sub(cpu.budget.max(0) as u64);
        Trial { exit, regs, writes: o.writes, executed }
    }
}
