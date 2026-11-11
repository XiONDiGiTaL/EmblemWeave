//! NVN command buffers and queue submission on the host.
//!
//! Commands are recorded on the host, not into the guest memory the game gives
//! the command buffer: every call between `BeginRecording` and `EndRecording`
//! becomes a [`Cmd`] holding its arguments and a copy of any guest memory they
//! point at, so the game is free to reuse its stack and scratch memory. The
//! guest command and control memory is accounted as given and never consumed,
//! which is why the out-of-memory callback never fires.
//!
//! `QueueSubmitCommands` executes the commands in order. Commands whose effect
//! is on memory the game can read back (buffer copies and clears, uniform
//! updates, reports, fences, events) are carried out on guest memory here.
//! Drawing, clearing render targets and texture copies are counted and kept
//! for the renderer; nothing is drawn yet, and the run report says so.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use crate::nvn::called;
use ew_lift::State;
use std::collections::{BTreeMap, VecDeque};

const NVN_TRUE: u64 = 1;

/// One recorded command.
#[derive(Debug, Clone)]
pub struct Cmd {
    /// The procedure, as its index among the run-time host functions.
    pub op: u32,
    pub x: [u64; 7],
    pub v: [u64; 4],
    /// Guest memory the arguments pointed at, copied when recorded.
    pub data: Vec<Vec<u8>>,
}

#[derive(Default)]
pub struct CommandBuffer {
    pub recording: Option<Vec<Cmd>>,
    pub command_memory: u64,
    pub control_memory: u64,
    pub callback: u64,
    pub callback_data: u64,
}

#[derive(Default)]
pub struct Commands {
    pub buffers: BTreeMap<u64, CommandBuffer>,
    /// Finished command sets by handle.
    pub handles: BTreeMap<u64, Vec<Cmd>>,
    /// Handles in creation order, for retiring old ones.
    pub order: VecDeque<u64>,
    pub next_handle: u64,
    /// Totals for the run report.
    pub submitted: u64,
    pub executed: BTreeMap<String, u64>,
}

/// Handles stay valid for this many newer ones; NVN invalidates a handle once
/// its command memory is reused, which happens far sooner in practice.
const LIVE_HANDLES: usize = 1 << 16;

const HANDLE_BASE: u64 = 0x7E00_0000_0000;

fn cmds<R>(state: &State, f: impl FnOnce(&mut Commands) -> R) -> R {
    state.host.ext.with::<Commands, R>(f)
}

fn op_index(state: &State) -> u32 {
    ((state.pc - state.host.dyn_base) / 16) as u32
}

pub fn op_name(state: &State, op: u32) -> String {
    state.host.dyn_stubs.get(op as usize).map(|n| n.trim_start_matches("nvn:").to_string()).unwrap_or_default()
}

/// Guest memory a command's arguments point at: (pointer argument, count
/// argument or none, bytes per element or fixed size). Indices are into x[].
fn pointer_args(name: &str) -> &'static [(usize, Option<usize>, u64)] {
    match name.trim_start_matches("nvnCommandBuffer") {
        "BindVertexBuffers" | "BindTransformFeedbackBuffers" => &[(3, Some(2), 16)],
        "BindUniformBuffers" | "BindStorageBuffers" | "BindUniformBuffersDeferred" | "BindStorageBuffersDeferred" => {
            &[(4, Some(3), 16)]
        }
        "BindTextures" | "BindImages" | "BindSeparateTextures" | "BindSeparateSamplers" | "BindTexturesDeferred"
        | "BindImagesDeferred" | "BindSeparateTexturesDeferred" | "BindSeparateSamplersDeferred" => &[(4, Some(3), 8)],
        "SetViewports" | "SetScissors" => &[(3, Some(2), 16)],
        "SetDepthRanges" => &[(3, Some(2), 8)],
        "ClearColor" | "ClearColori" | "ClearColorui" => &[(2, None, 16)],
        "SetBlendColor" => &[(1, None, 16)],
        "SetRenderTargets" => &[(2, Some(1), 8), (3, Some(1), 8)],
        "UpdateUniformBuffer" => &[(5, Some(4), 1)],
        "CopyBufferToTexture" => &[(4, None, 24)],
        "CopyTextureToBuffer" => &[(3, None, 24)],
        "CopyTextureToTexture" => &[(3, None, 24), (6, None, 24)],
        "DrawTexture" => &[(2, None, 16), (3, None, 16)],
        "ClearTexture" | "ClearTexturei" | "ClearTextureui" => &[(3, None, 24), (4, None, 16)],
        "BindVertexAttribState" => &[(2, Some(1), 4)],
        "BindVertexStreamState" => &[(2, Some(1), 8)],
        "BindChannelMaskState" | "BindColorState" | "BindPolygonState" => &[(1, None, 4)],
        "BindBlendState" | "BindDepthStencilState" | "BindMultisampleState" => &[(1, None, 8)],
        _ => &[],
    }
}

/// Every recordable `nvnCommandBuffer*` procedure.
fn record(state: &mut State) -> HostResult {
    let name = called(state);
    let cb = state.x[0];
    let mut data = Vec::new();
    for &(ptr, count, size) in pointer_args(&name) {
        let p = state.x[ptr];
        let n = count.map(|c| state.x[c] & 0xFFFF_FFFF).unwrap_or(1);
        if p == 0 || n == 0 {
            data.push(Vec::new());
            continue;
        }
        match read_bytes(state, p, n * size) {
            Ok(b) => data.push(b),
            Err(e) => return e,
        }
    }
    let mut x = [0u64; 7];
    x.copy_from_slice(&state.x[1..8]);
    let v = [state.v[0][0], state.v[1][0], state.v[2][0], state.v[3][0]];
    let cmd = Cmd { op: op_index(state), x, v, data };
    let ok = cmds(state, |c| match c.buffers.get_mut(&cb).and_then(|b| b.recording.as_mut()) {
        Some(list) => {
            list.push(cmd);
            true
        }
        None => false,
    });
    if ok {
        HostResult::Returned(0)
    } else {
        HostResult::Unavailable(format!("{name} on command buffer 0x{cb:x}, which is not recording"))
    }
}

fn cb_initialize(state: &mut State) -> HostResult {
    let cb = state.x[0];
    cmds(state, |c| c.buffers.insert(cb, CommandBuffer::default()));
    HostResult::Returned(NVN_TRUE)
}

fn cb_finalize(state: &mut State) -> HostResult {
    let cb = state.x[0];
    cmds(state, |c| c.buffers.remove(&cb));
    HostResult::Returned(0)
}

fn with_cb(state: &mut State, f: impl FnOnce(&mut CommandBuffer, &[u64]) -> u64) -> HostResult {
    let cb = state.x[0];
    let args = state.x[1..8].to_vec();
    match cmds(state, |c| c.buffers.get_mut(&cb).map(|b| f(b, &args))) {
        Some(v) => HostResult::Returned(v),
        None => HostResult::Unavailable(format!("{} on command buffer 0x{cb:x}, which was never initialized", called(state))),
    }
}

fn add_command_memory(state: &mut State) -> HostResult {
    with_cb(state, |b, a| {
        b.command_memory += a[2];
        0
    })
}

fn add_control_memory(state: &mut State) -> HostResult {
    with_cb(state, |b, a| {
        b.control_memory += a[1];
        0
    })
}

fn command_memory_size(state: &mut State) -> HostResult {
    with_cb(state, |b, _| b.command_memory)
}

fn control_memory_size(state: &mut State) -> HostResult {
    with_cb(state, |b, _| b.control_memory)
}

fn memory_used(state: &mut State) -> HostResult {
    with_cb(state, |_, _| 0)
}

fn set_memory_callback(state: &mut State) -> HostResult {
    with_cb(state, |b, a| {
        b.callback = a[0];
        0
    })
}

fn set_memory_callback_data(state: &mut State) -> HostResult {
    with_cb(state, |b, a| {
        b.callback_data = a[0];
        0
    })
}

fn get_memory_callback(state: &mut State) -> HostResult {
    with_cb(state, |b, _| b.callback)
}

fn get_memory_callback_data(state: &mut State) -> HostResult {
    with_cb(state, |b, _| b.callback_data)
}

fn ignore_setting(state: &mut State) -> HostResult {
    with_cb(state, |_, _| 0)
}

fn begin_recording(state: &mut State) -> HostResult {
    with_cb(state, |b, _| {
        b.recording = Some(Vec::new());
        0
    })
}

fn is_recording(state: &mut State) -> HostResult {
    with_cb(state, |b, _| b.recording.is_some() as u64)
}

/// `nvnCommandBufferEndRecording(cb)`: the commands become a handle.
fn end_recording(state: &mut State) -> HostResult {
    let cb = state.x[0];
    let handle = cmds(state, |c| {
        let list = c.buffers.get_mut(&cb)?.recording.take()?;
        c.next_handle += 1;
        let h = HANDLE_BASE + c.next_handle * 0x100;
        c.handles.insert(h, list);
        c.order.push_back(h);
        while c.order.len() > LIVE_HANDLES {
            if let Some(old) = c.order.pop_front() {
                c.handles.remove(&old);
            }
        }
        Some(h)
    });
    match handle {
        Some(h) => HostResult::Returned(h),
        None => HostResult::Unavailable(format!("nvnCommandBufferEndRecording on 0x{cb:x}, which is not recording")),
    }
}

/// `nvnDeviceFinalizeCommandHandle(device, handle)`: the game is done with
/// the commands behind a handle, so the host list is dropped.
fn finalize_command_handle(state: &mut State) -> HostResult {
    let h = state.x[1];
    cmds(state, |c| {
        c.handles.remove(&h);
        c.order.retain(|o| *o != h);
    });
    HostResult::Returned(0)
}

/// A storage-backed pool's GPU address is its guest address; anything else
/// cannot be reached from the CPU.
fn guest_of(state: &State, gpu: u64, len: u64) -> Option<u64> {
    state.host.ext.with::<crate::nvn_mem::Memory, _>(|m| {
        m.pools.values().find_map(|p| {
            (p.storage != 0 && gpu >= p.gpu && gpu + len <= p.gpu + p.size).then(|| p.storage + (gpu - p.gpu))
        })
    })
}

fn now_ns() -> u64 {
    crate::threads::now_ticks().saturating_mul(1_000_000_000) / crate::threads::TICK_FREQ
}

/// Run one command. Memory effects happen now; everything else is counted.
fn execute(state: &mut State, cmd: &Cmd, depth: u32) -> Result<(), HostResult> {
    let name = op_name(state, cmd.op);
    let short = name.trim_start_matches("nvnCommandBuffer").to_string();
    let x = &cmd.x;
    match short.as_str() {
        "CallCommands" | "CopyCommands" => {
            // (cb, count, const NVNcommandHandle*): the handles were read at
            // record time only as a pointer, so read them now.
            let (n, p) = (x[0] & 0xFFFF_FFFF, x[1]);
            let raw = read_bytes(state, p, n * 8)?;
            for h in raw.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())) {
                run_handle(state, h, depth + 1)?;
            }
        }
        "CopyBufferToBuffer" => {
            // (cb, src, dst, size, flags)
            let (src, dst, size) = (x[0], x[1], x[2]);
            if let (Some(s), Some(d)) = (guest_of(state, src, size), guest_of(state, dst, size)) {
                let bytes = read_bytes(state, s, size)?;
                write_bytes(state, d, &bytes)?;
            }
        }
        "ClearBuffer" => {
            // (cb, dst, size, value)
            let (dst, size, value) = (x[0], x[1], x[2] as u32);
            if let Some(d) = guest_of(state, dst, size) {
                let bytes: Vec<u8> = value.to_le_bytes().iter().copied().cycle().take(size as usize).collect();
                write_bytes(state, d, &bytes)?;
            }
        }
        "UpdateUniformBuffer" => {
            // (cb, buffer, bufferSize, updateOffset, updateSize, data)
            let (buffer, offset) = (x[0], x[2]);
            let bytes = cmd.data.first().cloned().unwrap_or_default();
            if let Some(d) = guest_of(state, buffer + offset, bytes.len() as u64) {
                write_bytes(state, d, &bytes)?;
            }
        }
        "ReportValue" => {
            // (cb, value, buffer): a 32-bit report.
            let (value, buffer) = (x[0] as u32, x[1]);
            if let Some(d) = guest_of(state, buffer, 4) {
                write_bytes(state, d, &value.to_le_bytes())?;
            }
        }
        "ReportCounter" => {
            // (cb, counterType, buffer): a 16-byte report of a 64-bit counter
            // and a 64-bit timestamp. The host executes no GPU work, so every
            // counter other than the timestamp reads zero.
            let buffer = x[1];
            if let Some(d) = guest_of(state, buffer, 16) {
                let mut rep = [0u8; 16];
                rep[8..].copy_from_slice(&now_ns().to_le_bytes());
                write_bytes(state, d, &rep)?;
            }
        }
        "SignalEvent" => {
            // (cb, event, mode, location, flags, value)
            crate::nvn_prog::signal(state, x[0], x[1] & 0xFFFF_FFFF, x[4] as u32)?;
        }
        "FenceSync" => {
            let sync = x[0];
            state.host.ext.with::<crate::nvn_mem::Memory, _>(|m| m.signalled.insert(sync, true));
        }
        _ => {}
    }
    cmds(state, |c| *c.executed.entry(short).or_insert(0) += 1);
    Ok(())
}

fn run_handle(state: &mut State, handle: u64, depth: u32) -> Result<(), HostResult> {
    if depth > 8 {
        return Err(HostResult::Unavailable("command handles nest deeper than 8".into()));
    }
    let Some(list) = cmds(state, |c| c.handles.get(&handle).cloned()) else {
        return Err(HostResult::Unavailable(format!("submitted command handle 0x{handle:x} does not exist")));
    };
    for cmd in &list {
        execute(state, cmd, depth)?;
    }
    Ok(())
}

/// `nvnQueueSubmitCommands(NVNqueue*, int count, const NVNcommandHandle*)`.
fn queue_submit_commands(state: &mut State) -> HostResult {
    let (n, p) = (state.x[1] & 0xFFFF_FFFF, state.x[2]);
    let raw = match read_bytes(state, p, n * 8) {
        Ok(b) => b,
        Err(e) => return e,
    };
    for h in raw.chunks(8).map(|c| u64::from_le_bytes(c.try_into().unwrap())) {
        if let Err(e) = run_handle(state, h, 0) {
            return e;
        }
    }
    cmds(state, |c| c.submitted += n);
    HostResult::Returned(0)
}

pub fn register(s: &mut HostServices) {
    s.register_prefix("nvn:nvnCommandBuffer", record, "recorded on the host with the memory its arguments point at");
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("nvnCommandBufferInitialize", cb_initialize, "a host command list"),
        ("nvnCommandBufferFinalize", cb_finalize, "forgets the command buffer"),
        ("nvnCommandBufferAddCommandMemory", add_command_memory, "accounted; commands are stored on the host"),
        ("nvnCommandBufferAddControlMemory", add_control_memory, "accounted; commands are stored on the host"),
        ("nvnCommandBufferGetCommandMemorySize", command_memory_size, "memory the game has given"),
        ("nvnCommandBufferGetControlMemorySize", control_memory_size, "memory the game has given"),
        ("nvnCommandBufferGetCommandMemoryUsed", memory_used, "host recording consumes none of it"),
        ("nvnCommandBufferGetControlMemoryUsed", memory_used, "host recording consumes none of it"),
        ("nvnCommandBufferGetCommandMemoryFree", command_memory_size, "all of it stays free"),
        ("nvnCommandBufferGetControlMemoryFree", control_memory_size, "all of it stays free"),
        ("nvnCommandBufferSetMemoryCallback", set_memory_callback, "kept; never called, memory never runs out"),
        ("nvnCommandBufferSetMemoryCallbackData", set_memory_callback_data, "kept for GetMemoryCallbackData"),
        ("nvnCommandBufferGetMemoryCallback", get_memory_callback, "what the game set"),
        ("nvnCommandBufferGetMemoryCallbackData", get_memory_callback_data, "what the game set"),
        ("nvnCommandBufferSetCommandMemoryCallbackEnabled", ignore_setting, "the callback never fires"),
        ("nvnCommandBufferSetControlMemoryCallbackEnabled", ignore_setting, "the callback never fires"),
        ("nvnCommandBufferSetRecordingMode", ignore_setting, "host recording has one mode"),
        ("nvnCommandBufferBeginRecording", begin_recording, "starts a host command list"),
        ("nvnCommandBufferEndRecording", end_recording, "returns a handle to the host command list"),
        ("nvnCommandBufferIsRecording", is_recording, "whether recording is open"),
        ("nvnDeviceFinalizeCommandHandle", finalize_command_handle, "drops the host command list"),
        ("nvnQueueSubmitCommands", queue_submit_commands, "executes memory effects; draws are counted, not rendered"),
    ];
    for (name, handler, claim) in table {
        s.register(&format!("nvn:{name}"), *handler, claim);
    }
}
