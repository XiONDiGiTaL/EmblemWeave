// SPDX-License-Identifier: GPL-3.0-or-later
// movie::VideoDecoder of the Switch 2 movie library on the host (src/media/movie_vdec.cpp): AV1 through dav1d into the player's CPU frames,
// for a title with game::Info::movie_video_decoder. [media] movies = host (default) | skip (every movie fails to open, core/switch2.cpp).
#pragma once

#include <cstddef>
#include <cstdint>

namespace nxrt::media {

// The input queue's bound (src/media/movie_vdec.cpp): the player feeds from its own MovieVideoHandler thread, so DecodeAsync waits for room
// instead of letting a 60 fps demuxer build a multi-second packet backlog. It never drops the waiting packet during normal backpressure: a
// dropped AV1 packet corrupts every frame until the next keyframe. The wait ends on room, stop, flush, or a five-second no-progress abort that
// returns EOS or a decoder failure. A packet captured before Flush or Stop belongs to the old stream and is discarded rather than fed to the
// new stream; movie_held_packet_is_valid() is that decision. Exposed for unit tests; the byte cap keeps a few 4K packets from taking the room
// of hundreds of small ones. EOS is never held back, so the queue may exceed these by one packet.
inline bool movie_held_packet_is_valid(uint64_t packet_generation, uint64_t queue_generation, bool stopped) {
    return !stopped && packet_generation == queue_generation;
}
constexpr size_t kMovieMaxQueuedPackets = 64;
constexpr size_t kMovieMaxQueuedBytes = 48u << 20;
inline bool movie_packet_queue_has_room(size_t queued_packets, size_t queued_bytes, size_t packet_bytes) {
    return queued_packets < kMovieMaxQueuedPackets && (queued_packets == 0 || queued_bytes + packet_bytes <= kMovieMaxQueuedBytes);
}

// Registers the host decoder and forces main's imports to it. False when off, not such a title, or built without dav1d (NXRT_HAVE_DAV1D).
// Call before the modules are linked; safe to call more than once.
bool install_movie_decoder_hle();

// True once install_movie_decoder_hle() routed the decoder to the host: the game module then puts its player on the CPU-frame path.
bool movie_decoder_enabled();

// Frames the game's player has taken from the host decoder so far; 0 when the decoder is off.
uint64_t movie_frames_acquired();

}  // namespace nxrt::media
