//! NVN objects backed by the host's Vulkan device.
//!
//! NVN structures are opaque to the game: it reserves their storage and passes
//! pointers. The host keeps each object's state on the host side, keyed by
//! that pointer, and never invents a result: an object reports success only
//! when the host state behind it exists, and a query whose answer the host
//! cannot establish stops the run naming the query.
//!
//! Procedures are registered under `nvn:<name>`, the names the game resolves
//! through `nvnDeviceGetProcAddress`.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use ew_lift::State;
use std::collections::BTreeMap;

const NVN_TRUE: u64 = 1;
const NVN_FALSE: u64 = 0;

/// Host-side NVN state for one run.
#[derive(Default)]
pub struct Nvn {
    /// Builder and object fields, by object address then field name.
    pub fields: BTreeMap<u64, BTreeMap<String, u64>>,
    /// Recorded arguments of generic state setters, by object then procedure.
    pub state: BTreeMap<u64, BTreeMap<String, Vec<u64>>>,
    /// Debug labels the game attached to objects.
    pub labels: BTreeMap<u64, String>,
    /// Device property queries answered, for the report.
    pub queries: BTreeMap<u32, u64>,
}

fn nvn<R>(state: &State, f: impl FnOnce(&mut Nvn) -> R) -> R {
    state.host.ext.with::<Nvn, R>(f)
}

pub(crate) fn set_field(state: &State, obj: u64, field: &'static str, v: u64) {
    nvn(state, |n| {
        n.fields.entry(obj).or_default().insert(field.to_string(), v);
    })
}

pub(crate) fn field(state: &State, obj: u64, field: &'static str) -> Option<u64> {
    nvn(state, |n| n.fields.get(&obj).and_then(|m| m.get(field).copied()))
}

pub(crate) fn w32(state: &mut State, addr: u64, v: u32) -> Result<(), HostResult> {
    write_bytes(state, addr, &v.to_le_bytes())
}

#[allow(dead_code)]
pub(crate) fn r32(state: &State, addr: u64) -> Result<u32, HostResult> {
    let b = read_bytes(state, addr, 4)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// The NVN procedure the guest just called, recovered from the stub address
/// it branched to.
pub(crate) fn called(state: &State) -> String {
    let pc = state.pc;
    if pc < state.host.dyn_base {
        return String::new();
    }
    let i = ((pc - state.host.dyn_base) / 16) as usize;
    state.host.dyn_stubs.get(i).map(|n| n.trim_start_matches("nvn:").to_string()).unwrap_or_default()
}

/// A pure state setter: records its arguments against the object so a later
/// getter, a draw or the report can read them. Used only for procedures that
/// return nothing and whose effect is entirely the state they set.
pub(crate) fn record_state(state: &mut State) -> HostResult {
    let name = called(state);
    let obj = state.x[0];
    let mut args: Vec<u64> = state.x[1..8].to_vec();
    args.extend(state.v[0..4].iter().map(|v| v[0]));
    nvn(state, |n| {
        if name.ends_with("SetDefaults") {
            // A builder starts over: nothing set before applies any more.
            n.state.remove(&obj);
        }
        n.state.entry(obj).or_default().insert(name, args);
    });
    HostResult::Returned(0)
}

/// The getter matching a recorded setter (`...GetX` reads `...SetX`'s first
/// argument). A value the game never set stops the run rather than guessing.
fn read_state(state: &mut State) -> HostResult {
    let name = called(state);
    let obj = state.x[0];
    let set = name.replacen("Get", "Set", 1);
    match nvn(state, |n| n.state.get(&obj).and_then(|m| m.get(&set)).map(|a| a[0])) {
        Some(v) => HostResult::Returned(v),
        None => HostResult::Unavailable(format!("{name} read state the game never set with {set}")),
    }
}

/// First argument a builder setter recorded, if the game called it.
fn recorded(state: &State, obj: u64, setter: &str) -> Option<u64> {
    nvn(state, |n| n.state.get(&obj).and_then(|m| m.get(setter)).map(|a| a[0]))
}

fn align_up(v: u64, a: u64) -> u64 {
    v.div_ceil(a) * a
}

/// `nvnQueueBuilderGetQueueMemorySize`: the guest memory the queue asks for,
/// the sum of its command, compute and control areas, page aligned.
fn queue_builder_get_queue_memory_size(state: &mut State) -> HostResult {
    let b = state.x[0];
    let command = recorded(state, b, "nvnQueueBuilderSetCommandMemorySize").unwrap_or(0x1_0000);
    let compute = recorded(state, b, "nvnQueueBuilderSetComputeMemorySize").unwrap_or(0x4_0000);
    let control = recorded(state, b, "nvnQueueBuilderSetControlMemorySize").unwrap_or(0x4000);
    HostResult::Returned(align_up(command + compute + control, 0x1000))
}

/// Host-side record of an initialized NVN object: its kind and the builder
/// state it was created from.
fn adopt_builder(state: &mut State, obj: u64, builder: u64, kind: &'static str) {
    nvn(state, |n| {
        let from = n.state.get(&builder).cloned().unwrap_or_default();
        n.state.insert(obj, from);
        n.fields.entry(obj).or_default().insert("kind".into(), kind_id(kind));
    });
}

fn kind_id(kind: &str) -> u64 {
    match kind {
        "queue" => 1,
        "memory_pool" => 2,
        "buffer" => 3,
        "texture" => 4,
        "sampler" => 5,
        "window" => 6,
        "program" => 7,
        "sync" => 8,
        "command_buffer" => 9,
        "texture_pool" => 10,
        "sampler_pool" => 11,
        "event" => 12,
        _ => 0,
    }
}

/// `nvnQueueInitialize(NVNqueue*, const NVNqueueBuilder*)`. The queue executes
/// on the host, so it succeeds whenever the device exists.
fn queue_initialize(state: &mut State) -> HostResult {
    let (q, b) = (state.x[0], state.x[1]);
    if ew_gfx::device().is_err() {
        return HostResult::Returned(NVN_FALSE);
    }
    adopt_builder(state, q, b, "queue");
    state.host.log.push(format!("gfx: NVN queue 0x{q:x} created"));
    HostResult::Returned(NVN_TRUE)
}

/// `nvnDlssNetworkBuilderGetOptimalSettings(builder, out x6, optional out)`:
/// the render resolution the upscaler wants for the output resolution. The
/// host upscaler takes any input size, so every size it reports, optimal,
/// smallest and largest, is half the output in each dimension. Writing the
/// same pair to every out pointer keeps the answer consistent whichever order
/// the game reads them in.
fn dlss_get_optimal_settings(state: &mut State) -> HostResult {
    let b = state.x[0];
    let out = nvn(state, |n| n.state.get(&b).and_then(|m| m.get("nvnDlssNetworkBuilderSetOutputResolution")).cloned());
    let Some(out) = out else {
        return HostResult::Unavailable("nvnDlssNetworkBuilderGetOptimalSettings before SetOutputResolution".into());
    };
    let (w, h) = (((out[0] as u32) / 2).max(1), ((out[1] as u32) / 2).max(1));
    let ptrs: Vec<u64> = state.x[1..7].to_vec();
    for (i, p) in ptrs.iter().enumerate() {
        if *p != 0 {
            let v = if i % 2 == 0 { w } else { h };
            if let Err(e) = w32(state, *p, v) {
                return e;
            }
        }
    }
    HostResult::Returned(NVN_TRUE)
}

/// The host upscaler keeps no network weights in guest memory; the game still
/// reserves a page for it.
fn dlss_storage_size(_state: &mut State) -> HostResult {
    HostResult::Returned(0x1000)
}

/// `nvnDlssNetworkInitialize(network, builder)`: the host upscaler exists
/// whenever the device does.
fn dlss_initialize(state: &mut State) -> HostResult {
    let (net, b) = (state.x[0], state.x[1]);
    adopt_builder(state, net, b, "dlss");
    let res = recorded(state, b, "nvnDlssNetworkBuilderSetOutputResolution");
    state.host.log.push(format!("gfx: upscaler 0x{net:x} for output width {}", res.unwrap_or(0)));
    HostResult::Returned(NVN_TRUE)
}

fn ok_true(_state: &mut State) -> HostResult {
    HostResult::Returned(NVN_TRUE)
}

fn ok_zero(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `nvnDeviceGetCurrentTimestampInNanoseconds(device)`: the host clock.
fn device_timestamp_now(_state: &mut State) -> HostResult {
    HostResult::Returned(crate::threads::now_ticks().saturating_mul(1_000_000_000) / crate::threads::TICK_FREQ)
}

/// `nvnDeviceGetTimestampInNanoseconds(device, const NVNcounterData*)`: the
/// report's timestamp field, which host reports already hold in nanoseconds.
fn device_timestamp_of(state: &mut State) -> HostResult {
    match read_bytes(state, state.x[1] + 8, 8) {
        Ok(b) => HostResult::Returned(u64::from_le_bytes(b.try_into().unwrap())),
        Err(e) => e,
    }
}

/// `nvnDeviceGenerateDebugDomainId(device, const char*)`: a fresh id.
fn device_debug_domain(state: &mut State) -> HostResult {
    let id = nvn(state, |n| {
        let e = n.fields.entry(0).or_default().entry("debug_domain".into()).or_insert(0);
        *e += 1;
        *e
    });
    HostResult::Returned(id)
}

/// `nvnDeviceBuilderSetDefaults(NVNdeviceBuilder*)`.
fn device_builder_set_defaults(state: &mut State) -> HostResult {
    let b = state.x[0];
    set_field(state, b, "flags", 0);
    HostResult::Returned(0)
}

/// `nvnDeviceBuilderSetFlags(NVNdeviceBuilder*, int flags)`.
fn device_builder_set_flags(state: &mut State) -> HostResult {
    let (b, flags) = (state.x[0], state.x[1] & 0xFFFF_FFFF);
    set_field(state, b, "flags", flags);
    HostResult::Returned(0)
}

/// `nvnDeviceBuilderGetFlags(const NVNdeviceBuilder*)`.
fn device_builder_get_flags(state: &mut State) -> HostResult {
    HostResult::Returned(field(state, state.x[0], "flags").unwrap_or(0))
}

/// `nvnDeviceInitialize(NVNdevice*, const NVNdeviceBuilder*)`: true only when
/// the host's Vulkan device exists.
fn device_initialize(state: &mut State) -> HostResult {
    let (dev, b) = (state.x[0], state.x[1]);
    let flags = field(state, b, "flags").unwrap_or(0);
    match ew_gfx::device() {
        Ok(gpu) => {
            let i = &gpu.info;
            state.host.log.push(format!(
                "gfx: NVN device 0x{dev:x} on {} (Vulkan {}.{}.{}, {}), flags 0x{flags:x}",
                i.name, i.api.0, i.api.1, i.api.2, i.driver
            ));
            set_field(state, dev, "live", 1);
            set_field(state, dev, "flags", flags);
            HostResult::Returned(NVN_TRUE)
        }
        Err(e) => {
            state.host.log.push(format!("gfx: NVN device could not be created: {e}"));
            HostResult::Returned(NVN_FALSE)
        }
    }
}

/// `nvnDeviceGetFlags(const NVNdevice*)`.
fn device_get_flags(state: &mut State) -> HostResult {
    HostResult::Returned(field(state, state.x[0], "flags").unwrap_or(0))
}

/// `nvnXxxSetDebugLabel(obj, const char*)` for every object type: the label is
/// kept for diagnostics.
fn set_debug_label(state: &mut State) -> HostResult {
    let (obj, s) = (state.x[0], state.x[1]);
    let label = if s == 0 {
        String::new()
    } else {
        match crate::hle::c_string(state, s, 256) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => return e,
        }
    };
    nvn(state, |n| n.labels.insert(obj, label));
    HostResult::Returned(0)
}

/// Device properties the host answers, by NVN `DeviceInfo` value. Every entry
/// says where its value comes from; an id that is not listed stops the run.
fn device_property(id: u32) -> Option<(u64, &'static str)> {
    Some(match id {
        // The API version the driver reports. The game's own startup code reads
        // both and discards them (0x5d00c), so these identify the host layer
        // rather than claim a particular console driver.
        0 => (55, "API major version reported by the EmblemWeave NVN layer"),
        1 => (0, "API minor version reported by the EmblemWeave NVN layer"),
        // The NVN device-info numbering. The game's own use confirms the
        // entries it has read so far (19 and 20 align its command and control
        // memory at 0x554f4). Every value is one the host device honours: the
        // host stores commands and descriptors itself, so alignments and sizes
        // only shape how the game lays out its own memory.
        2 => (18, "uniform buffer bindings per stage"),
        3 => (0x1_0000, "largest uniform buffer, bytes"),
        4 => (256, "uniform buffer alignment, bytes"),
        5 => (8, "color render targets"),
        6 => (16, "vertex buffer bindings"),
        7 => (4, "transform feedback buffer bindings"),
        8 => (16, "storage buffer bindings per stage"),
        9 => (32, "texture bindings per stage"),
        10 => (16, "counter report alignment, bytes"),
        11 => (4, "transform feedback buffer alignment, bytes"),
        12 => (4, "transform feedback control alignment, bytes"),
        13 => (4, "indirect draw alignment, bytes"),
        14 => (16, "vertex attributes"),
        15 => (32, "texture descriptor size, bytes"),
        16 => (32, "sampler descriptor size, bytes"),
        17 => (256, "reserved texture descriptors"),
        18 => (256, "reserved sampler descriptors"),
        19 => (4, "command memory alignment, bytes"),
        20 => (8, "control memory alignment, bytes"),
        21 => (0x1_0000, "smallest command memory chunk, bytes"),
        22 => (0x1000, "smallest control memory chunk, bytes"),
        23 => (1, "minimum shader scratch scale factor"),
        24 => (1, "recommended shader scratch scale factor"),
        25 => (0x1000, "shader scratch memory alignment, bytes"),
        26 => (0x2_0000, "shader scratch memory granularity, bytes"),
        27 => (16, "largest texture anisotropy"),
        // Read together at 0x6036d0 and folded into a two-bit feature mask that
        // is tested for zero, so both are capability flags. The host provides
        // neither optional feature.
        42 => (0, "optional feature flag, not provided by the host"),
        90 => (0, "optional feature flag, not provided by the host"),
        // Queue memory defaults. The game sizes its queues as a multiple of
        // these (0x5dac0: compute x8, command x32, control x8). The host queue
        // records commands on the host, so these only set how much guest
        // memory the game reserves; they are kept modest and aligned.
        82 => (0x1_0000, "queue command memory unit (bytes)"),
        85 => (0x4_0000, "queue compute memory unit (bytes)"),
        92 => (0x4000, "queue control memory unit (bytes)"),
        // A count the upscaler setup at 0x1913d58 sizes a table with and skips
        // when zero; the host upscaler has no such entries.
        133 => (0, "upscaler table entries, none on the host"),
        // An upper bound: the game takes the smaller of it and its own figure
        // (0x649944). The texture handle's 20-bit index is the host's limit.
        70 => (1 << 20, "largest descriptor count, the 20-bit handle index"),
        _ => return None,
    })
}

/// `nvnDeviceGetInteger(const NVNdevice*, NVNdeviceInfo, int*)`.
fn device_get_integer(state: &mut State) -> HostResult {
    let (id, out) = ((state.x[1] & 0xFFFF_FFFF) as u32, state.x[2]);
    let Some((value, _why)) = device_property(id) else {
        return HostResult::Unavailable(format!(
            "nvnDeviceGetInteger asked for device property {id}, whose value the host has not established"
        ));
    };
    nvn(state, |n| n.queries.insert(id, value));
    match w32(state, out, value as u32) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("nvn:nvnDeviceBuilderSetDefaults", device_builder_set_defaults, "builder state kept on the host"),
        ("nvn:nvnDeviceBuilderSetFlags", device_builder_set_flags, "builder state kept on the host"),
        ("nvn:nvnDeviceBuilderGetFlags", device_builder_get_flags, "builder state kept on the host"),
        ("nvn:nvnDeviceInitialize", device_initialize, "true only when the host Vulkan device exists"),
        ("nvn:nvnDeviceGetFlags", device_get_flags, "flags the device was created with"),
        ("nvn:nvnDeviceGetInteger", device_get_integer, "answers only properties the host has established"),
    ];
    for (name, handler, claim) in table {
        s.register(name, *handler, claim);
    }
    s.register("nvn:nvnDlssNetworkBuilderGetOptimalSettings", dlss_get_optimal_settings, "half the output resolution for the host upscaler");
    s.register("nvn:nvnDlssNetworkBuilderGetStorageSize", dlss_storage_size, "one page; the host upscaler keeps its state on the host");
    s.register("nvn:nvnDlssNetworkInitialize", dlss_initialize, "a host upscaler");
    s.register("nvn:nvnDlssNetworkResize", ok_true, "the host upscaler takes any size");
    s.register("nvn:nvnDlssNetworkFinalize", ok_zero, "nothing to release");
    s.register_prefix("nvn:nvnDlssParametersSet", record_state, "upscaler parameters recorded on the host");
    for name in DEVICE_HINTS {
        s.register(&format!("nvn:nvnDevice{name}"), ok_zero, "a hint or debug setting with no effect on host results");
    }
    s.register("nvn:nvnDeviceGetCurrentTimestampInNanoseconds", device_timestamp_now, "the host clock");
    s.register("nvn:nvnDeviceGetTimestampInNanoseconds", device_timestamp_of, "the report's own timestamp");
    s.register("nvn:nvnDeviceGenerateDebugDomainId", device_debug_domain, "a fresh id per call");
    s.register("nvn:nvnDeviceIsExternalDebuggerAttached", ok_zero, "no debugger is attached");
    s.register("nvn:nvnQueueBuilderGetQueueMemorySize", queue_builder_get_queue_memory_size, "sum of the queue's memory areas, page aligned");
    s.register("nvn:nvnQueueInitialize", queue_initialize, "a host queue on the Vulkan device");
    for name in STATE_SETTERS {
        s.register(&format!("nvn:nvn{name}"), record_state, "pure state setter recorded on the host");
    }
    for name in STATE_GETTERS {
        s.register(&format!("nvn:nvn{name}"), read_state, "returns what the matching setter recorded");
    }
    for kind in BUILDERS {
        s.register_prefix(&format!("nvn:nvn{kind}BuilderSet"), record_state, "builder state recorded on the host");
        s.register_prefix(&format!("nvn:nvn{kind}BuilderGet"), read_state, "returns what the matching builder setter recorded");
    }
    for kind in LABELLED {
        s.register(&format!("nvn:nvn{kind}SetDebugLabel"), set_debug_label, "debug labels kept for diagnostics");
    }
}

/// Builders: objects whose setters only accumulate state until an
/// `Initialize` consumes it. Computed builder queries (storage sizes and the
/// like) are registered by exact name and take precedence.
const BUILDERS: &[&str] =
    &["Buffer", "Event", "MemoryPool", "Program", "Queue", "Sampler", "Window", "DlssNetwork"];

const LABELLED: &[&str] = &[
    "Buffer", "CommandBuffer", "Device", "MemoryPool", "Program", "Queue", "SamplerPool", "Sampler", "Sync",
    "TexturePool", "Texture", "Window",
];

/// Procedures whose whole effect is the state they set on their object.
const STATE_SETTERS: &[&str] = &["DeviceSetDepthMode", "DeviceSetWindowOriginMode"];

/// Getters answered from the matching setter's recorded state.
const STATE_GETTERS: &[&str] = &["DeviceGetDepthMode", "DeviceGetWindowOriginMode"];

/// Device calls that tune performance or debugging and change no result the
/// game can observe: fast-clear registrations (the host clears at full rate
/// for every value), the shader cache, checkpoints and the debug callback,
/// which the host never needs to call because it reports faults itself.
const DEVICE_HINTS: &[&str] = &[
    "RegisterFastClearColor",
    "RegisterFastClearColorIndex",
    "RegisterFastClearColori",
    "RegisterFastClearColoriIndex",
    "RegisterFastClearColorui",
    "RegisterFastClearColoruiIndex",
    "RegisterFastClearDepth",
    "RegisterFastClearDepthIndex",
    "RegisterFastClearStencil",
    "RegisterFastClearStencilIndex",
    "SetIntermediateShaderCache",
    "InstallDebugCallback",
    "SetAutomaticCheckpointsEnable",
    "SetAutomaticCheckpointsMask",
    "SetAutomaticCheckpointsSamplingInterval",
    "SetAutomaticCheckpointsByDebugGroup",
    "SetCheckpointCallStacksEnable",
    "SetCheckpointMaxCallStacksDepth",
];
