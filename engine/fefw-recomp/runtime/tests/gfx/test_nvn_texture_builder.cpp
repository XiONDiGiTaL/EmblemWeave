// SPDX-License-Identifier: GPL-3.0-or-later
// NVNtextureBuilder and NVNtextureView through the typed thunks (family texture_builder). The five builder functions that
// the manager implements (storage size and alignment, storage class, sparse tile layout, zcull size) are not tested here.
#include <cstring>

#include "nvn_impl.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_texture_builder)

using namespace nxrt;
using namespace nxrt::nvn;

namespace {
void setup() {
    install();
    abi::reset_mismatches();
    register_api_texture_builder();
    CHECK_EQ(abi::mismatches(), 0u);
}

// Every function of this family (49 = 35 builder + 14 view), in the order of sigs.txt, without the manager's five.
const char* const kFamily[] = {
    "nvnTextureBuilderSetDevice", "nvnTextureBuilderSetDefaults", "nvnTextureBuilderSetFlags", "nvnTextureBuilderSetTarget",
    "nvnTextureBuilderSetWidth", "nvnTextureBuilderSetHeight", "nvnTextureBuilderSetDepth", "nvnTextureBuilderSetSize1D",
    "nvnTextureBuilderSetSize2D", "nvnTextureBuilderSetSize3D", "nvnTextureBuilderSetLevels", "nvnTextureBuilderSetFormat",
    "nvnTextureBuilderSetSamples", "nvnTextureBuilderSetSwizzle", "nvnTextureBuilderSetDepthStencilMode",
    "nvnTextureBuilderSetStorage", "nvnTextureBuilderSetPackagedTextureData", "nvnTextureBuilderSetPackagedTextureLayout",
    "nvnTextureBuilderSetStride", "nvnTextureBuilderSetGLTextureName", "nvnTextureBuilderGetFlags", "nvnTextureBuilderGetTarget",
    "nvnTextureBuilderGetWidth", "nvnTextureBuilderGetHeight", "nvnTextureBuilderGetDepth", "nvnTextureBuilderGetLevels",
    "nvnTextureBuilderGetFormat", "nvnTextureBuilderGetSamples", "nvnTextureBuilderGetSwizzle",
    "nvnTextureBuilderGetDepthStencilMode", "nvnTextureBuilderGetPackagedTextureData", "nvnTextureBuilderGetStride",
    "nvnTextureBuilderGetGLTextureName", "nvnTextureBuilderGetMemoryPool", "nvnTextureBuilderGetMemoryOffset",
    "nvnTextureViewSetDefaults", "nvnTextureViewSetLevels", "nvnTextureViewSetLayers", "nvnTextureViewSetFormat",
    "nvnTextureViewSetSwizzle", "nvnTextureViewSetDepthStencilMode", "nvnTextureViewSetTarget", "nvnTextureViewGetLevels",
    "nvnTextureViewGetLayers", "nvnTextureViewGetFormat", "nvnTextureViewGetSwizzle", "nvnTextureViewGetDepthStencilMode",
    "nvnTextureViewGetTarget", "nvnTextureViewCompare",
};
}  // namespace

NXTEST(all_functions_registered) {
    setup();
    CHECK_EQ(sizeof kFamily / sizeof kFamily[0], 49u);
    for (const char* name : kFamily) CHECK(is_implemented(name));
}

NXTEST(builder_defaults_and_zero_object) {
    setup();
    auto defaults = abi::proc<PFNNVNTEXTUREBUILDERSETDEFAULTSPROC>("nvnTextureBuilderSetDefaults");
    auto get_flags = abi::proc<PFNNVNTEXTUREBUILDERGETFLAGSPROC>("nvnTextureBuilderGetFlags");
    auto get_target = abi::proc<PFNNVNTEXTUREBUILDERGETTARGETPROC>("nvnTextureBuilderGetTarget");
    auto get_width = abi::proc<PFNNVNTEXTUREBUILDERGETWIDTHPROC>("nvnTextureBuilderGetWidth");
    auto get_height = abi::proc<PFNNVNTEXTUREBUILDERGETHEIGHTPROC>("nvnTextureBuilderGetHeight");
    auto get_depth = abi::proc<PFNNVNTEXTUREBUILDERGETDEPTHPROC>("nvnTextureBuilderGetDepth");
    auto get_levels = abi::proc<PFNNVNTEXTUREBUILDERGETLEVELSPROC>("nvnTextureBuilderGetLevels");
    auto get_format = abi::proc<PFNNVNTEXTUREBUILDERGETFORMATPROC>("nvnTextureBuilderGetFormat");
    auto get_samples = abi::proc<PFNNVNTEXTUREBUILDERGETSAMPLESPROC>("nvnTextureBuilderGetSamples");
    auto get_swizzle = abi::proc<PFNNVNTEXTUREBUILDERGETSWIZZLEPROC>("nvnTextureBuilderGetSwizzle");
    auto get_dsm = abi::proc<PFNNVNTEXTUREBUILDERGETDEPTHSTENCILMODEPROC>("nvnTextureBuilderGetDepthStencilMode");
    auto get_packaged = abi::proc<PFNNVNTEXTUREBUILDERGETPACKAGEDTEXTUREDATAPROC>("nvnTextureBuilderGetPackagedTextureData");
    auto get_stride = abi::proc<PFNNVNTEXTUREBUILDERGETSTRIDEPROC>("nvnTextureBuilderGetStride");
    auto get_gl = abi::proc<PFNNVNTEXTUREBUILDERGETGLTEXTURENAMEPROC>("nvnTextureBuilderGetGLTextureName");
    auto get_pool = abi::proc<PFNNVNTEXTUREBUILDERGETMEMORYPOOLPROC>("nvnTextureBuilderGetMemoryPool");
    auto get_offset = abi::proc<PFNNVNTEXTUREBUILDERGETMEMORYOFFSETPROC>("nvnTextureBuilderGetMemoryOffset");

    NVNtextureBuilder b;
    std::memset(&b, 0xAB, sizeof b);   // a dirty object must be fully overwritten
    defaults(&b);
    CHECK_EQ(static_cast<int>(get_flags(&b)), 0);
    CHECK_EQ(static_cast<int>(get_target(&b)), static_cast<int>(NVN_TEXTURE_TARGET_2D));
    CHECK_EQ(get_width(&b), 0);
    CHECK_EQ(get_height(&b), 0);
    CHECK_EQ(get_depth(&b), 0);
    CHECK_EQ(get_levels(&b), 1);
    CHECK_EQ(static_cast<int>(get_format(&b)), 0);
    CHECK_EQ(get_samples(&b), 0);
    NVNtextureSwizzle r, g, bl, a;
    get_swizzle(&b, &r, &g, &bl, &a);
    CHECK_EQ(static_cast<int>(r), static_cast<int>(NVN_TEXTURE_SWIZZLE_R));
    CHECK_EQ(static_cast<int>(g), static_cast<int>(NVN_TEXTURE_SWIZZLE_G));
    CHECK_EQ(static_cast<int>(bl), static_cast<int>(NVN_TEXTURE_SWIZZLE_B));
    CHECK_EQ(static_cast<int>(a), static_cast<int>(NVN_TEXTURE_SWIZZLE_A));
    CHECK_EQ(static_cast<int>(get_dsm(&b)), static_cast<int>(NVN_TEXTURE_DEPTH_STENCIL_MODE_DEPTH));
    CHECK(get_packaged(&b) == nullptr);
    CHECK_EQ(static_cast<long long>(get_stride(&b)), 0ll);
    CHECK_EQ(get_gl(&b), 0u);
    CHECK_EQ(static_cast<long long>(get_offset(&b)), 0ll);
    CHECK(get_pool(&b) == nullptr);
    const TextureBuilderInfo d = decode(&b);
    CHECK(d.pool == nullptr);
    CHECK(d.device == nullptr);
    CHECK(!d.has_packaged_layout);
    CHECK_EQ(d.levels, 1);
    CHECK_EQ(d.target, static_cast<int>(NVN_TEXTURE_TARGET_2D));
    CHECK_EQ(d.swizzle[0], static_cast<int>(NVN_TEXTURE_SWIZZLE_R));
    CHECK_EQ(d.swizzle[3], static_cast<int>(NVN_TEXTURE_SWIZZLE_A));

    // a zero-filled object decodes to all zero, with no defaults applied
    NVNtextureBuilder z;
    std::memset(&z, 0, sizeof z);
    const TextureBuilderInfo zd = decode(&z);
    CHECK_EQ(zd.target, 0);
    CHECK_EQ(zd.levels, 0);
    CHECK_EQ(zd.width, 0);
    CHECK_EQ(zd.format, 0);
    CHECK_EQ(zd.swizzle[0], 0);
    CHECK_EQ(zd.swizzle[3], 0);
    CHECK_EQ(zd.offset, 0);
    CHECK_EQ(zd.stride, 0);
    CHECK_EQ(zd.gl_texture_name, 0u);
    CHECK(!zd.has_packaged_layout);
    CHECK(zd.pool == nullptr);
    CHECK(zd.packaged_data == nullptr);
}

NXTEST(builder_set_get_round_trip) {
    setup();
    auto defaults = abi::proc<PFNNVNTEXTUREBUILDERSETDEFAULTSPROC>("nvnTextureBuilderSetDefaults");
    auto set_flags = abi::proc<PFNNVNTEXTUREBUILDERSETFLAGSPROC>("nvnTextureBuilderSetFlags");
    auto get_flags = abi::proc<PFNNVNTEXTUREBUILDERGETFLAGSPROC>("nvnTextureBuilderGetFlags");
    auto set_target = abi::proc<PFNNVNTEXTUREBUILDERSETTARGETPROC>("nvnTextureBuilderSetTarget");
    auto get_target = abi::proc<PFNNVNTEXTUREBUILDERGETTARGETPROC>("nvnTextureBuilderGetTarget");
    auto set_width = abi::proc<PFNNVNTEXTUREBUILDERSETWIDTHPROC>("nvnTextureBuilderSetWidth");
    auto get_width = abi::proc<PFNNVNTEXTUREBUILDERGETWIDTHPROC>("nvnTextureBuilderGetWidth");
    auto set_height = abi::proc<PFNNVNTEXTUREBUILDERSETHEIGHTPROC>("nvnTextureBuilderSetHeight");
    auto get_height = abi::proc<PFNNVNTEXTUREBUILDERGETHEIGHTPROC>("nvnTextureBuilderGetHeight");
    auto set_depth = abi::proc<PFNNVNTEXTUREBUILDERSETDEPTHPROC>("nvnTextureBuilderSetDepth");
    auto get_depth = abi::proc<PFNNVNTEXTUREBUILDERGETDEPTHPROC>("nvnTextureBuilderGetDepth");
    auto set_levels = abi::proc<PFNNVNTEXTUREBUILDERSETLEVELSPROC>("nvnTextureBuilderSetLevels");
    auto get_levels = abi::proc<PFNNVNTEXTUREBUILDERGETLEVELSPROC>("nvnTextureBuilderGetLevels");
    auto set_format = abi::proc<PFNNVNTEXTUREBUILDERSETFORMATPROC>("nvnTextureBuilderSetFormat");
    auto get_format = abi::proc<PFNNVNTEXTUREBUILDERGETFORMATPROC>("nvnTextureBuilderGetFormat");
    auto set_samples = abi::proc<PFNNVNTEXTUREBUILDERSETSAMPLESPROC>("nvnTextureBuilderSetSamples");
    auto get_samples = abi::proc<PFNNVNTEXTUREBUILDERGETSAMPLESPROC>("nvnTextureBuilderGetSamples");
    auto set_swizzle = abi::proc<PFNNVNTEXTUREBUILDERSETSWIZZLEPROC>("nvnTextureBuilderSetSwizzle");
    auto get_swizzle = abi::proc<PFNNVNTEXTUREBUILDERGETSWIZZLEPROC>("nvnTextureBuilderGetSwizzle");
    auto set_dsm = abi::proc<PFNNVNTEXTUREBUILDERSETDEPTHSTENCILMODEPROC>("nvnTextureBuilderSetDepthStencilMode");
    auto get_dsm = abi::proc<PFNNVNTEXTUREBUILDERGETDEPTHSTENCILMODEPROC>("nvnTextureBuilderGetDepthStencilMode");
    auto set_stride = abi::proc<PFNNVNTEXTUREBUILDERSETSTRIDEPROC>("nvnTextureBuilderSetStride");
    auto get_stride = abi::proc<PFNNVNTEXTUREBUILDERGETSTRIDEPROC>("nvnTextureBuilderGetStride");
    auto set_gl = abi::proc<PFNNVNTEXTUREBUILDERSETGLTEXTURENAMEPROC>("nvnTextureBuilderSetGLTextureName");
    auto get_gl = abi::proc<PFNNVNTEXTUREBUILDERGETGLTEXTURENAMEPROC>("nvnTextureBuilderGetGLTextureName");

    NVNtextureBuilder b;
    std::memset(&b, 0, sizeof b);
    defaults(&b);

    // flags: a few bit patterns, including the top bit of a signed int
    for (int v : {0, 1, 0x40, 0x400, 0x7FFFFFFF}) {
        set_flags(&b, v);
        CHECK_EQ(static_cast<int>(get_flags(&b)), v);
    }
    // target: every value of the enum, then the first and last again
    for (int t = NVN_TEXTURE_TARGET_1D; t <= NVN_TEXTURE_TARGET_BUFFER; ++t) {
        set_target(&b, static_cast<NVNtextureTarget>(t));
        CHECK_EQ(static_cast<int>(get_target(&b)), t);
    }
    set_width(&b, 1);
    CHECK_EQ(get_width(&b), 1);
    set_width(&b, 0x7FFFFFFF);
    CHECK_EQ(get_width(&b), 0x7FFFFFFF);
    set_height(&b, 4096);
    CHECK_EQ(get_height(&b), 4096);
    set_depth(&b, -1);
    CHECK_EQ(get_depth(&b), -1);
    set_levels(&b, 13);
    CHECK_EQ(get_levels(&b), 13);
    set_format(&b, NVN_FORMAT_RGBA32F);
    CHECK_EQ(static_cast<int>(get_format(&b)), static_cast<int>(NVN_FORMAT_RGBA32F));
    set_format(&b, NVN_FORMAT_R8);
    CHECK_EQ(static_cast<int>(get_format(&b)), static_cast<int>(NVN_FORMAT_R8));
    set_samples(&b, 8);
    CHECK_EQ(get_samples(&b), 8);
    set_swizzle(&b, NVN_TEXTURE_SWIZZLE_ZERO, NVN_TEXTURE_SWIZZLE_ONE, NVN_TEXTURE_SWIZZLE_B, NVN_TEXTURE_SWIZZLE_A);
    NVNtextureSwizzle r, g, bl, a;
    get_swizzle(&b, &r, &g, &bl, &a);
    CHECK_EQ(static_cast<int>(r), static_cast<int>(NVN_TEXTURE_SWIZZLE_ZERO));
    CHECK_EQ(static_cast<int>(g), static_cast<int>(NVN_TEXTURE_SWIZZLE_ONE));
    CHECK_EQ(static_cast<int>(bl), static_cast<int>(NVN_TEXTURE_SWIZZLE_B));
    CHECK_EQ(static_cast<int>(a), static_cast<int>(NVN_TEXTURE_SWIZZLE_A));
    get_swizzle(&b, nullptr, &g, nullptr, nullptr);   // null out pointers are allowed
    CHECK_EQ(static_cast<int>(g), static_cast<int>(NVN_TEXTURE_SWIZZLE_ONE));
    set_dsm(&b, NVN_TEXTURE_DEPTH_STENCIL_MODE_STENCIL);
    CHECK_EQ(static_cast<int>(get_dsm(&b)), static_cast<int>(NVN_TEXTURE_DEPTH_STENCIL_MODE_STENCIL));
    set_dsm(&b, NVN_TEXTURE_DEPTH_STENCIL_MODE_DEPTH);
    CHECK_EQ(static_cast<int>(get_dsm(&b)), static_cast<int>(NVN_TEXTURE_DEPTH_STENCIL_MODE_DEPTH));
    set_stride(&b, -256);
    CHECK_EQ(static_cast<long long>(get_stride(&b)), -256ll);
    set_stride(&b, 0x123456789ll);
    CHECK_EQ(static_cast<long long>(get_stride(&b)), 0x123456789ll);
    set_gl(&b, 0xFFFFFFFFu);
    CHECK_EQ(get_gl(&b), 0xFFFFFFFFu);

    const TextureBuilderInfo d = decode(&b);
    CHECK_EQ(d.flags, 0x7FFFFFFF);
    CHECK_EQ(d.target, static_cast<int>(NVN_TEXTURE_TARGET_BUFFER));
    CHECK_EQ(d.width, 0x7FFFFFFF);
    CHECK_EQ(d.height, 4096);
    CHECK_EQ(d.depth, -1);
    CHECK_EQ(d.levels, 13);
    CHECK_EQ(d.format, static_cast<int>(NVN_FORMAT_R8));
    CHECK_EQ(d.samples, 8);
    CHECK_EQ(d.swizzle[0], static_cast<int>(NVN_TEXTURE_SWIZZLE_ZERO));
    CHECK_EQ(d.swizzle[1], static_cast<int>(NVN_TEXTURE_SWIZZLE_ONE));
    CHECK_EQ(d.depth_stencil_mode, static_cast<int>(NVN_TEXTURE_DEPTH_STENCIL_MODE_DEPTH));
    CHECK_EQ(static_cast<long long>(d.stride), 0x123456789ll);
    CHECK_EQ(d.gl_texture_name, 0xFFFFFFFFu);
}

NXTEST(builder_sizes_follow_smo) {
    setup();
    auto defaults = abi::proc<PFNNVNTEXTUREBUILDERSETDEFAULTSPROC>("nvnTextureBuilderSetDefaults");
    auto set_target = abi::proc<PFNNVNTEXTUREBUILDERSETTARGETPROC>("nvnTextureBuilderSetTarget");
    auto set_size1d = abi::proc<PFNNVNTEXTUREBUILDERSETSIZE1DPROC>("nvnTextureBuilderSetSize1D");
    auto set_size2d = abi::proc<PFNNVNTEXTUREBUILDERSETSIZE2DPROC>("nvnTextureBuilderSetSize2D");
    auto set_size3d = abi::proc<PFNNVNTEXTUREBUILDERSETSIZE3DPROC>("nvnTextureBuilderSetSize3D");
    auto get_target = abi::proc<PFNNVNTEXTUREBUILDERGETTARGETPROC>("nvnTextureBuilderGetTarget");
    auto get_width = abi::proc<PFNNVNTEXTUREBUILDERGETWIDTHPROC>("nvnTextureBuilderGetWidth");
    auto get_height = abi::proc<PFNNVNTEXTUREBUILDERGETHEIGHTPROC>("nvnTextureBuilderGetHeight");
    auto get_depth = abi::proc<PFNNVNTEXTUREBUILDERGETDEPTHPROC>("nvnTextureBuilderGetDepth");

    NVNtextureBuilder b;
    std::memset(&b, 0, sizeof b);
    defaults(&b);
    set_target(&b, NVN_TEXTURE_TARGET_2D_ARRAY);
    set_size3d(&b, 64, 32, 8);
    CHECK_EQ(get_width(&b), 64);
    CHECK_EQ(get_height(&b), 32);
    CHECK_EQ(get_depth(&b), 8);
    set_size2d(&b, 640, 480);            // depth becomes 1, target stays
    CHECK_EQ(get_width(&b), 640);
    CHECK_EQ(get_height(&b), 480);
    CHECK_EQ(get_depth(&b), 1);
    CHECK_EQ(static_cast<int>(get_target(&b)), static_cast<int>(NVN_TEXTURE_TARGET_2D_ARRAY));
    set_size1d(&b, 256);                 // height and depth become 1, target stays
    CHECK_EQ(get_width(&b), 256);
    CHECK_EQ(get_height(&b), 1);
    CHECK_EQ(get_depth(&b), 1);
    CHECK_EQ(static_cast<int>(get_target(&b)), static_cast<int>(NVN_TEXTURE_TARGET_2D_ARRAY));
}

NXTEST(builder_storage_layout_and_pointers) {
    setup();
    auto defaults = abi::proc<PFNNVNTEXTUREBUILDERSETDEFAULTSPROC>("nvnTextureBuilderSetDefaults");
    auto set_device = abi::proc<PFNNVNTEXTUREBUILDERSETDEVICEPROC>("nvnTextureBuilderSetDevice");
    auto set_storage = abi::proc<PFNNVNTEXTUREBUILDERSETSTORAGEPROC>("nvnTextureBuilderSetStorage");
    auto get_offset = abi::proc<PFNNVNTEXTUREBUILDERGETMEMORYOFFSETPROC>("nvnTextureBuilderGetMemoryOffset");
    auto get_pool = abi::proc<PFNNVNTEXTUREBUILDERGETMEMORYPOOLPROC>("nvnTextureBuilderGetMemoryPool");
    auto set_packaged = abi::proc<PFNNVNTEXTUREBUILDERSETPACKAGEDTEXTUREDATAPROC>("nvnTextureBuilderSetPackagedTextureData");
    auto get_packaged = abi::proc<PFNNVNTEXTUREBUILDERGETPACKAGEDTEXTUREDATAPROC>("nvnTextureBuilderGetPackagedTextureData");
    auto set_layout = abi::proc<PFNNVNTEXTUREBUILDERSETPACKAGEDTEXTURELAYOUTPROC>("nvnTextureBuilderSetPackagedTextureLayout");

    NVNtextureBuilder b;
    std::memset(&b, 0, sizeof b);
    defaults(&b);
    NVNdevice dev;
    std::memset(&dev, 0x11, sizeof dev);
    set_device(&b, &dev);
    CHECK(decode(&b).device == &dev);

    NVNmemoryPool pool;
    std::memset(&pool, 0x5A, sizeof pool);
    set_storage(&b, &pool, 4096);
    const TextureBuilderInfo s = decode(&b);
    CHECK(s.pool == &pool);
    CHECK_EQ(static_cast<long long>(s.offset), 4096ll);
    CHECK_EQ(static_cast<long long>(get_offset(&b)), 4096ll);
    CHECK(get_pool(&b) == &pool);
    CHECK_EQ(static_cast<long long>(get_offset(&b)), 4096ll);

    uint8_t data[16] = {1, 2, 3, 4};
    set_packaged(&b, data);
    CHECK(get_packaged(&b) == data);
    CHECK(decode(&b).packaged_data == data);

    // layout: a null pointer changes nothing; a real one is copied and sets has_packaged_layout
    set_layout(&b, nullptr);
    CHECK(!decode(&b).has_packaged_layout);
    NVNpackagedTextureLayout lay{};
    for (int i = 0; i < 8; ++i) lay.layout[i] = static_cast<unsigned char>(0xF0 + i);
    set_layout(&b, &lay);
    const TextureBuilderInfo L = decode(&b);
    CHECK(L.has_packaged_layout);
    for (int i = 0; i < 8; ++i) CHECK_EQ(static_cast<int>(L.packaged_layout[i]), 0xF0 + i);
}

NXTEST(builder_setting_one_field_keeps_the_others) {
    setup();
    auto defaults = abi::proc<PFNNVNTEXTUREBUILDERSETDEFAULTSPROC>("nvnTextureBuilderSetDefaults");
    auto set_format = abi::proc<PFNNVNTEXTUREBUILDERSETFORMATPROC>("nvnTextureBuilderSetFormat");
    auto set_levels = abi::proc<PFNNVNTEXTUREBUILDERSETLEVELSPROC>("nvnTextureBuilderSetLevels");
    auto set_dsm = abi::proc<PFNNVNTEXTUREBUILDERSETDEPTHSTENCILMODEPROC>("nvnTextureBuilderSetDepthStencilMode");

    NVNtextureBuilder b;
    std::memset(&b, 0, sizeof b);
    defaults(&b);
    set_format(&b, NVN_FORMAT_RGBA32F);
    TextureBuilderInfo d = decode(&b);
    CHECK_EQ(d.format, static_cast<int>(NVN_FORMAT_RGBA32F));
    CHECK_EQ(d.levels, 1);
    CHECK_EQ(d.target, static_cast<int>(NVN_TEXTURE_TARGET_2D));
    CHECK_EQ(d.swizzle[2], static_cast<int>(NVN_TEXTURE_SWIZZLE_B));
    CHECK_EQ(d.depth_stencil_mode, 0);
    set_levels(&b, 3);
    set_dsm(&b, NVN_TEXTURE_DEPTH_STENCIL_MODE_STENCIL);
    d = decode(&b);
    CHECK_EQ(d.levels, 3);
    CHECK_EQ(d.format, static_cast<int>(NVN_FORMAT_RGBA32F));
    CHECK_EQ(d.depth_stencil_mode, static_cast<int>(NVN_TEXTURE_DEPTH_STENCIL_MODE_STENCIL));
}

NXTEST(builder_copies_stay_independent) {
    setup();
    auto defaults = abi::proc<PFNNVNTEXTUREBUILDERSETDEFAULTSPROC>("nvnTextureBuilderSetDefaults");
    auto set_width = abi::proc<PFNNVNTEXTUREBUILDERSETWIDTHPROC>("nvnTextureBuilderSetWidth");
    auto get_width = abi::proc<PFNNVNTEXTUREBUILDERGETWIDTHPROC>("nvnTextureBuilderGetWidth");

    NVNtextureBuilder a, c;
    std::memset(&a, 0, sizeof a);
    defaults(&a);
    set_width(&a, 320);
    std::memcpy(&c, &a, sizeof c);   // the game copies builders around freely
    set_width(&a, 640);
    CHECK_EQ(get_width(&c), 320);
    CHECK_EQ(get_width(&a), 640);
    CHECK_EQ(decode(&c).width, 320);
}

NXTEST(view_defaults_and_decode) {
    setup();
    auto view_defaults = abi::proc<PFNNVNTEXTUREVIEWSETDEFAULTSPROC>("nvnTextureViewSetDefaults");
    auto get_levels = abi::proc<PFNNVNTEXTUREVIEWGETLEVELSPROC>("nvnTextureViewGetLevels");
    auto get_layers = abi::proc<PFNNVNTEXTUREVIEWGETLAYERSPROC>("nvnTextureViewGetLayers");
    auto get_format = abi::proc<PFNNVNTEXTUREVIEWGETFORMATPROC>("nvnTextureViewGetFormat");
    auto get_swizzle = abi::proc<PFNNVNTEXTUREVIEWGETSWIZZLEPROC>("nvnTextureViewGetSwizzle");
    auto get_dsm = abi::proc<PFNNVNTEXTUREVIEWGETDEPTHSTENCILMODEPROC>("nvnTextureViewGetDepthStencilMode");
    auto get_target = abi::proc<PFNNVNTEXTUREVIEWGETTARGETPROC>("nvnTextureViewGetTarget");

    NVNtextureView v;
    std::memset(&v, 0xCD, sizeof v);
    view_defaults(&v);
    int lo = 77, ln = 77, lay = 77, lays = 77;
    CHECK(!get_levels(&v, &lo, &ln));
    CHECK_EQ(lo, 0);   // a field never set reads back as the stored zero, as smo
    CHECK_EQ(ln, 0);
    CHECK(!get_layers(&v, &lay, &lays));
    CHECK_EQ(lay, 0);
    NVNformat f = NVN_FORMAT_RGBA32F;
    CHECK(!get_format(&v, &f));
    CHECK_EQ(static_cast<int>(f), 0);
    NVNtextureSwizzle r = NVN_TEXTURE_SWIZZLE_A, g, bl, a;
    CHECK(!get_swizzle(&v, &r, &g, &bl, &a));
    CHECK_EQ(static_cast<int>(r), 0);
    NVNtextureDepthStencilMode m = NVN_TEXTURE_DEPTH_STENCIL_MODE_STENCIL;
    CHECK(!get_dsm(&v, &m));
    CHECK_EQ(static_cast<int>(m), 0);
    NVNtextureTarget t = NVN_TEXTURE_TARGET_BUFFER;
    CHECK(!get_target(&v, &t));
    CHECK_EQ(static_cast<int>(t), 0);

    const TextureViewInfo d = decode(&v);
    CHECK(!d.has_levels);
    CHECK(!d.has_layers);
    CHECK(!d.has_format);
    CHECK(!d.has_swizzle);
    CHECK(!d.has_depth_stencil_mode);
    CHECK(!d.has_target);

    // a zero-filled view decodes to all zero
    NVNtextureView z;
    std::memset(&z, 0, sizeof z);
    const TextureViewInfo zd = decode(&z);
    CHECK_EQ(zd.num_levels, 0);
    CHECK_EQ(zd.num_layers, 0);
    CHECK_EQ(zd.format, 0);
    CHECK_EQ(zd.swizzle[0], 0);
    CHECK_EQ(zd.target, 0);
    CHECK(!zd.has_target);
}

NXTEST(view_set_get_round_trip) {
    setup();
    auto view_defaults = abi::proc<PFNNVNTEXTUREVIEWSETDEFAULTSPROC>("nvnTextureViewSetDefaults");
    auto set_levels = abi::proc<PFNNVNTEXTUREVIEWSETLEVELSPROC>("nvnTextureViewSetLevels");
    auto get_levels = abi::proc<PFNNVNTEXTUREVIEWGETLEVELSPROC>("nvnTextureViewGetLevels");
    auto set_layers = abi::proc<PFNNVNTEXTUREVIEWSETLAYERSPROC>("nvnTextureViewSetLayers");
    auto get_layers = abi::proc<PFNNVNTEXTUREVIEWGETLAYERSPROC>("nvnTextureViewGetLayers");
    auto set_format = abi::proc<PFNNVNTEXTUREVIEWSETFORMATPROC>("nvnTextureViewSetFormat");
    auto get_format = abi::proc<PFNNVNTEXTUREVIEWGETFORMATPROC>("nvnTextureViewGetFormat");
    auto set_swizzle = abi::proc<PFNNVNTEXTUREVIEWSETSWIZZLEPROC>("nvnTextureViewSetSwizzle");
    auto get_swizzle = abi::proc<PFNNVNTEXTUREVIEWGETSWIZZLEPROC>("nvnTextureViewGetSwizzle");
    auto set_dsm = abi::proc<PFNNVNTEXTUREVIEWSETDEPTHSTENCILMODEPROC>("nvnTextureViewSetDepthStencilMode");
    auto get_dsm = abi::proc<PFNNVNTEXTUREVIEWGETDEPTHSTENCILMODEPROC>("nvnTextureViewGetDepthStencilMode");
    auto set_target = abi::proc<PFNNVNTEXTUREVIEWSETTARGETPROC>("nvnTextureViewSetTarget");
    auto get_target = abi::proc<PFNNVNTEXTUREVIEWGETTARGETPROC>("nvnTextureViewGetTarget");
    auto compare = abi::proc<PFNNVNTEXTUREVIEWCOMPAREPROC>("nvnTextureViewCompare");

    NVNtextureView v;
    std::memset(&v, 0, sizeof v);
    view_defaults(&v);

    set_levels(&v, 2, 5);
    int lo = 0, ln = 0;
    CHECK(get_levels(&v, &lo, &ln));
    CHECK_EQ(lo, 2);
    CHECK_EQ(ln, 5);
    set_levels(&v, 0x7FFF, -1);        // 16 bits per field: the game's view is 16 bytes wide (boot note 122)
    CHECK(get_levels(&v, &lo, &ln));
    CHECK_EQ(lo, 0x7FFF);
    CHECK_EQ(ln, -1);

    set_layers(&v, 0, 6);
    int lay = -9, lays = -9;
    CHECK(get_layers(&v, &lay, &lays));
    CHECK_EQ(lay, 0);
    CHECK_EQ(lays, 6);

    for (int fv : {static_cast<int>(NVN_FORMAT_NONE), static_cast<int>(NVN_FORMAT_R8), static_cast<int>(NVN_FORMAT_RGBA32F)}) {
        set_format(&v, static_cast<NVNformat>(fv));
        NVNformat f = NVN_FORMAT_R8;
        CHECK(get_format(&v, &f));
        CHECK_EQ(static_cast<int>(f), fv);
    }

    set_swizzle(&v, NVN_TEXTURE_SWIZZLE_A, NVN_TEXTURE_SWIZZLE_B, NVN_TEXTURE_SWIZZLE_G, NVN_TEXTURE_SWIZZLE_ONE);
    NVNtextureSwizzle r, g, bl, a;
    CHECK(get_swizzle(&v, &r, &g, &bl, &a));
    CHECK_EQ(static_cast<int>(r), static_cast<int>(NVN_TEXTURE_SWIZZLE_A));
    CHECK_EQ(static_cast<int>(g), static_cast<int>(NVN_TEXTURE_SWIZZLE_B));
    CHECK_EQ(static_cast<int>(bl), static_cast<int>(NVN_TEXTURE_SWIZZLE_G));
    CHECK_EQ(static_cast<int>(a), static_cast<int>(NVN_TEXTURE_SWIZZLE_ONE));
    CHECK(get_swizzle(&v, nullptr, nullptr, nullptr, nullptr));   // null out pointers are allowed

    set_dsm(&v, NVN_TEXTURE_DEPTH_STENCIL_MODE_STENCIL);
    NVNtextureDepthStencilMode m = NVN_TEXTURE_DEPTH_STENCIL_MODE_DEPTH;
    CHECK(get_dsm(&v, &m));
    CHECK_EQ(static_cast<int>(m), static_cast<int>(NVN_TEXTURE_DEPTH_STENCIL_MODE_STENCIL));

    for (int t = NVN_TEXTURE_TARGET_1D; t <= NVN_TEXTURE_TARGET_BUFFER; ++t) {
        set_target(&v, static_cast<NVNtextureTarget>(t));
        NVNtextureTarget out = NVN_TEXTURE_TARGET_1D;
        CHECK(get_target(&v, &out));
        CHECK_EQ(static_cast<int>(out), t);
    }

    const TextureViewInfo d = decode(&v);
    CHECK(d.has_levels && d.has_layers && d.has_format && d.has_swizzle && d.has_depth_stencil_mode && d.has_target);
    CHECK_EQ(d.min_level, 0x7FFF);
    CHECK_EQ(d.num_levels, -1);
    CHECK_EQ(d.min_layer, 0);
    CHECK_EQ(d.num_layers, 6);
    CHECK_EQ(d.format, static_cast<int>(NVN_FORMAT_RGBA32F));
    CHECK_EQ(d.swizzle[0], static_cast<int>(NVN_TEXTURE_SWIZZLE_A));
    CHECK_EQ(d.depth_stencil_mode, static_cast<int>(NVN_TEXTURE_DEPTH_STENCIL_MODE_STENCIL));
    CHECK_EQ(d.target, static_cast<int>(NVN_TEXTURE_TARGET_BUFFER));

    // compare: equal objects, then the same stored values with a different set flag
    NVNtextureView same, other;
    std::memset(&same, 0, sizeof same);
    std::memset(&other, 0, sizeof other);
    view_defaults(&same);
    view_defaults(&other);
    CHECK_EQ(static_cast<int>(compare(&same, &other)), 1);
    set_target(&same, NVN_TEXTURE_TARGET_2D);
    set_target(&other, NVN_TEXTURE_TARGET_2D);
    CHECK_EQ(static_cast<int>(compare(&same, &other)), 1);
    set_target(&other, NVN_TEXTURE_TARGET_3D);
    CHECK_EQ(static_cast<int>(compare(&same, &other)), 0);
    NVNtextureView unset;
    std::memset(&unset, 0, sizeof unset);
    view_defaults(&unset);
    CHECK_EQ(static_cast<int>(compare(&same, &unset)), 0);   // set flags count
}

NXTEST(view_setting_one_field_keeps_the_others) {
    setup();
    auto view_defaults = abi::proc<PFNNVNTEXTUREVIEWSETDEFAULTSPROC>("nvnTextureViewSetDefaults");
    auto set_format = abi::proc<PFNNVNTEXTUREVIEWSETFORMATPROC>("nvnTextureViewSetFormat");
    auto set_levels = abi::proc<PFNNVNTEXTUREVIEWSETLEVELSPROC>("nvnTextureViewSetLevels");

    NVNtextureView v;
    std::memset(&v, 0, sizeof v);
    view_defaults(&v);
    set_format(&v, NVN_FORMAT_R8);
    TextureViewInfo d = decode(&v);
    CHECK(d.has_format);
    CHECK(!d.has_levels);
    CHECK(!d.has_target);
    set_levels(&v, 1, 2);
    d = decode(&v);
    CHECK(d.has_format);
    CHECK_EQ(d.format, static_cast<int>(NVN_FORMAT_R8));
    CHECK(d.has_levels);
    CHECK_EQ(d.min_level, 1);
    CHECK_EQ(d.num_levels, 2);
    CHECK(!d.has_swizzle);
}

NXTEST(view_copies_stay_independent) {
    setup();
    auto view_defaults = abi::proc<PFNNVNTEXTUREVIEWSETDEFAULTSPROC>("nvnTextureViewSetDefaults");
    auto set_target = abi::proc<PFNNVNTEXTUREVIEWSETTARGETPROC>("nvnTextureViewSetTarget");
    auto get_target = abi::proc<PFNNVNTEXTUREVIEWGETTARGETPROC>("nvnTextureViewGetTarget");

    NVNtextureView a, c;
    std::memset(&a, 0, sizeof a);
    view_defaults(&a);
    set_target(&a, NVN_TEXTURE_TARGET_CUBEMAP);
    std::memcpy(&c, &a, sizeof c);
    set_target(&a, NVN_TEXTURE_TARGET_RECTANGLE);
    NVNtextureTarget t = NVN_TEXTURE_TARGET_1D;
    CHECK(get_target(&c, &t));
    CHECK_EQ(static_cast<int>(t), static_cast<int>(NVN_TEXTURE_TARGET_CUBEMAP));
    CHECK_EQ(static_cast<int>(decode(&a).target), static_cast<int>(NVN_TEXTURE_TARGET_RECTANGLE));
}
