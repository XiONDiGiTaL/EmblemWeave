//! The audio renderer's update request, served on the host.
//!
//! nnSdk's own `RequestUpdateAudioRenderer` (in `sdk`) serialises the game's
//! renderer configuration into an input buffer, calls the renderer service
//! through the handle's vtable, and hands the output buffer to its managers,
//! which check every section's size against the output header. The host
//! implements that one service call.
//!
//! Input: a 64-byte header of section sizes, then behaviour, memory pools,
//! voice channel resources, voices, effects, splitters, mixes, sinks and
//! performance, in that order. Output: a 64-byte header, then memory pool,
//! voice, effect and sink statuses, performance, behaviour and render info.
//! The header offsets are the ones nnSdk's code reads and writes.
//!
//! Voices advance by the time that passes between updates, at their sample
//! rate and pitch: a wave buffer is consumed when its samples have played,
//! and a stopped voice releases all of its buffers. That is what lets the
//! game reuse a sound's handle. Nothing is mixed to the speakers yet.

use crate::hle::{read_bytes, write_bytes, HostResult};
use ew_lift::State;
use std::collections::HashMap;

const HEADER: u64 = 64;

/// Input header: byte offsets of each section's size.
mod input {
    pub const BEHAVIOR: u64 = 4;
    pub const MEMORY_POOLS: u64 = 8;
    pub const VOICES: u64 = 12;
    pub const VOICE_RESOURCES: u64 = 16;
    pub const EFFECTS: u64 = 20;
    pub const MIXES: u64 = 24;
    pub const SINKS: u64 = 28;
    pub const PERFORMANCE: u64 = 32;
    pub const SPLITTERS: u64 = 36;
}

/// Output header: byte offsets of each section's size, as nnSdk checks them.
mod output {
    pub const BEHAVIOR: usize = 4;
    pub const MEMORY_POOLS: usize = 8;
    pub const VOICES: usize = 12;
    pub const EFFECTS: usize = 20;
    pub const SINKS: usize = 28;
    pub const PERFORMANCE: usize = 32;
    pub const RENDER_INFO: usize = 40;
    pub const TOTAL: usize = 60;
}

/// Entry sizes of the renderer's IPC structures.
const POOL_IN: u64 = 0x20;
const POOL_OUT: usize = 0x10;
/// This SDK's voice parameter: biquad coefficients are floats, so the wave
/// buffers start at 0x78 (read from nnSdk's `UpdateVoiceInParameter`).
const VOICE_IN: u64 = 0x188;
const VOICE_WAVE_BUFFERS: usize = 0x78;
const VOICE_FLUSH_COUNT: usize = 0x171;
const VOICE_OUT: usize = 0x10;
const EFFECT_IN: u64 = 0xC0;
/// Effect statuses carry a result-state block: 0x90 bytes (the sdk checks the
/// total against its own buffer size).
const EFFECT_OUT: usize = 0x90;
const SINK_IN: u64 = 0x140;
const SINK_OUT: usize = 0x20;
const PERFORMANCE_OUT: usize = 0x10;
const BEHAVIOR_OUT: usize = 0xB0;
const RENDER_INFO_OUT: usize = 0x10;

/// Memory pool states, as the renderer protocol numbers them.
const POOL_REQUEST_DETACH: u32 = 2;
const POOL_DETACHED: u32 = 3;
const POOL_REQUEST_ATTACH: u32 = 4;
const POOL_ATTACHED: u32 = 5;

/// Voice play states.
const PLAY_STARTED: u8 = 0;
const PLAY_STOPPED: u8 = 1;

/// One wave buffer slot as the voice parameter carries it (0x38 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct WaveBuffer {
    /// Sample data address; 0 is an empty slot.
    pub address: u64,
    pub start: u32,
    pub end: u32,
    pub looping: bool,
    pub sent: bool,
}

/// What a voice parameter says, reduced to what playback needs.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VoiceIn {
    pub id: u32,
    pub is_new: bool,
    pub in_use: bool,
    pub play_state: u8,
    pub sample_rate: u32,
    pub pitch: f32,
    pub flush: u8,
    pub buffers: [WaveBuffer; 4],
}

/// The renderer's own playback state for a voice.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Voice {
    /// Slots the renderer holds and has not finished.
    valid: [bool; 4],
    /// The slot playing now.
    index: usize,
    /// Samples played of the current slot.
    offset: u64,
    /// Fractional samples carried between updates.
    carry: f64,
    pub played_samples: u64,
    pub consumed: u32,
}

impl Voice {
    /// Take in this update's parameter and play `seconds` of it.
    pub fn update(&mut self, p: &VoiceIn, seconds: f64) {
        if p.is_new {
            *self = Voice::default();
        }
        // A filled slot the client has not sent before is a newly appended
        // buffer; empty slots (no address) are skipped.
        for (i, b) in p.buffers.iter().enumerate() {
            if !b.sent && b.address != 0 && !self.valid[i] {
                self.valid[i] = true;
            }
        }
        for _ in 0..p.flush {
            self.release_current();
        }
        match p.play_state {
            PLAY_STOPPED => {
                while self.valid.iter().any(|v| *v) {
                    self.release_current();
                }
                self.carry = 0.0;
            }
            PLAY_STARTED => {
                let rate = p.sample_rate as f64 * if p.pitch > 0.0 { p.pitch as f64 } else { 1.0 };
                let want = rate * seconds + self.carry;
                let mut samples = want.floor() as u64;
                self.carry = want - samples as f64;
                // Every slot is visited at most once a pass; a looping buffer
                // plays on and never finishes.
                let mut idle = 0;
                while samples > 0 && idle < 4 {
                    if !self.valid[self.index] {
                        self.index = (self.index + 1) % 4;
                        idle += 1;
                        continue;
                    }
                    idle = 0;
                    let b = p.buffers[self.index];
                    let length = b.end.saturating_sub(b.start) as u64;
                    let left = length.saturating_sub(self.offset);
                    if b.looping && length > 0 {
                        self.played_samples += samples;
                        self.offset = (self.offset + samples) % length;
                        samples = 0;
                    } else if samples >= left {
                        samples -= left;
                        self.played_samples += left;
                        self.release_current();
                    } else {
                        self.offset += samples;
                        self.played_samples += samples;
                        samples = 0;
                    }
                }
                if !self.valid.iter().any(|v| *v) {
                    self.carry = 0.0;
                }
            }
            // Paused: nothing plays.
            _ => {}
        }
    }

    /// Finish the slot being played (or the next held one) and move on.
    fn release_current(&mut self) {
        for _ in 0..4 {
            if self.valid[self.index] {
                self.valid[self.index] = false;
                self.consumed += 1;
                self.offset = 0;
                self.index = (self.index + 1) % 4;
                return;
            }
            self.index = (self.index + 1) % 4;
        }
    }
}

/// Parse one voice parameter (0x188 bytes).
pub fn parse_voice(b: &[u8]) -> VoiceIn {
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let mut buffers = [WaveBuffer::default(); 4];
    for (i, w) in buffers.iter_mut().enumerate() {
        let o = VOICE_WAVE_BUFFERS + i * 0x38;
        *w = WaveBuffer { address: u64::from_le_bytes(b[o..o + 8].try_into().unwrap()), start: u32_at(o + 0x10), end: u32_at(o + 0x14), looping: b[o + 0x18] != 0, sent: b[o + 0x1A] != 0 };
    }
    VoiceIn {
        id: u32_at(0),
        is_new: b[8] != 0,
        in_use: b[9] != 0,
        play_state: b[10],
        sample_rate: u32_at(0x0C),
        pitch: f32::from_bits(u32_at(0x1C)),
        flush: b[VOICE_FLUSH_COUNT],
        buffers,
    }
}

/// Per-renderer state kept between updates.
#[derive(Default)]
struct Server {
    voices: HashMap<u32, Voice>,
    last_tick: Option<u64>,
    frames: u64,
}

#[derive(Default)]
struct Servers(HashMap<u64, Server>);

fn r32(state: &State, a: u64) -> Result<u32, HostResult> {
    Ok(u32::from_le_bytes(read_bytes(state, a, 4)?.try_into().unwrap()))
}

fn r64(state: &State, a: u64) -> Result<u64, HostResult> {
    Ok(u64::from_le_bytes(read_bytes(state, a, 8)?.try_into().unwrap()))
}

fn put32(out: &mut [u8], at: usize, v: u32) {
    out[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

/// `IAudioRenderer::RequestUpdate(this, {out, size}*, {performance, size}*,
/// {in, size}*)`, reached through the renderer handle's vtable.
pub fn request_update(state: &mut State) -> HostResult {
    let renderer = state.x[0];
    match update(state, renderer, state.x[1], state.x[3]) {
        Ok(()) => HostResult::Returned(0),
        Err(e) => e,
    }
}

fn update(state: &mut State, renderer: u64, out_span: u64, in_span: u64) -> Result<(), HostResult> {
    let (out, out_size) = (r64(state, out_span)?, r64(state, out_span + 8)?);
    let (inp, in_size) = (r64(state, in_span)?, r64(state, in_span + 8)?);
    let size = |field: u64| r32(state, inp + field).map(|v| v as u64);
    let (behavior, pools, resources, voices) = (size(input::BEHAVIOR)?, size(input::MEMORY_POOLS)?, size(input::VOICE_RESOURCES)?, size(input::VOICES)?);
    let (effects, splitters, mixes, sinks) = (size(input::EFFECTS)?, size(input::SPLITTERS)?, size(input::MIXES)?, size(input::SINKS)?);
    let _performance = size(input::PERFORMANCE)?;
    for (name, bytes, entry) in [("memory pool", pools, POOL_IN), ("voice", voices, VOICE_IN), ("effect", effects, EFFECT_IN), ("sink", sinks, SINK_IN)] {
        if bytes % entry != 0 {
            return Err(HostResult::Unavailable(format!(
                "audio update: {name} section of {bytes} bytes is not a whole number of 0x{entry:x}-byte entries"
            )));
        }
    }
    let pool_at = inp + HEADER + behavior;
    let voice_at = pool_at + pools + resources;
    let effect_at = voice_at + voices;
    let sink_at = effect_at + effects + splitters + mixes;
    if sink_at + sinks > inp + in_size {
        return Err(HostResult::Unavailable(format!("audio update: sections run past the {in_size}-byte input")));
    }

    let (n_pools, n_voices, n_effects, n_sinks) =
        ((pools / POOL_IN) as usize, (voices / VOICE_IN) as usize, (effects / EFFECT_IN) as usize, (sinks / SINK_IN) as usize);
    let sizes = [n_pools * POOL_OUT, n_voices * VOICE_OUT, n_effects * EFFECT_OUT, n_sinks * SINK_OUT, PERFORMANCE_OUT, BEHAVIOR_OUT, RENDER_INFO_OUT];
    let total = HEADER as usize + sizes.iter().sum::<usize>();
    if total as u64 > out_size {
        return Err(HostResult::Unavailable(format!("audio update: {total} bytes of output do not fit the {out_size}-byte buffer")));
    }
    let mut o = vec![0u8; total];
    o[0..4].copy_from_slice(&read_bytes(state, inp, 4)?);
    put32(&mut o, output::MEMORY_POOLS, sizes[0] as u32);
    put32(&mut o, output::VOICES, sizes[1] as u32);
    put32(&mut o, output::EFFECTS, sizes[2] as u32);
    put32(&mut o, output::SINKS, sizes[3] as u32);
    put32(&mut o, output::PERFORMANCE, sizes[4] as u32);
    put32(&mut o, output::BEHAVIOR, sizes[5] as u32);
    put32(&mut o, output::RENDER_INFO, sizes[6] as u32);
    put32(&mut o, output::TOTAL, total as u32);

    // Memory pools: a requested attach or detach takes effect.
    let mut at = HEADER as usize;
    for i in 0..n_pools {
        let s = r32(state, pool_at + i as u64 * POOL_IN + 16)?;
        let next = match s {
            POOL_REQUEST_ATTACH => POOL_ATTACHED,
            POOL_REQUEST_DETACH => POOL_DETACHED,
            other => other,
        };
        put32(&mut o, at, next);
        at += POOL_OUT;
    }

    // Voices: advance by the time since this renderer's last update.
    let now = crate::threads::now_ticks();
    let parsed: Vec<VoiceIn> = (0..n_voices)
        .map(|i| read_bytes(state, voice_at + i as u64 * VOICE_IN, VOICE_IN).map(|b| parse_voice(&b)))
        .collect::<Result<_, _>>()?;
    let statuses = state.host.ext.with::<Servers, _>(|s| {
        let server = s.0.entry(renderer).or_default();
        let seconds = server.last_tick.map(|t| now.saturating_sub(t) as f64 / crate::threads::TICK_FREQ as f64).unwrap_or(0.0);
        server.last_tick = Some(now);
        server.frames += 1;
        let statuses: Vec<(u64, u32)> = parsed
            .iter()
            .map(|p| {
                if !p.in_use {
                    server.voices.remove(&p.id);
                    return (0, 0);
                }
                let v = server.voices.entry(p.id).or_default();
                v.update(p, seconds.min(0.25));
                (v.played_samples, v.consumed)
            })
            .collect();
        (statuses, server.frames)
    });
    let (statuses, frames) = statuses;
    if frames == 1 {
        state.host.log.push(format!(
            "audio: first update: {n_pools} memory pools, {n_voices} voices, {n_effects} effects, {n_sinks} sinks; {total} bytes out of {out_size}"
        ));
    }
    for (played, consumed) in statuses {
        o[at..at + 8].copy_from_slice(&played.to_le_bytes());
        put32(&mut o, at + 8, consumed);
        at += VOICE_OUT;
    }
    // Effects, sinks, performance and behaviour report nothing to say: no
    // errors, no history. Render info carries the elapsed frame count.
    at += sizes[2] + sizes[3] + sizes[4] + sizes[5];
    o[at..at + 8].copy_from_slice(&frames.to_le_bytes());
    write_bytes(state, out, &o)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(state: u8, buffers: &[(u32, u32, bool)]) -> VoiceIn {
        let mut v = VoiceIn { id: 1, in_use: true, play_state: state, sample_rate: 48_000, pitch: 1.0, ..Default::default() };
        for (i, (start, end, sent)) in buffers.iter().enumerate() {
            v.buffers[i] = WaveBuffer { address: 0x1000 * (i as u64 + 1), start: *start, end: *end, looping: false, sent: *sent };
        }
        v
    }

    #[test]
    fn buffers_are_consumed_as_their_samples_play() {
        let mut v = Voice::default();
        // Two 4800-sample buffers (0.1 s each) appended.
        let p = voice(PLAY_STARTED, &[(0, 4800, false), (0, 4800, false)]);
        v.update(&p, 0.05);
        assert_eq!((v.played_samples, v.consumed), (2400, 0));
        let sent = voice(PLAY_STARTED, &[(0, 4800, true), (0, 4800, true)]);
        v.update(&sent, 0.1);
        assert_eq!((v.played_samples, v.consumed), (7200, 1));
        v.update(&sent, 1.0);
        assert_eq!((v.played_samples, v.consumed), (9600, 2));
        // Nothing more to play.
        v.update(&sent, 1.0);
        assert_eq!(v.consumed, 2);
    }

    #[test]
    fn a_stopped_voice_releases_everything() {
        let mut v = Voice::default();
        v.update(&voice(PLAY_STARTED, &[(0, 48_000, false), (0, 48_000, false), (0, 48_000, false)]), 0.01);
        v.update(&voice(PLAY_STOPPED, &[(0, 48_000, true), (0, 48_000, true), (0, 48_000, true)]), 0.01);
        assert_eq!(v.consumed, 3);
    }

    #[test]
    fn a_reused_slot_is_a_new_buffer_and_new_voices_restart() {
        let mut v = Voice::default();
        v.update(&voice(PLAY_STARTED, &[(0, 480, false)]), 0.01);
        assert_eq!(v.consumed, 1);
        // The client appends again into slot 0.
        v.update(&voice(PLAY_STARTED, &[(0, 480, false)]), 0.005);
        assert_eq!((v.consumed, v.played_samples), (1, 720));
        let mut fresh = voice(PLAY_STARTED, &[]);
        fresh.is_new = true;
        v.update(&fresh, 0.0);
        assert_eq!((v.consumed, v.played_samples), (0, 0));
    }

    #[test]
    fn looping_buffers_never_finish() {
        let mut v = Voice::default();
        let mut p = voice(PLAY_STARTED, &[(0, 1000, false)]);
        p.buffers[0].looping = true;
        v.update(&p, 10.0);
        assert_eq!(v.consumed, 0);
        assert_eq!(v.played_samples, 480_000);
    }
}
