// SPDX-License-Identifier: GPL-3.0-or-later
// vi:u display services, the layer's binder driver, and nvdrv (S3c). Request bytes and expected replies are written out
// here from Switchbrew (NV_services, Display_services: command ids, the Parcel header, the nvmap structs and their ioctl words)
// and from Ryujinx (NvError numbers, the GM20B characteristics words). Nothing is read back from the service's own code.
#include "svc_test_util.h"

#include <cstring>

NXTEST_GROUP(svc_vi_nv)

using namespace nxrt;

namespace {

void put32(uint8_t* p, uint32_t v) { std::memcpy(p, &v, 4); }
void put64(uint8_t* p, uint64_t v) { std::memcpy(p, &v, 8); }
uint32_t get32(const uint8_t* p) { uint32_t v; std::memcpy(&v, p, 4); return v; }
uint64_t get64(const uint8_t* p) { uint64_t v; std::memcpy(&v, p, 8); return v; }

// A binder request: the Parcel header (data size, data offset 16, objects size 0, objects offset), then the interface token
// (strict policy 0, the descriptor "android.gui.IGraphicBufferProducer" as length 34 and UTF-16LE units with a NUL, padded to 4),
// then the call's words. Total data for the token: 4 + 4 + 72 = 80 bytes.
std::vector<uint8_t> binder_request(const std::vector<uint32_t>& words) {
    const char* desc = "android.gui.IGraphicBufferProducer";
    std::vector<uint8_t> data(80 + 4 * words.size(), 0);
    put32(&data[0], 0);      // strict policy
    put32(&data[4], 34);     // descriptor length in UTF-16 units
    for (size_t i = 0; i < 34; ++i) data[8 + 2 * i] = static_cast<uint8_t>(desc[i]);
    for (size_t i = 0; i < words.size(); ++i) put32(&data[80 + 4 * i], words[i]);
    std::vector<uint8_t> parcel(16 + data.size(), 0);
    put32(&parcel[0], static_cast<uint32_t>(data.size()));
    put32(&parcel[4], 16);
    put32(&parcel[8], 0);
    put32(&parcel[12], static_cast<uint32_t>(16 + data.size()));
    std::memcpy(parcel.data() + 16, data.data(), data.size());
    return parcel;
}

// The Default display's application service through vi:u (GetDisplayService with policy 0).
nxsvc::Session open_app_display() {
    nxsvc::Session vi = nxsvc::connect("vi:u");
    nxsvc::Reply r = vi.call(0, nxsvc::Args().u32(0));
    return vi.sub(r);
}

}  // namespace

NXTEST(vi_nv_display_and_resolution) {
    nxsvc::Env env;
    nxsvc::Session app = open_app_display();
    REQUIRE(app.valid());
    nxsvc::Reply d = app.call(1011);   // OpenDefaultDisplay -> u64 display id
    CHECK_EQ(d.result, 0u);
    CHECK_EQ(d.get<uint64_t>(0), 0ull);
    // Docked console (the profile default): the 1080p resolution, as two s64s
    nxsvc::Reply res = app.call(1102, nxsvc::Args().u64(0));
    CHECK_EQ(res.result, 0u);
    CHECK_EQ(res.get<int64_t>(0), static_cast<int64_t>(1920));
    CHECK_EQ(res.get<int64_t>(8), static_cast<int64_t>(1080));
}

NXTEST(vi_nv_list_displays_and_open_by_name) {
    nxsvc::Env env;
    nxsvc::Session app = open_app_display();
    uint8_t info[0x60 * 6] = {};
    nxsvc::Reply r = app.call(1000, nxsvc::Args().buf_b(info, sizeof info));
    CHECK_EQ(r.result, 0u);
    CHECK_EQ(r.get<uint64_t>(0), 1ull);                       // one display
    CHECK(std::memcmp(info, "Default", 8) == 0);               // DisplayInfo name at 0
    CHECK_EQ(get64(info + 0x50), 1920ull);                    // 1080p even in handheld (Switchbrew Display_services)
    CHECK_EQ(get64(info + 0x58), 1080ull);

    char name[0x40] = {};
    std::memcpy(name, "Default", 7);
    nxsvc::Reply ok = app.call(1010, nxsvc::Args().bytes(name, 0x40));
    CHECK_EQ(ok.result, 0u);
    CHECK_EQ(ok.get<uint64_t>(0), 0ull);
    char bogus[0x40] = {};
    std::memcpy(bogus, "Bogus", 5);
    nxsvc::Reply bad = app.call(1010, nxsvc::Args().bytes(bogus, 0x40));
    CHECK(bad.result != 0u);
}

NXTEST(vi_nv_stray_layer_native_window) {
    nxsvc::Env env;
    nxsvc::Session app = open_app_display();
    uint8_t window[0x100] = {};
    // CreateStrayLayer(u32 flags, pad, u64 display) -> (u64 layer id, i64 native window size)
    nxsvc::Reply r = app.call(2030, nxsvc::Args().u32(0).pad(4).u64(0).buf_b(window, sizeof window));
    CHECK_EQ(r.result, 0u);
    CHECK(r.get<uint64_t>(0) != 0ull);
    CHECK_EQ(r.get<int64_t>(8), static_cast<int64_t>(0x3c));   // a parcel of 0x3c bytes
    // Switchbrew Display_services, Parcel: header, then the data (0x2, the process word, the binder id, "dispdrv\0")
    CHECK_EQ(get32(window + 0), 0x28u);                        // data size
    CHECK_EQ(get32(window + 4), 0x10u);                        // data offset
    CHECK_EQ(get32(window + 8), 4u);                           // objects size
    CHECK_EQ(get32(window + 12), 0x38u);                       // objects offset
    CHECK_EQ(get32(window + 0x10), 2u);                        // magic
    CHECK(std::memcmp(window + 0x28, "dispdrv", 8) == 0);      // NUL-terminated name at data offset 0x18
    CHECK_EQ(get32(window + 0x38), 0u);                        // no objects
    CHECK(get64(window + 0x18) != 0ull);                       // the binder id
}

NXTEST(vi_nv_binder_connect_query_dequeue) {
    nxsvc::Env env;
    nxsvc::Session app = open_app_display();
    uint8_t window[0x100] = {};
    nxsvc::Reply layer = app.call(2030, nxsvc::Args().u32(0).pad(4).u64(0).buf_b(window, sizeof window));
    REQUIRE(layer.result == 0u);
    const int32_t binder = static_cast<int32_t>(get64(window + 0x18));
    REQUIRE(binder != 0);

    nxsvc::Reply relay = app.call(100);   // GetRelayService -> IHOSBinderDriver
    REQUIRE(relay.result == 0u);
    nxsvc::Session driver = app.sub(relay);
    REQUIRE(driver.valid());

    uint8_t reply[0x100] = {};
    // Connect(bool has_listener = 0, u32 api = 1, bool controlled = 0): width, height, transform hint 0x10 (NoVSyncCapability), pending 0, status 0
    std::vector<uint8_t> connect = binder_request({0, 1, 0});
    nxsvc::Reply c = driver.call(0, nxsvc::Args().u32(static_cast<uint32_t>(binder)).u32(10).u32(0).buf_a(connect.data(), connect.size()).buf_b(reply, sizeof reply));
    CHECK_EQ(c.result, 0u);
    CHECK_EQ(get32(reply + 0), 20u);                 // reply data size: five words
    CHECK_EQ(get32(reply + 4), 16u);                 // data offset
    CHECK_EQ(get32(reply + 24), 0x10u);              // transform hint (the third output word)
    CHECK_EQ(get32(reply + 32), 0u);                 // status, the last word of the data

    // Query(u32 what = 0: width) -> value, status: the status word is 0
    std::vector<uint8_t> query = binder_request({0});
    std::memset(reply, 0, sizeof reply);
    c = driver.call(0, nxsvc::Args().u32(static_cast<uint32_t>(binder)).u32(9).u32(0).buf_a(query.data(), query.size()).buf_b(reply, sizeof reply));
    CHECK_EQ(c.result, 0u);
    CHECK_EQ(get32(reply + 0), 8u);
    CHECK(get32(reply + 16) != 0u);                  // the width
    CHECK_EQ(get32(reply + 20), 0u);                 // status

    // DequeueBuffer(bool async, width, height, format, usage): no free buffer, so -EBUSY and no slot
    std::vector<uint8_t> dequeue = binder_request({0, 1280, 720, 1, 0});
    std::memset(reply, 0, sizeof reply);
    c = driver.call(0, nxsvc::Args().u32(static_cast<uint32_t>(binder)).u32(3).u32(0).buf_a(dequeue.data(), dequeue.size()).buf_b(reply, sizeof reply));
    CHECK_EQ(c.result, 0u);
    CHECK_EQ(get32(reply + 16), 0xFFFFFFFFu);        // slot -1
    CHECK_EQ(get32(reply + 20), static_cast<uint32_t>(-16));   // -EBUSY

    // GetBufferHistory: the status first, then a count of 0
    std::vector<uint8_t> history = binder_request({1});
    std::memset(reply, 0, sizeof reply);
    c = driver.call(0, nxsvc::Args().u32(static_cast<uint32_t>(binder)).u32(17).u32(0).buf_a(history.data(), history.size()).buf_b(reply, sizeof reply));
    CHECK_EQ(c.result, 0u);
    CHECK_EQ(get32(reply + 16), 0u);
    CHECK_EQ(get32(reply + 20), 0u);
}

NXTEST(vi_nv_vsync_event_signals) {
    nxsvc::Env env;
    nxsvc::Session app = open_app_display();
    nxsvc::Reply r = app.call(5202, nxsvc::Args().u64(0));   // GetDisplayVsyncEvent -> copy handle
    CHECK_EQ(r.result, 0u);
    REQUIRE(r.copy.size() == 1u);
    uint32_t h = r.copy[0];
    int32_t idx = -1;
    // The host thread signals at 60 Hz: a half-second wait is ample
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 500000000, &idx), 0u);
    CHECK_EQ(idx, 0);
    kernel::handle_close(h);
}

NXTEST(nvdrv_initialize_open_close) {
    nxsvc::Env env;
    nxsvc::Session nv = nxsvc::connect("nvdrv");
    REQUIRE(nv.valid());
    // Open before Initialize: NotInitialized (3), with fd -1 (Ryujinx's EnsureInitialized)
    char nvmap_path[] = "/dev/nvmap";
    nxsvc::Reply early = nv.call(0, nxsvc::Args().buf_a(nvmap_path, sizeof nvmap_path));
    CHECK_EQ(early.get<uint32_t>(4), 3u);

    nxsvc::Reply init = nv.call(3, nxsvc::Args().u32(0x30000));   // Initialize(u32 transfer memory size) -> u32 error
    CHECK_EQ(init.result, 0u);
    CHECK_EQ(init.get<uint32_t>(0), 0u);

    nxsvc::Reply open = nv.call(0, nxsvc::Args().buf_a(nvmap_path, sizeof nvmap_path));
    CHECK_EQ(open.result, 0u);
    CHECK_EQ(open.get<uint32_t>(4), 0u);
    CHECK(open.get<uint32_t>(0) != 0xFFFFFFFFu);

    char bogus[] = "/dev/nope";
    nxsvc::Reply nope = nv.call(0, nxsvc::Args().buf_a(bogus, sizeof bogus));
    CHECK_EQ(nope.get<uint32_t>(4), 0x30003u);       // FileOperationFailed (Ryujinx NvResult)
}

NXTEST(nvdrv_nvmap_create_alloc_free) {
    nxsvc::Env env;
    nxsvc::Session nv = nxsvc::connect("nvdrv");
    nv.call(3, nxsvc::Args().u32(0x30000));
    char nvmap_path[] = "/dev/nvmap";
    nxsvc::Reply open = nv.call(0, nxsvc::Args().buf_a(nvmap_path, sizeof nvmap_path));
    const uint32_t fd = open.get<uint32_t>(0);

    // NVMAP_IOC_CREATE 0xC0080101: in {u32 size = 0x10000}, out {u32 handle at 4}
    uint8_t create_in[8] = {};
    put32(create_in, 0x10000);
    uint8_t create_out[8] = {};
    nxsvc::Reply c = nv.call(1, nxsvc::Args().u32(fd).u32(0xC0080101).buf_a(create_in, 8).buf_b(create_out, 8));
    CHECK_EQ(c.result, 0u);
    CHECK_EQ(c.get<uint32_t>(0), 0u);
    const uint32_t handle = get32(create_out + 4);
    REQUIRE(handle != 0u);

    // NVMAP_IOC_ALLOC 0xC0200104: {handle, heapmask, flags = 1 read-write, align inout, kind u8, pad, addr u64}
    uint8_t alloc[32] = {};
    put32(alloc + 0, handle);
    put32(alloc + 8, 1);
    put32(alloc + 12, 0x1000);
    nxsvc::Reply al = nv.call(1, nxsvc::Args().u32(fd).u32(0xC0200104).buf_a(alloc, 32).buf_b(alloc, 32));
    CHECK_EQ(al.get<uint32_t>(0), 0u);
    CHECK_EQ(get32(alloc + 12), 0x1000u);            // the alignment comes back as it was asked

    // NVMAP_IOC_FREE 0xC0180105: {handle, pad, out u64 address, out u32 size at 16, out u32 flags at 20}
    uint8_t free_in[24] = {};
    put32(free_in, handle);
    uint8_t free_out[24] = {};
    nxsvc::Reply fr = nv.call(1, nxsvc::Args().u32(fd).u32(0xC0180105).buf_a(free_in, 24).buf_b(free_out, 24));
    CHECK_EQ(fr.get<uint32_t>(0), 0u);
    CHECK_EQ(get32(free_out + 16), 0x10000u);        // the size that was created
    CHECK_EQ(get32(free_out + 20), 0u);              // freed, not "not freed yet"
}

NXTEST(nvdrv_unhandled_ioctl_answers_zeros) {
    nxsvc::Env env;
    nxsvc::Session nv = nxsvc::connect("nvdrv");
    nv.call(3, nxsvc::Args().u32(0x30000));
    char nvmap_path[] = "/dev/nvmap";
    nxsvc::Reply open = nv.call(0, nxsvc::Args().buf_a(nvmap_path, sizeof nvmap_path));
    const uint32_t fd = open.get<uint32_t>(0);
    // An nvmap command number the table does not list (0x99), read and write, 8 bytes: success with zeros
    uint8_t in[8];
    std::memset(in, 0xAA, sizeof in);
    uint8_t out[8];
    std::memset(out, 0xBB, sizeof out);
    nxsvc::Reply r = nv.call(1, nxsvc::Args().u32(fd).u32(0xC0080199).buf_a(in, 8).buf_b(out, 8));
    CHECK_EQ(r.get<uint32_t>(0), 0u);
    for (uint8_t b : out) CHECK_EQ(b, 0u);
}

NXTEST(nvdrv_ctrl_gpu_characteristics) {
    nxsvc::Env env;
    nxsvc::Session nv = nxsvc::connect("nvdrv");
    nv.call(3, nxsvc::Args().u32(0x30000));
    char path[] = "/dev/nvhost-ctrl-gpu";
    nxsvc::Reply open = nv.call(0, nxsvc::Args().buf_a(path, sizeof path));
    const uint32_t fd = open.get<uint32_t>(0);
    // NVGPU GET_CHARACTERISTICS (type 0x47, nr 5): 16-byte header, then the GM20B characteristics (0xA0 bytes) at 16
    uint8_t in[176] = {};
    put64(in, 0xA0);
    uint8_t out[176] = {};
    nxsvc::Reply r = nv.call(1, nxsvc::Args().u32(fd).u32(0xC0B04705).buf_a(in, 176).buf_b(out, 176));
    CHECK_EQ(r.get<uint32_t>(0), 0u);
    CHECK_EQ(get64(out), 0xA0ull);                   // buffer size
    CHECK_EQ(get32(out + 16), 0x120u);               // arch
    CHECK_EQ(get32(out + 20), 0xBu);                 // impl
    CHECK_EQ(get32(out + 88), 0x28u);                // GPU VA bits
    CHECK_EQ(get32(out + 160), 0x30326D67u);         // chip name "gm20b", low word
    CHECK_EQ(get32(out + 164), 0x62u);
}

NXTEST(nvdrv_as_gpu_va_regions_and_fixed_alloc) {
    nxsvc::Env env;
    nxsvc::Session nv = nxsvc::connect("nvdrv");
    nv.call(3, nxsvc::Args().u32(0x30000));
    char path[] = "/dev/nvhost-as-gpu";
    nxsvc::Reply open = nv.call(0, nxsvc::Args().buf_a(path, sizeof path));
    const uint32_t fd = open.get<uint32_t>(0);

    // GET_VA_REGIONS 0xC0404108: buffer size 48 (two 24-byte regions), two regions: small pages from 4 GiB, big pages from 16 GiB
    uint8_t va_in[64] = {};
    put32(va_in + 8, 48);
    uint8_t va_out[64] = {};
    nxsvc::Reply va = nv.call(1, nxsvc::Args().u32(fd).u32(0xC0404108).buf_a(va_in, 64).buf_b(va_out, 64));
    CHECK_EQ(va.get<uint32_t>(0), 0u);
    CHECK_EQ(get32(va_out + 8), 48u);
    CHECK_EQ(get64(va_out + 16), 0x100000000ull);
    CHECK_EQ(get32(va_out + 24), 0x1000u);
    CHECK_EQ(get64(va_out + 32), 0x300000ull);
    CHECK_EQ(get64(va_out + 40), 0x400000000ull);
    CHECK_EQ(get32(va_out + 48), 0x10000u);
    CHECK_EQ(get64(va_out + 56), 0x1C0000ull);

    // ALLOC_SPACE 0xC0184102 fixed at 4 GiB, one 4 KiB page: granted; the same range again: out of memory (Ryujinx NvResult 6)
    uint8_t alloc_in[24] = {};
    put32(alloc_in + 0, 1);
    put32(alloc_in + 4, 0x1000);
    put32(alloc_in + 8, 1);                          // FIXED_OFFSET
    put64(alloc_in + 16, 0x100000000ull);
    uint8_t alloc_out[24];
    std::memcpy(alloc_out, alloc_in, 24);
    nxsvc::Reply a1 = nv.call(1, nxsvc::Args().u32(fd).u32(0xC0184102).buf_a(alloc_in, 24).buf_b(alloc_out, 24));
    CHECK_EQ(a1.get<uint32_t>(0), 0u);
    CHECK_EQ(get64(alloc_out + 16), 0x100000000ull);
    uint8_t alloc_again[24];
    std::memcpy(alloc_again, alloc_in, 24);
    uint8_t alloc_again_out[24];
    std::memcpy(alloc_again_out, alloc_in, 24);
    nxsvc::Reply a2 = nv.call(1, nxsvc::Args().u32(fd).u32(0xC0184102).buf_a(alloc_again, 24).buf_b(alloc_again_out, 24));
    CHECK_EQ(a2.get<uint32_t>(0), 6u);
    CHECK_EQ(get64(alloc_again_out + 16), 0ull);
}

NXTEST(nvdrv_query_event_on_ctrl_gpu) {
    nxsvc::Env env;
    nxsvc::Session nv = nxsvc::connect("nvdrv");
    nv.call(3, nxsvc::Args().u32(0x30000));
    char path[] = "/dev/nvhost-ctrl-gpu";
    nxsvc::Reply open = nv.call(0, nxsvc::Args().buf_a(path, sizeof path));
    const uint32_t fd = open.get<uint32_t>(0);
    // QueryEvent(fd, 1): the ErrorEvent of the ctrl-gpu device (Switchbrew NV_services) -> (u32 error, copy handle)
    nxsvc::Reply q = nv.call(4, nxsvc::Args().u32(fd).u32(1));
    CHECK_EQ(q.get<uint32_t>(0), 0u);
    REQUIRE(q.copy.size() == 1u);
    kernel::handle_close(q.copy[0]);
    // An id the device does not have: InvalidValue (11)
    nxsvc::Reply bad = nv.call(4, nxsvc::Args().u32(fd).u32(9));
    CHECK_EQ(bad.get<uint32_t>(0), 11u);
    CHECK_EQ(bad.copy.size(), 0u);
}
