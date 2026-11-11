//! `nn::movie` video playback.
//!
//! The host does not decode video yet. The game's movie objects can be built
//! and torn down, but opening a movie reports that the media cannot be
//! opened, which the game treats as a movie it cannot play, so it goes on
//! without it. The path asked for is logged.

use crate::hle::{c_string, HostResult, HostServices};
use ew_lift::State;

/// `nn::movie` result for media the extractor cannot open (module 74,
/// description 1): any failure is enough for the game to skip the movie.
const RESULT_MEDIA_UNSUPPORTED: u64 = 74 | (1 << 9);

fn ok(_state: &mut State) -> HostResult {
    HostResult::Returned(0)
}

/// `MediaExtractor::SetUriMedia(const char* uri, int length)`.
fn set_uri_media(state: &mut State) -> HostResult {
    let uri = c_string(state, state.x[1], 1024).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
    state.host.log.push(format!("movie: {uri} cannot be played yet; reported as unsupported media"));
    HostResult::Returned(RESULT_MEDIA_UNSUPPORTED)
}

pub fn register(s: &mut HostServices) {
    let table: &[(&str, fn(&mut State) -> HostResult, &'static str)] = &[
        ("_ZN2nn5movie10InitializeEv", ok, "nothing to set up"),
        ("_ZN2nn5movie8FinalizeEv", ok, "nothing to tear down"),
        ("_ZN2nn5movie13MediaPropertyC1Ev", ok, "an empty property set"),
        ("_ZN2nn5movie6BufferC1Ev", ok, "an empty buffer"),
        ("_ZN2nn5movie14MediaExtractorC1Ev", ok, "an extractor with no media"),
        ("_ZN2nn5movie14MediaExtractorD1Ev", ok, "nothing to release"),
        ("_ZN2nn5movie14MediaExtractor10InitializeEPvmm", ok, "accepts its work memory"),
        ("_ZN2nn5movie14MediaExtractor8FinalizeEv", ok, "nothing to release"),
        ("_ZN2nn5movie14MediaExtractor11SetUriMediaEPKci", set_uri_media, "video is not decoded yet: the media is reported unsupported"),
        ("_ZN5movie12VideoDecoderC1Ev", ok, "a decoder that is never started"),
        ("_ZN5movie12VideoDecoderD1Ev", ok, "nothing to release"),
        ("_ZN5movie12VideoDecoder8FinalizeEv", ok, "nothing to release"),
        ("_ZN5movie10VideoFrameC1Ev", ok, "an empty frame"),
    ];
    for (name, handler, claim) in table {
        s.register(name, *handler, claim);
    }
}
