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

// Where the configOBUs start in the movie library's codec configuration, or kMovieAv1ConfigInvalid when the bytes are neither shape. The
// library sends the configOBUs themselves for this title (its record is a bare sequence-header OBU, 0x0A) and, for other tracks, an av1C
// record: four header bytes (marker/version 0x81, profile/level, flags, presentation delay) then the configOBUs. src/media/movie_vdec.cpp
// feeds the OBUs to dav1d before the first packet; treating a bare sequence header as an av1C record skips four of its bytes and leaves
// dav1d parsing a reserved OBU type ("Unknown OBU type 0 of size 12") with no sequence header at all. Exposed for unit tests.
inline constexpr size_t kMovieAv1ConfigInvalid = static_cast<size_t>(-1);
inline size_t movie_av1_config_offset(const uint8_t* data, size_t size) {
    const auto is_obus = [](const uint8_t* p, size_t n) {
        size_t i = 0;
        bool any = false;
        while (i < n) {
            const uint8_t header = p[i++];
            if (header & 0x80) return false;                            // forbidden bit
            const int type = (header >> 3) & 0x0F;
            if (type == 0 || (type >= 8 && type <= 14)) return false;    // reserved in AV1
            if (header & 0x04) {                                        // extension header
                if (i >= n) return false;
                ++i;
            }
            if (header & 0x02) {                                        // has_size: leb128, then the payload
                size_t len = 0;
                int shift = 0;
                bool done = false;
                while (i < n && shift < 56) {
                    const uint8_t b = p[i++];
                    len |= static_cast<size_t>(b & 0x7F) << shift;
                    shift += 7;
                    if (!(b & 0x80)) { done = true; break; }
                }
                if (!done || len > n - i) return false;
                i += len;
            } else {
                i = n;                                                  // no size field: runs to the end
            }
            any = true;
        }
        return any;
    };
    if (size && is_obus(data, size)) return 0;
    if (size >= 4 && data[0] == 0x81 && (size == 4 || is_obus(data + 4, size - 4))) return 4;
    return kMovieAv1ConfigInvalid;
}

// Registers the host decoder and forces main's imports to it. False when off, not such a title, or built without dav1d (NXRT_HAVE_DAV1D).
// Call before the modules are linked; safe to call more than once.
bool install_movie_decoder_hle();

// True once install_movie_decoder_hle() routed the decoder to the host: the game module then puts its player on the CPU-frame path.
bool movie_decoder_enabled();

// Frames the game's player has taken from the host decoder so far; 0 when the decoder is off.
uint64_t movie_frames_acquired();

}  // namespace nxrt::media
