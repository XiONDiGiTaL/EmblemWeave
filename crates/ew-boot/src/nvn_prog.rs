//! NVN samplers, programs and events on the host.
//!
//! A program holds the game's compiled shaders: per stage, the GPU address of
//! the code and a pointer to its control section. The host records both so the
//! renderer can translate the stages when a draw uses them; nothing is
//! translated here, and the run report counts programs created.
//!
//! An event is a 32-bit value in pool memory that queues and the CPU signal
//! and read, so it lives in guest memory exactly where the game put it.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use crate::nvn::called;
use ew_lift::State;
use std::collections::BTreeMap;

const NVN_TRUE: u64 = 1;

#[derive(Debug, Clone, Default)]
pub struct Program {
    /// Per stage: GPU address of the code and the control section, copied.
    pub stages: Vec<(u64, u64)>,
}

#[derive(Default)]
pub struct Programs {
    pub programs: BTreeMap<u64, Program>,
    pub samplers: BTreeMap<u64, u64>,
    /// Events: object to guest address of its 32-bit value.
    pub events: BTreeMap<u64, u64>,
    pub shader_sets: u64,
}

fn progs<R>(state: &State, f: impl FnOnce(&mut Programs) -> R) -> R {
    state.host.ext.with::<Programs, R>(f)
}

fn recorded_args(state: &State, obj: u64, setter: &str) -> Option<Vec<u64>> {
    state.host.ext.with::<crate::nvn::Nvn, _>(|n| n.state.get(&obj).and_then(|m| m.get(setter)).cloned())
}

/// `nvnSamplerInitialize(NVNsampler*, const NVNsamplerBuilder*)`: the sampler
/// keeps its builder's state.
fn sampler_initialize(state: &mut State) -> HostResult {
    let (s, b) = (state.x[0], state.x[1]);
    state.host.ext.with::<crate::nvn::Nvn, _>(|n| {
        let from = n.state.get(&b).cloned().unwrap_or_default();
        n.state.insert(s, from);
    });
    progs(state, |p| p.samplers.insert(s, b));
    HostResult::Returned(NVN_TRUE)
}

fn sampler_finalize(state: &mut State) -> HostResult {
    let s = state.x[0];
    progs(state, |p| p.samplers.remove(&s));
    HostResult::Returned(0)
}

fn program_initialize(state: &mut State) -> HostResult {
    let p = state.x[0];
    progs(state, |x| x.programs.insert(p, Program::default()));
    HostResult::Returned(NVN_TRUE)
}

fn program_finalize(state: &mut State) -> HostResult {
    let p = state.x[0];
    progs(state, |x| x.programs.remove(&p));
    HostResult::Returned(0)
}

/// `nvnProgramSetShaders(NVNprogram*, int count, const NVNshaderData*)`: each
/// entry is the code's GPU address and a pointer to its control section.
fn program_set_shaders(state: &mut State) -> HostResult {
    let (p, count, list) = (state.x[0], state.x[1] & 0xFFFF_FFFF, state.x[2]);
    if count > 6 {
        return HostResult::Unavailable(format!("{} with {count} stages", called(state)));
    }
    let raw = match read_bytes(state, list, count * 16) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let stages: Vec<(u64, u64)> = raw
        .chunks(16)
        .map(|c| (u64::from_le_bytes(c[0..8].try_into().unwrap()), u64::from_le_bytes(c[8..16].try_into().unwrap())))
        .collect();
    let first = progs(state, |x| {
        x.shader_sets += 1;
        x.programs.entry(p).or_default().stages = stages;
        x.shader_sets == 1
    });
    if first {
        state.host.log.push(format!("gfx: first shader program 0x{p:x} set with {count} stage(s)"));
    }
    HostResult::Returned(NVN_TRUE)
}

/// `nvnProgramSetShadersExt(NVNprogram*, int count, const NVNshaderDataExt*)`:
/// 24-byte records (code GPU address, control section, one more word, laid out
/// by the game at 0x6045e0), recorded like `SetShaders`.
fn program_set_shaders_ext(state: &mut State) -> HostResult {
    let (p, count, list) = (state.x[0], state.x[1] & 0xFFFF_FFFF, state.x[2]);
    if count > 9 {
        return HostResult::Unavailable(format!("nvnProgramSetShadersExt with {count} stages"));
    }
    let raw = match read_bytes(state, list, count * 24) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let stages: Vec<(u64, u64)> = raw
        .chunks(24)
        .map(|c| (u64::from_le_bytes(c[0..8].try_into().unwrap()), u64::from_le_bytes(c[8..16].try_into().unwrap())))
        .collect();
    let first = progs(state, |x| {
        x.shader_sets += 1;
        x.programs.entry(p).or_default().stages = stages;
        x.shader_sets == 1
    });
    if first {
        state.host.log.push(format!("gfx: first shader program 0x{p:x} set with {count} stage(s)"));
    }
    HostResult::Returned(NVN_TRUE)
}

/// `nvnEventInitialize(NVNevent*, const NVNeventBuilder*)`: the event's value
/// sits at the builder's pool and offset.
fn event_initialize(state: &mut State) -> HostResult {
    let (e, b) = (state.x[0], state.x[1]);
    let Some(st) = recorded_args(state, b, "nvnEventBuilderSetStorage") else {
        return HostResult::Unavailable("nvnEventInitialize with no storage set".into());
    };
    let (pool, offset) = (st[0], st[1]);
    let at = state.host.ext.with::<crate::nvn_mem::Memory, _>(|m| {
        m.pools.get(&pool).filter(|p| p.storage != 0).map(|p| p.storage + offset)
    });
    let Some(at) = at else {
        return HostResult::Unavailable(format!("nvnEventInitialize on pool 0x{pool:x}, which has no CPU storage"));
    };
    progs(state, |x| x.events.insert(e, at));
    HostResult::Returned(NVN_TRUE)
}

fn event_addr(state: &State, e: u64) -> Option<u64> {
    progs(state, |x| x.events.get(&e).copied())
}

fn event_get_value(state: &mut State) -> HostResult {
    let e = state.x[0];
    let Some(at) = event_addr(state, e) else {
        return HostResult::Unavailable(format!("nvnEventGetValue on event 0x{e:x}, which was never initialized"));
    };
    match read_bytes(state, at, 4) {
        Ok(b) => HostResult::Returned(u32::from_le_bytes(b.try_into().unwrap()) as u64),
        Err(e) => e,
    }
}

/// NVN event signal modes: write the value, or add it.
const SIGNAL_ADD: u64 = 1;

/// Signal an event from the CPU or from an executed command.
pub fn signal(state: &mut State, e: u64, mode: u64, value: u32) -> Result<(), HostResult> {
    let Some(at) = event_addr(state, e) else {
        return Err(HostResult::Unavailable(format!("signal of event 0x{e:x}, which was never initialized")));
    };
    let old = u32::from_le_bytes(read_bytes(state, at, 4)?.try_into().unwrap());
    let new = if mode == SIGNAL_ADD { old.wrapping_add(value) } else { value };
    write_bytes(state, at, &new.to_le_bytes())
}

/// `nvnEventSignal(NVNevent*, NVNeventSignalMode, uint32 value)`.
fn event_signal(state: &mut State) -> HostResult {
    let (e, mode, value) = (state.x[0], state.x[1] & 0xFFFF_FFFF, state.x[2] as u32);
    match signal(state, e, mode, value) {
        Ok(()) => HostResult::Returned(0),
        Err(err) => err,
    }
}

fn event_finalize(state: &mut State) -> HostResult {
    let e = state.x[0];
    progs(state, |x| x.events.remove(&e));
    HostResult::Returned(0)
}

/// Where a sampler getter's values come from: the setter, then for each out
/// pointer the recorded argument (x1.. are 0..6, v0.. are 7..10) and the
/// value used when the game never called the setter.
struct Getter {
    name: &'static str,
    setter: &'static str,
    outs: &'static [(usize, u64)],
    /// Returned instead of written: (argument, default, in v0 as a float).
    ret: Option<(usize, u64, bool)>,
}

const ONE_F32: u64 = 0x3F80_0000;
const LOD_MAX_F32: u64 = 0x447A_0000; // 1000.0

const SAMPLER_GETTERS: &[Getter] = &[
    Getter { name: "MinMagFilter", setter: "MinMagFilter", outs: &[(0, 0), (1, 0)], ret: None },
    Getter { name: "WrapMode", setter: "WrapMode", outs: &[(0, 0), (1, 0), (2, 0)], ret: None },
    Getter { name: "LodClamp", setter: "LodClamp", outs: &[(7, 0), (8, LOD_MAX_F32)], ret: None },
    Getter { name: "Compare", setter: "Compare", outs: &[(0, 0), (1, 0)], ret: None },
    Getter { name: "LodBias", setter: "LodBias", outs: &[], ret: Some((7, 0, true)) },
    Getter { name: "MaxAnisotropy", setter: "MaxAnisotropy", outs: &[], ret: Some((7, ONE_F32, true)) },
    Getter { name: "ReductionFilter", setter: "ReductionFilter", outs: &[], ret: Some((0, 0, false)) },
];

/// `nvnSamplerGet*`: the value its builder set, or the default.
fn sampler_get(state: &mut State) -> HostResult {
    let name = called(state);
    let smp = state.x[0];
    let field = name.trim_start_matches("nvnSamplerGet");
    if field.starts_with("BorderColor") {
        // (sampler, out[4]): copy the 16 bytes the setter pointed at.
        let rec = recorded_args(state, smp, &format!("nvnSamplerBuilderSet{field}"));
        let bytes = match rec.map(|a| a[0]).filter(|p| *p != 0) {
            Some(p) => match read_bytes(state, p, 16) {
                Ok(b) => b,
                Err(e) => return e,
            },
            None => vec![0u8; 16],
        };
        return match write_bytes(state, state.x[1], &bytes) {
            Ok(()) => HostResult::Returned(0),
            Err(e) => e,
        };
    }
    let Some(g) = SAMPLER_GETTERS.iter().find(|g| g.name == field) else {
        return HostResult::Unavailable(format!("{name} is not modelled"));
    };
    let rec = recorded_args(state, smp, &format!("nvnSamplerBuilderSet{}", g.setter));
    let arg = |i: usize, d: u64| rec.as_ref().map(|a| a[i]).unwrap_or(d);
    for (k, &(i, d)) in g.outs.iter().enumerate() {
        let out = state.x[1 + k];
        if out != 0 {
            if let Err(e) = write_bytes(state, out, &(arg(i, d) as u32).to_le_bytes()) {
                return e;
            }
        }
    }
    match g.ret {
        Some((i, d, true)) => {
            state.v[0] = [arg(i, d) & 0xFFFF_FFFF, 0];
            HostResult::Returned(state.x[0])
        }
        Some((i, d, false)) => HostResult::Returned(arg(i, d) & 0xFFFF_FFFF),
        None => HostResult::Returned(0),
    }
}

/// The sampler state that matters, as the getters see it.
fn sampler_key(state: &State, smp: u64) -> Vec<u64> {
    let mut key = Vec::new();
    for g in SAMPLER_GETTERS {
        let rec = recorded_args(state, smp, &format!("nvnSamplerBuilderSet{}", g.setter));
        for &(i, d) in g.outs {
            key.push(rec.as_ref().map(|a| a[i]).unwrap_or(d) & 0xFFFF_FFFF);
        }
        if let Some((i, d, _)) = g.ret {
            key.push(rec.as_ref().map(|a| a[i]).unwrap_or(d) & 0xFFFF_FFFF);
        }
    }
    let border = recorded_args(state, smp, "nvnSamplerBuilderSetBorderColor").map(|a| a[0]).filter(|p| *p != 0);
    key.extend(border.and_then(|p| read_bytes(state, p, 16).ok()).unwrap_or_default().iter().map(|b| *b as u64));
    key
}

/// `nvnSamplerCompare(const NVNsampler*, const NVNsampler*)`.
fn sampler_compare(state: &mut State) -> HostResult {
    let (a, b) = (state.x[0], state.x[1]);
    HostResult::Returned((sampler_key(state, a) == sampler_key(state, b)) as u64)
}

pub fn register(s: &mut HostServices) {
    s.register("nvn:nvnSamplerCompare", sampler_compare, "equal when every getter would report the same state");
    s.register_prefix("nvn:nvnSamplerGet", sampler_get, "the value the sampler's builder set, or its default");
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("nvnSamplerInitialize", sampler_initialize, "a sampler with its builder's state"),
        ("nvnSamplerFinalize", sampler_finalize, "forgets the sampler"),
        ("nvnProgramInitialize", program_initialize, "a program record on the host"),
        ("nvnProgramFinalize", program_finalize, "forgets the program"),
        ("nvnProgramSetShaders", program_set_shaders, "stage code addresses and control sections recorded for translation"),
        ("nvnProgramSetShadersExt", program_set_shaders_ext, "stage code addresses and control sections recorded for translation"),
        ("nvnEventInitialize", event_initialize, "a 32-bit value in the game's pool memory"),
        ("nvnEventFinalize", event_finalize, "forgets the event"),
        ("nvnEventGetValue", event_get_value, "reads the value from guest memory"),
        ("nvnEventSignal", event_signal, "writes or adds to the value in guest memory"),
    ];
    for (name, handler, claim) in table {
        s.register(&format!("nvn:{name}"), *handler, claim);
    }
}
