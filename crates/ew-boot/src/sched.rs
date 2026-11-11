//! The guest thread scheduler.
//!
//! One guest thread runs on the interpreter at a time. The others are saved
//! register files. A thread leaves the CPU when its quantum expires, when a
//! host service blocks it, or when its function returns; the next thread is
//! chosen round robin. A blocked thread is reconsidered by calling its
//! continuation with its own registers loaded, so the service that blocked it
//! decides, with the guest's current memory, whether it may continue.
//!
//! Scheduling is deterministic for a given sequence of host clock readings: the
//! quantum counts guest instructions, not wall time.

use crate::hle::{Handler, HostResult};
use ew_lift::State;

/// Guest instructions a thread runs before another runnable thread gets a turn.
pub const QUANTUM: u64 = 200_000;
/// Return address given to every thread entry. Reaching it means the thread's
/// function returned, which ends the thread.
pub const THREAD_EXIT: u64 = 0xFFFF_FFFF_FFFF_FFF0;

#[derive(Clone)]
pub struct Context {
    pub x: [u64; 31],
    pub v: [[u64; 2]; 32],
    pub sp: u64,
    pub pc: u64,
    pub nzcv: u32,
    pub tls: [u64; 16],
    /// The thread-local region (`tpidrro_el0`), `tpidr_el0`, and the
    /// floating-point control and status registers.
    pub tpidrro: u64,
    pub tpidr: u64,
    pub fpcr: u64,
    pub fpsr: u64,
}

/// Bytes of a thread-local region, as the kernel gives each thread.
pub const TLS_SIZE: u64 = 0x200;
/// Offset of the kernel thread handle in the thread-local region. nnSdk's
/// mutexes use it as the owner tag (`ldr w, [tpidrro, #0x110]`).
pub const TLS_HANDLE: u64 = 0x110;
/// Offset of the thread's `nn::os::ThreadType` pointer, which nnSdk's own
/// code reads for the current thread.
pub const TLS_THREAD: u64 = 0x1F8;

/// Create a thread-local region for a new thread, holding its handle and its
/// `ThreadType` address.
pub fn new_tls(state: &mut State, handle: u32, thread: u64) -> Result<u64, String> {
    let at = state.host.allocate(TLS_SIZE, TLS_SIZE)?;
    let o = (at + TLS_HANDLE) as usize;
    state.mem[at as usize..(at + TLS_SIZE) as usize].fill(0);
    state.mem[o..o + 4].copy_from_slice(&handle.to_le_bytes());
    let t = (at + TLS_THREAD) as usize;
    state.mem[t..t + 8].copy_from_slice(&thread.to_le_bytes());
    Ok(at)
}

/// Thread handles: non-zero and clear of bit 30, which a lock word uses to
/// mark waiters.
pub fn handle_for(index: u64) -> u32 {
    0x0001_0000 | (index as u32 & 0xFFFF)
}

impl Context {
    pub fn save(state: &State) -> Context {
        Context {
            x: state.x,
            v: state.v,
            sp: state.sp,
            pc: state.pc,
            nzcv: state.nzcv,
            tls: state.host.tls,
            tpidrro: state.tpidrro,
            tpidr: state.tpidr,
            fpcr: state.fpcr,
            fpsr: state.fpsr,
        }
    }
    pub fn load(&self, state: &mut State) {
        state.x = self.x;
        state.v = self.v;
        state.sp = self.sp;
        state.pc = self.pc;
        state.nzcv = self.nzcv;
        state.host.tls = self.tls;
        state.tpidrro = self.tpidrro;
        state.tpidr = self.tpidr;
        state.fpcr = self.fpcr;
        state.fpsr = self.fpsr;
        // A context switch clears the exclusive monitor, as an exception
        // return does on hardware, so a stxr never pairs with another
        // thread's ldxr.
        state.exclusive = None;
    }
}

#[derive(Clone)]
pub enum Status {
    Ready,
    /// Parked in a host service. `return_to` is the caller's return address.
    Blocked { resume: Handler, timed: bool, return_to: u64, call: String },
    Exited,
}

pub struct Thread {
    /// The guest `ThreadType` address, which is also the thread's identity.
    pub object: u64,
    pub ctx: Context,
    pub status: Status,
}

pub struct Scheduler {
    pub threads: Vec<Thread>,
    pub current: usize,
    pub slice_start: u64,
    pub switches: u64,
    pub created: u64,
}

/// Why no thread could be scheduled.
pub enum NoRunnable {
    /// Every live thread waits on something only another thread could provide.
    Deadlock(String),
    /// Every thread has exited.
    AllExited,
    /// A continuation reported that its call cannot be served.
    Fault { call: String, message: String },
}

impl Scheduler {
    pub fn new(main_object: u64, state: &State) -> Scheduler {
        Scheduler {
            threads: vec![Thread { object: main_object, ctx: Context::save(state), status: Status::Ready }],
            current: 0,
            slice_start: state.steps,
            switches: 0,
            created: 1,
        }
    }

    pub fn current_object(&self) -> u64 {
        self.threads[self.current].object
    }

    pub fn is_main(&self) -> bool {
        self.current == 0
    }

    /// Give a context to every thread the guest has started since last time.
    pub fn adopt_started(&mut self, state: &mut State) {
        let fresh: Vec<(u64, u64, u64, u64)> = state
            .host
            .threads
            .iter()
            .filter(|(object, info)| info.started && !info.exited && !self.threads.iter().any(|t| t.object == **object))
            .map(|(&object, info)| (object, info.arg, info.stack_top, info.entry))
            .collect();
        for (object, arg, stack_top, entry) in fresh {
            let mut ctx = Context::save(state);
            ctx.x = [0; 31];
            ctx.x[0] = arg;
            ctx.x[30] = THREAD_EXIT;
            ctx.v = [[0; 2]; 32];
            ctx.sp = stack_top;
            ctx.pc = entry;
            ctx.nzcv = 0;
            ctx.tls = [0; 16];
            ctx.tpidr = 0;
            ctx.fpcr = 0;
            ctx.fpsr = 0;
            ctx.tpidrro = new_tls(state, handle_for(self.created), object).unwrap_or(0);
            self.threads.push(Thread { object, ctx, status: Status::Ready });
            self.created += 1;
        }
    }

    pub fn runnable_others(&self) -> bool {
        self.threads
            .iter()
            .enumerate()
            .any(|(i, t)| i != self.current && !matches!(t.status, Status::Exited))
    }

    /// Park the running thread with `status` and switch to the next thread that
    /// can run. On success the chosen thread's registers are in `state`.
    pub fn switch(&mut self, state: &mut State, status: Status) -> Result<(), NoRunnable> {
        self.threads[self.current].ctx = Context::save(state);
        self.threads[self.current].status = status;
        let n = self.threads.len();
        let mut idle_rounds = 0u64;
        loop {
            let mut any_timed = false;
            let mut any_live = false;
            for k in 1..=n {
                let i = (self.current + k) % n;
                let status = self.threads[i].status.clone();
                match status {
                    Status::Exited => {}
                    Status::Ready => {
                        self.enter(state, i);
                        return Ok(());
                    }
                    Status::Blocked { resume, timed, return_to, call } => {
                        any_live = true;
                        self.threads[i].ctx.load(state);
                        state.host.thread_object = self.threads[i].object;
                        match resume(state) {
                            HostResult::Returned(v) => {
                                state.x[0] = v;
                                state.pc = return_to;
                                self.threads[i].ctx = Context::save(state);
                                self.enter(state, i);
                                return Ok(());
                            }
                            HostResult::Jump(target) => {
                                state.pc = target;
                                self.threads[i].ctx = Context::save(state);
                                self.enter(state, i);
                                return Ok(());
                            }
                            HostResult::Block { resume, timed: t } => {
                                any_timed |= t;
                                self.threads[i].ctx = Context::save(state);
                                self.threads[i].status = Status::Blocked { resume, timed: t, return_to, call };
                            }
                            HostResult::Unavailable(message) => {
                                return Err(NoRunnable::Fault { call, message });
                            }
                        }
                        let _ = timed;
                    }
                }
            }
            if !any_live {
                return Err(NoRunnable::AllExited);
            }
            if !any_timed {
                return Err(NoRunnable::Deadlock(self.describe_waits()));
            }
            // Only the clock can release a thread now. Sleep briefly instead of
            // spinning, then look again.
            idle_rounds += 1;
            std::thread::sleep(std::time::Duration::from_micros(if idle_rounds < 64 { 100 } else { 1000 }));
        }
    }

    fn enter(&mut self, state: &mut State, i: usize) {
        if i != self.current {
            self.switches += 1;
        }
        self.current = i;
        self.threads[i].ctx.load(state);
        self.threads[i].status = Status::Ready;
        state.host.thread_object = self.threads[i].object;
        self.slice_start = state.steps;
    }

    /// One line per thread: identity, state and where it is.
    pub fn summary(&self, state: &State) -> Vec<String> {
        self.threads
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let pc = if i == self.current { state.pc } else { t.ctx.pc };
                let what = match &t.status {
                    _ if i == self.current => "running".to_string(),
                    Status::Ready => "ready".to_string(),
                    Status::Blocked { call, timed, .. } => {
                        format!("waiting in {call}{}", if *timed { " (timed)" } else { "" })
                    }
                    Status::Exited => "exited".to_string(),
                };
                let entry = state.host.threads.get(&t.object).map(|i| i.entry).unwrap_or(0);
                // Callers, from the frame-pointer chain: the return address of
                // the blocked call, then each saved link register.
                let (mut fp, ret) = match &t.status {
                    _ if i == self.current => (state.x[29], state.x[30]),
                    Status::Blocked { return_to, .. } => (t.ctx.x[29], *return_to),
                    _ => (t.ctx.x[29], t.ctx.x[30]),
                };
                let read = |a: u64| {
                    let o = a as usize;
                    state.mem.get(o..o + 8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
                };
                let mut chain = vec![format!("0x{ret:x}")];
                for _ in 0..10 {
                    let (Some(next), Some(lr)) = (read(fp), read(fp.wrapping_add(8))) else { break };
                    if lr == 0 || next <= fp {
                        break;
                    }
                    chain.push(format!("0x{lr:x}"));
                    fp = next;
                }
                format!("thread 0x{:x} (entry 0x{entry:x}) {what}, pc 0x{pc:x}, callers {}", t.object, chain.join(" < "))
            })
            .collect()
    }

    pub fn describe_waits(&self) -> String {
        self.threads
            .iter()
            .filter_map(|t| match &t.status {
                Status::Blocked { call, .. } => Some(format!("thread 0x{:x} in {call}", t.object)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("; ")
    }
}
