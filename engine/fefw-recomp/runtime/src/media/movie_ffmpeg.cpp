// SPDX-License-Identifier: GPL-3.0-or-later
// FFmpeg's hardware AV1 decoder for the Switch 2 movie decoder. See movie_ffmpeg.h.
//
// No FFmpeg library is linked. A matching avcodec-*.dll / avutil-*.dll pair (plus its swresample-*.dll) is loaded from
// [media] ffmpeg_dir, the play folder's upscalers\ffmpeg or upscalers folder, or beside fefw.exe. The public headers in
// third_party/ffmpeg/include pin the ABI; a pair with a different libavcodec/libavutil major version is refused.
//
// FFmpeg's own AV1 decoder ("av1") is hardware-only: without a working hwaccel it fails with ENOSYS at the first frame header. Its
// software path would be the libdav1d wrapper, which runs outside the engine's thread policy; movie_vdec.cpp falls back to the
// engine's own dav1d instead.
#include <windows.h>

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/avutil.h>
#include <libavutil/error.h>
#include <libavutil/buffer.h>
#include <libavutil/frame.h>
#include <libavutil/hwcontext.h>
#include <libavutil/log.h>
#include <libavutil/mem.h>
#include <libavutil/pixfmt.h>
#include <libavutil/version.h>
}

#include <algorithm>
#include <atomic>
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
    decltype(&::av_log_set_callback) av_log_set_callback = nullptr;
    decltype(&::av_log_format_line2) av_log_format_line2 = nullptr;
    decltype(&::av_strerror) av_strerror = nullptr;
    decltype(&::av_mallocz) av_mallocz = nullptr;
    decltype(&::avcodec_find_decoder_by_name) avcodec_find_decoder_by_name = nullptr;
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
    decltype(&::av_buffer_ref) av_buffer_ref = nullptr;
    decltype(&::av_buffer_unref) av_buffer_unref = nullptr;
};

std::mutex g_mu;
const Api* g_api = nullptr;
std::string g_version;

// FFmpeg's warnings and errors, into the log under their own tag (the game's Opus decoder in services/misc.cpp shares the DLLs and so this
// callback): why a GPU refused a movie is only in them (the Vulkan device's own complaint). A broken stream can warn on every packet, so
// the count is capped; open() starts it again for every stream.
decltype(&::av_log_format_line2) g_format_line = nullptr;
std::atomic<int> g_log_lines{0};
constexpr int kMaxLogLines = 200;

void log_callback(void* avcl, int level, const char* fmt, va_list vl) {
    if (level > AV_LOG_WARNING || !g_format_line) return;
    const int n = g_log_lines.fetch_add(1, std::memory_order_relaxed);
    if (n >= kMaxLogLines) {
        if (n == kMaxLogLines) NXLOG_W("ffmpeg", "further messages are not logged until the next movie");
        return;
    }
    thread_local int print_prefix = 1;   // FFmpeg's state for a message that comes in pieces
    char line[1024];
    if (g_format_line(avcl, level, fmt, vl, line, sizeof line, &print_prefix) < 0) return;
    size_t len = std::strlen(line);
    while (len && (line[len - 1] == '\n' || line[len - 1] == '\r')) line[--len] = '\0';
    if (len) NXLOG_W("ffmpeg", "%s", line);
}

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
    if (!resolve(u, "av_log_set_callback", api->av_log_set_callback)) return fail("av_log_set_callback");
    if (!resolve(u, "av_log_format_line2", api->av_log_format_line2)) return fail("av_log_format_line2");
    if (!resolve(u, "av_strerror", api->av_strerror)) return fail("av_strerror");
    if (!resolve(u, "av_mallocz", api->av_mallocz)) return fail("av_mallocz");
    if (!resolve(c, "avcodec_find_decoder_by_name", api->avcodec_find_decoder_by_name)) return fail("avcodec_find_decoder_by_name");
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
    if (!resolve(u, "av_buffer_ref", api->av_buffer_ref)) return fail("av_buffer_ref");
    if (!resolve(u, "av_buffer_unref", api->av_buffer_unref)) return fail("av_buffer_unref");
    api->av_log_set_level(AV_LOG_WARNING);
    g_format_line = api->av_log_format_line2;
    api->av_log_set_callback(log_callback);
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

std::string error_text(const Api* api, int err) {
    char text[AV_ERROR_MAX_STRING_SIZE] = {};
    if (api && api->av_strerror) api->av_strerror(err, text, sizeof text);
    return std::string(text[0] ? text : "error") + " (" + std::to_string(err) + ")";
}

uint64_t nv12_bytes(uint32_t w, uint32_t h) {
    return static_cast<uint64_t>(w) * h + static_cast<uint64_t>((w + 1) / 2) * 2 * ((h + 1) / 2);
}

#ifdef __APPLE__
constexpr AVHWDeviceType kDeviceType = AV_HWDEVICE_TYPE_VIDEOTOOLBOX;   // the Mac's one GPU: no device name
constexpr const char* kHardwareName = "VideoToolbox";
#else
constexpr AVHWDeviceType kDeviceType = AV_HWDEVICE_TYPE_VULKAN;
constexpr const char* kHardwareName = "Vulkan";
#endif

// The hardware device, created once (by prepare() during the boot, else by the first movie) and shared by every decoder after it (FFmpeg's
// device context locks its queues). A failed creation is kept as well: the next movie goes to dav1d at once instead of trying again.
struct Device {
    bool tried = false;
    std::string gpu, why;
    AVBufferRef* ref = nullptr;
};
Device g_device;   // under g_mu

// True when the loaded FFmpeg can decode into host memory on `gpu`. Before 9.1 (libavutil 61.17), FFmpeg gives the decoder's pictures
// host-transfer usage on every Vulkan driver but NVIDIA's (libavutil/hwcontext_vulkan.c, vulkan_frames_init), and AMD's driver has no
// memory for such an image: every picture fails with VK_ERROR_OUT_OF_DEVICE_MEMORY (Radeon 780M). 9.1 drops the usage where it cannot be
// allocated (vulkan_host_transfer_usable). The check is on the DLL's version, as 9.0 has the same major. NVIDIA's device names start
// with "NVIDIA".
bool decodes_on(const Api* api, const std::string& gpu) {
    constexpr unsigned kHostTransferChecked = AV_VERSION_INT(61, 17, 100);
    return kDeviceType != AV_HWDEVICE_TYPE_VULKAN || api->avutil_version() >= kHostTransferChecked || gpu.rfind("NVIDIA", 0) == 0;
}

// A new reference to the device for `gpu`, or null with `why`.
AVBufferRef* device_ref(const Api* api, const std::string& gpu, std::string& why) {
    std::lock_guard<std::mutex> lk(g_mu);
    if (!g_device.tried || g_device.gpu != gpu) {
        if (g_device.ref) api->av_buffer_unref(&g_device.ref);
        g_device = Device{true, gpu, {}, nullptr};
        const char* name = kDeviceType == AV_HWDEVICE_TYPE_VULKAN && !gpu.empty() ? gpu.c_str() : nullptr;
        if (!decodes_on(api, gpu)) {
            g_device.why = "FFmpeg " + g_version + " decodes on NVIDIA's GPUs only; other GPUs need FFmpeg 9.1";
        } else if (const int r = api->av_hwdevice_ctx_create(&g_device.ref, kDeviceType, name, nullptr, 0); r < 0 || !g_device.ref) {
            if (g_device.ref) api->av_buffer_unref(&g_device.ref);
            g_device.why = std::string("no ") + kHardwareName + " device" + (name ? std::string(" named ") + name : std::string()) + ": " +
                           error_text(api, r);
        }
    }
    if (!g_device.ref) {
        why = g_device.why;
        return nullptr;
    }
    AVBufferRef* ref = api->av_buffer_ref(g_device.ref);
    if (!ref) why = "out of memory for a device reference";
    return ref;
}

}  // namespace

bool available() { return find_api() != nullptr; }

const char* version() {
    std::lock_guard<std::mutex> lk(g_mu);
    return g_api ? g_version.c_str() : "";
}

const char* hardware_name() { return kHardwareName; }

struct Decoder {
    const Api* api = nullptr;
    AVCodecContext* ctx = nullptr;
    AVPacket* pkt = nullptr;
    AVFrame* frame = nullptr;
    AVFrame* sw = nullptr;     // the picture copied back from the GPU; its buffer is kept while the size and format stay
    int64_t last_time = 0;
    uint64_t pictures = 0;     // pictures handed out
    uint64_t errors = 0;
    bool warned_lost = false;
};

namespace {

void note_error(Decoder* d, const char* what, int err) {
    if (++d->errors <= 3) NXLOG_W(kTag, "FFmpeg %s: %s", what, error_text(d->api, err).c_str());
}

// Copies one decoded picture back from the GPU into `out`. False when it is lost: the copy failed, or it did not come back as NV12 (the
// device's format for 8-bit 4:2:0; FFmpeg's av1 decoder decodes on the GPU only).
bool emit(Decoder* d, const AVFrame* src, int64_t time, std::vector<Frame>& out) {
    if (!src->hw_frames_ctx || src->width <= 0 || src->height <= 0) return false;
    const auto* frames = reinterpret_cast<const AVHWFramesContext*>(src->hw_frames_ctx->data);
    AVFrame* pic = d->sw;
    if (pic->buf[0] && (pic->format != frames->sw_format || pic->width != src->width || pic->height != src->height)) d->api->av_frame_unref(pic);
    const int r = d->api->av_hwframe_transfer_data(pic, src, 0);   // into the kept buffer, else a new one in the device's format
    if (r < 0 || pic->format != AV_PIX_FMT_NV12) {
        if (!d->warned_lost) {
            const std::string reason = r < 0 ? error_text(d->api, r) : "format " + std::to_string(pic->format) + " is not NV12";
            NXLOG_W(kTag, "FFmpeg: a picture could not come back from the GPU: %s", reason.c_str());
        }
        d->warned_lost = true;
        return false;
    }
    const uint32_t w = static_cast<uint32_t>(src->width), h = static_cast<uint32_t>(src->height);
    const uint32_t chroma_rows = (h + 1) / 2, chroma_row = (w + 1) / 2 * 2;
    Frame f{std::vector<uint8_t>(nv12_bytes(w, h)), w, h, src->pts != AV_NOPTS_VALUE ? src->pts : time};
    uint8_t* const luma = f.nv12.data();
    uint8_t* const chroma = luma + static_cast<uint64_t>(w) * h;
    for (uint32_t y = 0; y < h; ++y)
        std::memcpy(luma + static_cast<uint64_t>(y) * w, pic->data[0] + static_cast<ptrdiff_t>(y) * pic->linesize[0], w);
    for (uint32_t y = 0; y < chroma_rows; ++y)
        std::memcpy(chroma + static_cast<uint64_t>(y) * chroma_row, pic->data[1] + static_cast<ptrdiff_t>(y) * pic->linesize[1], chroma_row);
    out.push_back(std::move(f));
    return true;
}

// Takes every picture FFmpeg has ready; `time` stamps one that carries no timestamp. False on an error or a lost picture.
bool receive(Decoder* d, std::vector<Frame>& out, int64_t time) {
    bool ok = true;
    for (;;) {
        const int r = d->api->avcodec_receive_frame(d->ctx, d->frame);
        if (r == AVERROR(EAGAIN) || r == AVERROR_EOF) return ok;
        if (r < 0) {
            note_error(d, "avcodec_receive_frame", r);
            return false;
        }
        if (emit(d, d->frame, time, out)) ++d->pictures;
        else ok = false;
        d->api->av_frame_unref(d->frame);
    }
}

}  // namespace

void prepare(const std::string& gpu) {
    const Api* api = find_api();
    if (!api) return;
    std::string why;
    if (AVBufferRef* device = device_ref(api, gpu, why)) api->av_buffer_unref(&device);
}

Decoder* open(const std::vector<uint8_t>& config, const std::string& gpu, std::string& why) {
    g_log_lines.store(0, std::memory_order_relaxed);
    const Api* api = find_api();
    if (!api) {
        why = "FFmpeg is not available";
        return nullptr;
    }
    const AVCodec* codec = api->avcodec_find_decoder_by_name("av1");
    if (!codec) {
        why = "this FFmpeg build has no av1 decoder";
        return nullptr;
    }
    AVBufferRef* device = device_ref(api, gpu, why);
    if (!device) return nullptr;
    auto* d = new Decoder();
    d->api = api;
    d->pkt = api->av_packet_alloc();
    d->frame = api->av_frame_alloc();
    d->sw = api->av_frame_alloc();
    d->ctx = api->avcodec_alloc_context3(codec);
    if (!d->pkt || !d->frame || !d->sw || !d->ctx) {
        why = "out of memory";
        api->av_buffer_unref(&device);
        close(d);
        return nullptr;
    }
    AVCodecContext* ctx = d->ctx;
    ctx->thread_count = 1;        // the GPU decodes; FFmpeg's av1 decoder has no threads of its own
    ctx->hw_device_ctx = device;  // the context owns this reference
    // The movie library's av1C record, as FFmpeg reads it (cbs_av1: a first byte with the top bit set is an AV1CodecConfigurationRecord):
    // the decoder takes the sequence header from its configOBUs, so the packets need not carry it.
    if (!config.empty()) {
        ctx->extradata = static_cast<uint8_t*>(api->av_mallocz(config.size() + AV_INPUT_BUFFER_PADDING_SIZE));
        if (!ctx->extradata) {
            why = "out of memory for the av1C extradata";
            close(d);
            return nullptr;
        }
        std::memcpy(ctx->extradata, config.data(), config.size());
        ctx->extradata_size = static_cast<int>(config.size());
    }
    const int r = api->avcodec_open2(ctx, codec, nullptr);
    if (r < 0) {
        why = "avcodec_open2: " + error_text(api, r);
        close(d);
        return nullptr;
    }
    return d;
}

void close(Decoder* d) {
    if (!d) return;
    if (d->errors > 3) NXLOG_W(kTag, "FFmpeg: %llu decode errors in this stream", static_cast<unsigned long long>(d->errors));
    if (d->api) {
        if (d->ctx) d->api->avcodec_free_context(&d->ctx);
        if (d->pkt) d->api->av_packet_free(&d->pkt);
        if (d->frame) d->api->av_frame_free(&d->frame);
        if (d->sw) d->api->av_frame_free(&d->sw);
    }
    delete d;
}

bool decode(Decoder* d, const uint8_t* data, size_t size, int64_t time, std::vector<Frame>& out) {
    if (!d || !d->ctx || !data || !size) return true;
    if (size > static_cast<size_t>(INT_MAX)) {
        NXLOG_W(kTag, "FFmpeg: a %zu-byte packet is too large", size);
        return true;
    }
    d->last_time = time;
    int r = d->api->av_new_packet(d->pkt, static_cast<int>(size));
    if (r < 0) {
        note_error(d, "av_new_packet", r);
        return true;
    }
    std::memcpy(d->pkt->data, data, size);
    d->pkt->pts = time;
    bool ok = true;
    for (int attempt = 0; attempt < 2; ++attempt) {
        r = d->api->avcodec_send_packet(d->ctx, d->pkt);
        if (r != AVERROR(EAGAIN)) break;
        ok &= receive(d, out, time);   // no room for the packet: take the pictures in the way
    }
    d->api->av_packet_unref(d->pkt);
    if (r < 0) {
        note_error(d, "avcodec_send_packet", r);
        ok = false;
    } else {
        ok &= receive(d, out, time);
    }
    return ok || d->pictures > 0;
}

void drain(Decoder* d, std::vector<Frame>& out) {
    if (!d || !d->ctx) return;
    const int r = d->api->avcodec_send_packet(d->ctx, nullptr);
    if (r < 0 && r != AVERROR_EOF) note_error(d, "avcodec_send_packet (drain)", r);
    receive(d, out, d->last_time);
}

}  // namespace nxrt::media::ffmpeg
