// SPDX-License-Identifier: GPL-3.0-or-later
// FFmpeg's hardware AV1 decoder (src/media/movie_ffmpeg.h) against a real GPU. It needs a shared FFmpeg 8.x build and an AV1 stream, both
// given by environment variables, so an ordinary test run skips it:
//
//   NXRT_FFMPEG_TEST_DIR=<folder with avcodec-62.dll, avutil-60.dll and swresample-6.dll>
//   NXRT_FFMPEG_TEST_IVF=<an AV1 stream in IVF>
//   NXRT_FFMPEG_TEST_NV12=<optional: the stream's pictures from a reference decoder, NV12 one after another (ffmpeg -pix_fmt nv12 -f rawvideo)>
//   NXRT_FFMPEG_TEST_GPU=<the Vulkan device name, as [media] movie_decoder takes it; before FFmpeg 9.1 an NVIDIA GPU's>
//
// The release ships the FFmpeg DLLs, but they are not a build dependency, and a GPU with AV1 decoding is not a CI machine.
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

#include "test_framework.h"
#include "nxrt/config.h"

#include "../src/media/movie_ffmpeg.h"

namespace ff = nxrt::media::ffmpeg;

namespace {

std::string env(const char* name) {
    const char* v = std::getenv(name);
    return v ? v : "";
}

std::vector<uint8_t> read_file(const std::string& path) {
    std::ifstream in(path, std::ios::binary);
    return std::vector<uint8_t>((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
}

uint32_t le32(const uint8_t* p) { return p[0] | p[1] << 8 | p[2] << 16 | static_cast<uint32_t>(p[3]) << 24; }

// The packets of an IVF file: a 32-byte header (its length at offset 6), then per packet 4 bytes of size, 8 of timestamp and the data.
std::vector<std::vector<uint8_t>> ivf_packets(const std::vector<uint8_t>& file) {
    std::vector<std::vector<uint8_t>> packets;
    if (file.size() < 32 || std::memcmp(file.data(), "DKIF", 4) != 0) return packets;
    for (size_t at = file[6] | file[7] << 8; at + 12 <= file.size();) {
        const size_t n = le32(file.data() + at);
        at += 12;
        if (at + n > file.size()) break;
        packets.emplace_back(file.begin() + static_cast<ptrdiff_t>(at), file.begin() + static_cast<ptrdiff_t>(at + n));
        at += n;
    }
    return packets;
}

bool have_ffmpeg() {
    const std::string dir = env("NXRT_FFMPEG_TEST_DIR");
    if (dir.empty()) return false;
    nxrt::config_set("media", "ffmpeg_dir", dir.c_str());
    return ff::available();
}

}  // namespace

NXTEST_GROUP(media_ffmpeg)

// Every packet shows one picture, stamped with that packet's time (not the time of whichever packet was sent last), in NV12 at the
// picture's width; with a reference, bit for bit what a software decoder makes of the stream.
NXTEST(ffmpeg_decodes_an_av1_stream_on_the_gpu) {
    if (env("NXRT_FFMPEG_TEST_IVF").empty() || env("NXRT_FFMPEG_TEST_GPU").empty() || !have_ffmpeg()) return;
    const auto packets = ivf_packets(read_file(env("NXRT_FFMPEG_TEST_IVF")));
    REQUIRE(!packets.empty());

    std::string why;
    ff::Decoder* decoder = ff::open({}, env("NXRT_FFMPEG_TEST_GPU"), why);
    REQUIRE(decoder != nullptr);
    std::vector<ff::Frame> frames;
    for (size_t i = 0; i < packets.size(); ++i)
        CHECK(ff::decode(decoder, packets[i].data(), packets[i].size(), 1000 + 33 * static_cast<int64_t>(i), frames));
    ff::drain(decoder, frames);
    ff::close(decoder);

    REQUIRE(frames.size() == packets.size());
    std::vector<uint8_t> all;
    for (size_t i = 0; i < frames.size(); ++i) {
        const ff::Frame& f = frames[i];
        CHECK_EQ(f.time, 1000 + 33 * static_cast<int64_t>(i));
        CHECK_EQ(f.nv12.size(), static_cast<size_t>(f.width) * f.height + static_cast<size_t>((f.width + 1) / 2) * 2 * ((f.height + 1) / 2));
        all.insert(all.end(), f.nv12.begin(), f.nv12.end());
    }
    if (!env("NXRT_FFMPEG_TEST_NV12").empty()) CHECK(all == read_file(env("NXRT_FFMPEG_TEST_NV12")));
}

// A stream the decoder cannot start reports it at its first packet, with nothing decoded, so the movie decoder hands that packet to dav1d.
NXTEST(ffmpeg_reports_a_stream_it_cannot_start) {
    if (env("NXRT_FFMPEG_TEST_GPU").empty() || !have_ffmpeg()) return;
    std::string why;
    ff::Decoder* decoder = ff::open({}, env("NXRT_FFMPEG_TEST_GPU"), why);
    REQUIRE(decoder != nullptr);
    const std::vector<uint8_t> garbage(64, 0xff);
    std::vector<ff::Frame> frames;
    CHECK(!ff::decode(decoder, garbage.data(), garbage.size(), 0, frames));
    CHECK(frames.empty());
    ff::close(decoder);
}

// A GPU that is not there fails at open with a reason, so the movie decoder can hand the stream to dav1d before its first packet.
NXTEST(ffmpeg_refuses_a_gpu_that_is_not_there) {
    if (!have_ffmpeg()) return;
    std::string why;
    CHECK(ff::open({}, "NVIDIA has no GPU of this name", why) == nullptr);
    CHECK(why.find("NVIDIA has no GPU of this name") != std::string::npos);
}
