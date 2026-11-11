//! Graphics bring-up: `nv::`, `nn::vi` and the NVN procedure table.
//!
//! The game reaches NVN through one C entry point, `nvnBootstrapLoader`, which
//! returns `nvnDeviceGetProcAddress`; that in turn returns a function pointer
//! for every NVN procedure by name. The host hands out a run-time host function
//! for each name it is asked for (see `hle::dynamic_stub`), so the procedure
//! table the game builds is complete and every NVN call is identified by its
//! real name. A procedure the host has not implemented stops the run naming it,
//! which is the work order for the Vulkan backend.
//!
//! `nn::vi` describes a single display and layer. Its answers are the PC's:
//! the output is SDR, and the buffer size comes from the launcher's setting.

use crate::hle::{c_string, dynamic_stub, write_bytes, HostResult, HostServices};
use ew_lift::State;

/// Distinct, recognisable handles for the display, layer and native window.
const DISPLAY_HANDLE: u64 = 0xD15_0000_0001;
const LAYER_HANDLE: u64 = 0x1A7_0000_0001;
const WINDOW_HANDLE: u64 = 0x3B0_0000_0001;

macro_rules! t {
    ($e:expr) => {
        match $e {
            Ok(v) => v,
            Err(e) => return e,
        }
    };
}

fn proc_name(state: &State, addr: u64) -> Result<String, HostResult> {
    c_string(state, addr, 256).map(|b| String::from_utf8_lossy(&b).into_owned())
}

// ---- nv:: ---------------------------------------------------------------------

/// `nv::SetGraphicsAllocator(alloc, free, realloc, user)`: the host driver
/// allocates nothing in guest memory, so the callbacks are recorded only.
fn set_graphics_allocator(state: &mut State) -> HostResult {
    for (k, i) in [("nv.alloc", 0), ("nv.free", 1), ("nv.realloc", 2), ("nv.user", 3)] {
        state.host.named.insert(k.into(), state.x[i]);
    }
    HostResult::Returned(0)
}

/// `nv::InitializeGraphics(void* memory, size_t size)`: the region the console
/// driver keeps its own state in. The host driver keeps its state on the host,
/// so the region is recorded and left untouched for the game.
fn initialize_graphics(state: &mut State) -> HostResult {
    let (mem, size) = (state.x[0], state.x[1]);
    state.host.named.insert("nv.memory".into(), mem);
    state.host.named.insert("nv.memory_size".into(), size);
    state.host.log.push(format!("gfx: nv::InitializeGraphics with {} KiB of driver memory", size >> 10));
    HostResult::Returned(0)
}

// ---- NVN procedure table -------------------------------------------------------------

/// `nvnBootstrapLoader(const char* name)`.
fn bootstrap_loader(state: &mut State) -> HostResult {
    let name = t!(proc_name(state, state.x[0]));
    match dynamic_stub(state, &format!("nvn:{name}")) {
        Ok(addr) => {
            state.host.log.push(format!("gfx: nvnBootstrapLoader(\"{name}\")"));
            HostResult::Returned(addr)
        }
        Err(e) => e,
    }
}

/// `nvnDeviceGetProcAddress(const NVNdevice*, const char* name)`: every name
/// gets its own host function. The count of procedures handed out is logged
/// once the table is complete enough to matter.
fn get_proc_address(state: &mut State) -> HostResult {
    let name = t!(proc_name(state, state.x[1]));
    if name.is_empty() {
        return HostResult::Returned(0);
    }
    match dynamic_stub(state, &format!("nvn:{name}")) {
        Ok(addr) => HostResult::Returned(addr),
        Err(e) => e,
    }
}

// ---- nn::vi ---------------------------------------------------------------------------

fn ok(_s: &mut State) -> HostResult {
    HostResult::Returned(0)
}

fn open_default_display(state: &mut State) -> HostResult {
    t!(write_bytes(state, state.x[0], &DISPLAY_HANDLE.to_le_bytes()));
    HostResult::Returned(0)
}

fn create_layer(state: &mut State) -> HostResult {
    t!(write_bytes(state, state.x[0], &LAYER_HANDLE.to_le_bytes()));
    state.host.log.push("gfx: nn::vi layer created".into());
    HostResult::Returned(0)
}

fn get_native_window(state: &mut State) -> HostResult {
    t!(write_bytes(state, state.x[0], &WINDOW_HANDLE.to_le_bytes()));
    HostResult::Returned(0)
}

/// Output size from the launcher (`resolution` as `WIDTHxHEIGHT`), 1920×1080
/// when unset.
fn output_size(state: &State) -> (u32, u32) {
    state
        .host
        .settings
        .get("resolution")
        .and_then(|r| {
            let (w, h) = r.split_once(['x', '×'])?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or((1920, 1080))
}

/// `QueryDisplayBufferMaxResolution(BufferMaxResolution*, const Display*)`:
/// two 32-bit integers, width then height.
fn buffer_max_resolution(state: &mut State) -> HostResult {
    let (w, h) = output_size(state);
    let mut b = Vec::with_capacity(8);
    b.extend_from_slice(&w.to_le_bytes());
    b.extend_from_slice(&h.to_le_bytes());
    t!(write_bytes(state, state.x[0], &b));
    HostResult::Returned(0)
}

/// `QueryDisplayHdrStatus(bool* enabled, HdrOutputFormat*, uint16_t*, const Display*)`:
/// the PC output is reported as SDR.
fn hdr_status(state: &mut State) -> HostResult {
    t!(write_bytes(state, state.x[0], &[0]));
    if state.x[1] != 0 {
        t!(write_bytes(state, state.x[1], &0u32.to_le_bytes()));
    }
    if state.x[2] != 0 {
        t!(write_bytes(state, state.x[2], &0u16.to_le_bytes()));
    }
    HostResult::Returned(0)
}

/// `QueryDisplayHdrPaperWhite(uint16_t* nits, const Display*)`: the SDR
/// reference white of 203 nits (ITU-R BT.2408).
fn hdr_paper_white(state: &mut State) -> HostResult {
    t!(write_bytes(state, state.x[0], &203u16.to_le_bytes()));
    HostResult::Returned(0)
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("_ZN2nv20SetGraphicsAllocatorEPFPvmmS0_EPFvS0_S0_EPFS0_S0_mS0_ES0_", set_graphics_allocator, "recorded; the host driver does not allocate guest memory"),
        ("_ZN2nv18InitializeGraphicsEPvm", initialize_graphics, "the driver region is recorded and left to the game"),
        ("nvnBootstrapLoader", bootstrap_loader, "returns a host function per NVN entry point"),
        ("nvn:nvnDeviceGetProcAddress", get_proc_address, "returns a host function per NVN procedure name"),
        ("_ZN2nn2vi10InitializeEv", ok, "one display and one layer, owned by the host window"),
        ("_ZN2nn2vi18OpenDefaultDisplayEPPNS0_7DisplayE", open_default_display, "the host's single display"),
        ("_ZN2nn2vi11CreateLayerEPPNS0_5LayerEPNS0_7DisplayE", create_layer, "the host window's single layer"),
        ("_ZN2nn2vi15GetNativeWindowEPPvPNS0_5LayerE", get_native_window, "a handle the NVN window binds to"),
        ("_ZN2nn2vi31QueryDisplayBufferMaxResolutionEPNS0_19BufferMaxResolutionEPKNS0_7DisplayE", buffer_max_resolution, "the launcher's output size, default 1920x1080"),
        ("_ZN2nn2vi21QueryDisplayHdrStatusEPbPNS0_15HdrOutputFormatEPtPKNS0_7DisplayE", hdr_status, "SDR output"),
        ("_ZN2nn2vi25QueryDisplayHdrPaperWhiteEPtPKNS0_7DisplayE", hdr_paper_white, "SDR reference white, 203 nits"),
        ("_ZN2nn2vi52SetLayerNextHdrStandardContentLuminanceRangeMetadataEPNS0_5LayerEtt", ok, "HDR metadata has no effect on SDR output"),
        ("_ZN2nn2vi52SetLayerNextHdrExtendedContentLuminanceRangeMetadataEPNS0_5LayerEtt", ok, "HDR metadata has no effect on SDR output"),
        ("_ZN2nn2vi35SetLayerNextHdrContentGamutMetadataEPNS0_5LayerENS0_15HdrContentGamutE", ok, "HDR metadata has no effect on SDR output"),
    ];
    for (name, handler, claim) in table {
        s.register(name, *handler, claim);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_nvn_name_gets_a_stable_host_function() {
        let mut s = State::default();
        s.host.dyn_base = 0x9000;
        s.host.dyn_capacity = 8;
        s.mem[0x100..0x119].copy_from_slice(b"nvnDeviceGetProcAddress\0\0");
        s.x[0] = 0x100;
        let HostResult::Returned(gpa) = bootstrap_loader(&mut s) else { panic!() };
        assert_eq!(gpa, 0x9000);
        s.mem[0x200..0x214].copy_from_slice(b"nvnDeviceInitialize\0");
        s.x[1] = 0x200;
        let HostResult::Returned(a) = get_proc_address(&mut s) else { panic!() };
        let HostResult::Returned(b) = get_proc_address(&mut s) else { panic!() };
        assert_eq!(a, b, "the same name maps to the same address");
        assert_eq!(s.host.dyn_stubs[1], "nvn:nvnDeviceInitialize");
    }

    #[test]
    fn the_buffer_size_follows_the_launcher_setting() {
        let mut s = State::default();
        s.x[0] = 0x100;
        buffer_max_resolution(&mut s);
        assert_eq!(&s.mem[0x100..0x108], &[0x80, 7, 0, 0, 0x38, 4, 0, 0]);
        s.host.settings.insert("resolution".into(), "2560 x 1440".into());
        buffer_max_resolution(&mut s);
        assert_eq!(u32::from_le_bytes(s.mem[0x100..0x104].try_into().unwrap()), 2560);
    }
}
