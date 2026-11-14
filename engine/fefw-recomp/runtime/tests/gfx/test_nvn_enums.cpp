// SPDX-License-Identifier: GPL-3.0-or-later
// NVN enum -> Vulkan converters (nvn_enums.cpp): every valid NVN enumerator maps to the Vulkan value with the same meaning,
// invalid values give the documented default and bump unknown_enum_count(), and the counter resets and is thread safe.
#include "nvn_enums.h"
#include "test_framework.h"

#include <cstdint>
#include <thread>
#include <vector>

#include <nvn/nvn.h>

NXTEST_GROUP(gfx_nvn_enums)

using namespace nxrt::nvn;

namespace {

struct Row {
    int nvn;
    int vk;
};

struct PrimRow {
    int nvn;
    PrimConversion conv;
};

}  // namespace

NXTEST(blend_factor_every_enumerator) {
    const Row rows[] = {
        {NVN_BLEND_FUNC_ZERO, VK_BLEND_FACTOR_ZERO},
        {NVN_BLEND_FUNC_ONE, VK_BLEND_FACTOR_ONE},
        {NVN_BLEND_FUNC_SRC_COLOR, VK_BLEND_FACTOR_SRC_COLOR},
        {NVN_BLEND_FUNC_ONE_MINUS_SRC_COLOR, VK_BLEND_FACTOR_ONE_MINUS_SRC_COLOR},
        {NVN_BLEND_FUNC_SRC_ALPHA, VK_BLEND_FACTOR_SRC_ALPHA},
        {NVN_BLEND_FUNC_ONE_MINUS_SRC_ALPHA, VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA},
        {NVN_BLEND_FUNC_DST_ALPHA, VK_BLEND_FACTOR_DST_ALPHA},
        {NVN_BLEND_FUNC_ONE_MINUS_DST_ALPHA, VK_BLEND_FACTOR_ONE_MINUS_DST_ALPHA},
        {NVN_BLEND_FUNC_DST_COLOR, VK_BLEND_FACTOR_DST_COLOR},
        {NVN_BLEND_FUNC_ONE_MINUS_DST_COLOR, VK_BLEND_FACTOR_ONE_MINUS_DST_COLOR},
        {NVN_BLEND_FUNC_SRC_ALPHA_SATURATE, VK_BLEND_FACTOR_SRC_ALPHA_SATURATE},
        {NVN_BLEND_FUNC_SRC1_COLOR, VK_BLEND_FACTOR_SRC1_COLOR},
        {NVN_BLEND_FUNC_ONE_MINUS_SRC1_COLOR, VK_BLEND_FACTOR_ONE_MINUS_SRC1_COLOR},
        {NVN_BLEND_FUNC_SRC1_ALPHA, VK_BLEND_FACTOR_SRC1_ALPHA},
        {NVN_BLEND_FUNC_ONE_MINUS_SRC1_ALPHA, VK_BLEND_FACTOR_ONE_MINUS_SRC1_ALPHA},
        {NVN_BLEND_FUNC_CONSTANT_COLOR, VK_BLEND_FACTOR_CONSTANT_COLOR},
        {NVN_BLEND_FUNC_ONE_MINUS_CONSTANT_COLOR, VK_BLEND_FACTOR_ONE_MINUS_CONSTANT_COLOR},
        {NVN_BLEND_FUNC_CONSTANT_ALPHA, VK_BLEND_FACTOR_CONSTANT_ALPHA},
        {NVN_BLEND_FUNC_ONE_MINUS_CONSTANT_ALPHA, VK_BLEND_FACTOR_ONE_MINUS_CONSTANT_ALPHA},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_blend_factor(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(blend_op_every_enumerator) {
    const Row rows[] = {
        {NVN_BLEND_EQUATION_ADD, VK_BLEND_OP_ADD},
        {NVN_BLEND_EQUATION_SUB, VK_BLEND_OP_SUBTRACT},
        {NVN_BLEND_EQUATION_REVERSE_SUB, VK_BLEND_OP_REVERSE_SUBTRACT},
        {NVN_BLEND_EQUATION_MIN, VK_BLEND_OP_MIN},
        {NVN_BLEND_EQUATION_MAX, VK_BLEND_OP_MAX},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_blend_op(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

// The depth, stencil, compare and alpha families share the values 1..8, so each row checks all four families.
NXTEST(compare_op_every_enumerator) {
    struct CmpRow {
        int depth, stencil, compare, alpha;
        int vk;
    };
    const CmpRow rows[] = {
        {NVN_DEPTH_FUNC_NEVER, NVN_STENCIL_FUNC_NEVER, NVN_COMPARE_FUNC_NEVER, NVN_ALPHA_FUNC_NEVER, VK_COMPARE_OP_NEVER},
        {NVN_DEPTH_FUNC_LESS, NVN_STENCIL_FUNC_LESS, NVN_COMPARE_FUNC_LESS, NVN_ALPHA_FUNC_LESS, VK_COMPARE_OP_LESS},
        {NVN_DEPTH_FUNC_EQUAL, NVN_STENCIL_FUNC_EQUAL, NVN_COMPARE_FUNC_EQUAL, NVN_ALPHA_FUNC_EQUAL, VK_COMPARE_OP_EQUAL},
        {NVN_DEPTH_FUNC_LEQUAL, NVN_STENCIL_FUNC_LEQUAL, NVN_COMPARE_FUNC_LEQUAL, NVN_ALPHA_FUNC_LEQUAL,
         VK_COMPARE_OP_LESS_OR_EQUAL},
        {NVN_DEPTH_FUNC_GREATER, NVN_STENCIL_FUNC_GREATER, NVN_COMPARE_FUNC_GREATER, NVN_ALPHA_FUNC_GREATER,
         VK_COMPARE_OP_GREATER},
        {NVN_DEPTH_FUNC_NOTEQUAL, NVN_STENCIL_FUNC_NOTEQUAL, NVN_COMPARE_FUNC_NOTEQUAL, NVN_ALPHA_FUNC_NOTEQUAL,
         VK_COMPARE_OP_NOT_EQUAL},
        {NVN_DEPTH_FUNC_GEQUAL, NVN_STENCIL_FUNC_GEQUAL, NVN_COMPARE_FUNC_GEQUAL, NVN_ALPHA_FUNC_GEQUAL,
         VK_COMPARE_OP_GREATER_OR_EQUAL},
        {NVN_DEPTH_FUNC_ALWAYS, NVN_STENCIL_FUNC_ALWAYS, NVN_COMPARE_FUNC_ALWAYS, NVN_ALPHA_FUNC_ALWAYS, VK_COMPARE_OP_ALWAYS},
    };
    const uint64_t before = unknown_enum_count();
    for (const CmpRow& r : rows) {
        CHECK_EQ(static_cast<int>(vk_compare_op(r.depth)), r.vk);
        CHECK_EQ(static_cast<int>(vk_compare_op(r.stencil)), r.vk);
        CHECK_EQ(static_cast<int>(vk_compare_op(r.compare)), r.vk);
        CHECK_EQ(static_cast<int>(vk_compare_op(r.alpha)), r.vk);
    }
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(stencil_op_every_enumerator) {
    const Row rows[] = {
        {NVN_STENCIL_OP_KEEP, VK_STENCIL_OP_KEEP},
        {NVN_STENCIL_OP_ZERO, VK_STENCIL_OP_ZERO},
        {NVN_STENCIL_OP_REPLACE, VK_STENCIL_OP_REPLACE},
        {NVN_STENCIL_OP_INCR, VK_STENCIL_OP_INCREMENT_AND_CLAMP},
        {NVN_STENCIL_OP_DECR, VK_STENCIL_OP_DECREMENT_AND_CLAMP},
        {NVN_STENCIL_OP_INVERT, VK_STENCIL_OP_INVERT},
        {NVN_STENCIL_OP_INCR_WRAP, VK_STENCIL_OP_INCREMENT_AND_WRAP},
        {NVN_STENCIL_OP_DECR_WRAP, VK_STENCIL_OP_DECREMENT_AND_WRAP},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_stencil_op(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(cull_mode_every_enumerator) {
    const Row rows[] = {
        {NVN_FACE_NONE, VK_CULL_MODE_NONE},
        {NVN_FACE_FRONT, VK_CULL_MODE_FRONT_BIT},
        {NVN_FACE_BACK, VK_CULL_MODE_BACK_BIT},
        {NVN_FACE_FRONT_AND_BACK, VK_CULL_MODE_FRONT_AND_BACK},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_cull_mode(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(front_face_every_enumerator) {
    const Row rows[] = {
        {NVN_FRONT_FACE_CW, VK_FRONT_FACE_CLOCKWISE},
        {NVN_FRONT_FACE_CCW, VK_FRONT_FACE_COUNTER_CLOCKWISE},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_front_face(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(polygon_mode_every_enumerator) {
    const Row rows[] = {
        {NVN_POLYGON_MODE_POINT, VK_POLYGON_MODE_POINT},
        {NVN_POLYGON_MODE_LINE, VK_POLYGON_MODE_LINE},
        {NVN_POLYGON_MODE_FILL, VK_POLYGON_MODE_FILL},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_polygon_mode(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(logic_op_every_enumerator) {
    const Row rows[] = {
        {NVN_LOGIC_OP_CLEAR, VK_LOGIC_OP_CLEAR},
        {NVN_LOGIC_OP_AND, VK_LOGIC_OP_AND},
        {NVN_LOGIC_OP_AND_REVERSE, VK_LOGIC_OP_AND_REVERSE},
        {NVN_LOGIC_OP_COPY, VK_LOGIC_OP_COPY},
        {NVN_LOGIC_OP_AND_INVERTED, VK_LOGIC_OP_AND_INVERTED},
        {NVN_LOGIC_OP_NOOP, VK_LOGIC_OP_NO_OP},
        {NVN_LOGIC_OP_XOR, VK_LOGIC_OP_XOR},
        {NVN_LOGIC_OP_OR, VK_LOGIC_OP_OR},
        {NVN_LOGIC_OP_NOR, VK_LOGIC_OP_NOR},
        {NVN_LOGIC_OP_EQUIV, VK_LOGIC_OP_EQUIVALENT},
        {NVN_LOGIC_OP_INVERT, VK_LOGIC_OP_INVERT},
        {NVN_LOGIC_OP_OR_REVERSE, VK_LOGIC_OP_OR_REVERSE},
        {NVN_LOGIC_OP_COPY_INVERTED, VK_LOGIC_OP_COPY_INVERTED},
        {NVN_LOGIC_OP_OR_INVERTED, VK_LOGIC_OP_OR_INVERTED},
        {NVN_LOGIC_OP_NAND, VK_LOGIC_OP_NAND},
        {NVN_LOGIC_OP_SET, VK_LOGIC_OP_SET},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_logic_op(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

// Not an NVN enum: the sample count is a plain integer. 0 and 1 both mean one sample and are not counted as unknown.
NXTEST(sample_count_values) {
    const Row rows[] = {
        {0, VK_SAMPLE_COUNT_1_BIT},
        {1, VK_SAMPLE_COUNT_1_BIT},
        {2, VK_SAMPLE_COUNT_2_BIT},
        {4, VK_SAMPLE_COUNT_4_BIT},
        {8, VK_SAMPLE_COUNT_8_BIT},
        {16, VK_SAMPLE_COUNT_16_BIT},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_sample_count(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(index_type_and_size_every_enumerator) {
    const Row rows[] = {
        {NVN_INDEX_TYPE_UNSIGNED_BYTE, VK_INDEX_TYPE_UINT8},
        {NVN_INDEX_TYPE_UNSIGNED_SHORT, VK_INDEX_TYPE_UINT16},
        {NVN_INDEX_TYPE_UNSIGNED_INT, VK_INDEX_TYPE_UINT32},
    };
    const uint32_t sizes[] = {1, 2, 4};
    const uint64_t before = unknown_enum_count();
    for (int i = 0; i < 3; ++i) {
        CHECK_EQ(static_cast<int>(vk_index_type(rows[i].nvn)), rows[i].vk);
        CHECK_EQ(index_size_bytes(rows[i].nvn), sizes[i]);
    }
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(topology_every_enumerator) {
    const Row rows[] = {
        {NVN_DRAW_PRIMITIVE_POINTS, VK_PRIMITIVE_TOPOLOGY_POINT_LIST},
        {NVN_DRAW_PRIMITIVE_LINES, VK_PRIMITIVE_TOPOLOGY_LINE_LIST},
        {NVN_DRAW_PRIMITIVE_LINE_LOOP, VK_PRIMITIVE_TOPOLOGY_LINE_STRIP},
        {NVN_DRAW_PRIMITIVE_LINE_STRIP, VK_PRIMITIVE_TOPOLOGY_LINE_STRIP},
        {NVN_DRAW_PRIMITIVE_TRIANGLES, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST},
        {NVN_DRAW_PRIMITIVE_TRIANGLE_STRIP, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP},
        {NVN_DRAW_PRIMITIVE_TRIANGLE_FAN, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_FAN},
        {NVN_DRAW_PRIMITIVE_QUADS, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST},
        {NVN_DRAW_PRIMITIVE_QUAD_STRIP, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST},
        {NVN_DRAW_PRIMITIVE_POLYGON, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST},
        {NVN_DRAW_PRIMITIVE_LINES_ADJACENCY, VK_PRIMITIVE_TOPOLOGY_LINE_LIST_WITH_ADJACENCY},
        {NVN_DRAW_PRIMITIVE_LINE_STRIP_ADJACENCY, VK_PRIMITIVE_TOPOLOGY_LINE_STRIP_WITH_ADJACENCY},
        {NVN_DRAW_PRIMITIVE_TRIANGLES_ADJACENCY, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST_WITH_ADJACENCY},
        {NVN_DRAW_PRIMITIVE_TRIANGLE_STRIP_ADJACENCY, VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP_WITH_ADJACENCY},
        {NVN_DRAW_PRIMITIVE_PATCHES, VK_PRIMITIVE_TOPOLOGY_PATCH_LIST},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_topology(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(prim_conversion_every_enumerator) {
    const PrimRow rows[] = {
        {NVN_DRAW_PRIMITIVE_POINTS, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_LINES, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_LINE_LOOP, PrimConversion::LineLoop},
        {NVN_DRAW_PRIMITIVE_LINE_STRIP, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_TRIANGLES, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_TRIANGLE_STRIP, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_TRIANGLE_FAN, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_QUADS, PrimConversion::Quads},
        {NVN_DRAW_PRIMITIVE_QUAD_STRIP, PrimConversion::QuadStrip},
        {NVN_DRAW_PRIMITIVE_POLYGON, PrimConversion::Polygon},
        {NVN_DRAW_PRIMITIVE_LINES_ADJACENCY, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_LINE_STRIP_ADJACENCY, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_TRIANGLES_ADJACENCY, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_TRIANGLE_STRIP_ADJACENCY, PrimConversion::None},
        {NVN_DRAW_PRIMITIVE_PATCHES, PrimConversion::None},
    };
    const uint64_t before = unknown_enum_count();
    for (const PrimRow& r : rows) CHECK_EQ(static_cast<int>(prim_conversion(r.nvn)), static_cast<int>(r.conv));
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(mag_filter_every_enumerator) {
    const Row rows[] = {
        {NVN_MAG_FILTER_NEAREST, VK_FILTER_NEAREST},
        {NVN_MAG_FILTER_LINEAR, VK_FILTER_LINEAR},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_mag_filter(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(min_filter_every_enumerator) {
    const Row rows[] = {
        {NVN_MIN_FILTER_NEAREST, VK_FILTER_NEAREST},
        {NVN_MIN_FILTER_LINEAR, VK_FILTER_LINEAR},
        {NVN_MIN_FILTER_NEAREST_MIPMAP_NEAREST, VK_FILTER_NEAREST},
        {NVN_MIN_FILTER_LINEAR_MIPMAP_NEAREST, VK_FILTER_LINEAR},
        {NVN_MIN_FILTER_NEAREST_MIPMAP_LINEAR, VK_FILTER_NEAREST},
        {NVN_MIN_FILTER_LINEAR_MIPMAP_LINEAR, VK_FILTER_LINEAR},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_min_filter(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

// The second word of each *_MIPMAP_* value is the mip filter: NEAREST_MIPMAP_LINEAR is nearest texels, linear between mips.
NXTEST(mipmap_mode_every_enumerator) {
    const Row rows[] = {
        {NVN_MIN_FILTER_NEAREST, VK_SAMPLER_MIPMAP_MODE_NEAREST},
        {NVN_MIN_FILTER_LINEAR, VK_SAMPLER_MIPMAP_MODE_NEAREST},
        {NVN_MIN_FILTER_NEAREST_MIPMAP_NEAREST, VK_SAMPLER_MIPMAP_MODE_NEAREST},
        {NVN_MIN_FILTER_LINEAR_MIPMAP_NEAREST, VK_SAMPLER_MIPMAP_MODE_NEAREST},
        {NVN_MIN_FILTER_NEAREST_MIPMAP_LINEAR, VK_SAMPLER_MIPMAP_MODE_LINEAR},
        {NVN_MIN_FILTER_LINEAR_MIPMAP_LINEAR, VK_SAMPLER_MIPMAP_MODE_LINEAR},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_mipmap_mode(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(min_filter_uses_mips_every_enumerator) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(min_filter_uses_mips(NVN_MIN_FILTER_NEAREST) ? 1 : 0, 0);
    CHECK_EQ(min_filter_uses_mips(NVN_MIN_FILTER_LINEAR) ? 1 : 0, 0);
    CHECK_EQ(min_filter_uses_mips(NVN_MIN_FILTER_NEAREST_MIPMAP_NEAREST) ? 1 : 0, 1);
    CHECK_EQ(min_filter_uses_mips(NVN_MIN_FILTER_LINEAR_MIPMAP_NEAREST) ? 1 : 0, 1);
    CHECK_EQ(min_filter_uses_mips(NVN_MIN_FILTER_NEAREST_MIPMAP_LINEAR) ? 1 : 0, 1);
    CHECK_EQ(min_filter_uses_mips(NVN_MIN_FILTER_LINEAR_MIPMAP_LINEAR) ? 1 : 0, 1);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(address_mode_every_enumerator) {
    const Row rows[] = {
        {NVN_WRAP_MODE_CLAMP, VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE},
        {NVN_WRAP_MODE_REPEAT, VK_SAMPLER_ADDRESS_MODE_REPEAT},
        {NVN_WRAP_MODE_MIRROR_CLAMP, VK_SAMPLER_ADDRESS_MODE_MIRROR_CLAMP_TO_EDGE},
        {NVN_WRAP_MODE_MIRROR_CLAMP_TO_EDGE, VK_SAMPLER_ADDRESS_MODE_MIRROR_CLAMP_TO_EDGE},
        // Vulkan has no mirror-clamp-to-border mode; the nearest equivalent is used.
        {NVN_WRAP_MODE_MIRROR_CLAMP_TO_BORDER, VK_SAMPLER_ADDRESS_MODE_MIRROR_CLAMP_TO_EDGE},
        {NVN_WRAP_MODE_CLAMP_TO_BORDER, VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_BORDER},
        {NVN_WRAP_MODE_MIRRORED_REPEAT, VK_SAMPLER_ADDRESS_MODE_MIRRORED_REPEAT},
        {NVN_WRAP_MODE_CLAMP_TO_EDGE, VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_address_mode(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(reduction_mode_every_enumerator) {
    const Row rows[] = {
        {NVN_SAMPLER_REDUCTION_AVERAGE, VK_SAMPLER_REDUCTION_MODE_WEIGHTED_AVERAGE},
        {NVN_SAMPLER_REDUCTION_MIN, VK_SAMPLER_REDUCTION_MODE_MIN},
        {NVN_SAMPLER_REDUCTION_MAX, VK_SAMPLER_REDUCTION_MODE_MAX},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_reduction_mode(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(swizzle_every_enumerator) {
    const Row rows[] = {
        {NVN_TEXTURE_SWIZZLE_ZERO, VK_COMPONENT_SWIZZLE_ZERO},
        {NVN_TEXTURE_SWIZZLE_ONE, VK_COMPONENT_SWIZZLE_ONE},
        {NVN_TEXTURE_SWIZZLE_R, VK_COMPONENT_SWIZZLE_R},
        {NVN_TEXTURE_SWIZZLE_G, VK_COMPONENT_SWIZZLE_G},
        {NVN_TEXTURE_SWIZZLE_B, VK_COMPONENT_SWIZZLE_B},
        {NVN_TEXTURE_SWIZZLE_A, VK_COMPONENT_SWIZZLE_A},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_swizzle(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(view_type_every_enumerator) {
    const Row rows[] = {
        {NVN_TEXTURE_TARGET_1D, VK_IMAGE_VIEW_TYPE_1D},
        {NVN_TEXTURE_TARGET_2D, VK_IMAGE_VIEW_TYPE_2D},
        {NVN_TEXTURE_TARGET_3D, VK_IMAGE_VIEW_TYPE_3D},
        {NVN_TEXTURE_TARGET_1D_ARRAY, VK_IMAGE_VIEW_TYPE_1D_ARRAY},
        {NVN_TEXTURE_TARGET_2D_ARRAY, VK_IMAGE_VIEW_TYPE_2D_ARRAY},
        {NVN_TEXTURE_TARGET_2D_MULTISAMPLE, VK_IMAGE_VIEW_TYPE_2D},
        {NVN_TEXTURE_TARGET_2D_MULTISAMPLE_ARRAY, VK_IMAGE_VIEW_TYPE_2D_ARRAY},
        {NVN_TEXTURE_TARGET_RECTANGLE, VK_IMAGE_VIEW_TYPE_2D},
        {NVN_TEXTURE_TARGET_CUBEMAP, VK_IMAGE_VIEW_TYPE_CUBE},
        {NVN_TEXTURE_TARGET_CUBEMAP_ARRAY, VK_IMAGE_VIEW_TYPE_CUBE_ARRAY},
        {NVN_TEXTURE_TARGET_BUFFER, VK_IMAGE_VIEW_TYPE_1D},
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_view_type(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(image_type_every_enumerator) {
    const Row rows[] = {
        {NVN_TEXTURE_TARGET_1D, VK_IMAGE_TYPE_1D},
        {NVN_TEXTURE_TARGET_2D, VK_IMAGE_TYPE_2D},
        {NVN_TEXTURE_TARGET_3D, VK_IMAGE_TYPE_3D},
        {NVN_TEXTURE_TARGET_1D_ARRAY, VK_IMAGE_TYPE_1D},
        {NVN_TEXTURE_TARGET_2D_ARRAY, VK_IMAGE_TYPE_2D},
        {NVN_TEXTURE_TARGET_2D_MULTISAMPLE, VK_IMAGE_TYPE_2D},
        {NVN_TEXTURE_TARGET_2D_MULTISAMPLE_ARRAY, VK_IMAGE_TYPE_2D},
        {NVN_TEXTURE_TARGET_RECTANGLE, VK_IMAGE_TYPE_2D},
        {NVN_TEXTURE_TARGET_CUBEMAP, VK_IMAGE_TYPE_2D},
        {NVN_TEXTURE_TARGET_CUBEMAP_ARRAY, VK_IMAGE_TYPE_2D},
        {NVN_TEXTURE_TARGET_BUFFER, VK_IMAGE_TYPE_1D},   // as its view type (VK_IMAGE_VIEW_TYPE_1D)
    };
    const uint64_t before = unknown_enum_count();
    for (const Row& r : rows) CHECK_EQ(static_cast<int>(vk_image_type(r.nvn)), r.vk);
    CHECK_EQ(unknown_enum_count(), before);
}

NXTEST(target_flags_every_enumerator) {
    struct FlagRow {
        int nvn;
        bool array, cube, multisample;
    };
    const FlagRow rows[] = {
        {NVN_TEXTURE_TARGET_1D, false, false, false},
        {NVN_TEXTURE_TARGET_2D, false, false, false},
        {NVN_TEXTURE_TARGET_3D, false, false, false},
        {NVN_TEXTURE_TARGET_1D_ARRAY, true, false, false},
        {NVN_TEXTURE_TARGET_2D_ARRAY, true, false, false},
        {NVN_TEXTURE_TARGET_2D_MULTISAMPLE, false, false, true},
        {NVN_TEXTURE_TARGET_2D_MULTISAMPLE_ARRAY, true, false, true},
        {NVN_TEXTURE_TARGET_RECTANGLE, false, false, false},
        {NVN_TEXTURE_TARGET_CUBEMAP, false, true, false},
        {NVN_TEXTURE_TARGET_CUBEMAP_ARRAY, true, true, false},
        {NVN_TEXTURE_TARGET_BUFFER, false, false, false},
    };
    const uint64_t before = unknown_enum_count();
    for (const FlagRow& r : rows) {
        CHECK_EQ(target_is_array(r.nvn) ? 1 : 0, r.array ? 1 : 0);
        CHECK_EQ(target_is_cube(r.nvn) ? 1 : 0, r.cube ? 1 : 0);
        CHECK_EQ(target_is_multisample(r.nvn) ? 1 : 0, r.multisample ? 1 : 0);
    }
    CHECK_EQ(unknown_enum_count(), before);
}

// Invalid values: each function gives its documented default and counts the value. Value 0 is invalid for the blend
// enums, and the LARGE sentinel stands for an out-of-range value in the others.

NXTEST(unknown_blend_factor_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_blend_factor(0)), static_cast<int>(VK_BLEND_FACTOR_ONE));
    CHECK_EQ(static_cast<int>(vk_blend_factor(NVN_BLEND_FUNC_LARGE)), static_cast<int>(VK_BLEND_FACTOR_ONE));
    CHECK_EQ(unknown_enum_count(), before + 2);
}

NXTEST(unknown_blend_op_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_blend_op(0)), static_cast<int>(VK_BLEND_OP_ADD));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_compare_op_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_compare_op(0)), static_cast<int>(VK_COMPARE_OP_ALWAYS));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_stencil_op_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_stencil_op(0)), static_cast<int>(VK_STENCIL_OP_KEEP));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_cull_mode_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_cull_mode(NVN_FACE_LARGE)), static_cast<int>(VK_CULL_MODE_NONE));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_front_face_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_front_face(7)), static_cast<int>(VK_FRONT_FACE_COUNTER_CLOCKWISE));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_polygon_mode_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_polygon_mode(7)), static_cast<int>(VK_POLYGON_MODE_FILL));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_logic_op_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_logic_op(0x10)), static_cast<int>(VK_LOGIC_OP_COPY));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_sample_count_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_sample_count(3)), static_cast<int>(VK_SAMPLE_COUNT_1_BIT));
    CHECK_EQ(static_cast<int>(vk_sample_count(32)), static_cast<int>(VK_SAMPLE_COUNT_1_BIT));
    CHECK_EQ(unknown_enum_count(), before + 2);
}

NXTEST(unknown_index_type_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_index_type(NVN_INDEX_TYPE_LARGE)), static_cast<int>(VK_INDEX_TYPE_UINT16));
    CHECK_EQ(index_size_bytes(NVN_INDEX_TYPE_LARGE), 2u);
    CHECK_EQ(unknown_enum_count(), before + 2);
}

NXTEST(unknown_topology_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_topology(NVN_DRAW_PRIMITIVE_LARGE)), static_cast<int>(VK_PRIMITIVE_TOPOLOGY_TRIANGLE_LIST));
    CHECK_EQ(static_cast<int>(prim_conversion(NVN_DRAW_PRIMITIVE_LARGE)), static_cast<int>(PrimConversion::None));
    CHECK_EQ(unknown_enum_count(), before + 2);
}

NXTEST(unknown_mag_filter_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_mag_filter(2)), static_cast<int>(VK_FILTER_LINEAR));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_min_filter_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_min_filter(6)), static_cast<int>(VK_FILTER_LINEAR));
    CHECK_EQ(static_cast<int>(vk_mipmap_mode(6)), static_cast<int>(VK_SAMPLER_MIPMAP_MODE_NEAREST));
    CHECK_EQ(min_filter_uses_mips(6) ? 1 : 0, 0);
    CHECK_EQ(unknown_enum_count(), before + 3);
}

NXTEST(unknown_address_mode_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_address_mode(8)), static_cast<int>(VK_SAMPLER_ADDRESS_MODE_REPEAT));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_reduction_mode_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_reduction_mode(3)), static_cast<int>(VK_SAMPLER_REDUCTION_MODE_WEIGHTED_AVERAGE));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_swizzle_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_swizzle(6)), static_cast<int>(VK_COMPONENT_SWIZZLE_IDENTITY));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_view_type_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_view_type(NVN_TEXTURE_TARGET_LARGE)), static_cast<int>(VK_IMAGE_VIEW_TYPE_2D));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_image_type_defaults_and_counts) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(static_cast<int>(vk_image_type(11)), static_cast<int>(VK_IMAGE_TYPE_2D));
    CHECK_EQ(unknown_enum_count(), before + 1);
}

NXTEST(unknown_target_flags_default_false_and_count) {
    const uint64_t before = unknown_enum_count();
    CHECK_EQ(target_is_array(11) ? 1 : 0, 0);
    CHECK_EQ(target_is_cube(11) ? 1 : 0, 0);
    CHECK_EQ(target_is_multisample(11) ? 1 : 0, 0);
    CHECK_EQ(unknown_enum_count(), before + 3);
}

NXTEST(reset_unknown_enum_count_clears_counter) {
    (void)vk_cull_mode(99);
    CHECK(unknown_enum_count() >= 1);
    reset_unknown_enum_count();
    CHECK_EQ(unknown_enum_count(), 0u);
    (void)vk_cull_mode(99);
    CHECK_EQ(unknown_enum_count(), 1u);
    reset_unknown_enum_count();
    CHECK_EQ(unknown_enum_count(), 0u);
}

NXTEST(unknown_counter_counts_across_threads) {
    const uint64_t before = unknown_enum_count();
    std::vector<std::thread> threads;
    for (int t = 0; t < 4; ++t) {
        threads.emplace_back([] {
            for (int i = 0; i < 1000; ++i) (void)vk_blend_factor(0);
        });
    }
    for (std::thread& t : threads) t.join();
    CHECK_EQ(unknown_enum_count(), before + 4000);
}
