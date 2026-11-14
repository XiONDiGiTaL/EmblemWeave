// SPDX-License-Identifier: GPL-3.0-or-later
// NVN builders (NVNdeviceBuilder, NVNqueueBuilder, NVNwindowBuilder, NVNmemoryPoolBuilder, NVNbufferBuilder, NVNeventBuilder)
// through the typed thunks: every function is called the way a game calls it, through abi::proc.
#include <cstdint>
#include <cstring>

#include "nvn_impl.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_builders)

using namespace nxrt;
using namespace nxrt::nvn;

namespace {

// Signatures of the entry points that have no PFN typedef in nvn.h (their table entries are unknown-signature).
using DevGetFlagsFn = int (*)(const NVNdeviceBuilder*);
using QueueSetDeviceFn = void (*)(NVNqueueBuilder*, NVNdevice*);
using QueueGetDeviceFn = NVNdevice* (*)(const NVNqueueBuilder*);
using QueueGetFlagsFn = int (*)(const NVNqueueBuilder*);
using QueueSizeGetFn = size_t (*)(const NVNqueueBuilder*);
using QueueGetMemoryFn = void* (*)(const NVNqueueBuilder*);
using QueueSetPriorityFn = void (*)(NVNqueueBuilder*, int);
using QueueGetPriorityFn = int (*)(const NVNqueueBuilder*);
using QueueSetFlagsFn = void (*)(NVNqueueBuilder*, int);
using WinGetDeviceFn = NVNdevice* (*)(const NVNwindowBuilder*);
using WinGetNumTexturesFn = int (*)(const NVNwindowBuilder*);
using WinGetTextureFn = NVNtexture* (*)(const NVNwindowBuilder*, int);
using WinSetNumActiveFn = void (*)(NVNwindowBuilder*, int);
using PoolGetDeviceFn = NVNdevice* (*)(const NVNmemoryPoolBuilder*);
using BufGetDeviceFn = NVNdevice* (*)(const NVNbufferBuilder*);
using EvtGetPoolFn = const NVNmemoryPool* (*)(const NVNeventBuilder*);
using EvtGetOffsetFn = ptrdiff_t (*)(const NVNeventBuilder*);

void setup() {
    install();
    abi::reset_mismatches();
    register_api_builders();
    CHECK_EQ(abi::mismatches(), 0u);
}

// Objects are first filled with a byte pattern, so a getter cannot pass on a zero object by accident.
template <class T>
void fill(T& o, unsigned char v) {
    std::memset(&o, v, sizeof o);
}

template <class T>
bool all_bytes_are(const T& o, unsigned char v) {
    const auto* p = reinterpret_cast<const unsigned char*>(&o);
    for (size_t i = 0; i < sizeof o; ++i)
        if (p[i] != v) return false;
    return true;
}

}  // namespace

NXTEST(every_listed_function_is_registered) {
    setup();
    const char* names[] = {
        // device builder
        "nvnDeviceBuilderSetDefaults", "nvnDeviceBuilderSetFlags", "nvnDeviceBuilderGetFlags",
        // queue builder
        "nvnQueueBuilderSetDevice", "nvnQueueBuilderGetDevice", "nvnQueueBuilderSetDefaults", "nvnQueueBuilderSetFlags",
        "nvnQueueBuilderGetFlags", "nvnQueueBuilderSetCommandMemorySize", "nvnQueueBuilderGetCommandMemorySize",
        "nvnQueueBuilderSetComputeMemorySize", "nvnQueueBuilderGetComputeMemorySize", "nvnQueueBuilderSetControlMemorySize",
        "nvnQueueBuilderGetControlMemorySize", "nvnQueueBuilderGetQueueMemorySize", "nvnQueueBuilderSetQueueMemory",
        "nvnQueueBuilderGetMemory", "nvnQueueBuilderGetMemorySize", "nvnQueueBuilderSetCommandFlushThreshold",
        "nvnQueueBuilderGetCommandFlushThreshold", "nvnQueueBuilderSetQueuePriority", "nvnQueueBuilderGetQueuePriority",
        // window builder
        "nvnWindowBuilderSetDevice", "nvnWindowBuilderGetDevice", "nvnWindowBuilderSetDefaults",
        "nvnWindowBuilderSetNativeWindow", "nvnWindowBuilderGetNativeWindow", "nvnWindowBuilderSetTextures",
        "nvnWindowBuilderGetNumTextures", "nvnWindowBuilderGetTexture", "nvnWindowBuilderSetPresentInterval",
        "nvnWindowBuilderGetPresentInterval", "nvnWindowBuilderSetNumActiveTextures",
        // memory pool builder
        "nvnMemoryPoolBuilderSetDevice", "nvnMemoryPoolBuilderGetDevice", "nvnMemoryPoolBuilderSetDefaults",
        "nvnMemoryPoolBuilderSetStorage", "nvnMemoryPoolBuilderGetMemory", "nvnMemoryPoolBuilderGetSize",
        "nvnMemoryPoolBuilderSetFlags", "nvnMemoryPoolBuilderGetFlags",
        // buffer builder
        "nvnBufferBuilderSetDevice", "nvnBufferBuilderGetDevice", "nvnBufferBuilderSetDefaults", "nvnBufferBuilderSetStorage",
        "nvnBufferBuilderGetMemoryOffset", "nvnBufferBuilderGetSize",
        // event builder
        "nvnEventBuilderSetDefaults", "nvnEventBuilderSetStorage", "nvnEventBuilderGetMemoryPool",
        "nvnEventBuilderGetMemoryOffset",
    };
    for (const char* n : names) CHECK_MSG(is_implemented(n), n);
}

NXTEST(device_builder_flags_defaults_and_decode) {
    setup();
    auto defaults = abi::proc<PFNNVNDEVICEBUILDERSETDEFAULTSPROC>("nvnDeviceBuilderSetDefaults");
    auto set_flags = abi::proc<PFNNVNDEVICEBUILDERSETFLAGSPROC>("nvnDeviceBuilderSetFlags");
    auto get_flags = abi::proc<DevGetFlagsFn>("nvnDeviceBuilderGetFlags");
    NVNdeviceBuilder b;
    fill(b, 0xAB);
    defaults(&b);
    CHECK(all_bytes_are(b, 0));
    CHECK_EQ(get_flags(&b), 0);
    CHECK_EQ(decode(&b).flags, 0);
    const int values[] = {1, 0x40, 0x7fffffff, -1, 0};
    for (int v : values) {
        set_flags(&b, v);
        CHECK_EQ(get_flags(&b), v);
        CHECK_EQ(decode(&b).flags, v);
    }
    set_flags(&b, 3);
    NVNdeviceBuilder copy;
    std::memcpy(&copy, &b, sizeof b);   // the game copies builders around
    set_flags(&b, 9);
    CHECK_EQ(get_flags(&copy), 3);
    CHECK_EQ(get_flags(&b), 9);
    NVNdeviceBuilder zero;
    std::memset(&zero, 0, sizeof zero);
    CHECK_EQ(decode(&zero).flags, 0);
}

NXTEST(queue_builder_defaults) {
    setup();
    auto defaults = abi::proc<PFNNVNQUEUEBUILDERSETDEFAULTSPROC>("nvnQueueBuilderSetDefaults");
    auto get_device = abi::proc<QueueGetDeviceFn>("nvnQueueBuilderGetDevice");
    auto get_flags = abi::proc<QueueGetFlagsFn>("nvnQueueBuilderGetFlags");
    auto get_cmd = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetCommandMemorySize");
    auto get_compute = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetComputeMemorySize");
    auto get_control = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetControlMemorySize");
    auto get_threshold = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetCommandFlushThreshold");
    auto get_memory = abi::proc<QueueGetMemoryFn>("nvnQueueBuilderGetMemory");
    auto get_memory_size = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetMemorySize");
    auto get_priority = abi::proc<QueueGetPriorityFn>("nvnQueueBuilderGetQueuePriority");
    auto get_queue_size = abi::proc<PFNNVNQUEUEBUILDERGETQUEUEMEMORYSIZEPROC>("nvnQueueBuilderGetQueueMemorySize");
    NVNqueueBuilder q;
    fill(q, 0xCD);
    defaults(&q);
    // smo-recomp runtime/gfx/nvn_gl.c: command 0x10000, compute 0x40000, control 0x4000, flush threshold 0, priority 100.
    CHECK(get_device(&q) == nullptr);
    CHECK_EQ(get_flags(&q), 0);
    CHECK_EQ(static_cast<unsigned long long>(get_cmd(&q)), 0x10000ull);
    CHECK_EQ(static_cast<unsigned long long>(get_compute(&q)), 0x40000ull);
    CHECK_EQ(static_cast<unsigned long long>(get_control(&q)), 0x4000ull);
    CHECK_EQ(static_cast<unsigned long long>(get_threshold(&q)), 0ull);
    CHECK(get_memory(&q) == nullptr);
    CHECK_EQ(static_cast<unsigned long long>(get_memory_size(&q)), 0ull);
    CHECK_EQ(get_priority(&q), 100);
    CHECK_EQ(static_cast<unsigned long long>(get_queue_size(&q)), 0x10000ull + 0x40000ull + 0x4000ull);

    const QueueBuilderInfo d = decode(&q);
    CHECK(d.device == nullptr);
    CHECK_EQ(d.flags, 0);
    CHECK_EQ(static_cast<unsigned long long>(d.command_memory), 0x10000ull);
    CHECK_EQ(static_cast<unsigned long long>(d.compute_memory), 0x40000ull);
    CHECK_EQ(static_cast<unsigned long long>(d.control_memory), 0x4000ull);
    CHECK_EQ(static_cast<unsigned long long>(d.flush_threshold), 0ull);
    CHECK(d.queue_memory == nullptr);
    CHECK_EQ(static_cast<unsigned long long>(d.queue_memory_size), 0ull);
}

NXTEST(queue_builder_round_trip_and_decode) {
    setup();
    auto set_device = abi::proc<QueueSetDeviceFn>("nvnQueueBuilderSetDevice");
    auto get_device = abi::proc<QueueGetDeviceFn>("nvnQueueBuilderGetDevice");
    auto defaults = abi::proc<PFNNVNQUEUEBUILDERSETDEFAULTSPROC>("nvnQueueBuilderSetDefaults");
    auto set_flags = abi::proc<QueueSetFlagsFn>("nvnQueueBuilderSetFlags");
    auto get_flags = abi::proc<QueueGetFlagsFn>("nvnQueueBuilderGetFlags");
    auto set_cmd = abi::proc<PFNNVNQUEUEBUILDERSETCOMMANDMEMORYSIZEPROC>("nvnQueueBuilderSetCommandMemorySize");
    auto get_cmd = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetCommandMemorySize");
    auto set_compute = abi::proc<PFNNVNQUEUEBUILDERSETCOMPUTEMEMORYSIZEPROC>("nvnQueueBuilderSetComputeMemorySize");
    auto get_compute = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetComputeMemorySize");
    auto set_control = abi::proc<PFNNVNQUEUEBUILDERSETCONTROLMEMORYSIZEPROC>("nvnQueueBuilderSetControlMemorySize");
    auto get_control = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetControlMemorySize");
    auto set_threshold = abi::proc<PFNNVNQUEUEBUILDERSETCOMMANDFLUSHTHRESHOLDPROC>("nvnQueueBuilderSetCommandFlushThreshold");
    auto get_threshold = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetCommandFlushThreshold");
    auto set_priority = abi::proc<QueueSetPriorityFn>("nvnQueueBuilderSetQueuePriority");
    auto get_priority = abi::proc<QueueGetPriorityFn>("nvnQueueBuilderGetQueuePriority");
    auto set_memory = abi::proc<PFNNVNQUEUEBUILDERSETQUEUEMEMORYPROC>("nvnQueueBuilderSetQueueMemory");
    auto get_memory = abi::proc<QueueGetMemoryFn>("nvnQueueBuilderGetMemory");
    auto get_memory_size = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetMemorySize");

    NVNqueueBuilder q;
    fill(q, 0x5A);
    defaults(&q);
    NVNdevice dev;
    fill(dev, 0);
    set_device(&q, &dev);
    CHECK(get_device(&q) == &dev);
    CHECK(decode(&q).device == &dev);
    set_device(&q, nullptr);
    CHECK(get_device(&q) == nullptr);

    const int flag_values[] = {0, 1, 0x7fffffff, -1};
    for (int v : flag_values) {
        set_flags(&q, v);
        CHECK_EQ(get_flags(&q), v);
        CHECK_EQ(decode(&q).flags, v);
    }

    const size_t size_values[] = {0, 1, 4096, 0x40000, static_cast<size_t>(-1)};
    for (size_t v : size_values) {
        set_cmd(&q, v);
        CHECK_EQ(static_cast<unsigned long long>(get_cmd(&q)), static_cast<unsigned long long>(v));
        CHECK_EQ(static_cast<unsigned long long>(decode(&q).command_memory), static_cast<unsigned long long>(v));
        set_compute(&q, v);
        CHECK_EQ(static_cast<unsigned long long>(get_compute(&q)), static_cast<unsigned long long>(v));
        CHECK_EQ(static_cast<unsigned long long>(decode(&q).compute_memory), static_cast<unsigned long long>(v));
        set_control(&q, v);
        CHECK_EQ(static_cast<unsigned long long>(get_control(&q)), static_cast<unsigned long long>(v));
        CHECK_EQ(static_cast<unsigned long long>(decode(&q).control_memory), static_cast<unsigned long long>(v));
        set_threshold(&q, v);
        CHECK_EQ(static_cast<unsigned long long>(get_threshold(&q)), static_cast<unsigned long long>(v));
        CHECK_EQ(static_cast<unsigned long long>(decode(&q).flush_threshold), static_cast<unsigned long long>(v));
    }

    const int priority_values[] = {0, 1, 100, 0x7fffffff, -0x7fffffff - 1};
    for (int v : priority_values) {
        set_priority(&q, v);
        CHECK_EQ(get_priority(&q), v);
    }

    int backing[16] = {};
    set_memory(&q, backing, 0x1000);
    CHECK(get_memory(&q) == static_cast<void*>(backing));
    CHECK_EQ(static_cast<unsigned long long>(get_memory_size(&q)), 0x1000ull);
    const QueueBuilderInfo d = decode(&q);
    CHECK(d.queue_memory == static_cast<void*>(backing));
    CHECK_EQ(static_cast<unsigned long long>(d.queue_memory_size), 0x1000ull);
    set_memory(&q, nullptr, 0);
    CHECK(get_memory(&q) == nullptr);
    CHECK_EQ(static_cast<unsigned long long>(get_memory_size(&q)), 0ull);
}

NXTEST(queue_builder_queue_memory_size_rounds_each_part_up) {
    setup();
    auto defaults = abi::proc<PFNNVNQUEUEBUILDERSETDEFAULTSPROC>("nvnQueueBuilderSetDefaults");
    auto set_cmd = abi::proc<PFNNVNQUEUEBUILDERSETCOMMANDMEMORYSIZEPROC>("nvnQueueBuilderSetCommandMemorySize");
    auto set_compute = abi::proc<PFNNVNQUEUEBUILDERSETCOMPUTEMEMORYSIZEPROC>("nvnQueueBuilderSetComputeMemorySize");
    auto set_control = abi::proc<PFNNVNQUEUEBUILDERSETCONTROLMEMORYSIZEPROC>("nvnQueueBuilderSetControlMemorySize");
    auto get_queue_size = abi::proc<PFNNVNQUEUEBUILDERGETQUEUEMEMORYSIZEPROC>("nvnQueueBuilderGetQueueMemorySize");
    NVNqueueBuilder q;
    defaults(&q);
    CHECK_EQ(static_cast<unsigned long long>(get_queue_size(&q)), 0x54000ull);   // the defaults are already 4 KiB multiples
    set_cmd(&q, 1);
    set_compute(&q, 1);
    set_control(&q, 1);
    CHECK_EQ(static_cast<unsigned long long>(get_queue_size(&q)), 3ull * 4096ull);
    set_cmd(&q, 4096);
    set_compute(&q, 4097);
    set_control(&q, 0);
    CHECK_EQ(static_cast<unsigned long long>(get_queue_size(&q)), 4096ull + 8192ull + 0ull);
}

NXTEST(queue_builder_setting_one_field_leaves_others) {
    setup();
    auto defaults = abi::proc<PFNNVNQUEUEBUILDERSETDEFAULTSPROC>("nvnQueueBuilderSetDefaults");
    auto set_device = abi::proc<QueueSetDeviceFn>("nvnQueueBuilderSetDevice");
    auto set_flags = abi::proc<QueueSetFlagsFn>("nvnQueueBuilderSetFlags");
    auto set_cmd = abi::proc<PFNNVNQUEUEBUILDERSETCOMMANDMEMORYSIZEPROC>("nvnQueueBuilderSetCommandMemorySize");
    auto get_cmd = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetCommandMemorySize");
    auto get_compute = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetComputeMemorySize");
    auto get_control = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetControlMemorySize");
    auto get_flags = abi::proc<QueueGetFlagsFn>("nvnQueueBuilderGetFlags");
    auto get_priority = abi::proc<QueueGetPriorityFn>("nvnQueueBuilderGetQueuePriority");
    auto get_threshold = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetCommandFlushThreshold");
    auto get_memory = abi::proc<QueueGetMemoryFn>("nvnQueueBuilderGetMemory");
    auto get_memory_size = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetMemorySize");
    auto get_device = abi::proc<QueueGetDeviceFn>("nvnQueueBuilderGetDevice");
    auto set_memory = abi::proc<PFNNVNQUEUEBUILDERSETQUEUEMEMORYPROC>("nvnQueueBuilderSetQueueMemory");
    NVNqueueBuilder q;
    defaults(&q);
    NVNdevice dev;
    fill(dev, 0);
    int backing[4] = {};
    set_device(&q, &dev);
    set_flags(&q, 5);
    set_memory(&q, backing, 0x2000);
    set_cmd(&q, 0x20000);   // the one field that changes
    CHECK_EQ(static_cast<unsigned long long>(get_cmd(&q)), 0x20000ull);
    CHECK_EQ(static_cast<unsigned long long>(get_compute(&q)), 0x40000ull);
    CHECK_EQ(static_cast<unsigned long long>(get_control(&q)), 0x4000ull);
    CHECK_EQ(get_flags(&q), 5);
    CHECK_EQ(get_priority(&q), 100);
    CHECK_EQ(static_cast<unsigned long long>(get_threshold(&q)), 0ull);
    CHECK(get_memory(&q) == static_cast<void*>(backing));
    CHECK_EQ(static_cast<unsigned long long>(get_memory_size(&q)), 0x2000ull);
    CHECK(get_device(&q) == &dev);
}

NXTEST(queue_builder_copies_stay_independent) {
    setup();
    auto defaults = abi::proc<PFNNVNQUEUEBUILDERSETDEFAULTSPROC>("nvnQueueBuilderSetDefaults");
    auto set_cmd = abi::proc<PFNNVNQUEUEBUILDERSETCOMMANDMEMORYSIZEPROC>("nvnQueueBuilderSetCommandMemorySize");
    auto get_cmd = abi::proc<QueueSizeGetFn>("nvnQueueBuilderGetCommandMemorySize");
    NVNqueueBuilder a, b;
    defaults(&a);
    set_cmd(&a, 0x30000);
    std::memcpy(&b, &a, sizeof a);
    set_cmd(&a, 0x50000);
    CHECK_EQ(static_cast<unsigned long long>(get_cmd(&b)), 0x30000ull);
    CHECK_EQ(static_cast<unsigned long long>(get_cmd(&a)), 0x50000ull);
}

NXTEST(window_builder_defaults_interval_and_native_window) {
    setup();
    auto defaults = abi::proc<PFNNVNWINDOWBUILDERSETDEFAULTSPROC>("nvnWindowBuilderSetDefaults");
    auto set_device = abi::proc<PFNNVNWINDOWBUILDERSETDEVICEPROC>("nvnWindowBuilderSetDevice");
    auto get_device = abi::proc<WinGetDeviceFn>("nvnWindowBuilderGetDevice");
    auto set_native = abi::proc<PFNNVNWINDOWBUILDERSETNATIVEWINDOWPROC>("nvnWindowBuilderSetNativeWindow");
    auto get_native = abi::proc<PFNNVNWINDOWBUILDERGETNATIVEWINDOWPROC>("nvnWindowBuilderGetNativeWindow");
    auto set_interval = abi::proc<PFNNVNWINDOWBUILDERSETPRESENTINTERVALPROC>("nvnWindowBuilderSetPresentInterval");
    auto get_interval = abi::proc<PFNNVNWINDOWBUILDERGETPRESENTINTERVALPROC>("nvnWindowBuilderGetPresentInterval");
    auto get_num = abi::proc<WinGetNumTexturesFn>("nvnWindowBuilderGetNumTextures");
    auto get_texture = abi::proc<WinGetTextureFn>("nvnWindowBuilderGetTexture");
    NVNwindowBuilder w;
    fill(w, 0xEE);
    defaults(&w);
    CHECK_EQ(get_interval(&w), 1);
    CHECK(get_native(&w) == nullptr);
    CHECK_EQ(get_num(&w), 0);
    CHECK(get_texture(&w, 0) == nullptr);
    CHECK_EQ(decode(&w).present_interval, 1);
    CHECK_EQ(decode(&w).texture_count, 0);
    CHECK(decode(&w).native_window == nullptr);

    const int intervals[] = {0, 1, 2, 0x7fffffff, -1};
    for (int v : intervals) {
        set_interval(&w, v);
        CHECK_EQ(get_interval(&w), v);
        CHECK_EQ(decode(&w).present_interval, v);
    }

    int window_handle = 0;
    set_native(&w, &window_handle);
    CHECK(get_native(&w) == static_cast<void*>(&window_handle));
    CHECK(decode(&w).native_window == static_cast<void*>(&window_handle));
    set_native(&w, nullptr);
    CHECK(get_native(&w) == nullptr);

    NVNdevice dev;
    fill(dev, 0);
    set_device(&w, &dev);
    CHECK(get_device(&w) == &dev);
    CHECK(decode(&w).device == &dev);
}

NXTEST(window_builder_textures_are_copied_and_clamped_for_decode) {
    setup();
    auto defaults = abi::proc<PFNNVNWINDOWBUILDERSETDEFAULTSPROC>("nvnWindowBuilderSetDefaults");
    auto set_textures = abi::proc<PFNNVNWINDOWBUILDERSETTEXTURESPROC>("nvnWindowBuilderSetTextures");
    auto get_num = abi::proc<WinGetNumTexturesFn>("nvnWindowBuilderGetNumTextures");
    auto get_texture = abi::proc<WinGetTextureFn>("nvnWindowBuilderGetTexture");
    auto set_interval = abi::proc<PFNNVNWINDOWBUILDERSETPRESENTINTERVALPROC>("nvnWindowBuilderSetPresentInterval");
    auto get_interval = abi::proc<PFNNVNWINDOWBUILDERGETPRESENTINTERVALPROC>("nvnWindowBuilderGetPresentInterval");
    auto set_active = abi::proc<WinSetNumActiveFn>("nvnWindowBuilderSetNumActiveTextures");
    NVNwindowBuilder w;
    defaults(&w);
    NVNtexture t[6];
    fill(t[0], 0);
    fill(t[1], 0);
    fill(t[2], 0);
    fill(t[3], 0);
    fill(t[4], 0);
    fill(t[5], 0);

    NVNtexture* three[3] = {&t[0], &t[1], &t[2]};
    set_textures(&w, 3, three);
    CHECK_EQ(get_num(&w), 3);
    CHECK(get_texture(&w, 0) == &t[0]);
    CHECK(get_texture(&w, 1) == &t[1]);
    CHECK(get_texture(&w, 2) == &t[2]);
    CHECK(get_texture(&w, 3) == nullptr);
    CHECK(get_texture(&w, -1) == nullptr);
    CHECK_EQ(decode(&w).texture_count, 3);
    CHECK(decode(&w).textures[0] == &t[0]);
    CHECK(decode(&w).textures[2] == &t[2]);
    CHECK(decode(&w).textures[3] == nullptr);

    // Six given: the count is kept as given, only four pointers are kept.
    NVNtexture* six[6] = {&t[0], &t[1], &t[2], &t[3], &t[4], &t[5]};
    set_textures(&w, 6, six);
    CHECK_EQ(get_num(&w), 6);
    CHECK(get_texture(&w, 3) == &t[3]);
    CHECK(get_texture(&w, 4) == nullptr);
    CHECK(get_texture(&w, 5) == nullptr);
    CHECK_EQ(decode(&w).texture_count, 4);
    CHECK(decode(&w).textures[3] == &t[3]);

    // A smaller set clears the slots that are no longer used.
    NVNtexture* two[2] = {&t[5], &t[4]};
    set_textures(&w, 2, two);
    CHECK(get_texture(&w, 0) == &t[5]);
    CHECK(get_texture(&w, 1) == &t[4]);
    CHECK(get_texture(&w, 2) == nullptr);
    CHECK(decode(&w).textures[2] == nullptr);
    CHECK(decode(&w).textures[3] == nullptr);

    // The caller's array may be gone later: the builder keeps its own copy.
    {
        NVNtexture* local[1] = {&t[0]};
        set_textures(&w, 1, local);
        local[0] = nullptr;
    }
    CHECK(get_texture(&w, 0) == &t[0]);

    set_textures(&w, 0, nullptr);
    CHECK_EQ(get_num(&w), 0);
    CHECK(get_texture(&w, 0) == nullptr);
    CHECK_EQ(decode(&w).texture_count, 0);

    // Setting the active count touches nothing else.
    set_textures(&w, 2, two);
    set_interval(&w, 2);
    set_active(&w, 5);
    CHECK_EQ(get_interval(&w), 2);
    CHECK_EQ(get_num(&w), 2);
    CHECK(get_texture(&w, 0) == &t[5]);
    CHECK(get_texture(&w, 1) == &t[4]);
}

NXTEST(window_builder_zero_object_decodes_to_zero) {
    NVNwindowBuilder w;
    std::memset(&w, 0, sizeof w);
    const WindowBuilderInfo i = decode(&w);
    CHECK(i.device == nullptr);
    CHECK(i.native_window == nullptr);
    CHECK_EQ(i.present_interval, 0);
    CHECK_EQ(i.texture_count, 0);
    for (int n = 0; n < 4; ++n) CHECK(i.textures[n] == nullptr);
}

NXTEST(memory_pool_builder_defaults_storage_flags_and_decode) {
    setup();
    auto defaults = abi::proc<PFNNVNMEMORYPOOLBUILDERSETDEFAULTSPROC>("nvnMemoryPoolBuilderSetDefaults");
    auto set_device = abi::proc<PFNNVNMEMORYPOOLBUILDERSETDEVICEPROC>("nvnMemoryPoolBuilderSetDevice");
    auto get_device = abi::proc<PoolGetDeviceFn>("nvnMemoryPoolBuilderGetDevice");
    auto set_storage = abi::proc<PFNNVNMEMORYPOOLBUILDERSETSTORAGEPROC>("nvnMemoryPoolBuilderSetStorage");
    auto get_memory = abi::proc<PFNNVNMEMORYPOOLBUILDERGETMEMORYPROC>("nvnMemoryPoolBuilderGetMemory");
    auto get_size = abi::proc<PFNNVNMEMORYPOOLBUILDERGETSIZEPROC>("nvnMemoryPoolBuilderGetSize");
    auto set_flags = abi::proc<PFNNVNMEMORYPOOLBUILDERSETFLAGSPROC>("nvnMemoryPoolBuilderSetFlags");
    auto get_flags = abi::proc<PFNNVNMEMORYPOOLBUILDERGETFLAGSPROC>("nvnMemoryPoolBuilderGetFlags");

    NVNmemoryPoolBuilder p;
    fill(p, 0x77);
    defaults(&p);
    CHECK(all_bytes_are(p, 0));
    CHECK(get_device(&p) == nullptr);
    CHECK_EQ(static_cast<int>(get_flags(&p)), 0);
    CHECK_EQ(static_cast<unsigned long long>(get_size(&p)), 0ull);
    get_memory(&p);   // returns nothing; the storage is visible through decode()
    CHECK(decode(&p).storage == nullptr);
    CHECK_EQ(static_cast<unsigned long long>(decode(&p).size), 0ull);
    CHECK_EQ(decode(&p).flags, 0);

    NVNdevice dev;
    fill(dev, 0);
    set_device(&p, &dev);
    CHECK(get_device(&p) == &dev);
    CHECK(decode(&p).device == &dev);

    const int flag_values[] = {NVN_MEMORY_POOL_FLAGS_CPU_NO_ACCESS, NVN_MEMORY_POOL_FLAGS_LARGE, 0};
    for (int v : flag_values) {
        set_flags(&p, v);
        CHECK_EQ(static_cast<int>(get_flags(&p)), v);
        CHECK_EQ(decode(&p).flags, v);
    }

    const size_t sizes[] = {0, 1, 0x100000, static_cast<size_t>(-1)};
    int backing[4] = {};
    for (size_t s : sizes) {
        set_storage(&p, backing, s);
        CHECK_EQ(static_cast<unsigned long long>(get_size(&p)), static_cast<unsigned long long>(s));
        CHECK(decode(&p).storage == static_cast<void*>(backing));
        CHECK_EQ(static_cast<unsigned long long>(decode(&p).size), static_cast<unsigned long long>(s));
        get_memory(&p);
    }

    // Setting the storage leaves the flags and the device alone.
    set_flags(&p, NVN_MEMORY_POOL_FLAGS_CPU_NO_ACCESS);
    set_storage(&p, backing, 0x200);
    CHECK_EQ(static_cast<int>(get_flags(&p)), static_cast<int>(NVN_MEMORY_POOL_FLAGS_CPU_NO_ACCESS));
    CHECK(get_device(&p) == &dev);

    NVNmemoryPoolBuilder copy;
    std::memcpy(&copy, &p, sizeof p);
    set_storage(&p, nullptr, 0);
    CHECK(decode(&copy).storage == static_cast<void*>(backing));
    CHECK(decode(&p).storage == nullptr);
}

NXTEST(buffer_builder_defaults_storage_and_decode) {
    setup();
    auto defaults = abi::proc<PFNNVNBUFFERBUILDERSETDEFAULTSPROC>("nvnBufferBuilderSetDefaults");
    auto set_device = abi::proc<PFNNVNBUFFERBUILDERSETDEVICEPROC>("nvnBufferBuilderSetDevice");
    auto get_device = abi::proc<BufGetDeviceFn>("nvnBufferBuilderGetDevice");
    auto set_storage = abi::proc<PFNNVNBUFFERBUILDERSETSTORAGEPROC>("nvnBufferBuilderSetStorage");
    auto get_offset = abi::proc<PFNNVNBUFFERBUILDERGETMEMORYOFFSETPROC>("nvnBufferBuilderGetMemoryOffset");
    auto get_size = abi::proc<PFNNVNBUFFERBUILDERGETSIZEPROC>("nvnBufferBuilderGetSize");

    NVNbufferBuilder b;
    fill(b, 0x33);
    defaults(&b);
    CHECK(all_bytes_are(b, 0));
    CHECK(get_device(&b) == nullptr);
    CHECK_EQ(static_cast<long long>(get_offset(&b)), 0ll);
    CHECK_EQ(static_cast<unsigned long long>(get_size(&b)), 0ull);
    CHECK(decode(&b).pool == nullptr);
    CHECK_EQ(static_cast<long long>(decode(&b).offset), 0ll);

    NVNdevice dev;
    fill(dev, 0);
    set_device(&b, &dev);
    CHECK(get_device(&b) == &dev);
    CHECK(decode(&b).device == &dev);

    NVNmemoryPool pool;
    fill(pool, 0);
    const ptrdiff_t offsets[] = {0, 8, 0x40000, PTRDIFF_MAX};
    const size_t sizes[] = {0, 1, 0x1000, static_cast<size_t>(-1)};
    for (ptrdiff_t o : offsets) {
        for (size_t s : sizes) {
            set_storage(&b, &pool, o, s);
            CHECK_EQ(static_cast<long long>(get_offset(&b)), static_cast<long long>(o));
            CHECK_EQ(static_cast<unsigned long long>(get_size(&b)), static_cast<unsigned long long>(s));
            const BufferBuilderInfo d = decode(&b);
            CHECK(d.pool == &pool);
            CHECK_EQ(static_cast<long long>(d.offset), static_cast<long long>(o));
            CHECK_EQ(static_cast<unsigned long long>(d.size), static_cast<unsigned long long>(s));
        }
    }

    // Setting the device afterwards leaves the storage alone.
    set_storage(&b, &pool, 0x100, 0x200);
    set_device(&b, nullptr);
    CHECK(get_device(&b) == nullptr);
    CHECK_EQ(static_cast<long long>(get_offset(&b)), 0x100ll);
    CHECK_EQ(static_cast<unsigned long long>(get_size(&b)), 0x200ull);
    CHECK(decode(&b).pool == &pool);
}

NXTEST(event_builder_defaults_storage_and_decode) {
    setup();
    auto defaults = abi::proc<PFNNVNEVENTBUILDERSETDEFAULTSPROC>("nvnEventBuilderSetDefaults");
    auto set_storage = abi::proc<PFNNVNEVENTBUILDERSETSTORAGEPROC>("nvnEventBuilderSetStorage");
    auto get_pool = abi::proc<EvtGetPoolFn>("nvnEventBuilderGetMemoryPool");
    auto get_offset = abi::proc<EvtGetOffsetFn>("nvnEventBuilderGetMemoryOffset");

    NVNeventBuilder e;
    fill(e, 0x99);
    defaults(&e);
    CHECK(all_bytes_are(e, 0));
    CHECK(get_pool(&e) == nullptr);
    CHECK_EQ(static_cast<long long>(get_offset(&e)), 0ll);
    CHECK(decode(&e).pool == nullptr);
    CHECK_EQ(static_cast<long long>(decode(&e).offset), 0ll);
    CHECK(decode(&e).device == nullptr);

    NVNmemoryPool pool;
    fill(pool, 0);
    const int64_t offsets[] = {0, 8, INT64_MAX};
    for (int64_t o : offsets) {
        set_storage(&e, &pool, o);
        CHECK(get_pool(&e) == &pool);
        CHECK_EQ(static_cast<long long>(get_offset(&e)), static_cast<long long>(o));
        CHECK(decode(&e).pool == &pool);
        CHECK_EQ(static_cast<long long>(decode(&e).offset), static_cast<long long>(o));
    }

    NVNeventBuilder copy;
    set_storage(&e, &pool, 0x40);
    std::memcpy(&copy, &e, sizeof e);
    defaults(&e);
    CHECK(get_pool(&copy) == &pool);
    CHECK_EQ(static_cast<long long>(get_offset(&copy)), 0x40ll);
    CHECK(get_pool(&e) == nullptr);
}

NXTEST(zero_objects_decode_to_zero) {
    NVNdeviceBuilder d;
    NVNqueueBuilder q;
    NVNmemoryPoolBuilder p;
    NVNbufferBuilder b;
    NVNeventBuilder e;
    std::memset(&d, 0, sizeof d);
    std::memset(&q, 0, sizeof q);
    std::memset(&p, 0, sizeof p);
    std::memset(&b, 0, sizeof b);
    std::memset(&e, 0, sizeof e);
    CHECK_EQ(decode(&d).flags, 0);
    const QueueBuilderInfo qi = decode(&q);
    CHECK(qi.device == nullptr);
    CHECK_EQ(qi.flags, 0);
    CHECK_EQ(static_cast<unsigned long long>(qi.command_memory), 0ull);
    CHECK_EQ(static_cast<unsigned long long>(qi.compute_memory), 0ull);
    CHECK_EQ(static_cast<unsigned long long>(qi.control_memory), 0ull);
    CHECK_EQ(static_cast<unsigned long long>(qi.flush_threshold), 0ull);
    CHECK(qi.queue_memory == nullptr);
    CHECK_EQ(static_cast<unsigned long long>(qi.queue_memory_size), 0ull);
    const MemoryPoolBuilderInfo pi = decode(&p);
    CHECK(pi.device == nullptr);
    CHECK(pi.storage == nullptr);
    CHECK_EQ(static_cast<unsigned long long>(pi.size), 0ull);
    CHECK_EQ(pi.flags, 0);
    const BufferBuilderInfo bi = decode(&b);
    CHECK(bi.device == nullptr);
    CHECK(bi.pool == nullptr);
    CHECK_EQ(static_cast<long long>(bi.offset), 0ll);
    CHECK_EQ(static_cast<unsigned long long>(bi.size), 0ull);
    const EventBuilderInfo ei = decode(&e);
    CHECK(ei.pool == nullptr);
    CHECK_EQ(static_cast<long long>(ei.offset), 0ll);
}
