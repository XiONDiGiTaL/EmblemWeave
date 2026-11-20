// SPDX-License-Identifier: GPL-3.0-or-later
// Optional FFmpeg libavcodec backend for the Switch 2 movie decoder. See movie_ffmpeg.h.
//
// No FFmpeg library is linked. A matching avcodec-*.dll / avutil-*.dll pair (plus its swresample-*.dll) is loaded from
// [media] ffmpeg_dir, the play folder's upscalers\ffmpeg or upscalers folder, or beside fefw.exe. The public headers in
// third_party/ffmpeg/include pin the ABI; a pair with a different libavcodec/libavutil major version is refused.
//
// Hardware uses FFmpeg's native AV1 decoder ("av1") because its libdav1d wrapper cannot expose D3D11VA. Software uses
// FFmpeg's libdav1d decoder; the engine's own dav1d remains the default, faster software choice.
#include <windows.h>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/avutil.h>
#include <libavutil/error.h>
#include <libavutil/frame.h>
#include <libavutil/hwcontext.h>
#include <libavutil/log.h>
#include <libavutil/mem.h>
#include <libavutil/pixfmt.h>
#include <libavutil/version.h>
}

#include <algorithm>
#include <cerrno>
#include <climits>
#include <cstddef>
#include <cstring>
#include <filesystem>
#include <mutex>
#include <string>
#include <vector>

#include "nxrt/config.h"
#include "nxrt/log.h"

#include "movie_ffmpeg.h"

namespace nxrt::media::ffmpeg {
namespace {

namespace fs = std::filesystem;

constexpr const char* kTag = "movie";

// The public FFmpeg API used here. Resolved from the DLLs at run time.
struct Api {
    HMODULE util_mod = nullptr, codec_mod = nullptr;
    decltype(&::avutil_version) avutil_version = nullptr;
    decltype(&::avcodec_version) avcodec_version = nullptr;
    decltype(&::av_version_info) av_version_info = nullptr;
    decltype(&::av_log_set_level) av_log_set_level = nullptr;
    decltype(&::av_strerror) av_strerror = nullptr;
    decltype(&::av_mallocz) av_mallocz = nullptr;
    decltype(&::avcodec_find_decoder_by_name) avcodec_find_decoder_by_name = nullptr;
    decltype(&::avcodec_find_decoder) avcodec_find_decoder = nullptr;
    decltype(&::avcodec_alloc_context3) avcodec_alloc_context3 = nullptr;
    decltype(&::avcodec_free_context) avcodec_free_context = nullptr;
    decltype(&::avcodec_open2) avcodec_open2 = nullptr;
    decltype(&::avcodec_send_packet) avcodec_send_packet = nullptr;
    decltype(&::avcodec_receive_frame) avcodec_receive_frame = nullptr;
    decltype(&::av_packet_alloc) av_packet_alloc = nullptr;
    decltype(&::av_packet_free) av_packet_free = nullptr;
    decltype(&::av_new_packet) av_new_packet = nullptr;
    decltype(&::av_packet_unref) av_packet_unref = nullptr;
    decltype(&::av_frame_alloc) av_frame_alloc = nullptr;
    decltype(&::av_frame_free) av_frame_free = nullptr;
    decltype(&::av_frame_unref) av_frame_unref = nullptr;
    decltype(&::av_hwdevice_ctx_create) av_hwdevice_ctx_create = nullptr;
    decltype(&::av_hwframe_transfer_data) av_hwframe_transfer_data = nullptr;
    decltype(&::av_buffer_unref) av_buffer_unref = nullptr;
};

std::mutex g_mu;
const Api* g_api = nullptr;
std::string g_version;

template <typename T>
bool resolve(HMODULE module, const char* name, T& fn) {
    fn = reinterpret_cast<T>(GetProcAddress(module, name));
    return fn != nullptr;
}

std::wstring widen(const std::string& s) {
    if (s.empty()) return {};
    const int n = MultiByteToWideChar(CP_UTF8, 0, s.c_str(), -1, nullptr, 0);
    if (n <= 0) return {};
    std::wstring w(static_cast<size_t>(n), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s.c_str(), -1, w.data(), n);
    if (!w.empty() && w.back() == L'\0') w.pop_back();
    return w;
}

std::string narrow(const std::wstring& s) {
    if (s.empty()) return {};
    const int n = WideCharToMultiByte(CP_UTF8, 0, s.c_str(), -1, nullptr, 0, nullptr, nullptr);
    if (n <= 0) return {};
    std::string out(static_cast<size_t>(n), '\0');
    WideCharToMultiByte(CP_UTF8, 0, s.c_str(), -1, out.data(), n, nullptr, nullptr);
    if (!out.empty() && out.back() == '\0') out.pop_back();
    return out;
}

std::wstring lower(std::wstring s) {
    for (wchar_t& c : s)
        if (c >= L'A' && c <= L'Z') c = static_cast<wchar_t>(c - L'A' + L'a');
    return s;
}

bool dll_named(const fs::path& p, const wchar_t* prefix, std::wstring& name) {
    name = lower(p.filename().wstring());
    const std::wstring pre(prefix);
    return name.size() > pre.size() + 4 && name.compare(0, pre.size(), pre) == 0 &&
           name.compare(name.size() - 4, 4, L".dll") == 0;
}

// A directory with any avcodec-*.dll and avutil-*.dll pair. The version check below decides whether the pair is usable.
bool pair_in(const fs::path& dir, fs::path& codec, fs::path& util) {
    std::error_code ec;
    if (!fs::is_directory(dir, ec)) return false;
    std::vector<fs::path> codecs, utils;
    for (fs::directory_iterator it(dir, fs::directory_options::skip_permission_denied, ec), end; it != end && !ec; it.increment(ec)) {
        std::wstring name;
        if (dll_named(it->path(), L"avcodec-", name)) codecs.push_back(it->path());
        else if (dll_named(it->path(), L"avutil-", name)) utils.push_back(it->path());
    }
    std::sort(codecs.begin(), codecs.end());
    std::sort(utils.begin(), utils.end());
    // Prefer the major pair this build's headers describe, then try the others so the loader can report a version mismatch.
    for (const auto& c : codecs)
        for (const auto& u : utils) {
            const std::wstring cn = lower(c.filename().wstring()), un = lower(u.filename().wstring());
            if (cn == L"avcodec-62.dll" && un == L"avutil-60.dll") {
                codec = c;
                util = u;
                return true;
            }
        }
    if (!codecs.empty() && !utils.empty()) {
        codec = codecs.front();
        util = utils.front();
        return true;
    }
    return false;
}

const Api* load_pair(const fs::path& codec, const fs::path& util, std::string& why) {
    HMODULE u = LoadLibraryExW(util.c_str(), nullptr, LOAD_WITH_ALTERED_SEARCH_PATH);
    if (!u) {
        why = "avutil could not be loaded (" + std::to_string(GetLastError()) + ")";
        return nullptr;
    }
    // avcodec imports swresample (and, in other builds, other FFmpeg DLLs) from its own folder. Add that folder to
    // the dependency search for the duration of the load; LOAD_WITH_ALTERED_SEARCH_PATH alone does not cover it.
    const fs::path dir = codec.parent_path();
    DLL_DIRECTORY_COOKIE cookie = AddDllDirectory(dir.c_str());
    HMODULE c = LoadLibraryExW(codec.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!c) c = LoadLibraryExW(codec.c_str(), nullptr, LOAD_WITH_ALTERED_SEARCH_PATH);
    if (cookie) RemoveDllDirectory(cookie);
    if (!c) {
        why = "avcodec could not be loaded (" + std::to_string(GetLastError()) + "); keep avutil-60.dll and swresample-6.dll beside it";
        FreeLibrary(u);
        return nullptr;
    }
    auto* api = new Api();
    api->util_mod = u;
    api->codec_mod = c;
    auto fail = [&](const char* missing) {
        why = std::string("a function is missing: ") + missing;
        FreeLibrary(c);
        FreeLibrary(u);
        delete api;
        return static_cast<const Api*>(nullptr);
    };
    if (!resolve(u, "avutil_version", api->avutil_version)) return fail("avutil_version");
    if (!resolve(c, "avcodec_version", api->avcodec_version)) return fail("avcodec_version");
    const unsigned uv = api->avutil_version(), cv = api->avcodec_version();
    if (AV_VERSION_MAJOR(uv) != LIBAVUTIL_VERSION_MAJOR || AV_VERSION_MAJOR(cv) != LIBAVCODEC_VERSION_MAJOR) {
        why = "version " + std::to_string(AV_VERSION_MAJOR(cv)) + "." + std::to_string(AV_VERSION_MINOR(cv)) +
              " does not match this build's " + std::to_string(LIBAVCODEC_VERSION_MAJOR) + ".x";
        FreeLibrary(c);
        FreeLibrary(u);
        delete api;
        return nullptr;
    }
    if (!resolve(u, "av_version_info", api->av_version_info)) return fail("av_version_info");
    if (!resolve(u, "av_log_set_level", api->av_log_set_level)) return fail("av_log_set_level");
    if (!resolve(u, "av_strerror", api->av_strerror)) return fail("av_strerror");
    if (!resolve(u, "av_mallocz", api->av_mallocz)) return fail("av_mallocz");
    if (!resolve(c, "avcodec_find_decoder_by_name", api->avcodec_find_decoder_by_name)) return fail("avcodec_find_decoder_by_name");
    if (!resolve(c, "avcodec_find_decoder", api->avcodec_find_decoder)) return fail("avcodec_find_decoder");
    if (!resolve(c, "avcodec_alloc_context3", api->avcodec_alloc_context3)) return fail("avcodec_alloc_context3");
    if (!resolve(c, "avcodec_free_context", api->avcodec_free_context)) return fail("avcodec_free_context");
    if (!resolve(c, "avcodec_open2", api->avcodec_open2)) return fail("avcodec_open2");
    if (!resolve(c, "avcodec_send_packet", api->avcodec_send_packet)) return fail("avcodec_send_packet");
    if (!resolve(c, "avcodec_receive_frame", api->avcodec_receive_frame)) return fail("avcodec_receive_frame");
    if (!resolve(c, "av_packet_alloc", api->av_packet_alloc)) return fail("av_packet_alloc");
    if (!resolve(c, "av_packet_free", api->av_packet_free)) return fail("av_packet_free");
    if (!resolve(c, "av_new_packet", api->av_new_packet)) return fail("av_new_packet");
    if (!resolve(c, "av_packet_unref", api->av_packet_unref)) return fail("av_packet_unref");
    if (!resolve(u, "av_frame_alloc", api->av_frame_alloc)) return fail("av_frame_alloc");
    if (!resolve(u, "av_frame_free", api->av_frame_free)) return fail("av_frame_free");
    if (!resolve(u, "av_frame_unref", api->av_frame_unref)) return fail("av_frame_unref");
    if (!resolve(u, "av_hwdevice_ctx_create", api->av_hwdevice_ctx_create)) return fail("av_hwdevice_ctx_create");
    if (!resolve(u, "av_hwframe_transfer_data", api->av_hwframe_transfer_data)) return fail("av_hwframe_transfer_data");
    if (!resolve(u, "av_buffer_unref", api->av_buffer_unref)) return fail("av_buffer_unref");
    api->av_log_set_level(AV_LOG_ERROR);
    g_version = api->av_version_info() ? api->av_version_info() : "";
    return api;
}

const Api* find_api() {
    std::lock_guard<std::mutex> lk(g_mu);
    if (g_api) return g_api;
    if (g_version == "!") return nullptr;   // a previous attempt failed; do not rescan on every movie
    std::vector<fs::path> dirs;
    const std::string configured = config_get_str("media", "ffmpeg_dir", "");
    if (!configured.empty()) dirs.emplace_back(widen(configured));
    wchar_t exe[MAX_PATH] = {};
    if (GetModuleFileNameW(nullptr, exe, MAX_PATH)) {
        const fs::path base = fs::path(exe).parent_path();
        dirs.push_back(base / L"upscalers" / L"ffmpeg");
        dirs.push_back(base / L"upscalers");
        dirs.push_back(base);
    }
    std::string first_why;
    for (const fs::path& dir : dirs) {
        fs::path codec, util;
        if (!pair_in(dir, codec, util)) continue;
        std::string why;
        if (const Api* api = load_pair(codec, util, why)) {
            NXLOG_I(kTag, "FFmpeg %s loaded from %s (libavcodec %u, libavutil %u)", g_version.c_str(), narrow(dir.wstring()).c_str(),
                    api->avcodec_version() >> 16, api->avutil_version() >> 16);
            g_api = api;
            return g_api;
        }
        if (first_why.empty()) first_why = why + " (" + narrow(codec.filename().wstring()) + ", " + narrow(util.filename().wstring()) + ")";
    }
    g_version = "!";
    if (!first_why.empty()) NXLOG_W(kTag, "FFmpeg is not usable: %s", first_why.c_str());
    else NXLOG_W(kTag, "FFmpeg is not installed: put avcodec-62.dll, avutil-60.dll and swresample-6.dll in upscalers\\ffmpeg");
    return nullptr;
}

void log_error(const char* what, int err) {
    char text[AV_ERROR_MAX_STRING_SIZE] = {};
    if (g_api && g_api->av_strerror) g_api->av_strerror(err, text, sizeof text);
    NXLOG_W(kTag, "FFmpeg %s: %s (%d)", what, text[0] ? text : "error", err);
}

uint64_t nv12_bytes(uint32_t w, uint32_t h) {
    return static_cast<uint64_t>(w) * h + static_cast<uint64_t>((w + 1) / 2) * 2 * ((h + 1) / 2);
}

}  // namespace

bool available() { return find_api() != nullptr; }

const char* version() {
    std::lock_guard<std::mutex> lk(g_mu);
    return g_api ? g_version.c_str() : "";
}

struct Decoder {
    const Api* api = nullptr;
    AVCodecContext* ctx = nullptr;
    AVPacket* pkt = nullptr;
    AVFrame* frame = nullptr;
    AVFrame* sw = nullptr;
    bool hardware = false;
    const char* codec_name = nullptr;
    std::vector<uint8_t> config;
    int threads = 2;
    int64_t last_time = 0;
    bool decoded_any = false;
    bool warned_format = false, warned_transfer = false;
};

namespace {

void emit(Decoder* d, AVFrame* src, int64_t time, std::vector<Frame>& out) {
    if (!src || src->width <= 0 || src->height <= 0) return;
    if (src->format == AV_PIX_FMT_D3D11) {
        if (!d->sw) return;
        d->api->av_frame_unref(d->sw);
        d->sw->format = AV_PIX_FMT_NV12;
        d->sw->width = src->width;
        d->sw->height = src->height;
        const int r = d->api->av_hwframe_transfer_data(d->sw, src, 0);
        if (r < 0) {
            if (!d->warned_transfer) NXLOG_W(kTag, "FFmpeg: D3D11VA frame could not be copied back to NV12; this picture is dropped");
            d->warned_transfer = true;
            return;
        }
        src = d->sw;
    }
    const uint32_t w = static_cast<uint32_t>(src->width), h = static_cast<uint32_t>(src->height);
    Frame f;
    f.width = w;
    f.height = h;
    f.time = time;
    f.nv12.resize(nv12_bytes(w, h));
    if (src->format == AV_PIX_FMT_NV12) {
        for (uint32_t y = 0; y < h; ++y)
            std::memcpy(f.nv12.data() + static_cast<uint64_t>(y) * w, src->data[0] + static_cast<ptrdiff_t>(y) * src->linesize[0], w);
        auto* uv = f.nv12.data() + static_cast<uint64_t>(w) * h;
        for (uint32_t y = 0; y < h / 2; ++y)
            std::memcpy(uv + static_cast<uint64_t>(y) * w, src->data[1] + static_cast<ptrdiff_t>(y) * src->linesize[1], w);
    } else if (src->format == AV_PIX_FMT_YUV420P || src->format == AV_PIX_FMT_YUVJ420P) {
        for (uint32_t y = 0; y < h; ++y)
            std::memcpy(f.nv12.data() + static_cast<uint64_t>(y) * w, src->data[0] + static_cast<ptrdiff_t>(y) * src->linesize[0], w);
        auto* uv = f.nv12.data() + static_cast<uint64_t>(w) * h;
        const uint32_t cw = (w + 1) / 2, ch = (h + 1) / 2;
        for (uint32_t y = 0; y < ch; ++y) {
            const uint8_t* u = src->data[1] + static_cast<ptrdiff_t>(y) * src->linesize[1];
            const uint8_t* v = src->data[2] + static_cast<ptrdiff_t>(y) * src->linesize[2];
            uint8_t* o = uv + static_cast<uint64_t>(y) * w;
            for (uint32_t x = 0; x < cw; ++x) {
                o[2 * x] = u[x];
                o[2 * x + 1] = v[x];
            }
        }
    } else {
        if (!d->warned_format) NXLOG_W(kTag, "FFmpeg: picture format %d is not handled (8-bit 4:2:0 only)", src->format);
        d->warned_format = true;
        return;
    }
    d->decoded_any = true;
    out.push_back(std::move(f));
}

void receive(Decoder* d, std::vector<Frame>& out, int64_t time) {
    if (!d->ctx || !d->frame) return;
    for (;;) {
        const int r = d->api->avcodec_receive_frame(d->ctx, d->frame);
        if (r == AVERROR(EAGAIN) || r == AVERROR_EOF) return;
        if (r < 0) {
            log_error("avcodec_receive_frame", r);
            return;
        }
        emit(d, d->frame, time, out);
        d->api->av_frame_unref(d->frame);
    }
}

bool open_context(Decoder* d, const AVCodec* codec, const std::vector<uint8_t>& config, int threads, bool hardware, std::string& why) {
    AVCodecContext* ctx = d->api->avcodec_alloc_context3(codec);
    if (!ctx) {
        why = "avcodec_alloc_context3 failed";
        return false;
    }
    ctx->thread_count = std::max(1, threads);
    ctx->thread_type = FF_THREAD_SLICE;
    ctx->pkt_timebase = AVRational{1, 1000000};
    // The movie library's av1C record is four bytes of record header followed by the configOBUs. FFmpeg's AV1 decoder
    // wants the OBUs alone; the 0x81 marker byte is not an OBU header.
    const size_t skip = config.size() > 4 && config[0] == 0x81 ? 4 : 0;
    if (config.size() > skip) {
        const size_t extra_size = config.size() - skip;
        ctx->extradata = static_cast<uint8_t*>(d->api->av_mallocz(extra_size + AV_INPUT_BUFFER_PADDING_SIZE));
        if (!ctx->extradata) {
            why = "out of memory for the av1C extradata";
            d->api->avcodec_free_context(&ctx);
            return false;
        }
        std::memcpy(ctx->extradata, config.data() + skip, extra_size);
        ctx->extradata_size = static_cast<int>(extra_size);
    }
    if (hardware) {
        AVBufferRef* device = nullptr;
        const int r = d->api->av_hwdevice_ctx_create(&device, AV_HWDEVICE_TYPE_D3D11VA, nullptr, nullptr, 0);
        if (r < 0 || !device) {
            why = "the D3D11VA device is unavailable";
            if (device) d->api->av_buffer_unref(&device);
            d->api->avcodec_free_context(&ctx);
            return false;
        }
        ctx->hw_device_ctx = device;   // the context owns this reference
    }
    const int r = d->api->avcodec_open2(ctx, codec, nullptr);
    if (r < 0) {
        char text[AV_ERROR_MAX_STRING_SIZE] = {};
        d->api->av_strerror(r, text, sizeof text);
        why = std::string("avcodec_open2: ") + (text[0] ? text : "error");
        d->api->avcodec_free_context(&ctx);
        return false;
    }
    d->ctx = ctx;
    d->hardware = ctx->hw_device_ctx != nullptr;
    return true;
}

// The movie library sends the av1C record once, out of band. FFmpeg's native av1 decoder reads it as extradata, but its
// libdav1d decoder only parses the record there; it still needs the sequence-header OBU as its first packet, as the
// engine's own dav1d path feeds it. A duplicate sequence header from a later packet is harmless.
void send_config(Decoder* d) {
    if (!d->ctx || d->config.size() <= 4) return;
    const size_t skip = d->config[0] == 0x81 ? 4 : 0;
    const size_t n = d->config.size() - skip;
    if (!n || n > static_cast<size_t>(INT_MAX)) return;
    int r = d->api->av_new_packet(d->pkt, static_cast<int>(n));
    if (r < 0) {
        log_error("av_new_packet (av1C)", r);
        return;
    }
    std::memcpy(d->pkt->data, d->config.data() + skip, n);
    r = d->api->avcodec_send_packet(d->ctx, d->pkt);
    d->api->av_packet_unref(d->pkt);
    if (r < 0 && r != AVERROR(EAGAIN)) {
        log_error("avcodec_send_packet (av1C)", r);
        return;
    }
    if (r == AVERROR(EAGAIN)) {
        std::vector<Frame> none;
        receive(d, none, 0);
    }
}

// A GPU can offer a D3D11VA device and still not decode AV1. FFmpeg only finds that out on the first packet, so switch
// to its libdav1d decoder then; no pictures have been produced yet, so nothing is lost.
bool reopen_software(Decoder* d) {
    const AVCodec* codec = d->api->avcodec_find_decoder_by_name("libdav1d");
    if (!codec) {
        NXLOG_W(kTag, "FFmpeg has no libdav1d decoder to fall back to");
        return false;
    }
    AVCodecContext* old = d->ctx;
    d->ctx = nullptr;
    d->api->avcodec_free_context(&old);
    d->hardware = false;
    std::string why;
    if (!open_context(d, codec, d->config, d->threads, false, why)) {
        NXLOG_W(kTag, "FFmpeg: D3D11VA failed and the software decoder could not start: %s", why.c_str());
        return false;
    }
    d->codec_name = codec->name;
    send_config(d);
    NXLOG_W(kTag, "FFmpeg: D3D11VA could not decode this movie; switched to its libdav1d software decoder");
    return true;
}

}  // namespace

Decoder* open(const std::vector<uint8_t>& config, int threads, bool hardware) {
    const Api* api = find_api();
    if (!api) return nullptr;
    // FFmpeg's native av1 decoder is hardware-only: without a D3D11VA device it returns ENOSYS. Its libdav1d
    // decoder is the software path. (When a build has neither, the caller falls back to the engine's own dav1d.)
    const AVCodec* hw_codec = api->avcodec_find_decoder_by_name("av1");
    const AVCodec* sw_codec = api->avcodec_find_decoder_by_name("libdav1d");
    const bool has_soft = sw_codec != nullptr;
    if (!sw_codec) sw_codec = hw_codec;
    if (!sw_codec) {
        NXLOG_W(kTag, "FFmpeg has no AV1 decoder");
        return nullptr;
    }
    if (!hardware && !has_soft) {
        NXLOG_W(kTag, "FFmpeg has no software AV1 decoder (libdav1d is missing): using the engine's dav1d");
        return nullptr;
    }
    auto* d = new Decoder();
    d->api = api;
    d->config = config;
    d->threads = threads;
    d->pkt = api->av_packet_alloc();
    d->frame = api->av_frame_alloc();
    d->sw = api->av_frame_alloc();
    if (!d->pkt || !d->frame || !d->sw) {
        NXLOG_W(kTag, "FFmpeg could not allocate its packet/frame");
        close(d);
        return nullptr;
    }
    std::string why;
    if (hardware && hw_codec && open_context(d, hw_codec, config, threads, true, why)) {
        d->codec_name = hw_codec->name;
        send_config(d);
        NXLOG_I(kTag, "FFmpeg AV1 decoder: %s, %d threads, D3D11VA", d->codec_name, d->ctx->thread_count);
        return d;
    }
    if (hardware && hw_codec) NXLOG_W(kTag, "FFmpeg: hardware AV1 decode did not open (%s); using its software decoder", why.c_str());
    if (!has_soft) {
        NXLOG_W(kTag, "FFmpeg has no libdav1d software decoder to fall back to: using the engine's dav1d");
        close(d);
        return nullptr;
    }
    why.clear();
    if (!open_context(d, sw_codec, config, threads, false, why)) {
        NXLOG_W(kTag, "FFmpeg could not open its AV1 decoder: %s", why.c_str());
        close(d);
        return nullptr;
    }
    d->codec_name = sw_codec->name;
    send_config(d);
    NXLOG_I(kTag, "FFmpeg AV1 decoder: %s, %d threads, software", d->codec_name, d->ctx->thread_count);
    return d;
}

bool hardware(const Decoder* d) { return d && d->hardware; }

void close(Decoder* d) {
    if (!d) return;
    if (d->api) {
        if (d->ctx) d->api->avcodec_free_context(&d->ctx);
        if (d->pkt) d->api->av_packet_free(&d->pkt);
        if (d->frame) d->api->av_frame_free(&d->frame);
        if (d->sw) d->api->av_frame_free(&d->sw);
    }
    delete d;
}

void decode(Decoder* d, const uint8_t* data, size_t size, int64_t time, std::vector<Frame>& out) {
    if (!d || !d->ctx || !data || !size) return;
    if (size > static_cast<size_t>(INT_MAX)) {
        NXLOG_W(kTag, "FFmpeg: a %zu-byte packet is too large", size);
        return;
    }
    d->last_time = time;
    int r = d->api->av_new_packet(d->pkt, static_cast<int>(size));
    if (r < 0) {
        log_error("av_new_packet", r);
        return;
    }
    std::memcpy(d->pkt->data, data, size);
    d->pkt->pts = time;
    for (int attempt = 0; attempt < 2; ++attempt) {
        r = d->api->avcodec_send_packet(d->ctx, d->pkt);
        if (r != AVERROR(EAGAIN)) break;
        receive(d, out, time);
    }
    if (r < 0 && d->hardware && !d->decoded_any && reopen_software(d)) {
        r = d->api->avcodec_send_packet(d->ctx, d->pkt);
    }
    d->api->av_packet_unref(d->pkt);
    if (r < 0) {
        log_error("avcodec_send_packet", r);
        return;
    }
    receive(d, out, time);
}

void drain(Decoder* d, std::vector<Frame>& out) {
    if (!d || !d->ctx) return;
    const int r = d->api->avcodec_send_packet(d->ctx, nullptr);
    if (r < 0 && r != AVERROR_EOF) log_error("avcodec_send_packet (drain)", r);
    receive(d, out, d->last_time);
}

}  // namespace nxrt::media::ffmpeg
