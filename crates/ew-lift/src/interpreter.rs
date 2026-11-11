//! A deliberately simple AArch64 interpreter, used as the *reference* for
//! differential testing of the C emitter.
//!
//! It executes the same `Op` set the lifter supports and shares nothing with the
//! emitter, so agreement between the two is meaningful evidence rather than a
//! tautology. It is not the product runtime: lifted C is.

use crate::aarch64::sysreg;
use crate::aarch64::{
    Cond, DataOp2, Extend, FpBinop, FpRound, FpUnop, LogicalOp, Op, Shift,
};

/// Guest memory used by the reference interpreter. Flat, little-endian and
/// zero-filled, matching the C harness byte for byte so a differential run
/// exercises real loads and stores.
pub const GUEST_MEMORY_SIZE: usize = 2 * 1024 * 1024;

/// Host-side bookkeeping that travels with a run.
///
/// This is not architectural state, but every host call needs it: where the
/// single guest thread object lives, and how much of the guest address space the
/// page allocator may still hand out. Keeping it here means a host handler stays
/// a plain `fn(&mut State)` with no global state, so two runs in one process
/// cannot interfere.
/// A guest thread the host knows about, keyed by its `nn::os::ThreadType`.
#[derive(Debug, Clone, Default)]
pub struct ThreadInfo {
    pub entry: u64,
    pub arg: u64,
    pub stack_top: u64,
    pub priority: i32,
    pub started: bool,
    pub exited: bool,
    /// Guest pointer to the thread's name, when the game set one.
    pub name: u64,
}

#[derive(Debug, Clone, Default)]
pub struct HostArena {
    /// Next free address the page allocator may return.
    pub alloc_next: u64,
    /// One past the last address the page allocator may return.
    pub alloc_end: u64,
    /// Address of the pseudo thread object standing for the guest's only thread.
    pub thread_object: u64,
    /// Pages handed out, for the run report.
    pub pages_allocated: u64,
    /// Bytes of the loaded program image, for `QueryMemoryInfo`.
    pub program_size: u64,
    /// Heap size the guest set with `SetMemoryHeapSize`.
    pub heap_size: u64,
    /// Bytes of guest thread stacks.
    pub stack_size: u64,
    /// Addresses of host-built guest objects and of module exports host
    /// services need, by name.
    pub named: std::collections::BTreeMap<String, u64>,
    /// Every guest thread by `ThreadType` address. `thread_object` names the
    /// thread currently running.
    pub threads: std::collections::BTreeMap<u64, ThreadInfo>,
    /// System settings the launcher supplies (language, display version, ...).
    pub settings: std::collections::BTreeMap<String, String>,
    /// Host-level events worth showing a user: files opened, mounts, aborts.
    pub log: Vec<String>,
    /// Host functions created at run time (for example NVN procedures handed
    /// out by `nvnDeviceGetProcAddress`), by index into the dynamic stub area.
    pub dyn_stubs: Vec<String>,
    pub dyn_base: u64,
    pub dyn_capacity: u64,
    /// Filesystem state for the `nn::fs` host layer: mount name to host root
    /// and writability, then open files and directories by handle.
    pub mounts: std::collections::BTreeMap<String, (std::path::PathBuf, bool)>,
    pub files: std::collections::BTreeMap<u64, (std::path::PathBuf, u32)>,
    pub dirs: std::collections::BTreeMap<u64, (std::path::PathBuf, u32, u64)>,
    pub next_handle: u64,
    /// Host directories the launcher configured: the user's romfs (read-only)
    /// and the save directory (writable, separate from the game files).
    pub romfs_root: Option<std::path::PathBuf>,
    pub save_root: Option<std::path::PathBuf>,
    /// Thread-local values belonging to the guest's only thread, plus the mask
    /// of slots handed out.
    pub tls: [u64; 16],
    pub tls_used: u16,
    /// Generator state for the entropy services, so a stream cannot repeat.
    pub rng: u64,
    /// Typed state owned by host layers outside this crate (for example the
    /// NVN object tables), one value per type.
    pub ext: Extensions,
    /// The system counter (`cntpct_el0`), in ticks of `counter_hz`. Without
    /// one the counter advances one tick per instruction.
    pub clock: Option<fn() -> u64>,
    /// Incremented whenever a guest thread is started, so a scheduler can
    /// notice new threads without scanning them all.
    pub thread_starts: u64,
}

/// Per-run storage for host layers that live in other crates. Cloning a
/// `HostArena` shares it, which is what a snapshot of a running guest wants.
#[derive(Clone, Default)]
pub struct Extensions(
    pub std::sync::Arc<std::sync::Mutex<std::collections::HashMap<std::any::TypeId, Box<dyn std::any::Any + Send>>>>,
);

impl std::fmt::Debug for Extensions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Extensions(..)")
    }
}

impl Extensions {
    /// Run `f` on this run's value of type `T`, created on first use.
    pub fn with<T: Default + Send + 'static, R>(&self, f: impl FnOnce(&mut T) -> R) -> R {
        let mut map = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let slot = map.entry(std::any::TypeId::of::<T>()).or_insert_with(|| Box::new(T::default()));
        f(slot.downcast_mut::<T>().expect("extension slot holds its own type"))
    }
}

impl HostArena {
    /// Reserve `size` bytes of guest memory at `align`, returning the address.
    /// Reports why it cannot, never returns a pointer it did not own.
    pub fn allocate(&mut self, size: u64, align: u64) -> Result<u64, String> {
        if self.alloc_next == 0 || self.alloc_end <= self.alloc_next {
            return Err("no guest page arena was reserved for this run".to_string());
        }
        let align = align.max(16).next_power_of_two();
        let start = (self.alloc_next + align - 1) & !(align - 1);
        let end = start.saturating_add(size.max(1));
        if end > self.alloc_end {
            return Err(format!(
                "page arena is exhausted: 0x{size:x} bytes requested, 0x{:x} left",
                self.alloc_end - self.alloc_next
            ));
        }
        self.alloc_next = end;
        self.pages_allocated += size.max(1).div_ceil(0x1000);
        Ok(start)
    }

    /// Reserve the fixed page a pseudo thread object lives in, once per run.
    pub fn take_thread_object(&mut self) -> Result<u64, String> {
        if self.thread_object != 0 {
            return Ok(self.thread_object);
        }
        // Room for a whole nnSdk ThreadType: library code that runs as the
        // title's own code keeps its per-thread mutex inside it.
        let at = self.allocate(0x400, 0x100)?;
        self.thread_object = at;
        Ok(at)
    }
}

/// One executed instruction and the register file it left behind, recorded
/// for lockstep comparison against an independent CPU model.
#[derive(Debug, Clone)]
pub struct StepRecord {
    pub pc: u64,
    pub x: [u64; 31],
    pub sp: u64,
    pub nzcv: u32,
    pub v: [[u64; 2]; 32],
}

#[derive(Debug, Clone)]
pub struct State {
    /// x0..x30; index 31 is not stored (it is XZR or SP by instruction context).
    pub x: [u64; 31],
    /// V0..V31, low eight bytes first. Only transfers and the bit copy forms
    /// touch these until the floating point and vector units are modelled.
    pub v: [[u64; 2]; 32],
    pub sp: u64,
    pub pc: u64,
    pub nzcv: u32,
    pub halted: bool,
    /// The exclusive monitor: the address an `ldxr` reserved, if any.
    pub exclusive: Option<u64>,
    /// When set, every executed instruction is appended here until the
    /// vector holds `step_log_limit` records.
    pub step_log: Option<Vec<StepRecord>>,
    pub step_log_limit: usize,
    pub halt_reason: String,
    pub steps: u64,
    /// The thread pointers: `tpidrro_el0` (read-only to the guest, set by the
    /// kernel per thread) and `tpidr_el0`.
    pub tpidrro: u64,
    pub tpidr: u64,
    /// Floating-point control and status registers, kept as written.
    pub fpcr: u64,
    pub fpsr: u64,
    /// A supervisor call the guest just made, for the host kernel layer.
    pub svc: Option<u16>,
    pub mem: Vec<u8>,
    /// When set, every store first records the bytes it overwrites, so a run
    /// can be taken back (`undo`). Used to compare native code with this
    /// interpreter.
    pub undo_log: Option<Vec<(u64, u8)>>,
    /// Host-side allocation and thread bookkeeping for the current run.
    pub host: HostArena,
}

impl Default for State {
    fn default() -> Self {
        State {
            x: [0; 31],
            v: [[0; 2]; 32],
            sp: 0,
            pc: 0,
            nzcv: 0,
            halted: false,
            exclusive: None,
            step_log: None,
            step_log_limit: 0,
            halt_reason: String::new(),
            tpidrro: 0,
            tpidr: 0,
            fpcr: 0,
            fpsr: 0,
            svc: None,
            steps: 0,
            mem: vec![0; GUEST_MEMORY_SIZE],
            undo_log: None,
            host: HostArena::default(),
        }
    }
}

impl State {
    /// Deterministic register pattern shared with the generated C harness.
    pub fn patterned() -> State {
        let mut s = State::default();
        for (i, r) in s.x.iter_mut().enumerate() {
            *r = 0x1000 + (i as u64) * 0x20;
        }
        for (i, r) in s.v.iter_mut().enumerate() {
            r[0] = 0x2000 + (i as u64) * 0x10;
            r[1] = 0x3000 + (i as u64) * 0x10;
        }
        s.sp = 0x100000;
        s
    }

    /// Take back every store recorded since `undo_log` was set, newest first.
    pub fn undo(&mut self) {
        if let Some(log) = self.undo_log.take() {
            for (a, b) in log.into_iter().rev() {
                self.mem[a as usize] = b;
            }
        }
    }

    pub(crate) fn load(&self, addr: u64, bytes: u8) -> Result<u64, String> {
        if bytes > 8 {
            return Err(format!("unsupported access width {bytes} bytes"));
        }
        let off = addr as usize;
        if off.saturating_add(bytes as usize) > self.mem.len() {
            return Err(format!(
                "guest read of {bytes} byte(s) at 0x{addr:x} is outside guest memory"
            ));
        }
        let mut v = 0u64;
        for i in 0..bytes as usize {
            v |= (self.mem[off + i] as u64) << (8 * i);
        }
        Ok(v)
    }

    pub(crate) fn store(&mut self, addr: u64, value: u64, bytes: u8) -> Result<(), String> {
        if bytes > 8 {
            return Err(format!("unsupported access width {bytes} bytes"));
        }
        let off = addr as usize;
        if off.saturating_add(bytes as usize) > self.mem.len() {
            return Err(format!(
                "guest write of {bytes} byte(s) at 0x{addr:x} is outside guest memory"
            ));
        }
        if let Some(log) = self.undo_log.as_mut() {
            for i in 0..bytes as usize {
                log.push((addr + i as u64, self.mem[off + i]));
            }
        }
        for i in 0..bytes as usize {
            self.mem[off + i] = ((value >> (8 * i)) & 0xFF) as u8;
        }
        Ok(())
    }

    fn read(&self, r: u8, sp_ok: bool) -> u64 {
        if r == 31 {
            if sp_ok {
                self.sp
            } else {
                0
            }
        } else {
            self.x[r as usize]
        }
    }

    fn write(&mut self, r: u8, sp_ok: bool, v: u64) {
        if r == 31 {
            if sp_ok {
                self.sp = v;
            }
        } else {
            self.x[r as usize] = v;
        }
    }

    /// Low bytes of a vector register. Writing a scalar width zeroes the rest of
    /// the register, which is what every AArch64 SIMD/FP write does.
    fn vread(&self, r: u8, bytes: u8) -> u64 {
        let lo = self.v[(r & 31) as usize][0];
        if bytes >= 8 {
            lo
        } else {
            lo & ((1u64 << (bytes * 8)) - 1)
        }
    }

    fn vwrite(&mut self, r: u8, bytes: u8, value: u64) {
        let idx = (r & 31) as usize;
        self.v[idx][0] = if bytes >= 8 { value } else { value & ((1u64 << (bytes * 8)) - 1) };
        self.v[idx][1] = 0;
    }

    fn vwrite128(&mut self, r: u8, lo: u64, hi: u64) {
        let idx = (r & 31) as usize;
        self.v[idx][0] = lo;
        self.v[idx][1] = hi;
    }

    /// Flags of the logical instructions (`ands`, `bics`, `tst`): N and Z from
    /// the result, C and V cleared, as the architecture specifies.
    fn set_nz(&mut self, v: u64, is64: bool) {
        let n = if is64 { v >> 63 } else { (v >> 31) & 1 };
        let z = if v == 0 { 1 } else { 0 };
        self.nzcv = ((n as u32) << 3) | ((z as u32) << 2);
    }

    fn set_add_flags(&mut self, a: u64, b: u64, r: u64, is64: bool) {
        if is64 {
            let carry = (r < a) as u32;
            let ov = (((a ^ r) & (b ^ r)) >> 63) as u32;
            let n = (r >> 63) as u32;
            let z = (r == 0) as u32;
            self.nzcv = n << 3 | z << 2 | carry << 1 | ov;
        } else {
            let (a32, b32, r32) = (a as u32, b as u32, r as u32);
            let carry = (r32 < a32) as u32;
            let ov = (((a32 ^ r32) & (b32 ^ r32)) >> 31) as u32;
            let n = (r32 >> 31) as u32;
            let z = (r32 == 0) as u32;
            self.nzcv = n << 3 | z << 2 | carry << 1 | ov;
        }
    }

    fn set_sub_flags(&mut self, a: u64, b: u64, r: u64, is64: bool) {
        if is64 {
            let carry = (a >= b) as u32;
            let ov = (((a ^ b) & (a ^ r)) >> 63) as u32;
            let n = (r >> 63) as u32;
            let z = (r == 0) as u32;
            self.nzcv = n << 3 | z << 2 | carry << 1 | ov;
        } else {
            let (a32, b32, r32) = (a as u32, b as u32, r as u32);
            let carry = (a32 >= b32) as u32;
            let ov = (((a32 ^ b32) & (a32 ^ r32)) >> 31) as u32;
            let n = (r32 >> 31) as u32;
            let z = (r32 == 0) as u32;
            self.nzcv = n << 3 | z << 2 | carry << 1 | ov;
        }
    }

    /// Read a single precision float from the low four bytes of a vector
    /// register. The bit pattern is reinterpreted, never converted.
    fn sread(&self, r: u8) -> f32 {
        f32::from_bits(self.vread(r, 4) as u32)
    }

    fn swrite(&mut self, r: u8, v: f32) {
        self.vwrite(r, 4, v.to_bits() as u64);
    }

    fn dread(&self, r: u8) -> f64 {
        f64::from_bits(self.vread(r, 8))
    }

    fn dwrite(&mut self, r: u8, v: f64) {
        self.vwrite(r, 8, v.to_bits());
    }

    /// FP compare writes NZCV with the architectural encoding: unordered sets
    /// C and V, equal sets Z and C, less than sets N and greater sets C.
    fn set_fp_flags(&mut self, a: f64, b: f64) {
        self.nzcv = if a.is_nan() || b.is_nan() {
            0b0011
        } else if a == b {
            0b0110
        } else if a < b {
            0b1000
        } else {
            0b0010
        };
    }

    fn cond_holds(&self, cond: Cond) -> bool {
        let n = (self.nzcv >> 3) & 1;
        let z = (self.nzcv >> 2) & 1;
        let c = (self.nzcv >> 1) & 1;
        let v = self.nzcv & 1;
        match cond {
            Cond::Eq => z == 1,
            Cond::Ne => z == 0,
            Cond::Cs => c == 1,
            Cond::Cc => c == 0,
            Cond::Mi => n == 1,
            Cond::Pl => n == 0,
            Cond::Vs => v == 1,
            Cond::Vc => v == 0,
            Cond::Hi => c == 1 && z == 0,
            Cond::Ls => c == 0 || z == 1,
            Cond::Ge => n == v,
            Cond::Lt => n != v,
            Cond::Gt => z == 0 && n == v,
            Cond::Le => z == 1 || n != v,
            Cond::Al | Cond::Nv => true,
        }
    }

    /// Execute one decoded instruction. Branches update `pc`; the caller decides
    /// whether to keep going.
    pub fn step(&mut self, op: &Op) -> Result<(), String> {
        let here = self.pc;
        let r = self.step_inner(op);
        if r.is_ok() {
            if let Some(log) = self.step_log.as_mut() {
                if log.len() < self.step_log_limit {
                    log.push(StepRecord { pc: here, x: self.x, sp: self.sp, nzcv: self.nzcv, v: self.v });
                }
            }
        }
        r
    }

    /// Value of a readable system register.
    pub fn read_sysreg(&self, reg: u16) -> Result<u64, String> {
        Ok(match reg {
            sysreg::TPIDRRO_EL0 => self.tpidrro,
            sysreg::TPIDR_EL0 => self.tpidr,
            sysreg::CNTPCT_EL0 | sysreg::CNTVCT_EL0 => match self.host.clock {
                Some(f) => f(),
                None => self.steps,
            },
            // The console's system counter runs at 19.2 MHz.
            sysreg::CNTFRQ_EL0 => 19_200_000,
            sysreg::FPCR => self.fpcr,
            sysreg::FPSR => self.fpsr,
            sysreg::NZCV => (self.nzcv as u64) << 28,
            // 64-byte cache lines for both caches (Cortex-A78C), the value
            // cache-maintenance loops step by.
            sysreg::CTR_EL0 => 0x8444_C004,
            // `dc zva` zeroes 64 bytes.
            sysreg::DCZID_EL0 => 4,
            _ => return Err(format!("mrs from system register 0x{reg:x} is not modelled")),
        })
    }

    fn step_inner(&mut self, op: &Op) -> Result<(), String> {
        self.steps += 1;
        let mask = |is64: bool, v: u64| if is64 { v } else { v & 0xFFFF_FFFF };
        match *op {
            Op::Nop => {}
            Op::B { target } => self.pc = target,
            Op::Bl { target } => {
                self.x[30] = self.pc.wrapping_add(4);
                self.pc = target;
            }
            Op::BCond { cond, target } => {
                if self.cond_holds(cond) {
                    self.pc = target;
                }
            }
            Op::CompareBranchZero { rt, target, nonzero, is64 } => {
                let v = mask(is64, self.read(rt, false));
                if (v != 0) == nonzero {
                    self.pc = target;
                }
            }
            Op::TestBitBranch { rt, bit, target, nonzero } => {
                let set = (self.read(rt, false) >> bit) & 1 == 1;
                if set == nonzero {
                    self.pc = target;
                }
            }
            Op::Br { rn } => self.pc = self.read(rn, false),
            Op::Blr { rn } => {
                let t = self.read(rn, false);
                self.x[30] = self.pc.wrapping_add(4);
                self.pc = t;
            }
            Op::Ret { rn } => self.pc = self.read(rn, false),
            Op::Adr { rd, target } => self.write(rd, false, target),
            Op::MoveWide { rd, imm, shift, kind, is64 } => {
                let v = match kind {
                    0 => (imm as u64) << shift,
                    1 => !((imm as u64) << shift),
                    _ => {
                        let cur = mask(is64, self.read(rd, false));
                        let m = !(0xFFFFu64 << shift);
                        (cur & m) | ((imm as u64) << shift)
                    }
                };
                self.write(rd, false, mask(is64, v));
            }
            Op::AddSubImm { rd, rn, imm, sub, set_flags, is64 } => {
                let a = mask(is64, self.read(rn, !set_flags));
                let b = imm as u64;
                let r = if sub { a.wrapping_sub(b) } else { a.wrapping_add(b) };
                let r = mask(is64, r);
                if set_flags {
                    if sub {
                        self.set_sub_flags(a, b, r, is64);
                    } else {
                        self.set_add_flags(a, b, r, is64);
                    }
                }
                self.write(rd, !set_flags, r);
            }
            Op::AddSubReg { rd, rn, rm, shift, amount, sub, set_flags, is64 } => {
                let a = mask(is64, self.read(rn, false));
                let b0 = mask(is64, self.read(rm, false));
                let b = apply_shift(b0, shift, amount, is64);
                let r = if sub { a.wrapping_sub(b) } else { a.wrapping_add(b) };
                let r = mask(is64, r);
                if set_flags {
                    if sub {
                        self.set_sub_flags(a, b, r, is64);
                    } else {
                        self.set_add_flags(a, b, r, is64);
                    }
                }
                self.write(rd, false, r);
            }
            Op::LogicalImm { rd, rn, imm, op, set_flags, is64 } => {
                let a = mask(is64, self.read(rn, false));
                let r = match op {
                    LogicalOp::And => a & imm,
                    LogicalOp::Orr => a | imm,
                    LogicalOp::Eor => a ^ imm,
                    LogicalOp::Ands => a & imm,
                };
                let r = mask(is64, r);
                if op == LogicalOp::Ands || set_flags {
                    self.set_nz(r, is64);
                }
                self.write(rd, false, r);
            }
            Op::LogicalReg { rd, rn, rm, shift, amount, op, set_flags, is64, invert } => {
                let a = mask(is64, self.read(rn, false));
                let b = apply_shift(mask(is64, self.read(rm, false)), shift, amount, is64);
                let b = if invert { mask(is64, !b) } else { b };
                let r = match op {
                    LogicalOp::And => a & b,
                    LogicalOp::Orr => a | b,
                    LogicalOp::Eor => a ^ b,
                    LogicalOp::Ands => a & b,
                };
                let r = mask(is64, r);
                if set_flags || op == LogicalOp::Ands {
                    self.set_nz(r, is64);
                }
                self.write(rd, false, r);
            }
            Op::MultiplyAdd { rd, rn, rm, ra, sub, is64 } => {
                let a = mask(is64, self.read(rn, false));
                let b = mask(is64, self.read(rm, false));
                let acc = mask(is64, self.read(ra, false));
                let r = if sub { acc.wrapping_sub(a.wrapping_mul(b)) } else { acc.wrapping_add(a.wrapping_mul(b)) };
                self.write(rd, false, mask(is64, r));
            }
            Op::CondSelect { rd, rn, rm, cond, kind, is64 } => {
                let a = mask(is64, self.read(rn, false));
                let b = mask(is64, self.read(rm, false));
                let r = if self.cond_holds(cond) {
                    a
                } else {
                    match kind {
                        0 => b,
                        1 => b.wrapping_add(1),
                        2 => !b,
                        _ => b.wrapping_neg(),
                    }
                };
                self.write(rd, false, mask(is64, r));
            }
            Op::AddSubExt { rd, rn, rm, option, shift, sub, set_flags, is64 } => {
                let a = mask(is64, self.read(rn, false));
                let b = mask(is64, extend_operand(self.read(rm, false), option) << shift);
                let r = if sub { a.wrapping_sub(b) } else { a.wrapping_add(b) };
                let r = mask(is64, r);
                if set_flags {
                    if sub {
                        self.set_sub_flags(a, b, r, is64);
                    } else {
                        self.set_add_flags(a, b, r, is64);
                    }
                }
                self.write(rd, false, r);
            }
            Op::Bitfield { rd, rn, immr, imms, opc, is64 } => {
                let src = mask(is64, self.read(rn, false));
                let dst = mask(is64, self.read(rd, false));
                let r = bitfield(src, dst, immr, imms, opc, is64);
                self.write(rd, false, r);
            }
            Op::MulLong { rd, rn, rm, ra, sub, unsigned } => {
                let a = (self.read(rn, false) & 0xFFFF_FFFF) as u32;
                let b = (self.read(rm, false) & 0xFFFF_FFFF) as u32;
                let product = if unsigned {
                    (a as u64).wrapping_mul(b as u64)
                } else {
                    ((a as i32 as i64).wrapping_mul(b as i32 as i64)) as u64
                };
                let acc = self.read(ra, false);
                let r = if sub {
                    acc.wrapping_sub(product)
                } else {
                    acc.wrapping_add(product)
                };
                self.write(rd, false, r);
            }
            Op::DataProc2 { rd, rn, rm, op, is64 } => {
                let a = mask(is64, self.read(rn, false));
                let b = mask(is64, self.read(rm, false));
                let r = match op {
                    DataOp2::Udiv => {
                        if b == 0 {
                            0
                        } else {
                            a / b
                        }
                    }
                    DataOp2::Sdiv => {
                        if b == 0 {
                            0
                        } else if is64 {
                            if a == 0x8000_0000_0000_0000 && b == u64::MAX {
                                a
                            } else {
                                ((a as i64).wrapping_div(b as i64)) as u64
                            }
                        } else {
                            let (a32, b32) = (a as u32, b as u32);
                            if a32 == 0x8000_0000 && b32 == 0xFFFF_FFFF {
                                a32 as u64
                            } else {
                                ((a32 as i32).wrapping_div(b32 as i32)) as u32 as u64
                            }
                        }
                    }
                    DataOp2::Lslv => apply_shift(a, Shift::Lsl, b as u8, is64),
                    DataOp2::Lsrv => apply_shift(a, Shift::Lsr, b as u8, is64),
                    DataOp2::Asrv => apply_shift(a, Shift::Asr, b as u8, is64),
                    DataOp2::Rorv => apply_shift(a, Shift::Ror, b as u8, is64),
                };
                self.write(rd, false, mask(is64, r));
            }
            Op::LoadStore { rt, rn, offset, bytes, load, signed, mode, is64 } => {
                let base = self.read(rn, true);
                let ea = if mode == crate::aarch64::MemMode::PostIndex {
                    base
                } else {
                    base.wrapping_add(offset as u64)
                };
                if load {
                    let raw = self.load(ea, bytes)?;
                    let v = if signed {
                        let sign = 63 - ((bytes as u32 * 8 - 1) as u64);
                        mask(is64, (((raw << sign) as i64) >> sign) as u64)
                    } else {
                        mask(is64, raw)
                    };
                    self.write(rt, false, v);
                } else {
                    let v = self.read(rt, false);
                    self.store(ea, v, bytes)?;
                }
                if mode != crate::aarch64::MemMode::Offset {
                    self.write(rn, true, base.wrapping_add(offset as u64));
                }
            }
            Op::LoadStorePair { rt, rt2, rn, offset, load, mode, is64 } => {
                let base = self.read(rn, true);
                let width = if is64 { 8u8 } else { 4u8 };
                let ea = if mode == crate::aarch64::MemMode::PostIndex {
                    base
                } else {
                    base.wrapping_add(offset as u64)
                };
                if load {
                    let v1 = mask(is64, self.load(ea, width)?);
                    let v2 = mask(is64, self.load(ea.wrapping_add(width as u64), width)?);
                    self.write(rt, false, v1);
                    self.write(rt2, false, v2);
                } else {
                    let v1 = self.read(rt, false);
                    let v2 = self.read(rt2, false);
                    self.store(ea, v1, width)?;
                    self.store(ea.wrapping_add(width as u64), v2, width)?;
                }
                if mode != crate::aarch64::MemMode::Offset {
                    self.write(rn, true, base.wrapping_add(offset as u64));
                }
            }
            Op::LoadPairSigned { rt, rt2, rn, offset, mode } => {
                let base = self.read(rn, true);
                let ea = if mode == crate::aarch64::MemMode::PostIndex { base } else { base.wrapping_add(offset as u64) };
                let v1 = self.load(ea, 4)? as u32 as i32 as i64 as u64;
                let v2 = self.load(ea.wrapping_add(4), 4)? as u32 as i32 as i64 as u64;
                self.write(rt, false, v1);
                self.write(rt2, false, v2);
                if mode != crate::aarch64::MemMode::Offset {
                    self.write(rn, true, base.wrapping_add(offset as u64));
                }
            }
            Op::LoadLiteral { rt, target, is64 } => {
                let bytes = if is64 { 8 } else { 4 };
                let v = mask(is64, self.load(target, bytes)?);
                self.write(rt, false, v);
            }
            Op::LoadStoreReg { rt, rn, rm, option, shift, bytes, load, signed, is64 } => {
                let base = self.read(rn, true);
                let extended = extend_operand(self.read(rm, false), option) << shift;
                let ea = base.wrapping_add(extended);
                if load {
                    let raw = self.load(ea, bytes)?;
                    let v = if signed {
                        let sign = 63 - ((bytes as u32 * 8 - 1) as u64);
                        mask(is64, (((raw << sign) as i64) >> sign) as u64)
                    } else {
                        mask(is64, raw)
                    };
                    self.write(rt, false, v);
                } else {
                    let v = self.read(rt, false);
                    self.store(ea, v, bytes)?;
                }
            }
            Op::BitScan { rd, rn, op, is64 } => {
                let src = if is64 { self.read(rn, false) } else { self.read(rn, false) & 0xFFFF_FFFF };
                let r = match op {
                    crate::aarch64::BitScan::Rbit => {
                        if is64 {
                            src.reverse_bits()
                        } else {
                            (src as u32).reverse_bits() as u64
                        }
                    }
                    crate::aarch64::BitScan::Rev => {
                        if is64 {
                            src.swap_bytes()
                        } else {
                            (src as u32).swap_bytes() as u64
                        }
                    }
                    crate::aarch64::BitScan::Rev32 => reverse_bytes_within(src, 4, is64),
                    crate::aarch64::BitScan::Rev16 => reverse_bytes_within(src, 2, is64),
                    crate::aarch64::BitScan::Clz => {
                        if is64 {
                            src.leading_zeros() as u64
                        } else {
                            (src as u32).leading_zeros() as u64
                        }
                    }
                    crate::aarch64::BitScan::Cls => {
                        let x = if is64 {
                            src ^ ((src as i64 >> 1) as u64)
                        } else {
                            ((src as u32) ^ (((src as u32 as i32) >> 1) as u32)) as u64
                        };
                        let zeros = if is64 { x.leading_zeros() } else { (x as u32).leading_zeros() };
                        zeros.saturating_sub(1) as u64
                    }
                };
                self.write(rd, false, mask(is64, r));
            }
            Op::Crc32 { rd, rn, rm, bytes, castagnoli } => {
                // The 64-bit operand form is the only one that reads a wide
                // register; every form returns a 32-bit checksum.
                let crc = self.read(rn, false) as u32;
                let data = self.read(rm, bytes == 8);
                let poly = if castagnoli { 0x82F6_3B78 } else { 0xEDB8_8320 };
                self.write(rd, false, u64::from(crc32(crc, data, bytes, poly)));
            }
            Op::CondCompare { rn, rm, imm5, immediate, cond, nzcv, sub, is64 } => {
                let a = mask(is64, self.read(rn, false));
                let b = if immediate {
                    imm5 as u64
                } else {
                    mask(is64, self.read(rm, false))
                };
                if self.cond_holds(cond) {
                    let r = mask(is64, if sub { a.wrapping_sub(b) } else { a.wrapping_add(b) });
                    if sub {
                        self.set_sub_flags(a, b, r, is64);
                    } else {
                        self.set_add_flags(a, b, r, is64);
                    }
                } else {
                    self.nzcv = (nzcv & 0xF) as u32;
                }
            }
            Op::AtomicRmw { rs, rt, rn, op, order: _, bytes } => {
                let ea = self.read(rn, true);
                let old = self.load(ea, bytes)?;
                let operand = self.read(rs, false);
                let width = bytes as u32 * 8;
                let keep = |v: u64| -> u64 {
                    if bytes == 8 {
                        v
                    } else {
                        v & ((1u64 << width) - 1)
                    }
                };
                let signed = |v: u64| -> i64 {
                    let sh = 64 - width;
                    ((v << sh) as i64) >> sh
                };
                let new = match op {
                    crate::aarch64::AtomicOp::Add => keep(old.wrapping_add(operand)),
                    crate::aarch64::AtomicOp::Clr => keep(old & !operand),
                    crate::aarch64::AtomicOp::Eor => keep(old ^ operand),
                    crate::aarch64::AtomicOp::Set => keep(old | operand),
                    crate::aarch64::AtomicOp::SMax => {
                        if signed(old) > signed(operand) { keep(old) } else { keep(operand) }
                    }
                    crate::aarch64::AtomicOp::SMin => {
                        if signed(old) < signed(operand) { keep(old) } else { keep(operand) }
                    }
                    crate::aarch64::AtomicOp::UMax => {
                        if keep(old) > keep(operand) { keep(old) } else { keep(operand) }
                    }
                    crate::aarch64::AtomicOp::UMin => {
                        if keep(old) < keep(operand) { keep(old) } else { keep(operand) }
                    }
                    crate::aarch64::AtomicOp::Swap => keep(operand),
                };
                self.store(ea, new, bytes)?;
                self.write(rt, false, keep(old));
            }
            Op::VectorLoadStore { rt, rn, offset, bytes, load, mode } => {
                let base = self.read(rn, true);
                let ea = if mode == crate::aarch64::MemMode::PostIndex {
                    base
                } else {
                    base.wrapping_add(offset as u64)
                };
                if load {
                    if bytes == 16 {
                        let lo = self.load(ea, 8)?;
                        let hi = self.load(ea + 8, 8)?;
                        self.vwrite128(rt, lo, hi);
                    } else {
                        let v = self.load(ea, bytes)?;
                        self.vwrite(rt, bytes, v);
                    }
                } else if bytes == 16 {
                    let idx = (rt & 31) as usize;
                    let (lo, hi) = (self.v[idx][0], self.v[idx][1]);
                    self.store(ea, lo, 8)?;
                    self.store(ea + 8, hi, 8)?;
                } else {
                    let v = self.vread(rt, bytes);
                    self.store(ea, v, bytes)?;
                }
                if mode != crate::aarch64::MemMode::Offset {
                    self.write(rn, true, base.wrapping_add(offset as u64));
                }
            }
            Op::VectorLoadStoreReg { rt, rn, rm, option, shift, bytes, load } => {
                let base = self.read(rn, true);
                let ea = base.wrapping_add(extend_operand(self.read(rm, false), option) << shift);
                if load {
                    if bytes == 16 {
                        let lo = self.load(ea, 8)?;
                        let hi = self.load(ea + 8, 8)?;
                        self.vwrite128(rt, lo, hi);
                    } else {
                        let v = self.load(ea, bytes)?;
                        self.vwrite(rt, bytes, v);
                    }
                } else if bytes == 16 {
                    let idx = (rt & 31) as usize;
                    let (lo, hi) = (self.v[idx][0], self.v[idx][1]);
                    self.store(ea, lo, 8)?;
                    self.store(ea + 8, hi, 8)?;
                } else {
                    let v = self.vread(rt, bytes);
                    self.store(ea, v, bytes)?;
                }
            }
            Op::VectorLoadStorePair { rt, rt2, rn, offset, bytes, load, mode } => {
                let base = self.read(rn, true);
                let ea = if mode == crate::aarch64::MemMode::PostIndex {
                    base
                } else {
                    base.wrapping_add(offset as u64)
                };
                for (n, reg) in [rt, rt2].into_iter().enumerate() {
                    let at = ea.wrapping_add(n as u64 * bytes as u64);
                    if load {
                        if bytes == 16 {
                            let lo = self.load(at, 8)?;
                            let hi = self.load(at + 8, 8)?;
                            self.vwrite128(reg, lo, hi);
                        } else {
                            let v = self.load(at, bytes)?;
                            self.vwrite(reg, bytes, v);
                        }
                    } else if bytes == 16 {
                        let idx = (reg & 31) as usize;
                        let (lo, hi) = (self.v[idx][0], self.v[idx][1]);
                        self.store(at, lo, 8)?;
                        self.store(at + 8, hi, 8)?;
                    } else {
                        let v = self.vread(reg, bytes);
                        self.store(at, v, bytes)?;
                    }
                }
                if mode != crate::aarch64::MemMode::Offset {
                    self.write(rn, true, base.wrapping_add(offset as u64));
                }
            }
            Op::VectorZero { rd, full: _ } => {
                self.vwrite128(rd, 0, 0);
            }
            Op::FpMoveGp { fp, gp, to_fp, is64 } => {
                let bytes = if is64 { 8 } else { 4 };
                if to_fp {
                    let v = self.read(gp, false);
                    self.vwrite(fp, bytes, v);
                } else {
                    let v = self.vread(fp, bytes);
                    self.write(gp, false, if is64 { v } else { v & 0xFFFF_FFFF });
                }
            }
            Op::FpCopy { rd, rn, is64 } => {
                let bytes = if is64 { 8 } else { 4 };
                let v = self.vread(rn, bytes);
                self.vwrite(rd, bytes, v);
            }
            Op::FpBinary { rd, rn, rm, op, is64 } => {
                let v = if is64 {
                    let a = self.dread(rn);
                    let b = self.dread(rm);
                    fp_binary64(op, a, b)
                } else {
                    let a = self.sread(rn);
                    let b = self.sread(rm);
                    fp_binary32(op, a, b) as f64
                };
                if is64 {
                    self.dwrite(rd, v);
                } else {
                    self.swrite(rd, v as f32);
                }
            }
            Op::FpUnary { rd, rn, op, is64 } => {
                // fcvt is the one case where the destination is not the source
                // width, so it cannot go through the same-width helpers.
                if op == FpUnop::Cvt {
                    if is64 {
                        let v = self.dread(rn);
                        self.swrite(rd, v as f32);
                    } else {
                        let v = self.sread(rn);
                        self.dwrite(rd, v as f64);
                    }
                } else if is64 {
                    let v = fp_unary64(op, self.dread(rn));
                    self.dwrite(rd, v);
                } else {
                    let v = fp_unary32(op, self.sread(rn));
                    self.swrite(rd, v);
                }
            }
            Op::FpCompare { rn, rm, is64, signal: _ } => {
                if is64 {
                    let (a, b) = (self.dread(rn), self.dread(rm));
                    self.set_fp_flags(a, b);
                } else {
                    let (a, b) = (self.sread(rn), self.sread(rm));
                    self.set_fp_flags(a as f64, b as f64);
                }
            }
            Op::FpCompareZero { rn, is64, signal: _ } => {
                if is64 {
                    let a = self.dread(rn);
                    self.set_fp_flags(a, 0.0);
                } else {
                    let a = self.sread(rn);
                    self.set_fp_flags(a as f64, 0.0);
                }
            }
            Op::FpCondCompare { rn, rm, cond, nzcv, is64 } => {
                if self.cond_holds(cond) {
                    if is64 {
                        let (a, b) = (self.dread(rn), self.dread(rm));
                        self.set_fp_flags(a, b);
                    } else {
                        let (a, b) = (self.sread(rn), self.sread(rm));
                        self.set_fp_flags(a as f64, b as f64);
                    }
                } else {
                    self.nzcv = nzcv as u32;
                }
            }
            Op::FpCondSelect { rd, rn, rm, cond, is64 } => {
                let src = if self.cond_holds(cond) { rn } else { rm };
                if is64 {
                    let v = self.dread(src);
                    self.dwrite(rd, v);
                } else {
                    let v = self.sread(src);
                    self.swrite(rd, v);
                }
            }
            Op::FpRoundFp { rd, rn, round, is64 } => {
                if is64 {
                    let v = fp_round_to_integral(self.dread(rn), round);
                    self.dwrite(rd, v);
                } else {
                    let v = fp_round_to_integral(self.sread(rn) as f64, round);
                    self.swrite(rd, v as f32);
                }
            }
            Op::FpToInt { rd, rn, round, unsigned, src_is64, dest_is64 } => {
                let v = if src_is64 { self.dread(rn) } else { self.sread(rn) as f64 };
                let r = fp_to_int(v, round, unsigned, dest_is64);
                self.write(rd, false, r);
            }
            Op::IntToFp { rd, rn, unsigned, src_is64, dest_is64 } => {
                let raw = self.read(rn, false);
                let v = int_to_fp(raw, unsigned, src_is64);
                if dest_is64 {
                    self.dwrite(rd, v);
                } else {
                    self.swrite(rd, v as f32);
                }
            }
            Op::IntToFpFixed { rd, rn, unsigned, src_is64, dest_is64, fbits } => {
                let raw = self.read(rn, false);
                let v = int_to_fp(raw, unsigned, src_is64) / 2f64.powi(fbits as i32);
                if dest_is64 {
                    self.dwrite(rd, v);
                } else {
                    self.swrite(rd, v as f32);
                }
            }
            Op::FpToIntFixed { rd, rn, unsigned, src_is64, dest_is64, fbits } => {
                let v = if src_is64 { self.dread(rn) } else { self.sread(rn) as f64 };
                let r = fp_to_int(v * 2f64.powi(fbits as i32), FpRound::Zero, unsigned, dest_is64);
                self.write(rd, false, r);
            }
            Op::FpMovImm { rd, imm8, is64 } => {
                let v = fp_immediate(imm8);
                if is64 {
                    self.dwrite(rd, v);
                } else {
                    self.swrite(rd, v as f32);
                }
            }
            Op::FpMulAdd { rd, rn, rm, ra, negate_product, negate_addend, is64 } => {
                if is64 {
                    let product = self.dread(rn) * self.dread(rm);
                    let product = if negate_product { -product } else { product };
                    let addend = if negate_addend { -self.dread(ra) } else { self.dread(ra) };
                    self.dwrite(rd, product + addend);
                } else {
                    let product = self.sread(rn) * self.sread(rm);
                    let product = if negate_product { -product } else { product };
                    let addend = if negate_addend { -self.sread(ra) } else { self.sread(ra) };
                    self.swrite(rd, product + addend);
                }
            }
            Op::Extr { rd, rn, rm, lsb, is64 } => {
                let (hi, lo) = (self.read(rn, false), self.read(rm, false));
                let v = if is64 {
                    if lsb == 0 {
                        lo
                    } else {
                        (lo >> lsb) | (hi << (64 - lsb as u32))
                    }
                } else {
                    let cat = ((hi & 0xFFFF_FFFF) << 32) | (lo & 0xFFFF_FFFF);
                    (cat >> lsb) & 0xFFFF_FFFF
                };
                self.write(rd, false, v);
            }
            Op::MulHigh { rd, rn, rm, unsigned } => {
                let (a, b) = (self.read(rn, false), self.read(rm, false));
                let v = if unsigned {
                    ((a as u128 * b as u128) >> 64) as u64
                } else {
                    ((a as i64 as i128 * b as i64 as i128) >> 64) as u64
                };
                self.write(rd, false, v);
            }
            Op::Exclusive { rs, rt, rt2, rn, bytes, load, pair } => {
                let ea = self.read(rn, true);
                let is64 = bytes == 8;
                if load {
                    let v = self.load(ea, bytes)?;
                    self.write(rt, false, v);
                    if pair {
                        let v2 = self.load(ea + bytes as u64, bytes)?;
                        self.write(rt2, false, v2);
                    }
                    self.exclusive = Some(ea);
                } else {
                    let ok = self.exclusive == Some(ea);
                    if ok {
                        let v = mask(is64, self.read(rt, false));
                        self.store(ea, v, bytes)?;
                        if pair {
                            let v2 = mask(is64, self.read(rt2, false));
                            self.store(ea + bytes as u64, v2, bytes)?;
                        }
                    }
                    self.write(rs, false, if ok { 0 } else { 1 });
                    self.exclusive = None;
                }
            }
            Op::Cas { rs, rt, rn, bytes, pair } => {
                let ea = self.read(rn, true);
                let m = |v: u64| if bytes >= 8 { v } else { v & ((1u64 << (bytes * 8)) - 1) };
                let old = self.load(ea, bytes)?;
                if pair {
                    let old2 = self.load(ea + bytes as u64, bytes)?;
                    let (c1, c2) = (m(self.read(rs, false)), m(self.read(rs + 1, false)));
                    if old == c1 && old2 == c2 {
                        let (n1, n2) = (m(self.read(rt, false)), m(self.read(rt + 1, false)));
                        self.store(ea, n1, bytes)?;
                        self.store(ea + bytes as u64, n2, bytes)?;
                    }
                    self.write(rs, false, old);
                    self.write(rs + 1, false, old2);
                } else {
                    if old == m(self.read(rs, false)) {
                        let n = m(self.read(rt, false));
                        self.store(ea, n, bytes)?;
                    }
                    self.write(rs, false, old);
                }
            }
            Op::Mrs { rt, reg } => {
                let v = self.read_sysreg(reg)?;
                self.write(rt, false, v);
            }
            Op::Msr { rt, reg } => {
                let v = self.read(rt, false);
                match reg {
                    sysreg::TPIDR_EL0 => self.tpidr = v,
                    sysreg::FPCR => self.fpcr = v,
                    sysreg::FPSR => self.fpsr = v,
                    sysreg::NZCV => self.nzcv = ((v >> 28) & 0xF) as u32,
                    _ => return Err(format!("msr to system register 0x{reg:x} is not modelled")),
                }
            }
            Op::Svc { imm } => {
                // The host kernel layer takes over at the next instruction;
                // the run loop sees the pending call and services it.
                self.svc = Some(imm);
                return Err(format!("svc 0x{imm:x}"));
            }
            Op::Simd(ref s) => crate::simd::exec(self, s)?,
            Op::SimdMem(ref m) => crate::simd::exec_mem(self, m)?,
            Op::Unsupported { word } | Op::UnsupportedBitfield { word } => {
                return Err(format!("unsupported instruction 0x{word:08x}"));
            }
        }
        Ok(())
    }
}

/// IEEE maximum with the architectural NaN rule: a single NaN operand loses to
/// the number, and the two signed zeros compare equal but keep a fixed sign.
fn fp_max(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a == b {
        if a == 0.0 && a.is_sign_negative() != b.is_sign_negative() {
            return 0.0;
        }
        return a;
    }
    if a > b {
        a
    } else {
        b
    }
}

fn fp_min(a: f64, b: f64) -> f64 {
    if a.is_nan() {
        return b;
    }
    if b.is_nan() {
        return a;
    }
    if a == b {
        if a == 0.0 && a.is_sign_negative() != b.is_sign_negative() {
            return -0.0;
        }
        return a;
    }
    if a < b {
        a
    } else {
        b
    }
}

fn fp_binary64(op: FpBinop, a: f64, b: f64) -> f64 {
    match op {
        FpBinop::Mul => a * b,
        FpBinop::Div => a / b,
        FpBinop::Add => a + b,
        FpBinop::Sub => a - b,
        FpBinop::NegMul => -(a * b),
        FpBinop::Max | FpBinop::MaxNm => fp_max(a, b),
        FpBinop::Min | FpBinop::MinNm => fp_min(a, b),
    }
}

fn fp_binary32(op: FpBinop, a: f32, b: f32) -> f32 {
    match op {
        FpBinop::Mul => a * b,
        FpBinop::Div => a / b,
        FpBinop::Add => a + b,
        FpBinop::Sub => a - b,
        FpBinop::NegMul => -(a * b),
        FpBinop::Max | FpBinop::MaxNm => fp_max(a as f64, b as f64) as f32,
        FpBinop::Min | FpBinop::MinNm => fp_min(a as f64, b as f64) as f32,
    }
}

fn fp_unary64(op: FpUnop, v: f64) -> f64 {
    match op {
        FpUnop::Neg => -v,
        FpUnop::Abs => v.abs(),
        FpUnop::Sqrt => v.sqrt(),
        FpUnop::RoundN | FpUnop::RoundX | FpUnop::RoundI => v.round_ties_even(),
        FpUnop::RoundA => v.round(),
        FpUnop::RoundP => v.ceil(),
        FpUnop::RoundM => v.floor(),
        FpUnop::RoundZ => v.trunc(),
        // Handled by the caller, which has to change the register width.
        FpUnop::Cvt => v,
    }
}

fn fp_unary32(op: FpUnop, v: f32) -> f32 {
    match op {
        FpUnop::Neg => -v,
        FpUnop::Abs => v.abs(),
        FpUnop::Sqrt => v.sqrt(),
        FpUnop::RoundN | FpUnop::RoundX | FpUnop::RoundI => v.round_ties_even(),
        FpUnop::RoundA => v.round(),
        FpUnop::RoundP => v.ceil(),
        FpUnop::RoundM => v.floor(),
        FpUnop::RoundZ => v.trunc(),
        FpUnop::Cvt => v,
    }
}

fn fp_round_to_integral(v: f64, round: FpRound) -> f64 {
    match round {
        FpRound::NearestEven => v.round_ties_even(),
        FpRound::NearestAway => v.round(),
        FpRound::PlusInf => v.ceil(),
        FpRound::MinusInf => v.floor(),
        FpRound::Zero => v.trunc(),
    }
}

/// Float to integer with the architectural saturation: NaN becomes zero and an
/// out of range value clamps to the destination's limits rather than trapping.
fn fp_to_int(v: f64, round: FpRound, unsigned: bool, dest_is64: bool) -> u64 {
    if v.is_nan() {
        return 0;
    }
    let r = fp_round_to_integral(v, round);
    if unsigned {
        if r <= 0.0 {
            0
        } else if dest_is64 {
            r as u64
        } else {
            (r as u32) as u64
        }
    } else if dest_is64 {
        (r as i64) as u64
    } else {
        ((r as i32) as u32) as u64
    }
}

fn int_to_fp(raw: u64, unsigned: bool, src_is64: bool) -> f64 {
    if src_is64 {
        if unsigned {
            raw as f64
        } else {
            (raw as i64) as f64
        }
    } else if unsigned {
        (raw as u32) as f64
    } else {
        ((raw as u32) as i32) as f64
    }
}

/// Expand the eight bit `fmov` immediate. The exponent is three bits with a
/// bias of three and the fraction is four bits in sixteenths; putting the two
/// together reproduces every value the assembler emits.
fn fp_immediate(imm8: u8) -> f64 {
    let sign = imm8 & 0x80 != 0;
    let exp_field = (((!imm8 >> 6) & 1) << 2) | ((imm8 >> 4) & 3);
    let exp = exp_field as i32 - 3;
    let frac = (imm8 & 0xF) as f64 / 16.0;
    let magnitude = (1.0 + frac) * 2f64.powi(exp);
    if sign {
        -magnitude
    } else {
        magnitude
    }
}

/// CRC-32 as `crc32b/h/w/x` compute it: polynomial 0x04C11DB7 reflected as
/// 0xEDB88320, no pre- or post-inversion, continuing from the checksum in the
/// first operand and absorbing `bytes` bytes of `data` from its low end.
fn crc32(crc: u32, data: u64, bytes: u8, poly: u32) -> u32 {
    let mut c = crc;
    let mut d = data;
    for _ in 0..bytes {
        c ^= (d & 0xFF) as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { (c >> 1) ^ poly } else { c >> 1 };
        }
        d >>= 8;
    }
    c
}

#[cfg(test)]
mod crc_tests {
    use super::crc32;

    #[test]
    fn crc32b_matches_the_published_check_value() {
        // The instruction's model has no inversion, so the published check
        // values of "123456789" (0xCBF43926 for IEEE, 0xE3069283 for Castagnoli)
        // are reproduced by starting from an inverted seed and inverting once at
        // the end, which is exactly how zlib computes them.
        let mut ieee = u32::MAX;
        let mut castagnoli = u32::MAX;
        for byte in b"123456789" {
            ieee = crc32(ieee, u64::from(*byte), 1, 0xEDB8_8320);
            castagnoli = crc32(castagnoli, u64::from(*byte), 1, 0x82F6_3B78);
        }
        assert_eq!(!ieee, 0xCBF4_3926);
        assert_eq!(!castagnoli, 0xE306_9283);
        // A zero seed gives the raw values the game sees when it chains crc32b
        // from a zeroed accumulator.
        assert_eq!(ieee, 0x340B_C6D9);
        assert_eq!(castagnoli, 0x1CF9_6D7C);
    }

    #[test]
    fn wider_forms_absorb_the_operand_little_endian() {
        let mut by_byte = 0u32;
        for byte in [0x01u8, 0x02, 0x03, 0x04] {
            by_byte = crc32(by_byte, u64::from(byte), 1, 0xEDB8_8320);
        }
        let by_word = crc32(0, 0x0403_0201, 4, 0xEDB8_8320);
        assert_eq!(by_byte, by_word);
    }
}

/// Reverse the byte order inside each `group`-byte unit of a 32 or 64-bit value,
/// which is what `rev16` and `rev32` do.
fn reverse_bytes_within(v: u64, group: u32, is64: bool) -> u64 {
    let bits = if is64 { 64 } else { 32 };
    let mut out = 0u64;
    let mut base = 0u32;
    while base < bits {
        for j in 0..group {
            let byte = (v >> (base + j * 8)) & 0xFF;
            out |= byte << (base + (group - 1 - j) * 8);
        }
        base += group * 8;
    }
    out
}

fn extend_operand(v: u64, option: Extend) -> u64 {
    match option {
        Extend::Uxtb => v & 0xFF,
        Extend::Uxth => v & 0xFFFF,
        Extend::Uxtw => v & 0xFFFF_FFFF,
        Extend::Uxtx => v,
        Extend::Sxtb => (v as u8 as i8) as i64 as u64,
        Extend::Sxth => (v as u16 as i16) as i64 as u64,
        Extend::Sxtw => (v as u32 as i32) as i64 as u64,
        Extend::Sxtx => v,
    }
}

fn ones(n: u32) -> u64 {
    if n >= 64 {
        u64::MAX
    } else {
        (1u64 << n) - 1
    }
}

fn bitfield(src: u64, dst: u64, immr: u8, imms: u8, opc: u8, is64: bool) -> u64 {
    let bits = if is64 { 64u32 } else { 32u32 };
    let r = immr as u32 % bits;
    let s = imms as u32 % bits;
    let (width, shift, field) = if r <= s {
        (s - r + 1, 0u32, (src >> r) & ones(s - r + 1))
    } else {
        (s + 1, bits - r, (src & ones(s + 1)) << (bits - r))
    };
    let result = match opc {
        2 => field,
        0 => {
            let top = shift + width - 1;
            let sh = 63 - top;
            (((field << sh) as i64) >> sh) as u64
        }
        _ => (dst & !(ones(width) << shift)) | field,
    };
    if is64 {
        result as u64
    } else {
        result as u32 as u64
    }
}

fn apply_shift(v: u64, shift: Shift, amount: u8, is64: bool) -> u64 {
    let width = if is64 { 64 } else { 32 };
    let amount = amount as u32 % width;
    let mask = if is64 { u64::MAX } else { 0xFFFF_FFFF };
    match shift {
        Shift::Lsl => v << amount & mask,
        Shift::Lsr => (v & mask) >> amount,
        Shift::Asr => {
            if is64 {
                ((v as i64) >> amount) as u64
            } else {
                (((v as u32) as i32) >> amount) as u32 as u64
            }
        }
        Shift::Ror => {
            if is64 {
                v.rotate_right(amount)
            } else {
                ((v as u32).rotate_right(amount)) as u64
            }
        }
    }
}

/// A recovered function with its instruction index built once.
///
/// A whole-program dispatcher enters functions millions of times, so the index
/// has to outlive a single call. Building it per entry address is what makes
/// that possible without changing the execution semantics.
/// A recovered function prepared once for repeated execution: its
/// instructions in a dense array indexed by address, so each step is an array
/// lookup rather than a map search. Behaves exactly like [`Program`].
pub struct Compiled {
    pub entry: u64,
    base: u64,
    ops: Vec<Option<Op>>,
}

impl Compiled {
    pub fn new(func: &crate::scan::LiftedFunction) -> Compiled {
        let addrs = func.blocks.iter().flat_map(|b| b.insns.iter().map(|i| i.addr));
        let (lo, hi) = addrs.fold((u64::MAX, 0u64), |(lo, hi), a| (lo.min(a), hi.max(a)));
        let base = if lo == u64::MAX { func.addr } else { lo };
        let len = if lo == u64::MAX { 0 } else { ((hi - lo) / 4 + 1) as usize };
        let mut ops = vec![None; len];
        for block in &func.blocks {
            for insn in &block.insns {
                ops[((insn.addr - base) / 4) as usize] = Some(insn.op);
            }
        }
        Compiled { entry: func.addr, base, ops }
    }

    #[inline]
    fn op_at(&self, addr: u64) -> Option<&Op> {
        if addr < self.base || addr % 4 != 0 {
            return None;
        }
        self.ops.get(((addr - self.base) / 4) as usize).and_then(|o| o.as_ref())
    }

    /// As [`Program::run_traced`].
    pub fn run_traced(&self, state: &mut State, max_steps: usize) -> Result<Option<Transfer>, String> {
        state.pc = self.entry;
        for _ in 0..max_steps {
            let op = match self.op_at(state.pc) {
                Some(op) => op,
                None => return Ok(Some(Transfer { from: state.pc, to: state.pc })),
            };
            let here = state.pc;
            state.step(op)?;
            let next = if state.pc == here { here + 4 } else { state.pc };
            if self.op_at(next).is_none() {
                return Ok(Some(Transfer { from: here, to: next }));
            }
            state.pc = next;
        }
        Err(format!("step budget {max_steps} exhausted in function 0x{:x}", self.entry))
    }
}

pub struct Program<'a> {
    func: &'a crate::scan::LiftedFunction,
    index: std::collections::BTreeMap<u64, &'a Op>,
}

/// Where control went when it left a function, and which instruction sent it
/// there. A whole-program dispatcher needs the origin: it is the difference
/// between "called something" and "returned to a caller".
pub struct Transfer {
    /// Address of the instruction that moved control out of the function.
    pub from: u64,
    /// Where `pc` ended up.
    pub to: u64,
}

impl<'a> Program<'a> {
    pub fn new(func: &'a crate::scan::LiftedFunction) -> Program<'a> {
        let mut index = std::collections::BTreeMap::new();
        for block in &func.blocks {
            for insn in &block.insns {
                index.insert(insn.addr, &insn.op);
            }
        }
        Program { func, index }
    }

    pub fn entry(&self) -> u64 {
        self.func.addr
    }

    pub fn contains(&self, addr: u64) -> bool {
        self.index.contains_key(&addr)
    }

    /// Run from the function's entry point until control leaves the block set,
    /// reporting the transfer that ended the run.
    pub fn run_traced(
        &self,
        state: &mut State,
        max_steps: usize,
    ) -> Result<Option<Transfer>, String> {
        state.pc = self.func.addr;
        for _ in 0..max_steps {
            let op = match self.index.get(&state.pc) {
                Some(op) => *op,
                None => return Ok(Some(Transfer { from: state.pc, to: state.pc })),
            };
            let here = state.pc;
            state.pc = here;
            state.step(op)?;
            let next = if state.pc == here { here + 4 } else { state.pc };
            if !self.index.contains_key(&next) {
                return Ok(Some(Transfer { from: here, to: next }));
            }
            state.pc = next;
        }
        Err(format!(
            "step budget {max_steps} exhausted in function 0x{:x}",
            self.func.addr
        ))
    }

    /// Run from the function's entry point until control leaves the block set.
    ///
    /// Leaving is not an error: it is the point where the emitted C returns to
    /// the dispatcher, so it is the point where a whole-program runner has to
    /// resolve the new `pc` itself.
    pub fn run(&self, state: &mut State, max_steps: usize) -> Result<(), String> {
        self.run_traced(state, max_steps).map(|_| ())
    }
}

/// Execute a recovered function, following its control-flow graph. Execution
/// stops when control leaves the function (a call, a return, or a branch to an
/// address that is not one of its blocks), which is exactly where the emitted C
/// returns to the dispatcher.
pub fn run_function(
    state: &mut State,
    func: &crate::scan::LiftedFunction,
    max_steps: usize,
) -> Result<(), String> {
    Program::new(func).run(state, max_steps)
}

/// Execute a straight-line run of ops from `state.pc`, stopping after the first
/// branch/terminal instruction or on error.
pub fn run_linear(state: &mut State, insns: &[(u64, Op)], max_steps: usize) -> Result<(), String> {
    for (addr, op) in insns.iter().take(max_steps) {
        state.pc = *addr;
        state.step(op)?;
        // Straight-line instructions leave pc pointing at themselves; a branch
        // (or a terminal op) moves it, which ends this run.
        if op.is_terminal() || state.pc != *addr {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aarch64::decode;

    #[test]
    fn adds_and_subtracts() {
        let mut s = State::patterned();
        // add x0, x1, #0x10
        let op = decode(0x9100_4020, 0);
        s.step(&op).unwrap();
        assert_eq!(s.x[0], s.x[1] + 0x10);

        // sub sp, sp, #0x20
        let sp = s.sp;
        let op = decode(0xD100_83FF, 0);
        s.step(&op).unwrap();
        assert_eq!(s.sp, sp - 0x20);
    }

    #[test]
    fn compare_sets_flags_and_keeps_xzr() {
        let mut s = State::patterned();
        // cmp x1, x1  (subs xzr, x1, x1) -> Z=1, C=1
        let word = 0xEB01_003F;
        let op = decode(word, 0);
        s.step(&op).unwrap();
        assert_eq!(s.nzcv & 0b0100, 0b0100, "zero flag");
        assert_eq!(s.nzcv & 0b0010, 0b0010, "carry flag");
        assert_eq!(s.x[1], 0x1020, "operand untouched");
    }

    #[test]
    fn executes_vector_and_atomic_families() {
        let words: [u32; 6] = [
            0x6F00_E400, // movi v0.2d, #0
            0x3D80_0040, // str q0, [x2]
            0x1E27_0101, // fmov s1, w8
            0xDAC0_1129, // clz x9, x9
            0xF82A_006B, // ldadd x10, x11, [x3]
            0xD65F_03C0, // ret
        ];
        let text: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let f = crate::scan::lift_function(&text, 0, 64).expect("one block");

        let mut st = State::patterned();
        // The atomic operand: the destination reads the old value, the old value
        // is replaced by old + operand.
        st.mem[0x1060] = 5;

        run_function(&mut st, &f, 1000).unwrap();

        assert_eq!(st.v[0], [0, 0], "movi clears the whole register");
        assert_eq!(&st.mem[0x1040..0x1050], &[0u8; 16], "str q writes sixteen bytes");
        assert_eq!(st.v[1][0], 0x1100, "fmov s1, w8 copies the low word");
        assert_eq!(st.v[1][1], 0, "and clears the upper half");
        assert_eq!(st.x[9], 51, "clz of 0x1120");
        assert_eq!(st.x[11], 5, "ldadd returns the previous value");
        assert_eq!(st.mem[0x1060], 0x45, "5 + 0x1140 lands in memory");
    }

    #[test]
    fn conditional_compare_writes_flags_conditionally() {
        let mut s = State::patterned();
        // ccmp x8, x9, #4, eq with equal operands: condition holds, so the
        // comparison result sets the flags instead of the immediate.
        let op = decode(0xFA49_0104, 0);
        s.x[9] = s.x[8];
        s.step(&op).unwrap();
        assert_eq!(s.nzcv & 0b0100, 0b0100, "equal operands set Z");

        // Same instruction with a non-equal pair: the condition fails and NZCV
        // is taken from the immediate field, here 4.
        let mut s = State::patterned();
        s.x[8] = s.x[9] ^ 1;
        s.step(&op).unwrap();
        assert_eq!(s.nzcv, 4);
    }

    #[test]
    fn logical_shifted_register_moves() {
        let mut s = State::patterned();
        // mov x0, x1
        let op = decode(0xAA01_03E0, 0);
        s.step(&op).unwrap();
        assert_eq!(s.x[0], s.x[1]);
    }

    #[test]
    fn memory_ops_read_and_write_zeroed_guest_memory() {
        let mut s = State::patterned();
        // ldr x0, [x1] on zeroed memory: the load succeeds and yields zero.
        let op = decode(0xF940_0020, 0);
        s.step(&op).unwrap();
        assert_eq!(s.x[0], 0);

        // str x2, [x1, #8] then ldr x3, [x1, #8] round-trips the value.
        let store = decode(0xF900_0422, 0);
        s.step(&store).unwrap();
        let load = decode(0xF940_0423, 0);
        s.step(&load).unwrap();
        assert_eq!(s.x[3], s.x[2]);
    }

    #[test]
    fn out_of_range_access_is_an_error_not_a_guess() {
        let mut s = State::patterned();
        s.x[1] = 0x7FFF_FFFF;
        let op = decode(0xF940_0020, 0);
        assert!(s.step(&op).is_err());
    }
}
