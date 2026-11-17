// SPDX-License-Identifier: GPL-3.0-or-later
// The key-file recorder of the pipeline prewarm (nvn_prewarm.cpp, rec_of): keys with tessellation or with the alpha-test / scene-gain
// fields after the recorded bytes must be refused. Those fields are not in a Rec, so recording such a key would make the prewarm load it
// as a different pipeline (the plain fragment shader or the other layout) on the next run.
#include "nvn_pipeline.h"
#include "test_framework.h"

NXTEST_GROUP(gfx_nvn_prewarm_record)

using namespace nxrt::nvn::be;
using namespace nxrt::nvn::be::gpu;

namespace {

ShaderInfo shader_with_hash(const char* hash) {
    ShaderInfo s;
    s.hash = hash;
    return s;
}

GraphicsKey key_with_vertex(const ShaderInfo& vs) {
    GraphicsKey k;
    k.stage[0] = &vs;
    return k;
}

}  // namespace

NXTEST(plain_graphics_key_is_recordable) {
    const ShaderInfo vs = shader_with_hash("0123456789abcdef");
    const GraphicsKey key = key_with_vertex(vs);
    CHECK(prewarm_key_recordable(key));

    GraphicsKey no_vertex = key;
    no_vertex.stage[0] = nullptr;
    CHECK(!prewarm_key_recordable(no_vertex));
}

NXTEST(alpha_and_gain_keys_are_refused) {
    const ShaderInfo vs = shader_with_hash("0123456789abcdef");
    GraphicsKey key = key_with_vertex(vs);
    REQUIRE(prewarm_key_recordable(key));       // the plain key is the control

    key.alpha_func = 1;
    CHECK(!prewarm_key_recordable(key));

    key = key_with_vertex(vs);
    key.alpha_layout = 1;
    CHECK(!prewarm_key_recordable(key));

    key = key_with_vertex(vs);
    key.scene_gain = 1;
    CHECK(!prewarm_key_recordable(key));
}

NXTEST(tessellated_keys_are_refused) {
    const ShaderInfo vs = shader_with_hash("0123456789abcdef");
    const ShaderInfo tcs = shader_with_hash("1111111111111111");
    GraphicsKey key = key_with_vertex(vs);
    key.tess[0] = &tcs;
    CHECK(!prewarm_key_recordable(key));
}
