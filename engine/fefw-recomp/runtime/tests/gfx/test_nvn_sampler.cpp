// SPDX-License-Identifier: GPL-3.0-or-later
// NVNsamplerBuilder and NVNsampler through the typed thunks (the sampler family, api_sampler.cpp). Every function is called
// through abi::proc, the way a game calls it.
#include <cstring>
#include <limits>

#include "nvn_impl.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_sampler)

using namespace nxrt;
using namespace nxrt::nvn;

namespace {
void setup() {
    install();
    abi::reset_mismatches();
    register_api_sampler();
    CHECK_EQ(abi::mismatches(), 0u);
}
uint32_t bits(float f) {
    uint32_t u;
    std::memcpy(&u, &f, sizeof u);
    return u;
}
}  // namespace

NXTEST(all_entry_points_registered) {
    setup();
    const char* names[] = {
        // builder: 25
        "nvnSamplerBuilderSetDevice", "nvnSamplerBuilderGetDevice", "nvnSamplerBuilderSetDefaults",
        "nvnSamplerBuilderSetMinMagFilter", "nvnSamplerBuilderGetMinMagFilter", "nvnSamplerBuilderSetWrapMode",
        "nvnSamplerBuilderGetWrapMode", "nvnSamplerBuilderSetLodClamp", "nvnSamplerBuilderGetLodClamp",
        "nvnSamplerBuilderSetLodBias", "nvnSamplerBuilderGetLodBias", "nvnSamplerBuilderSetCompare",
        "nvnSamplerBuilderGetCompare", "nvnSamplerBuilderSetBorderColor", "nvnSamplerBuilderGetBorderColor",
        "nvnSamplerBuilderSetBorderColori", "nvnSamplerBuilderGetBorderColori", "nvnSamplerBuilderSetBorderColorui",
        "nvnSamplerBuilderGetBorderColorui", "nvnSamplerBuilderSetMaxAnisotropy", "nvnSamplerBuilderGetMaxAnisotropy",
        "nvnSamplerBuilderSetReductionFilter", "nvnSamplerBuilderGetReductionFilter", "nvnSamplerBuilderSetLodSnap",
        "nvnSamplerBuilderGetLodSnap",
        // sampler: 15
        "nvnSamplerInitialize", "nvnSamplerFinalize", "nvnSamplerSetDebugLabel", "nvnSamplerGetMinMagFilter",
        "nvnSamplerGetWrapMode", "nvnSamplerGetLodClamp", "nvnSamplerGetLodBias", "nvnSamplerGetCompare",
        "nvnSamplerGetBorderColor", "nvnSamplerGetBorderColori", "nvnSamplerGetBorderColorui", "nvnSamplerGetMaxAnisotropy",
        "nvnSamplerGetReductionFilter", "nvnSamplerCompare", "nvnSamplerGetDebugID",
    };
    for (const char* n : names) CHECK_MSG(is_implemented(n), n);
    CHECK_EQ(sizeof names / sizeof names[0], 40u);
    CHECK_EQ(abi::mismatches(), 0u);
}

NXTEST(builder_set_get_round_trip) {
    setup();
    auto set_device = abi::proc<PFNNVNSAMPLERBUILDERSETDEVICEPROC>("nvnSamplerBuilderSetDevice");
    auto get_device = abi::proc<NVNdevice* (*)(const NVNsamplerBuilder*)>("nvnSamplerBuilderGetDevice");
    auto set_minmag = abi::proc<PFNNVNSAMPLERBUILDERSETMINMAGFILTERPROC>("nvnSamplerBuilderSetMinMagFilter");
    auto get_minmag = abi::proc<PFNNVNSAMPLERBUILDERGETMINMAGFILTERPROC>("nvnSamplerBuilderGetMinMagFilter");
    auto set_wrap = abi::proc<PFNNVNSAMPLERBUILDERSETWRAPMODEPROC>("nvnSamplerBuilderSetWrapMode");
    auto get_wrap = abi::proc<PFNNVNSAMPLERBUILDERGETWRAPMODEPROC>("nvnSamplerBuilderGetWrapMode");
    auto set_lodclamp = abi::proc<PFNNVNSAMPLERBUILDERSETLODCLAMPPROC>("nvnSamplerBuilderSetLodClamp");
    auto get_lodclamp = abi::proc<PFNNVNSAMPLERBUILDERGETLODCLAMPPROC>("nvnSamplerBuilderGetLodClamp");
    auto set_bias = abi::proc<PFNNVNSAMPLERBUILDERSETLODBIASPROC>("nvnSamplerBuilderSetLodBias");
    auto get_bias = abi::proc<PFNNVNSAMPLERBUILDERGETLODBIASPROC>("nvnSamplerBuilderGetLodBias");
    auto set_compare = abi::proc<PFNNVNSAMPLERBUILDERSETCOMPAREPROC>("nvnSamplerBuilderSetCompare");
    auto get_compare = abi::proc<PFNNVNSAMPLERBUILDERGETCOMPAREPROC>("nvnSamplerBuilderGetCompare");
    auto set_border = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORPROC>("nvnSamplerBuilderSetBorderColor");
    auto get_border = abi::proc<PFNNVNSAMPLERBUILDERGETBORDERCOLORPROC>("nvnSamplerBuilderGetBorderColor");
    auto set_borderi = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORIPROC>("nvnSamplerBuilderSetBorderColori");
    auto get_borderi = abi::proc<PFNNVNSAMPLERBUILDERGETBORDERCOLORIPROC>("nvnSamplerBuilderGetBorderColori");
    auto set_borderui = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORUIPROC>("nvnSamplerBuilderSetBorderColorui");
    auto get_borderui = abi::proc<PFNNVNSAMPLERBUILDERGETBORDERCOLORUIPROC>("nvnSamplerBuilderGetBorderColorui");
    auto set_aniso = abi::proc<PFNNVNSAMPLERBUILDERSETMAXANISOTROPYPROC>("nvnSamplerBuilderSetMaxAnisotropy");
    auto get_aniso = abi::proc<PFNNVNSAMPLERBUILDERGETMAXANISOTROPYPROC>("nvnSamplerBuilderGetMaxAnisotropy");
    auto set_reduction = abi::proc<PFNNVNSAMPLERBUILDERSETREDUCTIONFILTERPROC>("nvnSamplerBuilderSetReductionFilter");
    auto get_reduction = abi::proc<PFNNVNSAMPLERBUILDERGETREDUCTIONFILTERPROC>("nvnSamplerBuilderGetReductionFilter");
    auto set_snap = abi::proc<PFNNVNSAMPLERBUILDERSETLODSNAPPROC>("nvnSamplerBuilderSetLodSnap");
    auto get_snap = abi::proc<PFNNVNSAMPLERBUILDERGETLODSNAPPROC>("nvnSamplerBuilderGetLodSnap");

    NVNsamplerBuilder b;
    std::memset(&b, 0xAB, sizeof b);
    NVNdevice dev{};   // a stand-in device: the builder keeps only its address
    set_device(&b, &dev);
    CHECK(get_device(&b) == &dev);

    // min filter: all six values; mag filter: NEAREST and LINEAR
    for (int v = 0; v <= 5; ++v) {
        for (int m = 0; m <= 1; ++m) {
            set_minmag(&b, static_cast<NVNminFilter>(v), static_cast<NVNmagFilter>(m));
            NVNminFilter mn = NVN_MIN_FILTER_LARGE;
            NVNmagFilter mg = NVN_MAG_FILTER_LARGE;
            get_minmag(&b, &mn, &mg);
            CHECK_EQ(static_cast<int>(mn), v);
            CHECK_EQ(static_cast<int>(mg), m);
        }
    }

    // wrap modes: every value on each axis, the extremes 0 and 7 included
    for (int v = 0; v <= 7; ++v) {
        set_wrap(&b, static_cast<NVNwrapMode>(v), static_cast<NVNwrapMode>(7 - v), static_cast<NVNwrapMode>(v));
        NVNwrapMode s = NVN_WRAP_MODE_LARGE, t = NVN_WRAP_MODE_LARGE, r = NVN_WRAP_MODE_LARGE;
        get_wrap(&b, &s, &t, &r);
        CHECK_EQ(static_cast<int>(s), v);
        CHECK_EQ(static_cast<int>(t), 7 - v);
        CHECK_EQ(static_cast<int>(r), v);
    }

    // lod clamp and lod bias
    set_lodclamp(&b, -1.5f, 12.25f);
    float lo = 0, hi = 0;
    get_lodclamp(&b, &lo, &hi);
    CHECK_EQ(lo, -1.5f);
    CHECK_EQ(hi, 12.25f);
    set_lodclamp(&b, 0.0f, 1000.0f);
    get_lodclamp(&b, &lo, &hi);
    CHECK_EQ(lo, 0.0f);
    CHECK_EQ(hi, 1000.0f);
    for (float v : {0.0f, -2.5f, 15.75f}) {
        set_bias(&b, v);
        CHECK_EQ(get_bias(&b), v);
    }

    // compare: mode NONE and COMPARE_R_TO_TEXTURE with several functions, NEVER (1) and ALWAYS (8) included
    set_compare(&b, NVN_COMPARE_MODE_NONE, NVN_COMPARE_FUNC_NEVER);
    NVNcompareMode cm = NVN_COMPARE_MODE_LARGE;
    NVNcompareFunc cf = NVN_COMPARE_FUNC_LARGE;
    get_compare(&b, &cm, &cf);
    CHECK_EQ(static_cast<int>(cm), static_cast<int>(NVN_COMPARE_MODE_NONE));
    CHECK_EQ(static_cast<int>(cf), static_cast<int>(NVN_COMPARE_FUNC_NEVER));
    set_compare(&b, NVN_COMPARE_MODE_COMPARE_R_TO_TEXTURE, NVN_COMPARE_FUNC_ALWAYS);
    get_compare(&b, &cm, &cf);
    CHECK_EQ(static_cast<int>(cm), static_cast<int>(NVN_COMPARE_MODE_COMPARE_R_TO_TEXTURE));
    CHECK_EQ(static_cast<int>(cf), static_cast<int>(NVN_COMPARE_FUNC_ALWAYS));
    set_compare(&b, NVN_COMPARE_MODE_COMPARE_R_TO_TEXTURE, NVN_COMPARE_FUNC_GEQUAL);
    get_compare(&b, &cm, &cf);
    CHECK_EQ(static_cast<int>(cf), static_cast<int>(NVN_COMPARE_FUNC_GEQUAL));

    // border colour: the words come back as they were set, for all three kinds
    const float fin[4] = {0.25f, -1.0f, 3.5f, 0.0f};
    set_border(&b, fin);
    float fout[4] = {};
    get_border(&b, fout);
    for (int k = 0; k < 4; ++k) CHECK_EQ(bits(fout[k]), bits(fin[k]));

    const int32_t iin[4] = {std::numeric_limits<int32_t>::min(), -1, 0, std::numeric_limits<int32_t>::max()};
    set_borderi(&b, iin);
    int32_t iout[4] = {};
    get_borderi(&b, iout);
    for (int k = 0; k < 4; ++k) CHECK_EQ(iout[k], iin[k]);

    const uint32_t uin[4] = {0u, 0xFFFFFFFFu, 0x80000000u, 0x7FFFFFFFu};
    set_borderui(&b, uin);
    uint32_t uout[4] = {};
    get_borderui(&b, uout);
    for (int k = 0; k < 4; ++k) CHECK_EQ(uout[k], uin[k]);

    // anisotropy, reduction filter, lod snap
    for (float v : {1.0f, 16.0f, 2.5f}) {
        set_aniso(&b, v);
        CHECK_EQ(get_aniso(&b), v);
    }
    for (int v = 0; v <= 2; ++v) {
        set_reduction(&b, static_cast<NVNsamplerReduction>(v));
        CHECK_EQ(static_cast<int>(get_reduction(&b)), v);
    }
    for (float v : {0.0f, 4.5f, 0.125f}) {
        set_snap(&b, v);
        CHECK_EQ(get_snap(&b), v);
    }
}

NXTEST(sampler_copies_builder_and_getters_agree) {
    setup();
    auto set_minmag = abi::proc<PFNNVNSAMPLERBUILDERSETMINMAGFILTERPROC>("nvnSamplerBuilderSetMinMagFilter");
    auto set_wrap = abi::proc<PFNNVNSAMPLERBUILDERSETWRAPMODEPROC>("nvnSamplerBuilderSetWrapMode");
    auto set_lodclamp = abi::proc<PFNNVNSAMPLERBUILDERSETLODCLAMPPROC>("nvnSamplerBuilderSetLodClamp");
    auto set_bias = abi::proc<PFNNVNSAMPLERBUILDERSETLODBIASPROC>("nvnSamplerBuilderSetLodBias");
    auto set_compare = abi::proc<PFNNVNSAMPLERBUILDERSETCOMPAREPROC>("nvnSamplerBuilderSetCompare");
    auto set_borderui = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORUIPROC>("nvnSamplerBuilderSetBorderColorui");
    auto set_aniso = abi::proc<PFNNVNSAMPLERBUILDERSETMAXANISOTROPYPROC>("nvnSamplerBuilderSetMaxAnisotropy");
    auto set_reduction = abi::proc<PFNNVNSAMPLERBUILDERSETREDUCTIONFILTERPROC>("nvnSamplerBuilderSetReductionFilter");
    auto set_snap = abi::proc<PFNNVNSAMPLERBUILDERSETLODSNAPPROC>("nvnSamplerBuilderSetLodSnap");
    auto init = abi::proc<PFNNVNSAMPLERINITIALIZEPROC>("nvnSamplerInitialize");
    auto fin = abi::proc<PFNNVNSAMPLERFINALIZEPROC>("nvnSamplerFinalize");
    auto label = abi::proc<PFNNVNSAMPLERSETDEBUGLABELPROC>("nvnSamplerSetDebugLabel");
    auto get_minmag = abi::proc<PFNNVNSAMPLERGETMINMAGFILTERPROC>("nvnSamplerGetMinMagFilter");
    auto get_wrap = abi::proc<PFNNVNSAMPLERGETWRAPMODEPROC>("nvnSamplerGetWrapMode");
    auto get_lodclamp = abi::proc<PFNNVNSAMPLERGETLODCLAMPPROC>("nvnSamplerGetLodClamp");
    auto get_bias = abi::proc<PFNNVNSAMPLERGETLODBIASPROC>("nvnSamplerGetLodBias");
    auto get_compare = abi::proc<PFNNVNSAMPLERGETCOMPAREPROC>("nvnSamplerGetCompare");
    auto get_borderui = abi::proc<PFNNVNSAMPLERGETBORDERCOLORUIPROC>("nvnSamplerGetBorderColorui");
    auto get_aniso = abi::proc<PFNNVNSAMPLERGETMAXANISOTROPYPROC>("nvnSamplerGetMaxAnisotropy");
    auto get_reduction = abi::proc<PFNNVNSAMPLERGETREDUCTIONFILTERPROC>("nvnSamplerGetReductionFilter");
    auto compare = abi::proc<PFNNVNSAMPLERCOMPAREPROC>("nvnSamplerCompare");
    auto debug_id = abi::proc<PFNNVNSAMPLERGETDEBUGIDPROC>("nvnSamplerGetDebugID");

    NVNsamplerBuilder b;
    std::memset(&b, 0x5A, sizeof b);
    set_minmag(&b, NVN_MIN_FILTER_LINEAR_MIPMAP_LINEAR, NVN_MAG_FILTER_NEAREST);
    set_wrap(&b, NVN_WRAP_MODE_MIRROR_CLAMP_TO_BORDER, NVN_WRAP_MODE_REPEAT, NVN_WRAP_MODE_CLAMP_TO_BORDER);
    set_lodclamp(&b, 2.0f, 500.0f);
    set_bias(&b, 1.25f);
    set_compare(&b, NVN_COMPARE_MODE_COMPARE_R_TO_TEXTURE, NVN_COMPARE_FUNC_LESS);
    const uint32_t border[4] = {1u, 2u, 3u, 0x80000001u};
    set_borderui(&b, border);
    set_aniso(&b, 4.0f);
    set_reduction(&b, NVN_SAMPLER_REDUCTION_MAX);
    set_snap(&b, 0.5f);

    NVNsampler s;
    std::memset(&s, 0xEE, sizeof s);
    CHECK_EQ(init(&s, &b), 1);
    NVNminFilter mn = NVN_MIN_FILTER_LARGE;
    NVNmagFilter mg = NVN_MAG_FILTER_LARGE;
    get_minmag(&s, &mn, &mg);
    CHECK_EQ(static_cast<int>(mn), static_cast<int>(NVN_MIN_FILTER_LINEAR_MIPMAP_LINEAR));
    CHECK_EQ(static_cast<int>(mg), static_cast<int>(NVN_MAG_FILTER_NEAREST));
    NVNwrapMode ws = NVN_WRAP_MODE_LARGE, wt = NVN_WRAP_MODE_LARGE, wr = NVN_WRAP_MODE_LARGE;
    get_wrap(&s, &ws, &wt, &wr);
    CHECK_EQ(static_cast<int>(ws), static_cast<int>(NVN_WRAP_MODE_MIRROR_CLAMP_TO_BORDER));
    CHECK_EQ(static_cast<int>(wt), static_cast<int>(NVN_WRAP_MODE_REPEAT));
    CHECK_EQ(static_cast<int>(wr), static_cast<int>(NVN_WRAP_MODE_CLAMP_TO_BORDER));
    float lo = 0, hi = 0;
    get_lodclamp(&s, &lo, &hi);
    CHECK_EQ(lo, 2.0f);
    CHECK_EQ(hi, 500.0f);
    CHECK_EQ(get_bias(&s), 1.25f);
    NVNcompareMode cm = NVN_COMPARE_MODE_LARGE;
    NVNcompareFunc cf = NVN_COMPARE_FUNC_LARGE;
    get_compare(&s, &cm, &cf);
    CHECK_EQ(static_cast<int>(cm), static_cast<int>(NVN_COMPARE_MODE_COMPARE_R_TO_TEXTURE));
    CHECK_EQ(static_cast<int>(cf), static_cast<int>(NVN_COMPARE_FUNC_LESS));
    uint32_t bout[4] = {};
    get_borderui(&s, bout);
    for (int k = 0; k < 4; ++k) CHECK_EQ(bout[k], border[k]);
    CHECK_EQ(get_aniso(&s), 4.0f);
    CHECK_EQ(static_cast<int>(get_reduction(&s)), static_cast<int>(NVN_SAMPLER_REDUCTION_MAX));

    // the sampler is a copy: changing the builder afterwards does not change it
    set_bias(&b, -7.0f);
    CHECK_EQ(get_bias(&s), 1.25f);
    CHECK_EQ(compare(&s, &s), 1);

    label(&s, "sampler under test");
    CHECK_EQ(debug_id(&s), static_cast<uint64_t>(0));
    fin(&s);
}

NXTEST(defaults) {
    setup();
    auto set_device = abi::proc<PFNNVNSAMPLERBUILDERSETDEVICEPROC>("nvnSamplerBuilderSetDevice");
    auto get_device = abi::proc<NVNdevice* (*)(const NVNsamplerBuilder*)>("nvnSamplerBuilderGetDevice");
    auto defaults = abi::proc<PFNNVNSAMPLERBUILDERSETDEFAULTSPROC>("nvnSamplerBuilderSetDefaults");
    auto get_minmag = abi::proc<PFNNVNSAMPLERBUILDERGETMINMAGFILTERPROC>("nvnSamplerBuilderGetMinMagFilter");
    auto get_wrap = abi::proc<PFNNVNSAMPLERBUILDERGETWRAPMODEPROC>("nvnSamplerBuilderGetWrapMode");
    auto get_lodclamp = abi::proc<PFNNVNSAMPLERBUILDERGETLODCLAMPPROC>("nvnSamplerBuilderGetLodClamp");
    auto get_bias = abi::proc<PFNNVNSAMPLERBUILDERGETLODBIASPROC>("nvnSamplerBuilderGetLodBias");
    auto get_compare = abi::proc<PFNNVNSAMPLERBUILDERGETCOMPAREPROC>("nvnSamplerBuilderGetCompare");
    auto get_border = abi::proc<PFNNVNSAMPLERBUILDERGETBORDERCOLORPROC>("nvnSamplerBuilderGetBorderColor");
    auto get_aniso = abi::proc<PFNNVNSAMPLERBUILDERGETMAXANISOTROPYPROC>("nvnSamplerBuilderGetMaxAnisotropy");
    auto get_reduction = abi::proc<PFNNVNSAMPLERBUILDERGETREDUCTIONFILTERPROC>("nvnSamplerBuilderGetReductionFilter");
    auto get_snap = abi::proc<PFNNVNSAMPLERBUILDERGETLODSNAPPROC>("nvnSamplerBuilderGetLodSnap");
    auto init = abi::proc<PFNNVNSAMPLERINITIALIZEPROC>("nvnSamplerInitialize");
    auto s_minmag = abi::proc<PFNNVNSAMPLERGETMINMAGFILTERPROC>("nvnSamplerGetMinMagFilter");
    auto s_wrap = abi::proc<PFNNVNSAMPLERGETWRAPMODEPROC>("nvnSamplerGetWrapMode");
    auto s_compare = abi::proc<PFNNVNSAMPLERGETCOMPAREPROC>("nvnSamplerGetCompare");
    auto s_reduction = abi::proc<PFNNVNSAMPLERGETREDUCTIONFILTERPROC>("nvnSamplerGetReductionFilter");

    NVNdevice dev{};
    NVNsamplerBuilder b;
    std::memset(&b, 0xCD, sizeof b);
    set_device(&b, &dev);
    defaults(&b);
    CHECK(get_device(&b) == &dev);   // the device is not a sampler parameter: SetDefaults keeps it

    NVNminFilter mn = NVN_MIN_FILTER_LARGE;
    NVNmagFilter mg = NVN_MAG_FILTER_LARGE;
    get_minmag(&b, &mn, &mg);
    CHECK_EQ(static_cast<int>(mn), static_cast<int>(NVN_MIN_FILTER_LINEAR));
    CHECK_EQ(static_cast<int>(mg), static_cast<int>(NVN_MAG_FILTER_LINEAR));
    NVNwrapMode ws = NVN_WRAP_MODE_LARGE, wt = NVN_WRAP_MODE_LARGE, wr = NVN_WRAP_MODE_LARGE;
    get_wrap(&b, &ws, &wt, &wr);
    CHECK_EQ(static_cast<int>(ws), static_cast<int>(NVN_WRAP_MODE_CLAMP_TO_EDGE));
    CHECK_EQ(static_cast<int>(wt), static_cast<int>(NVN_WRAP_MODE_CLAMP_TO_EDGE));
    CHECK_EQ(static_cast<int>(wr), static_cast<int>(NVN_WRAP_MODE_CLAMP_TO_EDGE));
    float lo = -1, hi = -1;
    get_lodclamp(&b, &lo, &hi);
    CHECK_EQ(lo, 0.0f);
    CHECK_EQ(hi, 1000.0f);
    CHECK_EQ(get_bias(&b), 0.0f);
    NVNcompareMode cm = NVN_COMPARE_MODE_LARGE;
    NVNcompareFunc cf = NVN_COMPARE_FUNC_LARGE;
    get_compare(&b, &cm, &cf);
    CHECK_EQ(static_cast<int>(cm), static_cast<int>(NVN_COMPARE_MODE_NONE));
    CHECK_EQ(static_cast<int>(cf), static_cast<int>(NVN_COMPARE_FUNC_LESS));
    float border[4] = {9.0f, 9.0f, 9.0f, 9.0f};
    get_border(&b, border);
    for (int k = 0; k < 4; ++k) CHECK_EQ(bits(border[k]), 0u);
    CHECK_EQ(get_aniso(&b), 1.0f);
    CHECK_EQ(static_cast<int>(get_reduction(&b)), static_cast<int>(NVN_SAMPLER_REDUCTION_AVERAGE));
    CHECK_EQ(get_snap(&b), 0.0f);

    // the sampler made from the defaults carries the same values
    NVNsampler s;
    std::memset(&s, 0xFF, sizeof s);
    CHECK_EQ(init(&s, &b), 1);
    NVNwrapMode sws = NVN_WRAP_MODE_LARGE, swt = NVN_WRAP_MODE_LARGE, swr = NVN_WRAP_MODE_LARGE;
    s_wrap(&s, &sws, &swt, &swr);
    CHECK_EQ(static_cast<int>(sws), static_cast<int>(NVN_WRAP_MODE_CLAMP_TO_EDGE));
    CHECK_EQ(static_cast<int>(swr), static_cast<int>(NVN_WRAP_MODE_CLAMP_TO_EDGE));
    s_minmag(&s, &mn, &mg);
    CHECK_EQ(static_cast<int>(mn), static_cast<int>(NVN_MIN_FILTER_LINEAR));
    s_compare(&s, &cm, &cf);
    CHECK_EQ(static_cast<int>(cf), static_cast<int>(NVN_COMPARE_FUNC_LESS));
    CHECK_EQ(static_cast<int>(s_reduction(&s)), static_cast<int>(NVN_SAMPLER_REDUCTION_AVERAGE));
}

NXTEST(setting_one_field_leaves_others_untouched) {
    setup();
    auto set_minmag = abi::proc<PFNNVNSAMPLERBUILDERSETMINMAGFILTERPROC>("nvnSamplerBuilderSetMinMagFilter");
    auto set_wrap = abi::proc<PFNNVNSAMPLERBUILDERSETWRAPMODEPROC>("nvnSamplerBuilderSetWrapMode");
    auto set_lodclamp = abi::proc<PFNNVNSAMPLERBUILDERSETLODCLAMPPROC>("nvnSamplerBuilderSetLodClamp");
    auto set_bias = abi::proc<PFNNVNSAMPLERBUILDERSETLODBIASPROC>("nvnSamplerBuilderSetLodBias");
    auto set_compare = abi::proc<PFNNVNSAMPLERBUILDERSETCOMPAREPROC>("nvnSamplerBuilderSetCompare");
    auto set_border = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORPROC>("nvnSamplerBuilderSetBorderColor");
    auto set_aniso = abi::proc<PFNNVNSAMPLERBUILDERSETMAXANISOTROPYPROC>("nvnSamplerBuilderSetMaxAnisotropy");
    auto set_reduction = abi::proc<PFNNVNSAMPLERBUILDERSETREDUCTIONFILTERPROC>("nvnSamplerBuilderSetReductionFilter");
    auto set_snap = abi::proc<PFNNVNSAMPLERBUILDERSETLODSNAPPROC>("nvnSamplerBuilderSetLodSnap");

    NVNsamplerBuilder b;
    std::memset(&b, 0, sizeof b);
    // a base state where every field is away from its default
    set_minmag(&b, NVN_MIN_FILTER_LINEAR_MIPMAP_NEAREST, NVN_MAG_FILTER_NEAREST);
    set_wrap(&b, NVN_WRAP_MODE_REPEAT, NVN_WRAP_MODE_MIRRORED_REPEAT, NVN_WRAP_MODE_MIRROR_CLAMP);
    set_lodclamp(&b, 2.0f, 500.0f);
    set_bias(&b, 1.25f);
    set_compare(&b, NVN_COMPARE_MODE_COMPARE_R_TO_TEXTURE, NVN_COMPARE_FUNC_GEQUAL);
    const float border[4] = {0.5f, 0.25f, 0.125f, 1.0f};
    set_border(&b, border);
    set_aniso(&b, 4.0f);
    set_reduction(&b, NVN_SAMPLER_REDUCTION_MIN);
    set_snap(&b, 0.5f);
    SamplerInfo cur = decode(&b);
    SamplerInfo exp = cur;

    set_minmag(&b, NVN_MIN_FILTER_NEAREST, NVN_MAG_FILTER_LINEAR);
    exp.min_filter = NVN_MIN_FILTER_NEAREST;
    exp.mag_filter = NVN_MAG_FILTER_LINEAR;
    CHECK(decode(&b) == exp);
    cur = exp;

    set_wrap(&b, NVN_WRAP_MODE_CLAMP_TO_EDGE, NVN_WRAP_MODE_CLAMP, NVN_WRAP_MODE_CLAMP_TO_BORDER);
    exp.wrap_s = NVN_WRAP_MODE_CLAMP_TO_EDGE;
    exp.wrap_t = NVN_WRAP_MODE_CLAMP;
    exp.wrap_r = NVN_WRAP_MODE_CLAMP_TO_BORDER;
    CHECK(decode(&b) == exp);
    cur = exp;

    set_lodclamp(&b, -3.0f, 9.0f);
    exp.lod_min = -3.0f;
    exp.lod_max = 9.0f;
    CHECK(decode(&b) == exp);
    cur = exp;

    set_bias(&b, -0.75f);
    exp.lod_bias = -0.75f;
    CHECK(decode(&b) == exp);
    cur = exp;

    set_compare(&b, NVN_COMPARE_MODE_NONE, NVN_COMPARE_FUNC_ALWAYS);
    exp.compare_mode = NVN_COMPARE_MODE_NONE;
    exp.compare_func = NVN_COMPARE_FUNC_ALWAYS;
    CHECK(decode(&b) == exp);
    cur = exp;

    const float border2[4] = {0.0f, 0.0f, 0.0f, 0.5f};
    set_border(&b, border2);
    for (int k = 0; k < 4; ++k) exp.border_color[k] = border2[k];
    exp.border_kind = 0;
    CHECK(decode(&b) == exp);
    cur = exp;

    set_aniso(&b, 16.0f);
    exp.max_anisotropy = 16.0f;
    CHECK(decode(&b) == exp);
    cur = exp;

    set_reduction(&b, NVN_SAMPLER_REDUCTION_MAX);
    exp.reduction = NVN_SAMPLER_REDUCTION_MAX;
    CHECK(decode(&b) == exp);
    cur = exp;

    set_snap(&b, 2.0f);
    exp.lod_snap = 2.0f;
    CHECK(decode(&b) == exp);
    CHECK(!(decode(&b) == cur));   // the last change is visible
}

NXTEST(decode_agrees_with_getters_and_zero_object) {
    setup();
    auto set_minmag = abi::proc<PFNNVNSAMPLERBUILDERSETMINMAGFILTERPROC>("nvnSamplerBuilderSetMinMagFilter");
    auto set_wrap = abi::proc<PFNNVNSAMPLERBUILDERSETWRAPMODEPROC>("nvnSamplerBuilderSetWrapMode");
    auto set_bias = abi::proc<PFNNVNSAMPLERBUILDERSETLODBIASPROC>("nvnSamplerBuilderSetLodBias");
    auto set_aniso = abi::proc<PFNNVNSAMPLERBUILDERSETMAXANISOTROPYPROC>("nvnSamplerBuilderSetMaxAnisotropy");
    auto set_reduction = abi::proc<PFNNVNSAMPLERBUILDERSETREDUCTIONFILTERPROC>("nvnSamplerBuilderSetReductionFilter");
    auto get_aniso = abi::proc<PFNNVNSAMPLERBUILDERGETMAXANISOTROPYPROC>("nvnSamplerBuilderGetMaxAnisotropy");
    auto get_reduction = abi::proc<PFNNVNSAMPLERBUILDERGETREDUCTIONFILTERPROC>("nvnSamplerBuilderGetReductionFilter");
    auto init = abi::proc<PFNNVNSAMPLERINITIALIZEPROC>("nvnSamplerInitialize");

    NVNsamplerBuilder b;
    std::memset(&b, 0, sizeof b);
    set_minmag(&b, NVN_MIN_FILTER_LINEAR_MIPMAP_LINEAR, NVN_MAG_FILTER_LINEAR);
    set_wrap(&b, NVN_WRAP_MODE_REPEAT, NVN_WRAP_MODE_CLAMP_TO_EDGE, NVN_WRAP_MODE_MIRROR_CLAMP_TO_EDGE);
    set_bias(&b, -2.5f);
    set_aniso(&b, 8.0f);
    set_reduction(&b, NVN_SAMPLER_REDUCTION_MIN);
    SamplerInfo d = decode(&b);
    CHECK_EQ(d.min_filter, static_cast<int>(NVN_MIN_FILTER_LINEAR_MIPMAP_LINEAR));
    CHECK_EQ(d.mag_filter, static_cast<int>(NVN_MAG_FILTER_LINEAR));
    CHECK_EQ(d.wrap_s, static_cast<int>(NVN_WRAP_MODE_REPEAT));
    CHECK_EQ(d.wrap_t, static_cast<int>(NVN_WRAP_MODE_CLAMP_TO_EDGE));
    CHECK_EQ(d.wrap_r, static_cast<int>(NVN_WRAP_MODE_MIRROR_CLAMP_TO_EDGE));
    CHECK_EQ(d.lod_bias, -2.5f);
    CHECK_EQ(d.max_anisotropy, get_aniso(&b));
    CHECK_EQ(d.reduction, static_cast<int>(get_reduction(&b)));

    // a zero-filled builder decodes to the all-zero value of every field
    NVNsamplerBuilder zb;
    std::memset(&zb, 0, sizeof zb);
    SamplerInfo z = decode(&zb);
    SamplerInfo zero;
    zero.lod_max = 0;
    zero.max_anisotropy = 0;
    CHECK(z == zero);
    CHECK_EQ(z.border_kind, 0);
    CHECK_EQ(bits(z.border_color[0]), 0u);
    CHECK_EQ(bits(z.lod_snap), 0u);

    // and so does a zero-filled sampler, both for the decode and for the copy made from it
    NVNsampler zs;
    std::memset(&zs, 0, sizeof zs);
    SamplerInfo zd = decode(&zs);
    CHECK(zd == zero);
    NVNsampler copy;
    std::memset(&copy, 0x77, sizeof copy);
    CHECK_EQ(init(&copy, &zb), 1);
    CHECK(decode(&copy) == zero);
}

NXTEST(border_colour_kinds_decode_bit_for_bit) {
    setup();
    auto set_borderi = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORIPROC>("nvnSamplerBuilderSetBorderColori");
    auto set_borderui = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORUIPROC>("nvnSamplerBuilderSetBorderColorui");
    auto set_border = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORPROC>("nvnSamplerBuilderSetBorderColor");
    auto get_borderi = abi::proc<PFNNVNSAMPLERBUILDERGETBORDERCOLORIPROC>("nvnSamplerBuilderGetBorderColori");
    auto get_border = abi::proc<PFNNVNSAMPLERBUILDERGETBORDERCOLORPROC>("nvnSamplerBuilderGetBorderColor");

    NVNsamplerBuilder b;
    std::memset(&b, 0, sizeof b);
    // a quiet NaN with a payload, and -0.0: the words must survive untouched
    const uint32_t words[4] = {0x7FC00001u, 0x80000000u, 0xFFFFFFFFu, 0x00000001u};
    set_borderui(&b, words);
    SamplerInfo d = decode(&b);
    CHECK_EQ(d.border_kind, 2);
    for (int k = 0; k < 4; ++k) CHECK_EQ(bits(d.border_color[k]), words[k]);
    uint32_t back[4] = {};
    get_border(&b, reinterpret_cast<float*>(back));   // the float getter reinterprets the same four words
    for (int k = 0; k < 4; ++k) CHECK_EQ(back[k], words[k]);

    const int32_t ints[4] = {-7, 0, 1, std::numeric_limits<int32_t>::min()};
    set_borderi(&b, ints);
    d = decode(&b);
    CHECK_EQ(d.border_kind, 1);
    for (int k = 0; k < 4; ++k) CHECK_EQ(bits(d.border_color[k]), static_cast<uint32_t>(ints[k]));
    int32_t iback[4] = {};
    get_borderi(&b, iback);
    for (int k = 0; k < 4; ++k) CHECK_EQ(iback[k], ints[k]);

    const float fl[4] = {0.5f, -0.0f, 2.0f, 1.0f};
    set_border(&b, fl);
    d = decode(&b);
    CHECK_EQ(d.border_kind, 0);
    for (int k = 0; k < 4; ++k) CHECK_EQ(bits(d.border_color[k]), bits(fl[k]));
}

NXTEST(objects_survive_memcpy_and_stay_independent) {
    setup();
    auto set_minmag = abi::proc<PFNNVNSAMPLERBUILDERSETMINMAGFILTERPROC>("nvnSamplerBuilderSetMinMagFilter");
    auto set_bias = abi::proc<PFNNVNSAMPLERBUILDERSETLODBIASPROC>("nvnSamplerBuilderSetLodBias");
    auto set_borderui = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORUIPROC>("nvnSamplerBuilderSetBorderColorui");
    auto init = abi::proc<PFNNVNSAMPLERINITIALIZEPROC>("nvnSamplerInitialize");
    auto compare = abi::proc<PFNNVNSAMPLERCOMPAREPROC>("nvnSamplerCompare");

    NVNsamplerBuilder a;
    std::memset(&a, 0, sizeof a);
    set_minmag(&a, NVN_MIN_FILTER_NEAREST_MIPMAP_LINEAR, NVN_MAG_FILTER_LINEAR);
    set_bias(&a, 3.0f);
    const uint32_t w[4] = {4u, 5u, 6u, 7u};
    set_borderui(&a, w);

    NVNsamplerBuilder copy;
    std::memcpy(&copy, &a, sizeof a);   // the game copies these objects around
    set_bias(&a, -1.0f);                // the original changes, the copy does not
    CHECK(decode(&copy).lod_bias == 3.0f);
    CHECK(decode(&a).lod_bias == -1.0f);

    NVNsampler s1, s2;
    std::memset(&s1, 0, sizeof s1);
    std::memset(&s2, 0, sizeof s2);
    CHECK_EQ(init(&s1, &copy), 1);
    std::memcpy(&s2, &s1, sizeof s1);
    CHECK_EQ(compare(&s1, &s2), 1);
    CHECK(decode(&s2) == decode(&copy));
}

NXTEST(sampler_compare_checks_full_state) {
    setup();
    auto set_bias = abi::proc<PFNNVNSAMPLERBUILDERSETLODBIASPROC>("nvnSamplerBuilderSetLodBias");
    auto set_minmag = abi::proc<PFNNVNSAMPLERBUILDERSETMINMAGFILTERPROC>("nvnSamplerBuilderSetMinMagFilter");
    auto set_snap = abi::proc<PFNNVNSAMPLERBUILDERSETLODSNAPPROC>("nvnSamplerBuilderSetLodSnap");
    auto init = abi::proc<PFNNVNSAMPLERINITIALIZEPROC>("nvnSamplerInitialize");
    auto compare = abi::proc<PFNNVNSAMPLERCOMPAREPROC>("nvnSamplerCompare");

    NVNsamplerBuilder a, b;
    std::memset(&a, 0, sizeof a);
    std::memset(&b, 0, sizeof b);
    set_minmag(&a, NVN_MIN_FILTER_LINEAR, NVN_MAG_FILTER_LINEAR);
    set_minmag(&b, NVN_MIN_FILTER_LINEAR, NVN_MAG_FILTER_LINEAR);
    NVNsampler sa, sb;
    CHECK_EQ(init(&sa, &a), 1);
    CHECK_EQ(init(&sb, &b), 1);
    CHECK_EQ(compare(&sa, &sb), 1);

    set_snap(&b, 0.25f);   // one field differs
    CHECK_EQ(init(&sb, &b), 1);
    CHECK_EQ(compare(&sa, &sb), 0);
    CHECK_EQ(compare(&sb, &sa), 0);

    // floats are compared bit for bit: +0.0 and -0.0 differ
    set_snap(&b, 0.0f);
    set_bias(&a, 0.0f);
    set_bias(&b, -0.0f);
    CHECK_EQ(init(&sa, &a), 1);
    CHECK_EQ(init(&sb, &b), 1);
    CHECK_EQ(compare(&sa, &sb), 0);

    // a null operand is not equal to anything
    CHECK_EQ(compare(nullptr, &sb), 0);
    CHECK_EQ(compare(&sa, nullptr), 0);
}

NXTEST(null_pointers_are_tolerated) {
    setup();
    auto get_minmag = abi::proc<PFNNVNSAMPLERBUILDERGETMINMAGFILTERPROC>("nvnSamplerBuilderGetMinMagFilter");
    auto get_wrap = abi::proc<PFNNVNSAMPLERBUILDERGETWRAPMODEPROC>("nvnSamplerBuilderGetWrapMode");
    auto get_lodclamp = abi::proc<PFNNVNSAMPLERBUILDERGETLODCLAMPPROC>("nvnSamplerBuilderGetLodClamp");
    auto get_compare = abi::proc<PFNNVNSAMPLERBUILDERGETCOMPAREPROC>("nvnSamplerBuilderGetCompare");
    auto get_border = abi::proc<PFNNVNSAMPLERBUILDERGETBORDERCOLORPROC>("nvnSamplerBuilderGetBorderColor");
    auto set_border = abi::proc<PFNNVNSAMPLERBUILDERSETBORDERCOLORPROC>("nvnSamplerBuilderSetBorderColor");
    auto s_minmag = abi::proc<PFNNVNSAMPLERGETMINMAGFILTERPROC>("nvnSamplerGetMinMagFilter");
    auto s_wrap = abi::proc<PFNNVNSAMPLERGETWRAPMODEPROC>("nvnSamplerGetWrapMode");
    auto s_lodclamp = abi::proc<PFNNVNSAMPLERGETLODCLAMPPROC>("nvnSamplerGetLodClamp");
    auto s_compare = abi::proc<PFNNVNSAMPLERGETCOMPAREPROC>("nvnSamplerGetCompare");
    auto s_border = abi::proc<PFNNVNSAMPLERGETBORDERCOLORPROC>("nvnSamplerGetBorderColor");

    NVNsamplerBuilder b;
    std::memset(&b, 0, sizeof b);
    get_minmag(&b, nullptr, nullptr);
    get_wrap(&b, nullptr, nullptr, nullptr);
    get_lodclamp(&b, nullptr, nullptr);
    get_compare(&b, nullptr, nullptr);
    get_border(&b, nullptr);
    set_border(&b, nullptr);   // nothing to copy: the stored colour is unchanged
    NVNsampler s;
    std::memset(&s, 0, sizeof s);
    s_minmag(&s, nullptr, nullptr);
    s_wrap(&s, nullptr, nullptr, nullptr);
    s_lodclamp(&s, nullptr, nullptr);
    s_compare(&s, nullptr, nullptr);
    s_border(&s, nullptr);
    CHECK(decode(&b) == decode(&s));
}

NXTEST(sampler_info_equality_is_bitwise) {
    setup();
    SamplerInfo a, b;
    CHECK(a == b);
    a.lod_bias = 0.0f;
    b.lod_bias = -0.0f;
    CHECK(!(a == b));   // -0.0 differs from +0.0 bit for bit
    a = b = SamplerInfo{};
    const float nan = std::numeric_limits<float>::quiet_NaN();
    a.border_color[2] = nan;
    b.border_color[2] = nan;
    CHECK(a == b);      // the same NaN bits are equal
    b.border_kind = 2;
    CHECK(!(a == b));
    a = SamplerInfo{};
    b = SamplerInfo{};
    b.reduction = 1;
    CHECK(!(a == b));
}
