//! `nn::audio` at the renderer service boundary.
//!
//! The title's renderer configuration (voices, mixes, effects, wave buffers)
//! is built by nnSdk's own code in `sdk`, which works entirely on game memory.
//! What reaches the system audio service is served here: opening, starting
//! and stopping the renderer, and its update and rendering requests. The
//! renderer's system event fires every 5 ms, the console's audio frame, so
//! the game's audio thread runs at its real cadence.
//!
//! An update runs nnSdk's own `RequestUpdateAudioRenderer`, which reaches the
//! renderer service through the handle: a guest object whose vtable the host
//! fills. Its update call is served by [`crate::audio_update`], which plays
//! voices forward in time and releases their wave buffers. The voices are not
//! mixed to the PC's speakers yet.

use crate::hle::{write_bytes, HostResult, HostServices};
use ew_lift::State;
use std::collections::BTreeMap;

const OK: u64 = 0;
/// The audio frame: 5 ms.
const FRAME_NS: u64 = 5_000_000;
/// The device name every output reports.
const DEVICE_NAME: &[u8] = b"AudioTvOutput";

#[derive(Default)]
struct Renderers {
    /// Handle to the guest address of its system event.
    events: BTreeMap<u64, u64>,
    next: u64,
    updates: u64,
}

fn renderers<R>(state: &State, f: impl FnOnce(&mut Renderers) -> R) -> R {
    state.host.ext.with::<Renderers, R>(f)
}

/// The renderer service's vtable: 64 slots, the update call at +64.
const VTABLE_SLOTS: u64 = 64;
const SLOT_REQUEST_UPDATE: u64 = 8;

/// A renderer object for the handle: a vtable pointer, then the vtable.
fn renderer_object(state: &mut State) -> Result<u64, HostResult> {
    let object = state.host.allocate(16 + VTABLE_SLOTS * 8, 16).map_err(HostResult::Unavailable)?;
    let update = crate::hle::dynamic_stub(state, "audio:IAudioRenderer::RequestUpdate")?;
    let other = crate::hle::dynamic_stub(state, "audio:IAudioRenderer::unimplemented")?;
    let vtable = object + 16;
    write_bytes(state, object, &vtable.to_le_bytes())?;
    for slot in 0..VTABLE_SLOTS {
        let f = if slot == SLOT_REQUEST_UPDATE { update } else { other };
        write_bytes(state, vtable + slot * 8, &f.to_le_bytes())?;
    }
    Ok(object)
}

fn unimplemented_service(state: &mut State) -> HostResult {
    HostResult::Unavailable(format!(
        "the audio renderer service was called at 0x{:x} through a vtable slot the host does not serve",
        state.x[30].wrapping_sub(4)
    ))
}

/// `OpenAudioRenderer(AudioRendererHandle*, os::SystemEvent*, const
/// AudioRendererParameter&, void* workBuffer, size_t)`.
fn open(state: &mut State) -> HostResult {
    let (out, event) = (state.x[0], state.x[1]);
    let handle = match renderer_object(state) {
        Ok(h) => h,
        Err(e) => return e,
    };
    renderers(state, |r| {
        r.next += 1;
        r.events.insert(handle, event);
    });
    // The event is a 5 ms periodic timer that auto-clears on wait.
    if let Err(e) = crate::threads::init_timer_event(state, event, true) {
        return e;
    }
    state.host.log.push(format!(
        "audio: renderer 0x{handle:x} opened; its event fires every 5 ms, voices play forward in time but are not mixed to the speakers yet"
    ));
    match write_bytes(state, out, &handle.to_le_bytes()) {
        Ok(()) => HostResult::Returned(OK),
        Err(e) => e,
    }
}

fn event_of(state: &State, handle: u64) -> Option<u64> {
    renderers(state, |r| r.events.get(&handle).copied())
}

/// `StartAudioRenderer(handle)`: the frame clock starts.
fn start(state: &mut State) -> HostResult {
    let handle = state.x[0];
    match event_of(state, handle) {
        Some(e) => match crate::threads::start_timer_event(state, e, FRAME_NS, FRAME_NS) {
            Ok(()) => HostResult::Returned(OK),
            Err(err) => err,
        },
        None => HostResult::Unavailable(format!("StartAudioRenderer on handle {handle}, which is not open")),
    }
}

/// `StopAudioRenderer(handle)`.
fn stop(state: &mut State) -> HostResult {
    let handle = state.x[0];
    if let Some(e) = event_of(state, handle) {
        if let Err(err) = crate::threads::stop_timer_event(state, e) {
            return err;
        }
    }
    HostResult::Returned(OK)
}

fn close(state: &mut State) -> HostResult {
    let handle = state.x[0];
    renderers(state, |r| r.events.remove(&handle));
    HostResult::Returned(OK)
}

/// `RequestUpdateAudioRenderer(handle, const AudioRendererConfig*)`: nnSdk's
/// own implementation, which calls the renderer object's update.
fn request_update(state: &mut State) -> HostResult {
    renderers(state, |r| r.updates += 1);
    let export = "_ZN2nn5audio26RequestUpdateAudioRendererENS0_19AudioRendererHandleEPKNS0_19AudioRendererConfigE";
    match state.host.named.get(&format!("export.{export}")) {
        Some(&addr) => HostResult::Jump(addr),
        None => HostResult::Unavailable("RequestUpdateAudioRenderer needs the sdk module, which is not loaded".into()),
    }
}

/// `GetAudioRendererWorkBufferSize(const AudioRendererParameter&)`: the work
/// buffer belongs to the audio service, which the host renderer replaces, so
/// the game reserves a single 64 KiB block for it.
fn work_buffer_size(_state: &mut State) -> HostResult {
    HostResult::Returned(0x1_0000)
}

fn ok(_state: &mut State) -> HostResult {
    HostResult::Returned(OK)
}

/// `AcquireAudioOutputDeviceNotification(os::SystemEvent*)`: the output
/// device never changes on the host, so the event is never signalled.
fn device_notification(state: &mut State) -> HostResult {
    match crate::threads::init_timer_event(state, state.x[0], true) {
        Ok(()) => HostResult::Returned(OK),
        Err(e) => e,
    }
}

fn write_name(state: &mut State, out: u64) -> Result<(), HostResult> {
    let mut name = [0u8; 256];
    name[..DEVICE_NAME.len()].copy_from_slice(DEVICE_NAME);
    write_bytes(state, out, &name)
}

/// `ListAudioOutputDeviceName(AudioDeviceName*, int count)`: one device.
fn list_devices(state: &mut State) -> HostResult {
    let (out, count) = (state.x[0], state.x[1] as i32);
    if count < 1 {
        return HostResult::Returned(0);
    }
    match write_name(state, out) {
        Ok(()) => HostResult::Returned(1),
        Err(e) => e,
    }
}

/// `GetActiveAudioOutputDeviceName(AudioDeviceName*)`.
fn active_device(state: &mut State) -> HostResult {
    match write_name(state, state.x[0]) {
        Ok(()) => HostResult::Returned(OK),
        Err(e) => e,
    }
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("_ZN2nn5audio17OpenAudioRendererEPNS0_19AudioRendererHandleEPNS_2os11SystemEventERKNS0_22AudioRendererParameterEPvm", open, "a renderer whose event fires every 5 ms; no mixing to the speakers yet"),
        ("_ZN2nn5audio30GetAudioRendererWorkBufferSizeERKNS0_22AudioRendererParameterE", work_buffer_size, "64 KiB; the host renderer does not use the service's work buffer"),
        ("_ZN2nn5audio18StartAudioRendererENS0_19AudioRendererHandleE", start, "starts the 5 ms frame clock"),
        ("_ZN2nn5audio17StopAudioRendererENS0_19AudioRendererHandleE", stop, "stops the frame clock"),
        ("_ZN2nn5audio18CloseAudioRendererENS0_19AudioRendererHandleE", close, "forgets the renderer"),
        ("_ZN2nn5audio26RequestUpdateAudioRendererENS0_19AudioRendererHandleEPKNS0_19AudioRendererConfigE", request_update, "nnSdk's own update, served by the host renderer"),
        ("audio:IAudioRenderer::RequestUpdate", crate::audio_update::request_update, "voices play forward in time; wave buffers are released as they finish"),
        ("audio:IAudioRenderer::unimplemented", unimplemented_service, "any other renderer service call stops the run"),
        ("_ZN2nn5audio29ExecuteAudioRendererRenderingENS0_19AudioRendererHandleE", ok, "accepted; nothing is mixed yet"),
        ("_ZN2nn5audio36AcquireAudioOutputDeviceNotificationEPNS_2os11SystemEventE", device_notification, "never signalled: the output device does not change"),
        ("_ZN2nn5audio25ListAudioOutputDeviceNameEPNS0_15AudioDeviceNameEi", list_devices, "one output device"),
        ("_ZN2nn5audio30GetActiveAudioOutputDeviceNameEPNS0_15AudioDeviceNameE", active_device, "the one output device"),
        ("_ZN2nn5audio26SetAudioDeviceOutputVolumeEPKNS0_15AudioDeviceNameEf", ok, "accepted"),
        ("_ZN2nn5audio40SetDeviceOutputVolumeAutoTuneModeEnabledEb", ok, "accepted"),
    ];
    for (name, handler, claim) in table {
        s.register(name, *handler, claim);
    }
}
