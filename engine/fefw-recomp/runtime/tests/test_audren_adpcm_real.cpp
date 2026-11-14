// SPDX-License-Identifier: GPL-3.0-or-later
// Real-data check of the audren DSP-ADPCM port. The streams are exported from the game's CSMP files by
// tools/media/csmp_ref.py into the folder named by NXRT_CSMP_DIR (index.txt lists the ids). For each id:
//   <id>.csmp     DATA chunk payload of the CSMP file: 0x10-byte prefix, 0x60-byte DSP header at 0x10,
//                 ADPCM frames (8 bytes, 14 samples) from 0x70
//   <id>.info     sample_count rate loop_flag c0..c15 ps hist1 hist2 (decimal, one line)
//   <id>.ref.pcm  reference decode, s16le, written by csmp_ref.py
// The port is driven like DataSourceHelper.h drives it: consecutive ranges of `chunk` samples, one
// AdpcmHelper::Decode call per range, one AdpcmLoopContext carried from call to call. Every sample must equal
// the reference. Skipped when NXRT_CSMP_DIR is unset.

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <sstream>
#include <string>
#include <vector>

#include "../src/services/audren/Dsp/AdpcmHelper.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_adpcm_real)

using namespace nxrt::audren;

namespace {

constexpr size_t kPrefixLen = 0x10;
constexpr size_t kDspHeaderLen = 0x60;
constexpr size_t kAdpcmOffset = kPrefixLen + kDspHeaderLen;  // 0x70 from the start of the DATA payload
constexpr size_t kFrameBytes = 8;
constexpr int32_t kSamplesPerFrame = 14;

struct Info {
    int32_t sample_count = 0;
    int32_t rate = 0;
    int32_t loop_flag = 0;
    int16_t coefs[16] = {};
    int32_t ps = 0;
    int32_t hist1 = 0;
    int32_t hist2 = 0;
};

bool ReadBytes(const std::string& path, std::vector<uint8_t>& out) {
    std::ifstream f(path, std::ios::binary);
    if (!f) return false;
    f.seekg(0, std::ios::end);
    std::streamoff n = f.tellg();
    f.seekg(0, std::ios::beg);
    if (n < 0) return false;
    out.resize(static_cast<size_t>(n));
    if (n > 0) f.read(reinterpret_cast<char*>(out.data()), n);
    return static_cast<bool>(f) || n == 0;
}

bool ReadInfo(const std::string& path, Info& info) {
    std::ifstream f(path);
    if (!f) return false;
    std::vector<long long> v;
    long long x;
    while (f >> x) v.push_back(x);
    if (v.size() != 3 + 16 + 3) return false;
    info.sample_count = static_cast<int32_t>(v[0]);
    info.rate = static_cast<int32_t>(v[1]);
    info.loop_flag = static_cast<int32_t>(v[2]);
    for (size_t i = 0; i < 16; i++) info.coefs[i] = static_cast<int16_t>(v[3 + i]);
    info.ps = static_cast<int32_t>(v[19]);
    info.hist1 = static_cast<int32_t>(v[20]);
    info.hist2 = static_cast<int32_t>(v[21]);
    return true;
}

std::vector<std::string> ReadIndex(const std::string& dir) {
    std::vector<std::string> ids;
    std::ifstream f(dir + "/index.txt");
    std::string line;
    while (std::getline(f, line)) {
        while (!line.empty() && (line.back() == '\r' || line.back() == ' ')) line.pop_back();
        if (!line.empty()) ids.push_back(line);
    }
    return ids;
}

uint32_t Le32(const uint8_t* p) {
    return static_cast<uint32_t>(p[0]) | (static_cast<uint32_t>(p[1]) << 8) | (static_cast<uint32_t>(p[2]) << 16) |
           (static_cast<uint32_t>(p[3]) << 24);
}

uint16_t Le16(const uint8_t* p) { return static_cast<uint16_t>(p[0] | (p[1] << 8)); }

// Loads and cross-checks one id. Returns an empty string when everything agrees, otherwise the reason.
std::string LoadId(const std::string& dir, const std::string& id, Info& info, std::vector<uint8_t>& csmp,
                   std::vector<int16_t>& ref) {
    if (!ReadInfo(dir + "/" + id + ".info", info)) return "cannot read or parse .info";
    if (!ReadBytes(dir + "/" + id + ".csmp", csmp)) return "cannot read .csmp";
    if (csmp.size() < kAdpcmOffset) return "csmp shorter than prefix + DSP header";

    const uint8_t* h = csmp.data() + kPrefixLen;
    if (static_cast<int32_t>(Le32(h + 0x00)) != info.sample_count) return "header sample count differs from .info";
    if (static_cast<int32_t>(Le32(h + 0x08)) != info.rate) return "header rate differs from .info";
    if (Le16(h + 0x0E) != 0) return "header format is not ADPCM (0)";
    for (size_t i = 0; i < 16; i++) {
        if (static_cast<int16_t>(Le16(h + 0x1C + 2 * i)) != info.coefs[i]) return "header coefficients differ from .info";
    }

    size_t frames = static_cast<size_t>((info.sample_count + kSamplesPerFrame - 1) / kSamplesPerFrame);
    if (csmp.size() - kAdpcmOffset < frames * kFrameBytes) return "ADPCM data shorter than the sample count needs";

    std::vector<uint8_t> raw;
    if (!ReadBytes(dir + "/" + id + ".ref.pcm", raw)) return "cannot read .ref.pcm";
    if (raw.size() != static_cast<size_t>(info.sample_count) * 2) return ".ref.pcm size is not 2 * sample_count";
    ref.resize(static_cast<size_t>(info.sample_count));
    for (size_t i = 0; i < ref.size(); i++) ref[i] = static_cast<int16_t>(Le16(raw.data() + 2 * i));
    return "";
}

// Decodes the whole stream the way the renderer does: consecutive `chunk` ranges, one Decode call each,
// one loop context carried across the calls (start of stream: predictor/scale and history from the header).
std::vector<int16_t> DecodePort(const Info& info, const uint8_t* adpcm, size_t adpcmSize, int32_t chunk, bool& ok) {
    std::vector<int16_t> all;
    all.reserve(static_cast<size_t>(info.sample_count));
    AdpcmLoopContext ctx{};
    ctx.pred_scale = static_cast<int16_t>(info.ps);
    ctx.history0 = static_cast<int16_t>(info.hist1);
    ctx.history1 = static_cast<int16_t>(info.hist2);
    std::vector<int16_t> buf(static_cast<size_t>(chunk));
    ok = true;
    for (int32_t off = 0; off < info.sample_count; off += chunk) {
        int32_t count = std::min(chunk, info.sample_count - off);
        int32_t n = AdpcmHelper::Decode(Span<int16_t>(buf.data(), static_cast<size_t>(count)),
                                        Span<const uint8_t>(adpcm, adpcmSize), 0, info.sample_count, off, count,
                                        Span<const int16_t>(info.coefs, 16), ctx);
        if (n != count) {
            ok = false;
            return all;
        }
        all.insert(all.end(), buf.begin(), buf.begin() + n);
    }
    return all;
}

// Describes the first mismatch for a failing id: the sample, both values, the frame and its neighbours.
std::string Describe(const std::string& id, const Info& info, const std::vector<uint8_t>& csmp, size_t at, int16_t got,
                     int16_t want) {
    char buf[512];
    size_t frame = at / static_cast<size_t>(kSamplesPerFrame);
    std::snprintf(buf, sizeof buf, "%s: first diff at sample %zu (frame %zu): port %d, reference %d", id.c_str(), at,
                  frame, got, want);
    std::string s = buf;
    for (size_t f = (frame > 0 ? frame - 1 : 0); f <= frame + 1; f++) {
        size_t off = kAdpcmOffset + f * kFrameBytes;
        if (off + kFrameBytes > csmp.size()) break;
        unsigned hdr = csmp[off];
        std::snprintf(buf, sizeof buf, "\n    frame %zu @0x%zx: %02x %02x %02x %02x %02x %02x %02x %02x (pair %u, scale 1<<%u)",
                      f, off, hdr, csmp[off + 1], csmp[off + 2], csmp[off + 3], csmp[off + 4], csmp[off + 5],
                      csmp[off + 6], csmp[off + 7], hdr >> 4, hdr & 15);
        s += buf;
    }
    unsigned pair = csmp[kAdpcmOffset + frame * kFrameBytes] >> 4;
    std::snprintf(buf, sizeof buf, "\n    pair %u coefficients c1=%d c2=%d; start history %d %d (rate %d, loop %d)",
                  pair, pair * 2 < 16 ? info.coefs[pair * 2] : 0, pair * 2 + 1 < 16 ? info.coefs[pair * 2 + 1] : 0,
                  info.hist1, info.hist2, info.rate, info.loop_flag);
    s += buf;
    return s;
}

void RunPass(int32_t chunk) {
    const char* env = std::getenv("NXRT_CSMP_DIR");
    if (env == nullptr || *env == '\0') {
        nxtest::skip("NXRT_CSMP_DIR not set (export the folder from tools/media/csmp_ref.py)");
        return;
    }
    const std::string dir = env;
    const std::vector<std::string> ids = ReadIndex(dir);
    CHECK_MSG(!ids.empty(), "index.txt missing or empty in " + dir);
    if (ids.empty()) return;

    int passed = 0;
    int total = 0;
    for (const std::string& id : ids) {
        total++;
        Info info;
        std::vector<uint8_t> csmp;
        std::vector<int16_t> ref;
        std::string err = LoadId(dir, id, info, csmp, ref);
        if (!err.empty()) {
            ::nxtest::fail(__FILE__, __LINE__, "load", id + ": " + err);
            continue;
        }

        bool ok = false;
        std::vector<int16_t> got = DecodePort(info, csmp.data() + kAdpcmOffset, csmp.size() - kAdpcmOffset, chunk, ok);
        if (!ok || got.size() != ref.size()) {
            ::nxtest::fail(__FILE__, __LINE__, "decode", id + ": port returned fewer samples than requested");
            continue;
        }
        size_t at = ref.size();
        for (size_t i = 0; i < ref.size(); i++) {
            if (got[i] != ref[i]) {
                at = i;
                break;
            }
        }
        if (at == ref.size()) {
            passed++;
            continue;
        }
        ::nxtest::fail(__FILE__, __LINE__, "port == reference", "chunk " + std::to_string(chunk) + ", " +
                       Describe(id, info, csmp, at, got[at], ref[at]));
    }
    std::printf("svc_audren_adpcm_real: chunk %d: %d of %d ids exactly equal the reference\n", chunk, passed, total);
}

}  // namespace

NXTEST(adpcm_real_chunks_240) { RunPass(240); }

NXTEST(adpcm_real_chunks_1000) { RunPass(1000); }

NXTEST(adpcm_real_chunks_77) { RunPass(77); }
