// SPDX-License-Identifier: GPL-3.0-or-later
// NVNdepthStencilState, NVNpolygonState and NVNmultisampleState through the typed thunks. Every function of the family is
// called through abi::proc, the way a game calls it.
#include <cstring>

#include "nvn_impl.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_depth_poly_ms)

using namespace nxrt;
using namespace nxrt::nvn;

namespace {
void setup() {
    install();
    abi::reset_mismatches();
    register_api_depth_poly_ms();
    CHECK_EQ(abi::mismatches(), 0u);
}

const char* const kNames[] = {
    // depth-stencil state (13)
    "nvnDepthStencilStateSetDefaults", "nvnDepthStencilStateSetDepthTestEnable", "nvnDepthStencilStateSetDepthWriteEnable",
    "nvnDepthStencilStateSetDepthFunc", "nvnDepthStencilStateSetStencilTestEnable", "nvnDepthStencilStateSetStencilFunc",
    "nvnDepthStencilStateSetStencilOp", "nvnDepthStencilStateGetDepthTestEnable", "nvnDepthStencilStateGetDepthWriteEnable",
    "nvnDepthStencilStateGetDepthFunc", "nvnDepthStencilStateGetStencilTestEnable", "nvnDepthStencilStateGetStencilFunc",
    "nvnDepthStencilStateGetStencilOp",
    // polygon state (9)
    "nvnPolygonStateSetDefaults", "nvnPolygonStateSetCullFace", "nvnPolygonStateSetFrontFace", "nvnPolygonStateSetPolygonMode",
    "nvnPolygonStateSetPolygonOffsetEnables", "nvnPolygonStateGetCullFace", "nvnPolygonStateGetFrontFace",
    "nvnPolygonStateGetPolygonMode", "nvnPolygonStateGetPolygonOffsetEnables",
    // multisample state (23)
    "nvnMultisampleStateSetDefaults", "nvnMultisampleStateSetMultisampleEnable", "nvnMultisampleStateSetSamples",
    "nvnMultisampleStateSetAlphaToCoverageEnable", "nvnMultisampleStateSetAlphaToCoverageDither",
    "nvnMultisampleStateGetMultisampleEnable", "nvnMultisampleStateGetSamples", "nvnMultisampleStateGetAlphaToCoverageEnable",
    "nvnMultisampleStateGetAlphaToCoverageDither", "nvnMultisampleStateSetRasterSamples", "nvnMultisampleStateGetRasterSamples",
    "nvnMultisampleStateSetCoverageModulationMode", "nvnMultisampleStateGetCoverageModulationMode",
    "nvnMultisampleStateSetCoverageToColorEnable", "nvnMultisampleStateGetCoverageToColorEnable",
    "nvnMultisampleStateSetCoverageToColorOutput", "nvnMultisampleStateGetCoverageToColorOutput",
    "nvnMultisampleStateSetSampleLocationsEnable", "nvnMultisampleStateGetSampleLocationsEnable",
    "nvnMultisampleStateGetSampleLocationsGrid", "nvnMultisampleStateSetSampleLocationsGridEnable",
    "nvnMultisampleStateGetSampleLocationsGridEnable", "nvnMultisampleStateSetSampleLocations",
};
}  // namespace

NXTEST(all_functions_registered_and_checked) {
    setup();
    CHECK_EQ(static_cast<unsigned>(sizeof(kNames) / sizeof(kNames[0])), 45u);
    for (const char* name : kNames) CHECK(is_implemented(name));
}

NXTEST(depth_stencil_defaults_and_round_trip) {
    setup();
    auto ds_defaults = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEFAULTSPROC>("nvnDepthStencilStateSetDefaults");
    auto set_dt = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEPTHTESTENABLEPROC>("nvnDepthStencilStateSetDepthTestEnable");
    auto get_dt = abi::proc<PFNNVNDEPTHSTENCILSTATEGETDEPTHTESTENABLEPROC>("nvnDepthStencilStateGetDepthTestEnable");
    auto set_dw = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEPTHWRITEENABLEPROC>("nvnDepthStencilStateSetDepthWriteEnable");
    auto get_dw = abi::proc<PFNNVNDEPTHSTENCILSTATEGETDEPTHWRITEENABLEPROC>("nvnDepthStencilStateGetDepthWriteEnable");
    auto set_df = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEPTHFUNCPROC>("nvnDepthStencilStateSetDepthFunc");
    auto get_df = abi::proc<PFNNVNDEPTHSTENCILSTATEGETDEPTHFUNCPROC>("nvnDepthStencilStateGetDepthFunc");
    auto set_st = abi::proc<PFNNVNDEPTHSTENCILSTATESETSTENCILTESTENABLEPROC>("nvnDepthStencilStateSetStencilTestEnable");
    auto get_st = abi::proc<PFNNVNDEPTHSTENCILSTATEGETSTENCILTESTENABLEPROC>("nvnDepthStencilStateGetStencilTestEnable");
    auto set_sf = abi::proc<PFNNVNDEPTHSTENCILSTATESETSTENCILFUNCPROC>("nvnDepthStencilStateSetStencilFunc");
    auto get_sf = abi::proc<PFNNVNDEPTHSTENCILSTATEGETSTENCILFUNCPROC>("nvnDepthStencilStateGetStencilFunc");
    auto set_so = abi::proc<PFNNVNDEPTHSTENCILSTATESETSTENCILOPPROC>("nvnDepthStencilStateSetStencilOp");
    auto get_so = abi::proc<PFNNVNDEPTHSTENCILSTATEGETSTENCILOPPROC>("nvnDepthStencilStateGetStencilOp");

    NVNdepthStencilState s;
    std::memset(&s, 0xEE, sizeof s);
    ds_defaults(&s);
    CHECK_EQ(static_cast<int>(get_dt(&s)), 0);
    CHECK_EQ(static_cast<int>(get_dw(&s)), 1);
    CHECK_EQ(static_cast<int>(get_df(&s)), static_cast<int>(NVN_DEPTH_FUNC_LESS));
    CHECK_EQ(static_cast<int>(get_st(&s)), 0);
    CHECK_EQ(static_cast<int>(get_sf(&s, NVN_FACE_FRONT)), static_cast<int>(NVN_STENCIL_FUNC_ALWAYS));
    CHECK_EQ(static_cast<int>(get_sf(&s, NVN_FACE_BACK)), static_cast<int>(NVN_STENCIL_FUNC_ALWAYS));
    NVNstencilOp fail{}, dfail{}, dpass{};
    get_so(&s, NVN_FACE_FRONT_AND_BACK, &fail, &dfail, &dpass);
    CHECK_EQ(static_cast<int>(fail), static_cast<int>(NVN_STENCIL_OP_KEEP));
    CHECK_EQ(static_cast<int>(dfail), static_cast<int>(NVN_STENCIL_OP_KEEP));
    CHECK_EQ(static_cast<int>(dpass), static_cast<int>(NVN_STENCIL_OP_KEEP));
    get_so(&s, NVN_FACE_BACK, &fail, &dfail, &dpass);
    CHECK_EQ(static_cast<int>(fail), static_cast<int>(NVN_STENCIL_OP_KEEP));

    // booleans
    set_dt(&s, 1);
    CHECK_EQ(static_cast<int>(get_dt(&s)), 1);
    set_dt(&s, 0);
    CHECK_EQ(static_cast<int>(get_dt(&s)), 0);
    set_dw(&s, 0);
    CHECK_EQ(static_cast<int>(get_dw(&s)), 0);
    set_dw(&s, 1);
    CHECK_EQ(static_cast<int>(get_dw(&s)), 1);
    set_st(&s, 1);
    CHECK_EQ(static_cast<int>(get_st(&s)), 1);
    set_st(&s, 0);
    CHECK_EQ(static_cast<int>(get_st(&s)), 0);

    // every depth function, the extremes included
    for (int v = NVN_DEPTH_FUNC_NEVER; v <= NVN_DEPTH_FUNC_ALWAYS; ++v) {
        set_df(&s, static_cast<NVNdepthFunc>(v));
        CHECK_EQ(static_cast<int>(get_df(&s)), v);
    }
    // every stencil function on each face
    for (int v = NVN_STENCIL_FUNC_NEVER; v <= NVN_STENCIL_FUNC_ALWAYS; ++v) {
        set_sf(&s, NVN_FACE_FRONT, static_cast<NVNstencilFunc>(v));
        CHECK_EQ(static_cast<int>(get_sf(&s, NVN_FACE_FRONT)), v);
        set_sf(&s, NVN_FACE_BACK, static_cast<NVNstencilFunc>(NVN_STENCIL_FUNC_ALWAYS + NVN_STENCIL_FUNC_NEVER - v));
        CHECK_EQ(static_cast<int>(get_sf(&s, NVN_FACE_BACK)), static_cast<int>(NVN_STENCIL_FUNC_ALWAYS + NVN_STENCIL_FUNC_NEVER - v));
    }
    // every stencil operation, on each of the three slots
    for (int v = NVN_STENCIL_OP_KEEP; v <= NVN_STENCIL_OP_DECR_WRAP; ++v) {
        const NVNstencilOp op = static_cast<NVNstencilOp>(v);
        set_so(&s, NVN_FACE_BACK, op, NVN_STENCIL_OP_KEEP, NVN_STENCIL_OP_KEEP);
        get_so(&s, NVN_FACE_BACK, &fail, &dfail, &dpass);
        CHECK_EQ(static_cast<int>(fail), v);
        CHECK_EQ(static_cast<int>(dfail), static_cast<int>(NVN_STENCIL_OP_KEEP));
        CHECK_EQ(static_cast<int>(dpass), static_cast<int>(NVN_STENCIL_OP_KEEP));
        set_so(&s, NVN_FACE_BACK, NVN_STENCIL_OP_KEEP, op, NVN_STENCIL_OP_KEEP);
        get_so(&s, NVN_FACE_BACK, &fail, &dfail, &dpass);
        CHECK_EQ(static_cast<int>(dfail), v);
        CHECK_EQ(static_cast<int>(dpass), static_cast<int>(NVN_STENCIL_OP_KEEP));
        set_so(&s, NVN_FACE_BACK, NVN_STENCIL_OP_KEEP, NVN_STENCIL_OP_KEEP, op);
        get_so(&s, NVN_FACE_BACK, &fail, &dfail, &dpass);
        CHECK_EQ(static_cast<int>(dpass), v);
        CHECK_EQ(static_cast<int>(dfail), static_cast<int>(NVN_STENCIL_OP_KEEP));
    }
}

NXTEST(depth_stencil_face_selection_and_decode) {
    setup();
    auto ds_defaults = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEFAULTSPROC>("nvnDepthStencilStateSetDefaults");
    auto set_dt = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEPTHTESTENABLEPROC>("nvnDepthStencilStateSetDepthTestEnable");
    auto set_dw = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEPTHWRITEENABLEPROC>("nvnDepthStencilStateSetDepthWriteEnable");
    auto set_df = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEPTHFUNCPROC>("nvnDepthStencilStateSetDepthFunc");
    auto set_st = abi::proc<PFNNVNDEPTHSTENCILSTATESETSTENCILTESTENABLEPROC>("nvnDepthStencilStateSetStencilTestEnable");
    auto get_sf = abi::proc<PFNNVNDEPTHSTENCILSTATEGETSTENCILFUNCPROC>("nvnDepthStencilStateGetStencilFunc");
    auto set_sf = abi::proc<PFNNVNDEPTHSTENCILSTATESETSTENCILFUNCPROC>("nvnDepthStencilStateSetStencilFunc");
    auto set_so = abi::proc<PFNNVNDEPTHSTENCILSTATESETSTENCILOPPROC>("nvnDepthStencilStateSetStencilOp");
    auto get_so = abi::proc<PFNNVNDEPTHSTENCILSTATEGETSTENCILOPPROC>("nvnDepthStencilStateGetStencilOp");

    NVNdepthStencilState s;
    std::memset(&s, 0, sizeof s);
    ds_defaults(&s);

    // BACK writes only the back face; the front keeps the default ALWAYS
    set_sf(&s, NVN_FACE_BACK, NVN_STENCIL_FUNC_GREATER);
    CHECK_EQ(static_cast<int>(get_sf(&s, NVN_FACE_FRONT)), static_cast<int>(NVN_STENCIL_FUNC_ALWAYS));
    CHECK_EQ(static_cast<int>(get_sf(&s, NVN_FACE_BACK)), static_cast<int>(NVN_STENCIL_FUNC_GREATER));
    // a getter asked for FRONT_AND_BACK reads the front
    CHECK_EQ(static_cast<int>(get_sf(&s, NVN_FACE_FRONT_AND_BACK)), static_cast<int>(NVN_STENCIL_FUNC_ALWAYS));
    // FRONT_AND_BACK writes both
    set_sf(&s, NVN_FACE_FRONT_AND_BACK, NVN_STENCIL_FUNC_NEVER);
    CHECK_EQ(static_cast<int>(get_sf(&s, NVN_FACE_FRONT)), static_cast<int>(NVN_STENCIL_FUNC_NEVER));
    CHECK_EQ(static_cast<int>(get_sf(&s, NVN_FACE_BACK)), static_cast<int>(NVN_STENCIL_FUNC_NEVER));

    // operations: FRONT then BACK, each leaves the other face alone
    set_so(&s, NVN_FACE_FRONT, NVN_STENCIL_OP_REPLACE, NVN_STENCIL_OP_INCR, NVN_STENCIL_OP_DECR);
    set_so(&s, NVN_FACE_BACK, NVN_STENCIL_OP_ZERO, NVN_STENCIL_OP_INVERT, NVN_STENCIL_OP_INCR_WRAP);
    NVNstencilOp fail{}, dfail{}, dpass{};
    get_so(&s, NVN_FACE_FRONT, &fail, &dfail, &dpass);
    CHECK_EQ(static_cast<int>(fail), static_cast<int>(NVN_STENCIL_OP_REPLACE));
    CHECK_EQ(static_cast<int>(dfail), static_cast<int>(NVN_STENCIL_OP_INCR));
    CHECK_EQ(static_cast<int>(dpass), static_cast<int>(NVN_STENCIL_OP_DECR));
    get_so(&s, NVN_FACE_BACK, &fail, &dfail, &dpass);
    CHECK_EQ(static_cast<int>(fail), static_cast<int>(NVN_STENCIL_OP_ZERO));
    CHECK_EQ(static_cast<int>(dfail), static_cast<int>(NVN_STENCIL_OP_INVERT));
    CHECK_EQ(static_cast<int>(dpass), static_cast<int>(NVN_STENCIL_OP_INCR_WRAP));
    // null out pointers are tolerated
    get_so(&s, NVN_FACE_FRONT, nullptr, &dfail, nullptr);
    CHECK_EQ(static_cast<int>(dfail), static_cast<int>(NVN_STENCIL_OP_INCR));

    // one field at a time leaves the others untouched
    set_dt(&s, 1);
    set_dw(&s, 0);
    set_df(&s, NVN_DEPTH_FUNC_LEQUAL);
    set_st(&s, 1);
    set_dw(&s, 1);
    get_so(&s, NVN_FACE_FRONT, &fail, &dfail, &dpass);
    CHECK_EQ(static_cast<int>(fail), static_cast<int>(NVN_STENCIL_OP_REPLACE));

    DepthStencilInfo d = decode(&s);
    CHECK_EQ(d.depth_test, true);
    CHECK_EQ(d.depth_write, true);
    CHECK_EQ(d.depth_func, static_cast<int>(NVN_DEPTH_FUNC_LEQUAL));
    CHECK_EQ(d.stencil_test, true);
    CHECK_EQ(d.stencil_func[0], static_cast<int>(NVN_STENCIL_FUNC_NEVER));
    CHECK_EQ(d.stencil_func[1], static_cast<int>(NVN_STENCIL_FUNC_NEVER));
    CHECK_EQ(d.stencil_fail[0], static_cast<int>(NVN_STENCIL_OP_REPLACE));
    CHECK_EQ(d.depth_fail[0], static_cast<int>(NVN_STENCIL_OP_INCR));
    CHECK_EQ(d.depth_pass[0], static_cast<int>(NVN_STENCIL_OP_DECR));
    CHECK_EQ(d.stencil_fail[1], static_cast<int>(NVN_STENCIL_OP_ZERO));
    CHECK_EQ(d.depth_fail[1], static_cast<int>(NVN_STENCIL_OP_INVERT));
    CHECK_EQ(d.depth_pass[1], static_cast<int>(NVN_STENCIL_OP_INCR_WRAP));

    // an all-zero object decodes to the all-zero value
    NVNdepthStencilState z;
    std::memset(&z, 0, sizeof z);
    DepthStencilInfo zd = decode(&z);
    CHECK_EQ(zd.depth_test, false);
    CHECK_EQ(zd.depth_write, false);
    CHECK_EQ(zd.depth_func, 0);
    CHECK_EQ(zd.stencil_test, false);
    CHECK_EQ(zd.stencil_func[0], 0);
    CHECK_EQ(zd.stencil_fail[1], 0);
    CHECK_EQ(zd.depth_pass[1], 0);
}

NXTEST(depth_stencil_copies_stay_independent) {
    setup();
    auto ds_defaults = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEFAULTSPROC>("nvnDepthStencilStateSetDefaults");
    auto set_df = abi::proc<PFNNVNDEPTHSTENCILSTATESETDEPTHFUNCPROC>("nvnDepthStencilStateSetDepthFunc");
    auto get_df = abi::proc<PFNNVNDEPTHSTENCILSTATEGETDEPTHFUNCPROC>("nvnDepthStencilStateGetDepthFunc");
    NVNdepthStencilState a, b;
    ds_defaults(&a);
    set_df(&a, NVN_DEPTH_FUNC_GEQUAL);
    std::memcpy(&b, &a, sizeof a);   // the game copies state objects around freely
    set_df(&a, NVN_DEPTH_FUNC_NEVER);
    CHECK_EQ(static_cast<int>(get_df(&b)), static_cast<int>(NVN_DEPTH_FUNC_GEQUAL));
    CHECK_EQ(static_cast<int>(get_df(&a)), static_cast<int>(NVN_DEPTH_FUNC_NEVER));
}

NXTEST(polygon_state_defaults_round_trip_and_decode) {
    setup();
    auto p_defaults = abi::proc<PFNNVNPOLYGONSTATESETDEFAULTSPROC>("nvnPolygonStateSetDefaults");
    auto set_cull = abi::proc<PFNNVNPOLYGONSTATESETCULLFACEPROC>("nvnPolygonStateSetCullFace");
    auto get_cull = abi::proc<PFNNVNPOLYGONSTATEGETCULLFACEPROC>("nvnPolygonStateGetCullFace");
    auto set_front = abi::proc<PFNNVNPOLYGONSTATESETFRONTFACEPROC>("nvnPolygonStateSetFrontFace");
    auto get_front = abi::proc<PFNNVNPOLYGONSTATEGETFRONTFACEPROC>("nvnPolygonStateGetFrontFace");
    auto set_mode = abi::proc<PFNNVNPOLYGONSTATESETPOLYGONMODEPROC>("nvnPolygonStateSetPolygonMode");
    auto get_mode = abi::proc<PFNNVNPOLYGONSTATEGETPOLYGONMODEPROC>("nvnPolygonStateGetPolygonMode");
    auto set_off = abi::proc<PFNNVNPOLYGONSTATESETPOLYGONOFFSETENABLESPROC>("nvnPolygonStateSetPolygonOffsetEnables");
    auto get_off = abi::proc<PFNNVNPOLYGONSTATEGETPOLYGONOFFSETENABLESPROC>("nvnPolygonStateGetPolygonOffsetEnables");

    NVNpolygonState p;
    std::memset(&p, 0xEE, sizeof p);
    p_defaults(&p);
    CHECK_EQ(static_cast<int>(get_cull(&p)), static_cast<int>(NVN_FACE_NONE));
    CHECK_EQ(static_cast<int>(get_front(&p)), static_cast<int>(NVN_FRONT_FACE_CCW));
    CHECK_EQ(static_cast<int>(get_mode(&p)), static_cast<int>(NVN_POLYGON_MODE_FILL));
    CHECK_EQ(static_cast<int>(get_off(&p)), 0);

    for (int v = NVN_FACE_NONE; v <= NVN_FACE_FRONT_AND_BACK; ++v) {
        set_cull(&p, static_cast<NVNface>(v));
        CHECK_EQ(static_cast<int>(get_cull(&p)), v);
    }
    for (int v = NVN_FRONT_FACE_CW; v <= NVN_FRONT_FACE_CCW; ++v) {
        set_front(&p, static_cast<NVNfrontFace>(v));
        CHECK_EQ(static_cast<int>(get_front(&p)), v);
    }
    for (int v = NVN_POLYGON_MODE_POINT; v <= NVN_POLYGON_MODE_FILL; ++v) {
        set_mode(&p, static_cast<NVNpolygonMode>(v));
        CHECK_EQ(static_cast<int>(get_mode(&p)), v);
    }
    for (int v : {0, 1, 2, 4, 7}) {
        set_off(&p, v);
        CHECK_EQ(static_cast<int>(get_off(&p)), v);
    }

    // one field at a time leaves the others untouched
    set_cull(&p, NVN_FACE_BACK);
    set_mode(&p, NVN_POLYGON_MODE_LINE);
    set_off(&p, 6);
    set_front(&p, NVN_FRONT_FACE_CW);
    PolygonInfo d = decode(&p);
    CHECK_EQ(d.cull_face, static_cast<int>(NVN_FACE_BACK));
    CHECK_EQ(d.front_face, static_cast<int>(NVN_FRONT_FACE_CW));
    CHECK_EQ(d.polygon_mode, static_cast<int>(NVN_POLYGON_MODE_LINE));
    CHECK_EQ(d.offset_enables, 6);
    CHECK_EQ(static_cast<int>(get_cull(&p)), static_cast<int>(NVN_FACE_BACK));

    NVNpolygonState z;
    std::memset(&z, 0, sizeof z);
    PolygonInfo zd = decode(&z);
    CHECK_EQ(zd.cull_face, 0);
    CHECK_EQ(zd.front_face, 0);
    CHECK_EQ(zd.polygon_mode, 0);
    CHECK_EQ(zd.offset_enables, 0);

    NVNpolygonState copy;
    std::memcpy(&copy, &p, sizeof p);
    set_cull(&p, NVN_FACE_NONE);
    CHECK_EQ(static_cast<int>(get_cull(&copy)), static_cast<int>(NVN_FACE_BACK));
}

NXTEST(multisample_defaults_round_trip_and_grid) {
    setup();
    auto m_defaults = abi::proc<PFNNVNMULTISAMPLESTATESETDEFAULTSPROC>("nvnMultisampleStateSetDefaults");
    auto set_en = abi::proc<PFNNVNMULTISAMPLESTATESETMULTISAMPLEENABLEPROC>("nvnMultisampleStateSetMultisampleEnable");
    auto get_en = abi::proc<PFNNVNMULTISAMPLESTATEGETMULTISAMPLEENABLEPROC>("nvnMultisampleStateGetMultisampleEnable");
    auto set_samples = abi::proc<PFNNVNMULTISAMPLESTATESETSAMPLESPROC>("nvnMultisampleStateSetSamples");
    auto get_samples = abi::proc<PFNNVNMULTISAMPLESTATEGETSAMPLESPROC>("nvnMultisampleStateGetSamples");
    auto set_a2c = abi::proc<PFNNVNMULTISAMPLESTATESETALPHATOCOVERAGEENABLEPROC>("nvnMultisampleStateSetAlphaToCoverageEnable");
    auto get_a2c = abi::proc<PFNNVNMULTISAMPLESTATEGETALPHATOCOVERAGEENABLEPROC>("nvnMultisampleStateGetAlphaToCoverageEnable");
    auto set_dither = abi::proc<PFNNVNMULTISAMPLESTATESETALPHATOCOVERAGEDITHERPROC>("nvnMultisampleStateSetAlphaToCoverageDither");
    auto get_dither = abi::proc<PFNNVNMULTISAMPLESTATEGETALPHATOCOVERAGEDITHERPROC>("nvnMultisampleStateGetAlphaToCoverageDither");
    auto set_raster = abi::proc<PFNNVNMULTISAMPLESTATESETRASTERSAMPLESPROC>("nvnMultisampleStateSetRasterSamples");
    auto get_raster = abi::proc<PFNNVNMULTISAMPLESTATEGETRASTERSAMPLESPROC>("nvnMultisampleStateGetRasterSamples");
    auto set_covmod = abi::proc<PFNNVNMULTISAMPLESTATESETCOVERAGEMODULATIONMODEPROC>("nvnMultisampleStateSetCoverageModulationMode");
    auto get_covmod = abi::proc<PFNNVNMULTISAMPLESTATEGETCOVERAGEMODULATIONMODEPROC>("nvnMultisampleStateGetCoverageModulationMode");
    auto set_c2c = abi::proc<PFNNVNMULTISAMPLESTATESETCOVERAGETOCOLORENABLEPROC>("nvnMultisampleStateSetCoverageToColorEnable");
    auto get_c2c = abi::proc<PFNNVNMULTISAMPLESTATEGETCOVERAGETOCOLORENABLEPROC>("nvnMultisampleStateGetCoverageToColorEnable");
    auto set_c2cout = abi::proc<PFNNVNMULTISAMPLESTATESETCOVERAGETOCOLOROUTPUTPROC>("nvnMultisampleStateSetCoverageToColorOutput");
    auto get_c2cout = abi::proc<PFNNVNMULTISAMPLESTATEGETCOVERAGETOCOLOROUTPUTPROC>("nvnMultisampleStateGetCoverageToColorOutput");
    auto set_loc_en = abi::proc<PFNNVNMULTISAMPLESTATESETSAMPLELOCATIONSENABLEPROC>("nvnMultisampleStateSetSampleLocationsEnable");
    auto get_loc_en = abi::proc<PFNNVNMULTISAMPLESTATEGETSAMPLELOCATIONSENABLEPROC>("nvnMultisampleStateGetSampleLocationsEnable");
    auto get_grid = abi::proc<PFNNVNMULTISAMPLESTATEGETSAMPLELOCATIONSGRIDPROC>("nvnMultisampleStateGetSampleLocationsGrid");
    auto set_grid_en = abi::proc<PFNNVNMULTISAMPLESTATESETSAMPLELOCATIONSGRIDENABLEPROC>("nvnMultisampleStateSetSampleLocationsGridEnable");
    auto get_grid_en = abi::proc<PFNNVNMULTISAMPLESTATEGETSAMPLELOCATIONSGRIDENABLEPROC>("nvnMultisampleStateGetSampleLocationsGridEnable");

    NVNmultisampleState m;
    std::memset(&m, 0xEE, sizeof m);
    m_defaults(&m);
    CHECK_EQ(static_cast<int>(get_en(&m)), 1);
    CHECK_EQ(static_cast<int>(get_a2c(&m)), 0);
    CHECK_EQ(static_cast<int>(get_dither(&m)), 1);
    CHECK_EQ(get_samples(&m), 0);
    CHECK_EQ(get_raster(&m), 0);
    CHECK_EQ(static_cast<int>(get_covmod(&m)), 0);
    CHECK_EQ(static_cast<int>(get_c2c(&m)), 0);
    CHECK_EQ(get_c2cout(&m), 0);
    CHECK_EQ(static_cast<int>(get_loc_en(&m)), 0);
    CHECK_EQ(static_cast<int>(get_grid_en(&m)), 0);
    int gw = 0, gh = 0;
    get_grid(&m, &gw, &gh);
    CHECK_EQ(gw, 1);
    CHECK_EQ(gh, 1);

    set_en(&m, 0);
    CHECK_EQ(static_cast<int>(get_en(&m)), 0);
    set_en(&m, 1);
    CHECK_EQ(static_cast<int>(get_en(&m)), 1);
    set_a2c(&m, 1);
    CHECK_EQ(static_cast<int>(get_a2c(&m)), 1);
    set_dither(&m, 0);
    CHECK_EQ(static_cast<int>(get_dither(&m)), 0);
    for (int v : {1, 2, 4, 8, 16, 0, 255}) {
        set_samples(&m, v);
        CHECK_EQ(get_samples(&m), v);
    }
    for (int v : {1, 2, 4, 8, 16}) {
        set_raster(&m, v);
        CHECK_EQ(get_raster(&m), v);
    }
    for (int v = NVN_COVERAGE_MODULATION_MODE_NONE; v <= NVN_COVERAGE_MODULATION_MODE_RGBA; ++v) {
        set_covmod(&m, static_cast<NVNcoverageModulationMode>(v));
        CHECK_EQ(static_cast<int>(get_covmod(&m)), v);
    }
    for (int v = 0; v <= 7; ++v) {
        set_c2cout(&m, v);
        CHECK_EQ(get_c2cout(&m), v);
    }
    set_c2c(&m, 1);
    CHECK_EQ(static_cast<int>(get_c2c(&m)), 1);
    set_loc_en(&m, 1);
    CHECK_EQ(static_cast<int>(get_loc_en(&m)), 1);
    set_grid_en(&m, 1);
    CHECK_EQ(static_cast<int>(get_grid_en(&m)), 1);
    set_grid_en(&m, 0);
    CHECK_EQ(static_cast<int>(get_grid_en(&m)), 0);

    // setting one field leaves the others untouched (the flags byte packs several)
    set_c2cout(&m, 5);
    set_covmod(&m, NVN_COVERAGE_MODULATION_MODE_RGBA);
    CHECK_EQ(get_c2cout(&m), 5);
    CHECK_EQ(static_cast<int>(get_c2c(&m)), 1);
    CHECK_EQ(static_cast<int>(get_loc_en(&m)), 1);
    set_c2c(&m, 0);
    CHECK_EQ(get_c2cout(&m), 5);
    CHECK_EQ(static_cast<int>(get_covmod(&m)), static_cast<int>(NVN_COVERAGE_MODULATION_MODE_RGBA));
    CHECK_EQ(static_cast<int>(get_loc_en(&m)), 1);

    // grid query with a null output is tolerated
    get_grid(&m, nullptr, &gh);
    CHECK_EQ(gh, 1);

    // decode agrees with the getters
    MultisampleInfo d = decode(&m);
    CHECK_EQ(d.enable, true);
    CHECK_EQ(d.alpha_to_coverage, true);
    CHECK_EQ(d.alpha_to_coverage_dither, false);
    CHECK_EQ(d.samples, 255);
    CHECK_EQ(d.raster_samples, 16);
    CHECK_EQ(d.coverage_modulation_mode, static_cast<int>(NVN_COVERAGE_MODULATION_MODE_RGBA));
    CHECK_EQ(d.coverage_to_color, false);
    CHECK_EQ(d.coverage_to_color_output, 5);
    CHECK_EQ(d.sample_locations, true);
    CHECK_EQ(d.sample_locations_grid, false);

    NVNmultisampleState z;
    std::memset(&z, 0, sizeof z);
    MultisampleInfo zd = decode(&z);
    CHECK_EQ(zd.enable, false);
    CHECK_EQ(zd.samples, 0);
    CHECK_EQ(zd.raster_samples, 0);
    CHECK_EQ(zd.coverage_to_color_output, 0);
    CHECK_EQ(zd.sample_locations, false);
    CHECK_EQ(zd.sample_locations_grid, false);
}

NXTEST(multisample_sample_locations_layout) {
    setup();
    auto m_defaults = abi::proc<PFNNVNMULTISAMPLESTATESETDEFAULTSPROC>("nvnMultisampleStateSetDefaults");
    auto set_loc = abi::proc<PFNNVNMULTISAMPLESTATESETSAMPLELOCATIONSPROC>("nvnMultisampleStateSetSampleLocations");

    NVNmultisampleState m;
    std::memset(&m, 0, sizeof m);
    m_defaults(&m);
    // defaults: every sample at the pixel centre
    for (int i = 0; i < 16; ++i) CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(m.reserved[8 + i])), 0x88u);

    // (0.25, 0.75) -> x 4, y 12; (0.5, 0) -> 8, 0; (1.5, -0.2) -> clamped to 15, 0
    const float values[6] = {0.25f, 0.75f, 0.5f, 0.0f, 1.5f, -0.2f};
    set_loc(&m, 0, 3, values);
    CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(m.reserved[8])), 0xC4u);
    CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(m.reserved[9])), 0x08u);
    CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(m.reserved[10])), 0x0Fu);
    CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(m.reserved[11])), 0x88u);

    // a range that runs past sample 15 writes only up to sample 15
    const float ten[10] = {0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.5f, 0.5f};
    set_loc(&m, 14, 5, ten);
    CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(m.reserved[22])), 0x00u);
    CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(m.reserved[23])), 0x00u);
    CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(m.reserved[21])), 0x88u);
    // a start at or past 16, or a zero count, changes nothing
    set_loc(&m, 16, 4, ten);
    set_loc(&m, 3, 0, ten);
    CHECK_EQ(static_cast<unsigned>(static_cast<unsigned char>(m.reserved[11])), 0x88u);
}

NXTEST(multisample_decode_zero_and_copies) {
    setup();
    auto m_defaults = abi::proc<PFNNVNMULTISAMPLESTATESETDEFAULTSPROC>("nvnMultisampleStateSetDefaults");
    auto set_samples = abi::proc<PFNNVNMULTISAMPLESTATESETSAMPLESPROC>("nvnMultisampleStateSetSamples");
    auto get_samples = abi::proc<PFNNVNMULTISAMPLESTATEGETSAMPLESPROC>("nvnMultisampleStateGetSamples");
    NVNmultisampleState a, b;
    m_defaults(&a);
    set_samples(&a, 4);
    std::memcpy(&b, &a, sizeof a);   // the game copies state objects around freely
    set_samples(&a, 8);
    CHECK_EQ(get_samples(&b), 4);
    CHECK_EQ(get_samples(&a), 8);
    CHECK_EQ(decode(&b).samples, 4);
}
