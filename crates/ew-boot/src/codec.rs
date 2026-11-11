//! `nn::codec` Opus decoders on the host.
//!
//! nnSdk serves Opus through a system decoder service. The host keeps each
//! decoder's parameters itself and decodes packets to silence of exactly the
//! right length, so the game's audio streaming advances at the rate the data
//! dictates. Producing the real waveform belongs with audio mixing, which the
//! host does not do yet; until then nothing would be heard anyway.
//!
//! Input follows Nintendo's Opus framing: each packet is preceded by an 8-byte
//! header, a big-endian payload size and the encoder's final range. The sample
//! count comes from the packet's own TOC byte (RFC 6716, section 3.1).
//!
//! ADPCM decoding is pure library code and runs as nnSdk's own code.

use crate::hle::{read_bytes, write_bytes, HostResult, HostServices};
use ew_lift::State;
use std::collections::BTreeMap;

const OK: u64 = 0;
/// `nn::codec::ResultOpusInvalidPacket`-style failure for malformed input.
const INVALID_PACKET: u64 = 0x6E1F;

#[derive(Default, Clone, Copy)]
struct Decoder {
    rate: u32,
    channels: u32,
    streams: u32,
    stereo: u32,
}

#[derive(Default)]
struct Decoders(BTreeMap<u64, Decoder>);

fn with<R>(state: &State, f: impl FnOnce(&mut BTreeMap<u64, Decoder>) -> R) -> R {
    state.host.ext.with::<Decoders, R>(|d| f(&mut d.0))
}

/// Samples per channel in one Opus packet at `rate`, from its TOC byte.
pub fn packet_samples(packet: &[u8], rate: u32) -> Option<u32> {
    let toc = *packet.first()?;
    let config = toc >> 3;
    // Frame duration in units of 2.5 ms.
    let quarter_ms = match config {
        0..=11 => [4u32, 8, 16, 24][(config % 4) as usize],
        12..=15 => [4u32, 8][(config % 2) as usize],
        _ => [1u32, 2, 4, 8][(config % 4) as usize],
    };
    let frames = match toc & 3 {
        0 => 1,
        1 | 2 => 2,
        _ => (*packet.get(1)? & 0x3F) as u32,
    };
    Some(frames * quarter_ms * rate / 400)
}

fn work_buffer_size(_state: &mut State) -> HostResult {
    // Decoder state is kept on the host; the game reserves one page.
    HostResult::Returned(0x1000)
}

/// `Initialize{,Hardware}OpusDecoder(decoder, rate, channels, buffer, size, ...)`.
fn initialize(state: &mut State) -> HostResult {
    let (d, rate, channels) = (state.x[0], state.x[1] as u32, state.x[2] as u32);
    with(state, |m| m.insert(d, Decoder { rate, channels, streams: 1, stereo: (channels == 2) as u32 }));
    HostResult::Returned(OK)
}

/// `Initialize{,Hardware}OpusMultiStreamDecoder(decoder, rate, channels,
/// totalStreams, stereoStreams, mapping, buffer, size, ...)`.
fn initialize_multi(state: &mut State) -> HostResult {
    let (d, rate, channels, streams, stereo) =
        (state.x[0], state.x[1] as u32, state.x[2] as u32, state.x[3] as u32, state.x[4] as u32);
    with(state, |m| m.insert(d, Decoder { rate, channels, streams, stereo }));
    HostResult::Returned(OK)
}

fn finalize(state: &mut State) -> HostResult {
    let d = state.x[0];
    with(state, |m| m.remove(&d));
    HostResult::Returned(OK)
}

fn reset(_state: &mut State) -> HostResult {
    HostResult::Returned(OK)
}

fn get(state: &mut State, f: fn(&Decoder) -> u32) -> HostResult {
    let d = state.x[0];
    match with(state, |m| m.get(&d).copied()) {
        Some(dec) => HostResult::Returned(f(&dec) as u64),
        None => HostResult::Unavailable(format!("Opus decoder 0x{d:x} was never initialised")),
    }
}

fn sample_rate(state: &mut State) -> HostResult {
    get(state, |d| d.rate)
}
fn channel_count(state: &mut State) -> HostResult {
    get(state, |d| d.channels)
}
fn total_streams(state: &mut State) -> HostResult {
    get(state, |d| d.streams)
}
fn stereo_streams(state: &mut State) -> HostResult {
    get(state, |d| d.stereo)
}

/// `Decode*Interleaved(decoder, size_t* consumed, int* samples, int16* out,
/// size_t outSize, const void* in, size_t inSize)`: one packet.
fn decode(state: &mut State) -> HostResult {
    let (d, consumed, samples_out, out, out_size, input, in_size) =
        (state.x[0], state.x[1], state.x[2], state.x[3], state.x[4], state.x[5], state.x[6]);
    let Some(dec) = with(state, |m| m.get(&d).copied()) else {
        return HostResult::Unavailable(format!("Opus decoder 0x{d:x} was never initialised"));
    };
    if in_size < 8 {
        return HostResult::Returned(INVALID_PACKET);
    }
    let header = match read_bytes(state, input, 8) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as u64;
    if size == 0 || 8 + size > in_size {
        return HostResult::Returned(INVALID_PACKET);
    }
    let packet = match read_bytes(state, input + 8, size.min(2)) {
        Ok(b) => b,
        Err(e) => return e,
    };
    let Some(per_channel) = packet_samples(&packet, dec.rate) else {
        return HostResult::Returned(INVALID_PACKET);
    };
    let bytes = per_channel as u64 * dec.channels.max(1) as u64 * 2;
    if bytes > out_size {
        return HostResult::Returned(INVALID_PACKET);
    }
    for r in [
        write_bytes(state, out, &vec![0u8; bytes as usize]),
        write_bytes(state, consumed, &(8 + size).to_le_bytes()),
        write_bytes(state, samples_out, &per_channel.to_le_bytes()),
    ] {
        if let Err(e) = r {
            return e;
        }
    }
    HostResult::Returned(OK)
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult)] = &[
        ("_ZN2nn5codec28GetOpusDecoderWorkBufferSizeEii", work_buffer_size),
        ("_ZN2nn5codec36GetHardwareOpusDecoderWorkBufferSizeEiii", work_buffer_size),
        ("_ZN2nn5codec39GetOpusMultiStreamDecoderWorkBufferSizeEiiii", work_buffer_size),
        ("_ZN2nn5codec47GetHardwareOpusMultiStreamDecoderWorkBufferSizeEiiiii", work_buffer_size),
        ("_ZN2nn5codec21InitializeOpusDecoderEPNS0_15OpusDecoderTypeEiiPvm", initialize),
        ("_ZN2nn5codec29InitializeHardwareOpusDecoderEPNS0_23HardwareOpusDecoderTypeEiiPvmi", initialize),
        ("_ZN2nn5codec32InitializeOpusMultiStreamDecoderEPNS0_26OpusMultiStreamDecoderTypeEiiiiPKhPvm", initialize_multi),
        ("_ZN2nn5codec40InitializeHardwareOpusMultiStreamDecoderEPNS0_34HardwareOpusMultiStreamDecoderTypeEiiiiPKhPvmi", initialize_multi),
        ("_ZN2nn5codec19FinalizeOpusDecoderEPNS0_15OpusDecoderTypeE", finalize),
        ("_ZN2nn5codec27FinalizeHardwareOpusDecoderEPNS0_23HardwareOpusDecoderTypeE", finalize),
        ("_ZN2nn5codec30FinalizeOpusMultiStreamDecoderEPNS0_26OpusMultiStreamDecoderTypeE", finalize),
        ("_ZN2nn5codec38FinalizeHardwareOpusMultiStreamDecoderEPNS0_34HardwareOpusMultiStreamDecoderTypeE", finalize),
        ("_ZN2nn5codec16ResetOpusDecoderEPNS0_15OpusDecoderTypeE", reset),
        ("_ZN2nn5codec24ResetHardwareOpusDecoderEPNS0_23HardwareOpusDecoderTypeE", reset),
        ("_ZN2nn5codec27ResetOpusMultiStreamDecoderEPNS0_26OpusMultiStreamDecoderTypeE", reset),
        ("_ZN2nn5codec35ResetHardwareOpusMultiStreamDecoderEPNS0_34HardwareOpusMultiStreamDecoderTypeE", reset),
        ("_ZN2nn5codec24GetOpusDecoderSampleRateEPKNS0_15OpusDecoderTypeE", sample_rate),
        ("_ZN2nn5codec32GetHardwareOpusDecoderSampleRateEPKNS0_23HardwareOpusDecoderTypeE", sample_rate),
        ("_ZN2nn5codec35GetOpusMultiStreamDecoderSampleRateEPKNS0_26OpusMultiStreamDecoderTypeE", sample_rate),
        ("_ZN2nn5codec43GetHardwareOpusMultiStreamDecoderSampleRateEPKNS0_34HardwareOpusMultiStreamDecoderTypeE", sample_rate),
        ("_ZN2nn5codec26GetOpusDecoderChannelCountEPKNS0_15OpusDecoderTypeE", channel_count),
        ("_ZN2nn5codec34GetHardwareOpusDecoderChannelCountEPKNS0_23HardwareOpusDecoderTypeE", channel_count),
        ("_ZN2nn5codec37GetOpusMultiStreamDecoderChannelCountEPKNS0_26OpusMultiStreamDecoderTypeE", channel_count),
        ("_ZN2nn5codec45GetHardwareOpusMultiStreamDecoderChannelCountEPKNS0_34HardwareOpusMultiStreamDecoderTypeE", channel_count),
        ("_ZN2nn5codec41GetOpusMultiStreamDecoderTotalStreamCountEPKNS0_26OpusMultiStreamDecoderTypeE", total_streams),
        ("_ZN2nn5codec49GetHardwareOpusMultiStreamDecoderTotalStreamCountEPKNS0_34HardwareOpusMultiStreamDecoderTypeE", total_streams),
        ("_ZN2nn5codec42GetOpusMultiStreamDecoderStereoStreamCountEPKNS0_26OpusMultiStreamDecoderTypeE", stereo_streams),
        ("_ZN2nn5codec50GetHardwareOpusMultiStreamDecoderStereoStreamCountEPKNS0_34HardwareOpusMultiStreamDecoderTypeE", stereo_streams),
        ("_ZN2nn5codec21DecodeOpusInterleavedEPNS0_15OpusDecoderTypeEPmPiPsmPKvm", decode),
        ("_ZN2nn5codec33DecodeOpusInterleavedWithHardwareEPNS0_23HardwareOpusDecoderTypeEPmPiPsmPKvm", decode),
        ("_ZN2nn5codec32DecodeOpusMultiStreamInterleavedEPNS0_26OpusMultiStreamDecoderTypeEPmPiPsmPKvm", decode),
        ("_ZN2nn5codec44DecodeOpusMultiStreamInterleavedWithHardwareEPNS0_34HardwareOpusMultiStreamDecoderTypeEPmPiPsmPKvm", decode),
    ];
    for (name, handler) in table {
        s.register(name, *handler, "Opus on the host: correct sample counts, decoded as silence until mixing lands");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_counts_follow_the_toc_byte() {
        // CELT 20 ms, one frame, at 48 kHz: 960 samples.
        assert_eq!(packet_samples(&[31 << 3], 48_000), Some(960));
        // SILK 60 ms (config 3), two frames (code 1): 5760.
        assert_eq!(packet_samples(&[(3 << 3) | 1], 48_000), Some(5760));
        // CELT 2.5 ms (config 16), code 3 with 4 frames: 480.
        assert_eq!(packet_samples(&[(16 << 3) | 3, 4], 48_000), Some(480));
    }
}
