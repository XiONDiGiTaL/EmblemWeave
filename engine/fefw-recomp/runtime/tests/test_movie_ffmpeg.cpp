// SPDX-License-Identifier: GPL-3.0-or-later
// Optional FFmpeg backend smoke test. It needs a shared FFmpeg 8.x build (avcodec-62.dll, avutil-60.dll) and a raw
// AV1 OBU file; both are given by environment variables, so an ordinary test run skips it:
//
//   NXRT_FFMPEG_TEST_DIR=<folder with avcodec-62.dll and avutil-60.dll>
//   NXRT_FFMPEG_TEST_FILE=<one-frame AV1 .obu>
//
// The engine ships no FFmpeg binary; this proves the run-time loader and the D3D11VA/software decoder path against a
// user-supplied build without making FFmpeg a build dependency.
#include <cstdlib>
#include <fstream>
#include <iterator>
#include <string>
#include <vector>

#include "test_framework.h"
#include "nxrt/config.h"

#include "../src/media/movie_ffmpeg.h"

NXTEST_GROUP(media_ffmpeg)

NXTEST(ffmpeg_decodes_a_raw_av1_packet) {
    const char* dir = std::getenv("NXRT_FFMPEG_TEST_DIR");
    const char* file = std::getenv("NXRT_FFMPEG_TEST_FILE");
    if (!dir || !*dir || !file || !*file) return;   // no user-supplied FFmpeg build: nothing to test
    nxrt::config_set("media", "ffmpeg_dir", dir);
    REQUIRE(nxrt::media::ffmpeg::available());
    CHECK(nxrt::media::ffmpeg::version()[0] != '\0');

    std::ifstream in(file, std::ios::binary);
    REQUIRE(in.good());
    const std::vector<uint8_t> data((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    REQUIRE(!data.empty());

    auto* decoder = nxrt::media::ffmpeg::open({}, 2, false);
    REQUIRE(decoder != nullptr);
    std::vector<nxrt::media::ffmpeg::Frame> frames;
    nxrt::media::ffmpeg::decode(decoder, data.data(), data.size(), 1234, frames);
    nxrt::media::ffmpeg::drain(decoder, frames);
    nxrt::media::ffmpeg::close(decoder);

    REQUIRE(!frames.empty());
    CHECK_EQ(frames.front().width, 64u);
    CHECK_EQ(frames.front().height, 64u);
    CHECK_EQ(frames.front().time, 1234);
    CHECK_EQ(frames.front().nv12.size(), 64u * 64u * 3 / 2);
}

NXTEST(ffmpeg_accepts_an_av1c_record_as_extradata) {
    const char* dir = std::getenv("NXRT_FFMPEG_TEST_DIR");
    const char* file = std::getenv("NXRT_FFMPEG_TEST_FILE");
    if (!dir || !*dir || !file || !*file) return;
    nxrt::config_set("media", "ffmpeg_dir", dir);
    REQUIRE(nxrt::media::ffmpeg::available());

    std::ifstream in(file, std::ios::binary);
    REQUIRE(in.good());
    const std::vector<uint8_t> data((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    // The movie library's av1C record header, with no configOBUs: the packet carries the sequence header.
    const std::vector<uint8_t> av1c = {0x81, 0x00, 0x0c, 0x00};
    auto* decoder = nxrt::media::ffmpeg::open(av1c, 2, false);
    REQUIRE(decoder != nullptr);
    std::vector<nxrt::media::ffmpeg::Frame> frames;
    nxrt::media::ffmpeg::decode(decoder, data.data(), data.size(), 0, frames);
    nxrt::media::ffmpeg::drain(decoder, frames);
    nxrt::media::ffmpeg::close(decoder);
    CHECK(!frames.empty());
}

NXTEST(ffmpeg_decodes_a_packet_using_only_av1c_extradata) {
    const char* dir = std::getenv("NXRT_FFMPEG_TEST_DIR");
    const char* extra_file = std::getenv("NXRT_FFMPEG_TEST_EXTRADATA");
    const char* packet_file = std::getenv("NXRT_FFMPEG_TEST_PACKET");
    if (!dir || !*dir || !extra_file || !*extra_file || !packet_file || !*packet_file) return;
    nxrt::config_set("media", "ffmpeg_dir", dir);
    REQUIRE(nxrt::media::ffmpeg::available());

    std::ifstream ein(extra_file, std::ios::binary);
    std::ifstream pin(packet_file, std::ios::binary);
    REQUIRE(ein.good());
    REQUIRE(pin.good());
    const std::vector<uint8_t> extra((std::istreambuf_iterator<char>(ein)), std::istreambuf_iterator<char>());
    const std::vector<uint8_t> packet((std::istreambuf_iterator<char>(pin)), std::istreambuf_iterator<char>());
    // The av1C record from an MP4, and a temporal unit with its sequence header removed: the actual movie shape.
    auto* decoder = nxrt::media::ffmpeg::open(extra, 2, false);
    REQUIRE(decoder != nullptr);
    std::vector<nxrt::media::ffmpeg::Frame> frames;
    nxrt::media::ffmpeg::decode(decoder, packet.data(), packet.size(), 0, frames);
    nxrt::media::ffmpeg::drain(decoder, frames);
    nxrt::media::ffmpeg::close(decoder);
    CHECK(!frames.empty());
}

NXTEST(ffmpeg_uses_d3d11va_when_requested) {
    const char* dir = std::getenv("NXRT_FFMPEG_TEST_DIR");
    const char* file = std::getenv("NXRT_FFMPEG_TEST_FILE");
    const char* extra_file = std::getenv("NXRT_FFMPEG_TEST_EXTRADATA");
    const char* packet_file = std::getenv("NXRT_FFMPEG_TEST_PACKET");
    if (!dir || !*dir || !std::getenv("NXRT_FFMPEG_TEST_HW")) return;
    nxrt::config_set("media", "ffmpeg_dir", dir);
    REQUIRE(nxrt::media::ffmpeg::available());

    std::vector<uint8_t> config, data;
    if (extra_file && *extra_file && packet_file && *packet_file) {
        std::ifstream ein(extra_file, std::ios::binary), pin(packet_file, std::ios::binary);
        REQUIRE(ein.good());
        REQUIRE(pin.good());
        config.assign(std::istreambuf_iterator<char>(ein), std::istreambuf_iterator<char>());
        data.assign(std::istreambuf_iterator<char>(pin), std::istreambuf_iterator<char>());
    } else {
        if (!file || !*file) return;
        std::ifstream in(file, std::ios::binary);
        REQUIRE(in.good());
        data.assign(std::istreambuf_iterator<char>(in), std::istreambuf_iterator<char>());
    }
    auto* decoder = nxrt::media::ffmpeg::open(config, 2, true);
    REQUIRE(decoder != nullptr);
    CHECK(nxrt::media::ffmpeg::hardware(decoder));
    std::vector<nxrt::media::ffmpeg::Frame> frames;
    nxrt::media::ffmpeg::decode(decoder, data.data(), data.size(), 0, frames);
    nxrt::media::ffmpeg::drain(decoder, frames);
    nxrt::media::ffmpeg::close(decoder);
    CHECK(!frames.empty());
}
