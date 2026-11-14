// SPDX-License-Identifier: GPL-3.0-or-later
// NVNblendState, NVNcolorState and NVNchannelMaskState through the typed thunks (the blend / colour / channel mask family).
#include <cstring>

#include "nvn_impl.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_blend_color)

using namespace nxrt;
using namespace nxrt::nvn;

namespace {
void setup() {
    install();
    abi::reset_mismatches();
    register_api_blend_color();
    CHECK_EQ(abi::mismatches(), 0u);
}

const char* const kNames[] = {
    "nvnBlendStateSetDefaults",
    "nvnBlendStateSetBlendTarget",
    "nvnBlendStateSetBlendFunc",
    "nvnBlendStateSetBlendEquation",
    "nvnBlendStateSetAdvancedMode",
    "nvnBlendStateSetAdvancedOverlap",
    "nvnBlendStateSetAdvancedPremultipliedSrc",
    "nvnBlendStateSetAdvancedNormalizedDst",
    "nvnBlendStateGetBlendTarget",
    "nvnBlendStateGetBlendFunc",
    "nvnBlendStateGetBlendEquation",
    "nvnBlendStateGetAdvancedMode",
    "nvnBlendStateGetAdvancedOverlap",
    "nvnBlendStateGetAdvancedPremultipliedSrc",
    "nvnBlendStateGetAdvancedNormalizedDst",
    "nvnColorStateSetDefaults",
    "nvnColorStateSetBlendEnable",
    "nvnColorStateSetLogicOp",
    "nvnColorStateSetAlphaTest",
    "nvnColorStateGetBlendEnable",
    "nvnColorStateGetLogicOp",
    "nvnColorStateGetAlphaTest",
    "nvnChannelMaskStateSetDefaults",
    "nvnChannelMaskStateSetChannelMask",
    "nvnChannelMaskStateGetChannelMask",
};
}  // namespace

NXTEST(all_entry_points_registered) {
    setup();
    CHECK_EQ(sizeof(kNames) / sizeof(kNames[0]), 25u);
    for (const char* name : kNames) CHECK(is_implemented(name));
}

NXTEST(blend_state_round_trip) {
    setup();
    auto defaults = abi::proc<PFNNVNBLENDSTATESETDEFAULTSPROC>("nvnBlendStateSetDefaults");
    auto set_target = abi::proc<PFNNVNBLENDSTATESETBLENDTARGETPROC>("nvnBlendStateSetBlendTarget");
    auto get_target = abi::proc<PFNNVNBLENDSTATEGETBLENDTARGETPROC>("nvnBlendStateGetBlendTarget");
    auto set_func = abi::proc<PFNNVNBLENDSTATESETBLENDFUNCPROC>("nvnBlendStateSetBlendFunc");
    auto get_func = abi::proc<PFNNVNBLENDSTATEGETBLENDFUNCPROC>("nvnBlendStateGetBlendFunc");
    auto set_eq = abi::proc<PFNNVNBLENDSTATESETBLENDEQUATIONPROC>("nvnBlendStateSetBlendEquation");
    auto get_eq = abi::proc<PFNNVNBLENDSTATEGETBLENDEQUATIONPROC>("nvnBlendStateGetBlendEquation");
    auto set_mode = abi::proc<PFNNVNBLENDSTATESETADVANCEDMODEPROC>("nvnBlendStateSetAdvancedMode");
    auto get_mode = abi::proc<PFNNVNBLENDSTATEGETADVANCEDMODEPROC>("nvnBlendStateGetAdvancedMode");
    auto set_overlap = abi::proc<PFNNVNBLENDSTATESETADVANCEDOVERLAPPROC>("nvnBlendStateSetAdvancedOverlap");
    auto get_overlap = abi::proc<PFNNVNBLENDSTATEGETADVANCEDOVERLAPPROC>("nvnBlendStateGetAdvancedOverlap");
    auto set_premult = abi::proc<PFNNVNBLENDSTATESETADVANCEDPREMULTIPLIEDSRCPROC>("nvnBlendStateSetAdvancedPremultipliedSrc");
    auto get_premult = abi::proc<PFNNVNBLENDSTATEGETADVANCEDPREMULTIPLIEDSRCPROC>("nvnBlendStateGetAdvancedPremultipliedSrc");
    auto set_normdst = abi::proc<PFNNVNBLENDSTATESETADVANCEDNORMALIZEDDSTPROC>("nvnBlendStateSetAdvancedNormalizedDst");
    auto get_normdst = abi::proc<PFNNVNBLENDSTATEGETADVANCEDNORMALIZEDDSTPROC>("nvnBlendStateGetAdvancedNormalizedDst");

    NVNblendState s;
    std::memset(&s, 0xAB, sizeof s);
    defaults(&s);

    // target: the low end, a middle value and the top of the eight render targets
    for (int t : {0, 1, 7}) {
        set_target(&s, t);
        CHECK_EQ(get_target(&s), t);
    }

    // blend funcs: the lowest and highest values of the enum, plus values from both ends of the list
    const NVNblendFunc funcs[][4] = {
        {NVN_BLEND_FUNC_ZERO, NVN_BLEND_FUNC_ONE, NVN_BLEND_FUNC_ZERO, NVN_BLEND_FUNC_ONE},
        {NVN_BLEND_FUNC_SRC_ALPHA_SATURATE, NVN_BLEND_FUNC_SRC1_ALPHA, NVN_BLEND_FUNC_SRC_COLOR, NVN_BLEND_FUNC_DST_ALPHA},
        {NVN_BLEND_FUNC_ONE_MINUS_CONSTANT_ALPHA, NVN_BLEND_FUNC_CONSTANT_COLOR, NVN_BLEND_FUNC_ONE_MINUS_SRC1_COLOR,
         NVN_BLEND_FUNC_ONE_MINUS_CONSTANT_ALPHA},
    };
    for (const auto& f : funcs) {
        set_func(&s, f[0], f[1], f[2], f[3]);
        NVNblendFunc a, b, c, d;
        get_func(&s, &a, &b, &c, &d);
        CHECK_EQ(static_cast<int>(a), static_cast<int>(f[0]));
        CHECK_EQ(static_cast<int>(b), static_cast<int>(f[1]));
        CHECK_EQ(static_cast<int>(c), static_cast<int>(f[2]));
        CHECK_EQ(static_cast<int>(d), static_cast<int>(f[3]));
    }

    // equations: first, last and middle values
    const NVNblendEquation eqs[][2] = {
        {NVN_BLEND_EQUATION_ADD, NVN_BLEND_EQUATION_ADD},
        {NVN_BLEND_EQUATION_REVERSE_SUB, NVN_BLEND_EQUATION_MIN},
        {NVN_BLEND_EQUATION_SUB, NVN_BLEND_EQUATION_MAX},
    };
    for (const auto& e : eqs) {
        set_eq(&s, e[0], e[1]);
        NVNblendEquation rgb, alpha;
        get_eq(&s, &rgb, &alpha);
        CHECK_EQ(static_cast<int>(rgb), static_cast<int>(e[0]));
        CHECK_EQ(static_cast<int>(alpha), static_cast<int>(e[1]));
    }

    // advanced mode: none, the first and the last value of the enum
    for (NVNblendAdvancedMode m : {NVN_BLEND_ADVANCED_MODE_NONE, NVN_BLEND_ADVANCED_MODE_ZERO, NVN_BLEND_ADVANCED_MODE_MULTIPLY,
                                   NVN_BLEND_ADVANCED_MODE_HSL_LUMINOSITY}) {
        set_mode(&s, m);
        CHECK_EQ(static_cast<int>(get_mode(&s)), static_cast<int>(m));
    }

    // overlap: all three values
    for (NVNblendAdvancedOverlap o : {NVN_BLEND_ADVANCED_OVERLAP_UNCORRELATED, NVN_BLEND_ADVANCED_OVERLAP_DISJOINT,
                                      NVN_BLEND_ADVANCED_OVERLAP_CONJOINT}) {
        set_overlap(&s, o);
        CHECK_EQ(static_cast<int>(get_overlap(&s)), static_cast<int>(o));
    }

    // booleans
    for (NVNboolean v : {NVNboolean(0), NVNboolean(1), NVNboolean(0)}) {
        set_premult(&s, v);
        CHECK_EQ(static_cast<int>(get_premult(&s)), static_cast<int>(v));
        set_normdst(&s, v);
        CHECK_EQ(static_cast<int>(get_normdst(&s)), static_cast<int>(v));
    }
}

NXTEST(blend_state_defaults) {
    setup();
    auto defaults = abi::proc<PFNNVNBLENDSTATESETDEFAULTSPROC>("nvnBlendStateSetDefaults");
    auto get_target = abi::proc<PFNNVNBLENDSTATEGETBLENDTARGETPROC>("nvnBlendStateGetBlendTarget");
    auto get_func = abi::proc<PFNNVNBLENDSTATEGETBLENDFUNCPROC>("nvnBlendStateGetBlendFunc");
    auto get_eq = abi::proc<PFNNVNBLENDSTATEGETBLENDEQUATIONPROC>("nvnBlendStateGetBlendEquation");
    auto get_mode = abi::proc<PFNNVNBLENDSTATEGETADVANCEDMODEPROC>("nvnBlendStateGetAdvancedMode");
    auto get_overlap = abi::proc<PFNNVNBLENDSTATEGETADVANCEDOVERLAPPROC>("nvnBlendStateGetAdvancedOverlap");
    auto get_premult = abi::proc<PFNNVNBLENDSTATEGETADVANCEDPREMULTIPLIEDSRCPROC>("nvnBlendStateGetAdvancedPremultipliedSrc");
    auto get_normdst = abi::proc<PFNNVNBLENDSTATEGETADVANCEDNORMALIZEDDSTPROC>("nvnBlendStateGetAdvancedNormalizedDst");

    NVNblendState s;
    std::memset(&s, 0xCD, sizeof s);
    defaults(&s);
    CHECK_EQ(get_target(&s), 0);
    NVNblendFunc a, b, c, d;
    get_func(&s, &a, &b, &c, &d);
    CHECK_EQ(static_cast<int>(a), static_cast<int>(NVN_BLEND_FUNC_ONE));
    CHECK_EQ(static_cast<int>(b), static_cast<int>(NVN_BLEND_FUNC_ZERO));
    CHECK_EQ(static_cast<int>(c), static_cast<int>(NVN_BLEND_FUNC_ONE));
    CHECK_EQ(static_cast<int>(d), static_cast<int>(NVN_BLEND_FUNC_ZERO));
    NVNblendEquation rgb, alpha;
    get_eq(&s, &rgb, &alpha);
    CHECK_EQ(static_cast<int>(rgb), static_cast<int>(NVN_BLEND_EQUATION_ADD));
    CHECK_EQ(static_cast<int>(alpha), static_cast<int>(NVN_BLEND_EQUATION_ADD));
    CHECK_EQ(static_cast<int>(get_mode(&s)), static_cast<int>(NVN_BLEND_ADVANCED_MODE_NONE));
    CHECK_EQ(static_cast<int>(get_overlap(&s)), static_cast<int>(NVN_BLEND_ADVANCED_OVERLAP_UNCORRELATED));
    CHECK_EQ(static_cast<int>(get_premult(&s)), 1);
    CHECK_EQ(static_cast<int>(get_normdst(&s)), 0);
}

NXTEST(blend_state_setting_one_field_keeps_the_others) {
    setup();
    auto defaults = abi::proc<PFNNVNBLENDSTATESETDEFAULTSPROC>("nvnBlendStateSetDefaults");
    auto set_target = abi::proc<PFNNVNBLENDSTATESETBLENDTARGETPROC>("nvnBlendStateSetBlendTarget");
    auto get_target = abi::proc<PFNNVNBLENDSTATEGETBLENDTARGETPROC>("nvnBlendStateGetBlendTarget");
    auto set_func = abi::proc<PFNNVNBLENDSTATESETBLENDFUNCPROC>("nvnBlendStateSetBlendFunc");
    auto get_func = abi::proc<PFNNVNBLENDSTATEGETBLENDFUNCPROC>("nvnBlendStateGetBlendFunc");
    auto set_eq = abi::proc<PFNNVNBLENDSTATESETBLENDEQUATIONPROC>("nvnBlendStateSetBlendEquation");
    auto get_eq = abi::proc<PFNNVNBLENDSTATEGETBLENDEQUATIONPROC>("nvnBlendStateGetBlendEquation");
    auto set_mode = abi::proc<PFNNVNBLENDSTATESETADVANCEDMODEPROC>("nvnBlendStateSetAdvancedMode");
    auto get_mode = abi::proc<PFNNVNBLENDSTATEGETADVANCEDMODEPROC>("nvnBlendStateGetAdvancedMode");
    auto set_overlap = abi::proc<PFNNVNBLENDSTATESETADVANCEDOVERLAPPROC>("nvnBlendStateSetAdvancedOverlap");
    auto get_overlap = abi::proc<PFNNVNBLENDSTATEGETADVANCEDOVERLAPPROC>("nvnBlendStateGetAdvancedOverlap");
    auto set_premult = abi::proc<PFNNVNBLENDSTATESETADVANCEDPREMULTIPLIEDSRCPROC>("nvnBlendStateSetAdvancedPremultipliedSrc");
    auto get_premult = abi::proc<PFNNVNBLENDSTATEGETADVANCEDPREMULTIPLIEDSRCPROC>("nvnBlendStateGetAdvancedPremultipliedSrc");
    auto set_normdst = abi::proc<PFNNVNBLENDSTATESETADVANCEDNORMALIZEDDSTPROC>("nvnBlendStateSetAdvancedNormalizedDst");
    auto get_normdst = abi::proc<PFNNVNBLENDSTATEGETADVANCEDNORMALIZEDDSTPROC>("nvnBlendStateGetAdvancedNormalizedDst");

    NVNblendState s;
    std::memset(&s, 0, sizeof s);
    defaults(&s);
    // a known full state
    set_target(&s, 5);
    set_func(&s, NVN_BLEND_FUNC_SRC_COLOR, NVN_BLEND_FUNC_ONE_MINUS_SRC_ALPHA, NVN_BLEND_FUNC_DST_ALPHA, NVN_BLEND_FUNC_ZERO);
    set_eq(&s, NVN_BLEND_EQUATION_MAX, NVN_BLEND_EQUATION_REVERSE_SUB);
    set_mode(&s, NVN_BLEND_ADVANCED_MODE_OVERLAY);
    set_overlap(&s, NVN_BLEND_ADVANCED_OVERLAP_CONJOINT);
    set_premult(&s, 0);
    set_normdst(&s, 1);

    // changing one field at a time leaves all the others as they were
    set_target(&s, 2);
    CHECK_EQ(get_target(&s), 2);
    CHECK_EQ(static_cast<int>(get_mode(&s)), static_cast<int>(NVN_BLEND_ADVANCED_MODE_OVERLAY));
    CHECK_EQ(static_cast<int>(get_overlap(&s)), static_cast<int>(NVN_BLEND_ADVANCED_OVERLAP_CONJOINT));
    CHECK_EQ(static_cast<int>(get_premult(&s)), 0);
    CHECK_EQ(static_cast<int>(get_normdst(&s)), 1);

    set_func(&s, NVN_BLEND_FUNC_ONE, NVN_BLEND_FUNC_ONE, NVN_BLEND_FUNC_ONE, NVN_BLEND_FUNC_ONE);
    CHECK_EQ(get_target(&s), 2);
    NVNblendEquation rgb, alpha;
    get_eq(&s, &rgb, &alpha);
    CHECK_EQ(static_cast<int>(rgb), static_cast<int>(NVN_BLEND_EQUATION_MAX));
    CHECK_EQ(static_cast<int>(alpha), static_cast<int>(NVN_BLEND_EQUATION_REVERSE_SUB));

    set_eq(&s, NVN_BLEND_EQUATION_ADD, NVN_BLEND_EQUATION_ADD);
    NVNblendFunc a, b, c, d;
    get_func(&s, &a, &b, &c, &d);
    CHECK_EQ(static_cast<int>(a), static_cast<int>(NVN_BLEND_FUNC_ONE));
    CHECK_EQ(static_cast<int>(b), static_cast<int>(NVN_BLEND_FUNC_ONE));

    set_premult(&s, 1);
    CHECK_EQ(static_cast<int>(get_normdst(&s)), 1);
    CHECK_EQ(static_cast<int>(get_overlap(&s)), static_cast<int>(NVN_BLEND_ADVANCED_OVERLAP_CONJOINT));
    CHECK_EQ(static_cast<int>(get_mode(&s)), static_cast<int>(NVN_BLEND_ADVANCED_MODE_OVERLAY));
    get_func(&s, &a, &b, nullptr, nullptr);   // null out pointers are allowed
    CHECK_EQ(static_cast<int>(a), static_cast<int>(NVN_BLEND_FUNC_ONE));
}

NXTEST(color_state_round_trip_and_defaults) {
    setup();
    auto defaults = abi::proc<PFNNVNCOLORSTATESETDEFAULTSPROC>("nvnColorStateSetDefaults");
    auto set_enable = abi::proc<PFNNVNCOLORSTATESETBLENDENABLEPROC>("nvnColorStateSetBlendEnable");
    auto get_enable = abi::proc<PFNNVNCOLORSTATEGETBLENDENABLEPROC>("nvnColorStateGetBlendEnable");
    auto set_logic = abi::proc<PFNNVNCOLORSTATESETLOGICOPPROC>("nvnColorStateSetLogicOp");
    auto get_logic = abi::proc<PFNNVNCOLORSTATEGETLOGICOPPROC>("nvnColorStateGetLogicOp");
    auto set_alpha = abi::proc<PFNNVNCOLORSTATESETALPHATESTPROC>("nvnColorStateSetAlphaTest");
    auto get_alpha = abi::proc<PFNNVNCOLORSTATEGETALPHATESTPROC>("nvnColorStateGetAlphaTest");

    NVNcolorState s;
    std::memset(&s, 0xEF, sizeof s);
    defaults(&s);
    for (int t = 0; t < 8; t++) CHECK_EQ(static_cast<int>(get_enable(&s, t)), 0);
    CHECK_EQ(static_cast<int>(get_logic(&s)), static_cast<int>(NVN_LOGIC_OP_COPY));
    CHECK_EQ(static_cast<int>(get_alpha(&s)), static_cast<int>(NVN_ALPHA_FUNC_ALWAYS));

    // blending on for the extremes and one middle target, then off again
    set_enable(&s, 0, 1);
    set_enable(&s, 7, 1);
    set_enable(&s, 3, 1);
    CHECK_EQ(static_cast<int>(get_enable(&s, 0)), 1);
    CHECK_EQ(static_cast<int>(get_enable(&s, 3)), 1);
    CHECK_EQ(static_cast<int>(get_enable(&s, 7)), 1);
    CHECK_EQ(static_cast<int>(get_enable(&s, 1)), 0);
    CHECK_EQ(static_cast<int>(get_enable(&s, 6)), 0);
    set_enable(&s, 3, 0);
    CHECK_EQ(static_cast<int>(get_enable(&s, 3)), 0);
    CHECK_EQ(static_cast<int>(get_enable(&s, 7)), 1);   // turning one target off keeps the others on

    // logic ops: first, last, and a middle value
    for (NVNlogicOp op : {NVN_LOGIC_OP_CLEAR, NVN_LOGIC_OP_SET, NVN_LOGIC_OP_COPY_INVERTED, NVN_LOGIC_OP_COPY}) {
        set_logic(&s, op);
        CHECK_EQ(static_cast<int>(get_logic(&s)), static_cast<int>(op));
    }
    // alpha tests: first, last, and a middle value
    for (NVNalphaFunc f : {NVN_ALPHA_FUNC_NEVER, NVN_ALPHA_FUNC_GEQUAL, NVN_ALPHA_FUNC_ALWAYS}) {
        set_alpha(&s, f);
        CHECK_EQ(static_cast<int>(get_alpha(&s)), static_cast<int>(f));
    }

    // setting the logic op and alpha test kept the blend enable bits
    CHECK_EQ(static_cast<int>(get_enable(&s, 7)), 1);
    CHECK_EQ(static_cast<int>(get_enable(&s, 0)), 1);
    set_logic(&s, NVN_LOGIC_OP_SET);
    CHECK_EQ(static_cast<int>(get_alpha(&s)), static_cast<int>(NVN_ALPHA_FUNC_ALWAYS));
    CHECK_EQ(static_cast<int>(get_enable(&s, 0)), 1);
}

NXTEST(channel_mask_round_trip_defaults_and_nulls) {
    setup();
    auto defaults = abi::proc<PFNNVNCHANNELMASKSTATESETDEFAULTSPROC>("nvnChannelMaskStateSetDefaults");
    auto set_mask = abi::proc<PFNNVNCHANNELMASKSTATESETCHANNELMASKPROC>("nvnChannelMaskStateSetChannelMask");
    auto get_mask = abi::proc<PFNNVNCHANNELMASKSTATEGETCHANNELMASKPROC>("nvnChannelMaskStateGetChannelMask");

    NVNchannelMaskState s;
    std::memset(&s, 0x12, sizeof s);
    defaults(&s);
    for (int t = 0; t < 8; t++) {
        NVNboolean r = 9, g = 9, b = 9, a = 9;
        get_mask(&s, t, &r, &g, &b, &a);
        CHECK_EQ(static_cast<int>(r), 1);
        CHECK_EQ(static_cast<int>(g), 1);
        CHECK_EQ(static_cast<int>(b), 1);
        CHECK_EQ(static_cast<int>(a), 1);
    }

    // target 0 red only, target 7 green and alpha, target 3 blue and alpha
    set_mask(&s, 0, 1, 0, 0, 0);
    set_mask(&s, 7, 0, 1, 0, 1);
    set_mask(&s, 3, 0, 0, 1, 1);
    NVNboolean r, g, b, a;
    get_mask(&s, 0, &r, &g, &b, &a);
    CHECK_EQ(static_cast<int>(r) * 1000 + static_cast<int>(g) * 100 + static_cast<int>(b) * 10 + static_cast<int>(a), 1000);
    get_mask(&s, 7, &r, &g, &b, &a);
    CHECK_EQ(static_cast<int>(r) * 1000 + static_cast<int>(g) * 100 + static_cast<int>(b) * 10 + static_cast<int>(a), 101);
    get_mask(&s, 3, &r, &g, &b, &a);
    CHECK_EQ(static_cast<int>(r) * 1000 + static_cast<int>(g) * 100 + static_cast<int>(b) * 10 + static_cast<int>(a), 11);
    // target 1 was not touched by any of the above
    get_mask(&s, 1, &r, &g, &b, &a);
    CHECK_EQ(static_cast<int>(r) * 1000 + static_cast<int>(g) * 100 + static_cast<int>(b) * 10 + static_cast<int>(a), 1111);
    // null out pointers are allowed
    get_mask(&s, 7, nullptr, &g, nullptr, nullptr);
    CHECK_EQ(static_cast<int>(g), 1);
}

NXTEST(decode_agrees_with_the_getters_and_zero_objects_decode_to_zero) {
    setup();
    auto b_defaults = abi::proc<PFNNVNBLENDSTATESETDEFAULTSPROC>("nvnBlendStateSetDefaults");
    auto b_target = abi::proc<PFNNVNBLENDSTATESETBLENDTARGETPROC>("nvnBlendStateSetBlendTarget");
    auto b_func = abi::proc<PFNNVNBLENDSTATESETBLENDFUNCPROC>("nvnBlendStateSetBlendFunc");
    auto b_eq = abi::proc<PFNNVNBLENDSTATESETBLENDEQUATIONPROC>("nvnBlendStateSetBlendEquation");
    auto b_mode = abi::proc<PFNNVNBLENDSTATESETADVANCEDMODEPROC>("nvnBlendStateSetAdvancedMode");
    auto b_overlap = abi::proc<PFNNVNBLENDSTATESETADVANCEDOVERLAPPROC>("nvnBlendStateSetAdvancedOverlap");
    auto b_premult = abi::proc<PFNNVNBLENDSTATESETADVANCEDPREMULTIPLIEDSRCPROC>("nvnBlendStateSetAdvancedPremultipliedSrc");
    auto b_normdst = abi::proc<PFNNVNBLENDSTATESETADVANCEDNORMALIZEDDSTPROC>("nvnBlendStateSetAdvancedNormalizedDst");
    auto c_defaults = abi::proc<PFNNVNCOLORSTATESETDEFAULTSPROC>("nvnColorStateSetDefaults");
    auto c_enable = abi::proc<PFNNVNCOLORSTATESETBLENDENABLEPROC>("nvnColorStateSetBlendEnable");
    auto c_logic = abi::proc<PFNNVNCOLORSTATESETLOGICOPPROC>("nvnColorStateSetLogicOp");
    auto c_alpha = abi::proc<PFNNVNCOLORSTATESETALPHATESTPROC>("nvnColorStateSetAlphaTest");
    auto m_defaults = abi::proc<PFNNVNCHANNELMASKSTATESETDEFAULTSPROC>("nvnChannelMaskStateSetDefaults");
    auto m_set = abi::proc<PFNNVNCHANNELMASKSTATESETCHANNELMASKPROC>("nvnChannelMaskStateSetChannelMask");

    NVNblendState bs;
    std::memset(&bs, 0, sizeof bs);
    b_defaults(&bs);
    b_target(&bs, 6);
    b_func(&bs, NVN_BLEND_FUNC_SRC1_COLOR, NVN_BLEND_FUNC_CONSTANT_ALPHA, NVN_BLEND_FUNC_SRC_ALPHA_SATURATE,
           NVN_BLEND_FUNC_ONE_MINUS_DST_COLOR);
    b_eq(&bs, NVN_BLEND_EQUATION_MIN, NVN_BLEND_EQUATION_SUB);
    b_mode(&bs, NVN_BLEND_ADVANCED_MODE_HARDLIGHT);
    b_overlap(&bs, NVN_BLEND_ADVANCED_OVERLAP_DISJOINT);
    b_premult(&bs, 0);
    b_normdst(&bs, 1);
    BlendInfo bi = decode(&bs);
    CHECK_EQ(bi.target, 6);
    CHECK_EQ(bi.src_rgb, static_cast<int>(NVN_BLEND_FUNC_SRC1_COLOR));
    CHECK_EQ(bi.dst_rgb, static_cast<int>(NVN_BLEND_FUNC_CONSTANT_ALPHA));
    CHECK_EQ(bi.src_a, static_cast<int>(NVN_BLEND_FUNC_SRC_ALPHA_SATURATE));
    CHECK_EQ(bi.dst_a, static_cast<int>(NVN_BLEND_FUNC_ONE_MINUS_DST_COLOR));
    CHECK_EQ(bi.eq_rgb, static_cast<int>(NVN_BLEND_EQUATION_MIN));
    CHECK_EQ(bi.eq_a, static_cast<int>(NVN_BLEND_EQUATION_SUB));
    CHECK_EQ(bi.advanced_mode, static_cast<int>(NVN_BLEND_ADVANCED_MODE_HARDLIGHT));
    CHECK_EQ(bi.advanced_overlap, static_cast<int>(NVN_BLEND_ADVANCED_OVERLAP_DISJOINT));
    CHECK(!bi.advanced_premultiplied_src);
    CHECK(bi.advanced_normalized_dst);

    NVNcolorState cs;
    std::memset(&cs, 0, sizeof cs);
    c_defaults(&cs);
    c_enable(&cs, 0, 1);
    c_enable(&cs, 7, 1);
    c_logic(&cs, NVN_LOGIC_OP_XOR);
    c_alpha(&cs, NVN_ALPHA_FUNC_LESS);
    ColorInfo ci = decode(&cs);
    CHECK_EQ(static_cast<int>(ci.blend_enable_mask), 0x81);
    CHECK_EQ(ci.logic_op, static_cast<int>(NVN_LOGIC_OP_XOR));
    CHECK_EQ(ci.alpha_test, static_cast<int>(NVN_ALPHA_FUNC_LESS));

    NVNchannelMaskState ms;
    std::memset(&ms, 0, sizeof ms);
    m_defaults(&ms);
    m_set(&ms, 7, 0, 1, 0, 1);
    ChannelMaskInfo mi = decode(&ms);
    CHECK_EQ(static_cast<int>(mi.mask[7]), 10);   // G and A, bits 1 and 3
    CHECK_EQ(static_cast<int>(mi.mask[0]), 15);   // default: all four channels

    // zero-filled objects decode to the all-zero value of every field
    NVNblendState zb;
    NVNcolorState zc;
    NVNchannelMaskState zm;
    std::memset(&zb, 0, sizeof zb);
    std::memset(&zc, 0, sizeof zc);
    std::memset(&zm, 0, sizeof zm);
    BlendInfo zbi = decode(&zb);
    CHECK_EQ(zbi.target, 0);
    CHECK_EQ(zbi.src_rgb, 0);
    CHECK_EQ(zbi.eq_a, 0);
    CHECK_EQ(zbi.advanced_mode, 0);
    CHECK(!zbi.advanced_premultiplied_src);
    CHECK(!zbi.advanced_normalized_dst);
    ColorInfo zci = decode(&zc);
    CHECK_EQ(static_cast<int>(zci.blend_enable_mask), 0);
    CHECK_EQ(zci.logic_op, 0);
    CHECK_EQ(zci.alpha_test, 0);
    ChannelMaskInfo zmi = decode(&zm);
    for (int t = 0; t < 8; t++) CHECK_EQ(static_cast<int>(zmi.mask[t]), 0);
}

NXTEST(objects_survive_memcpy) {
    setup();
    auto b_defaults = abi::proc<PFNNVNBLENDSTATESETDEFAULTSPROC>("nvnBlendStateSetDefaults");
    auto b_target = abi::proc<PFNNVNBLENDSTATESETBLENDTARGETPROC>("nvnBlendStateSetBlendTarget");
    auto b_get_target = abi::proc<PFNNVNBLENDSTATEGETBLENDTARGETPROC>("nvnBlendStateGetBlendTarget");
    auto c_defaults = abi::proc<PFNNVNCOLORSTATESETDEFAULTSPROC>("nvnColorStateSetDefaults");
    auto c_enable = abi::proc<PFNNVNCOLORSTATESETBLENDENABLEPROC>("nvnColorStateSetBlendEnable");
    auto c_get_enable = abi::proc<PFNNVNCOLORSTATEGETBLENDENABLEPROC>("nvnColorStateGetBlendEnable");
    auto m_defaults = abi::proc<PFNNVNCHANNELMASKSTATESETDEFAULTSPROC>("nvnChannelMaskStateSetDefaults");
    auto m_set = abi::proc<PFNNVNCHANNELMASKSTATESETCHANNELMASKPROC>("nvnChannelMaskStateSetChannelMask");
    auto m_get = abi::proc<PFNNVNCHANNELMASKSTATEGETCHANNELMASKPROC>("nvnChannelMaskStateGetChannelMask");

    NVNblendState ba{}, bb{};
    b_defaults(&ba);
    b_target(&ba, 2);
    std::memcpy(&bb, &ba, sizeof ba);   // the game copies state objects around freely
    b_target(&ba, 4);
    CHECK_EQ(b_get_target(&bb), 2);
    CHECK_EQ(b_get_target(&ba), 4);

    NVNcolorState ca{}, cb{};
    c_defaults(&ca);
    c_enable(&ca, 5, 1);
    std::memcpy(&cb, &ca, sizeof ca);
    c_enable(&ca, 5, 0);
    CHECK_EQ(static_cast<int>(c_get_enable(&cb, 5)), 1);
    CHECK_EQ(static_cast<int>(c_get_enable(&ca, 5)), 0);

    NVNchannelMaskState ma{}, mb{};
    m_defaults(&ma);
    m_set(&ma, 1, 0, 0, 0, 0);
    std::memcpy(&mb, &ma, sizeof ma);
    m_set(&ma, 1, 1, 1, 1, 1);
    NVNboolean r, g, b, a;
    m_get(&mb, 1, &r, &g, &b, &a);
    CHECK_EQ(static_cast<int>(r) + static_cast<int>(g) + static_cast<int>(b) + static_cast<int>(a), 0);
    m_get(&ma, 1, &r, &g, &b, &a);
    CHECK_EQ(static_cast<int>(r) + static_cast<int>(g) + static_cast<int>(b) + static_cast<int>(a), 4);
}
