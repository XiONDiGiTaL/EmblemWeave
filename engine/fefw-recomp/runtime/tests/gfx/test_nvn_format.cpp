#include "nvn_format.h"
#include "test_framework.h"

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <set>
#include <string>

NXTEST_GROUP(gfx_nvn_format)

// SPDX-License-Identifier: GPL-3.0-or-later
// Unit tests for nvn_format.cpp. kEnumerators is every NVN_FORMAT_* enumerator of nvn.h (third_party/nvn/nvn.h,
// NVNformat) except NONE and LARGE, in declaration order: 156 entries, generated from the header by a throwaway script.

using namespace nxrt::nvn;

namespace {

struct Enumerator {
    const char* name;
    int value;
};

constexpr Enumerator kEnumerators[] = {
    {"R8", NVN_FORMAT_R8},
    {"R8SN", NVN_FORMAT_R8SN},
    {"R8UI", NVN_FORMAT_R8UI},
    {"R8I", NVN_FORMAT_R8I},
    {"R16F", NVN_FORMAT_R16F},
    {"R16", NVN_FORMAT_R16},
    {"R16SN", NVN_FORMAT_R16SN},
    {"R16UI", NVN_FORMAT_R16UI},
    {"R16I", NVN_FORMAT_R16I},
    {"R32F", NVN_FORMAT_R32F},
    {"R32UI", NVN_FORMAT_R32UI},
    {"R32I", NVN_FORMAT_R32I},
    {"RG8", NVN_FORMAT_RG8},
    {"RG8SN", NVN_FORMAT_RG8SN},
    {"RG8UI", NVN_FORMAT_RG8UI},
    {"RG8I", NVN_FORMAT_RG8I},
    {"RG16F", NVN_FORMAT_RG16F},
    {"RG16", NVN_FORMAT_RG16},
    {"RG16SN", NVN_FORMAT_RG16SN},
    {"RG16UI", NVN_FORMAT_RG16UI},
    {"RG16I", NVN_FORMAT_RG16I},
    {"RG32F", NVN_FORMAT_RG32F},
    {"RG32UI", NVN_FORMAT_RG32UI},
    {"RG32I", NVN_FORMAT_RG32I},
    {"RGB8", NVN_FORMAT_RGB8},
    {"RGB8SN", NVN_FORMAT_RGB8SN},
    {"RGB8UI", NVN_FORMAT_RGB8UI},
    {"RGB8I", NVN_FORMAT_RGB8I},
    {"RGB16F", NVN_FORMAT_RGB16F},
    {"RGB16", NVN_FORMAT_RGB16},
    {"RGB16SN", NVN_FORMAT_RGB16SN},
    {"RGB16UI", NVN_FORMAT_RGB16UI},
    {"RGB16I", NVN_FORMAT_RGB16I},
    {"RGB32F", NVN_FORMAT_RGB32F},
    {"RGB32UI", NVN_FORMAT_RGB32UI},
    {"RGB32I", NVN_FORMAT_RGB32I},
    {"RGBA8", NVN_FORMAT_RGBA8},
    {"RGBA8SN", NVN_FORMAT_RGBA8SN},
    {"RGBA8UI", NVN_FORMAT_RGBA8UI},
    {"RGBA8I", NVN_FORMAT_RGBA8I},
    {"RGBA16F", NVN_FORMAT_RGBA16F},
    {"RGBA16", NVN_FORMAT_RGBA16},
    {"RGBA16SN", NVN_FORMAT_RGBA16SN},
    {"RGBA16UI", NVN_FORMAT_RGBA16UI},
    {"RGBA16I", NVN_FORMAT_RGBA16I},
    {"RGBA32F", NVN_FORMAT_RGBA32F},
    {"RGBA32UI", NVN_FORMAT_RGBA32UI},
    {"RGBA32I", NVN_FORMAT_RGBA32I},
    {"STENCIL8", NVN_FORMAT_STENCIL8},
    {"DEPTH16", NVN_FORMAT_DEPTH16},
    {"DEPTH24", NVN_FORMAT_DEPTH24},
    {"DEPTH32F", NVN_FORMAT_DEPTH32F},
    {"DEPTH24_STENCIL8", NVN_FORMAT_DEPTH24_STENCIL8},
    {"DEPTH32F_STENCIL8", NVN_FORMAT_DEPTH32F_STENCIL8},
    {"RGBX8_SRGB", NVN_FORMAT_RGBX8_SRGB},
    {"RGBA8_SRGB", NVN_FORMAT_RGBA8_SRGB},
    {"RGBA4", NVN_FORMAT_RGBA4},
    {"RGB5", NVN_FORMAT_RGB5},
    {"RGB5A1", NVN_FORMAT_RGB5A1},
    {"RGB565", NVN_FORMAT_RGB565},
    {"RGB10A2", NVN_FORMAT_RGB10A2},
    {"RGB10A2UI", NVN_FORMAT_RGB10A2UI},
    {"R11G11B10F", NVN_FORMAT_R11G11B10F},
    {"RGB9E5F", NVN_FORMAT_RGB9E5F},
    {"RGB_DXT1", NVN_FORMAT_RGB_DXT1},
    {"RGBA_DXT1", NVN_FORMAT_RGBA_DXT1},
    {"RGBA_DXT3", NVN_FORMAT_RGBA_DXT3},
    {"RGBA_DXT5", NVN_FORMAT_RGBA_DXT5},
    {"RGB_DXT1_SRGB", NVN_FORMAT_RGB_DXT1_SRGB},
    {"RGBA_DXT1_SRGB", NVN_FORMAT_RGBA_DXT1_SRGB},
    {"RGBA_DXT3_SRGB", NVN_FORMAT_RGBA_DXT3_SRGB},
    {"RGBA_DXT5_SRGB", NVN_FORMAT_RGBA_DXT5_SRGB},
    {"RGTC1_UNORM", NVN_FORMAT_RGTC1_UNORM},
    {"RGTC1_SNORM", NVN_FORMAT_RGTC1_SNORM},
    {"RGTC2_UNORM", NVN_FORMAT_RGTC2_UNORM},
    {"RGTC2_SNORM", NVN_FORMAT_RGTC2_SNORM},
    {"BPTC_UNORM", NVN_FORMAT_BPTC_UNORM},
    {"BPTC_UNORM_SRGB", NVN_FORMAT_BPTC_UNORM_SRGB},
    {"BPTC_SFLOAT", NVN_FORMAT_BPTC_SFLOAT},
    {"BPTC_UFLOAT", NVN_FORMAT_BPTC_UFLOAT},
    {"R8_UI2F", NVN_FORMAT_R8_UI2F},
    {"R8_I2F", NVN_FORMAT_R8_I2F},
    {"R16_UI2F", NVN_FORMAT_R16_UI2F},
    {"R16_I2F", NVN_FORMAT_R16_I2F},
    {"R32_UI2F", NVN_FORMAT_R32_UI2F},
    {"R32_I2F", NVN_FORMAT_R32_I2F},
    {"RG8_UI2F", NVN_FORMAT_RG8_UI2F},
    {"RG8_I2F", NVN_FORMAT_RG8_I2F},
    {"RG16_UI2F", NVN_FORMAT_RG16_UI2F},
    {"RG16_I2F", NVN_FORMAT_RG16_I2F},
    {"RG32_UI2F", NVN_FORMAT_RG32_UI2F},
    {"RG32_I2F", NVN_FORMAT_RG32_I2F},
    {"RGB8_UI2F", NVN_FORMAT_RGB8_UI2F},
    {"RGB8_I2F", NVN_FORMAT_RGB8_I2F},
    {"RGB16_UI2F", NVN_FORMAT_RGB16_UI2F},
    {"RGB16_I2F", NVN_FORMAT_RGB16_I2F},
    {"RGB32_UI2F", NVN_FORMAT_RGB32_UI2F},
    {"RGB32_I2F", NVN_FORMAT_RGB32_I2F},
    {"RGBA8_UI2F", NVN_FORMAT_RGBA8_UI2F},
    {"RGBA8_I2F", NVN_FORMAT_RGBA8_I2F},
    {"RGBA16_UI2F", NVN_FORMAT_RGBA16_UI2F},
    {"RGBA16_I2F", NVN_FORMAT_RGBA16_I2F},
    {"RGBA32_UI2F", NVN_FORMAT_RGBA32_UI2F},
    {"RGBA32_I2F", NVN_FORMAT_RGBA32_I2F},
    {"RGB10A2SN", NVN_FORMAT_RGB10A2SN},
    {"RGB10A2I", NVN_FORMAT_RGB10A2I},
    {"RGB10A2_UI2F", NVN_FORMAT_RGB10A2_UI2F},
    {"RGB10A2_I2F", NVN_FORMAT_RGB10A2_I2F},
    {"RGBX8", NVN_FORMAT_RGBX8},
    {"RGBX8SN", NVN_FORMAT_RGBX8SN},
    {"RGBX8UI", NVN_FORMAT_RGBX8UI},
    {"RGBX8I", NVN_FORMAT_RGBX8I},
    {"RGBX16F", NVN_FORMAT_RGBX16F},
    {"RGBX16", NVN_FORMAT_RGBX16},
    {"RGBX16SN", NVN_FORMAT_RGBX16SN},
    {"RGBX16UI", NVN_FORMAT_RGBX16UI},
    {"RGBX16I", NVN_FORMAT_RGBX16I},
    {"RGBX32F", NVN_FORMAT_RGBX32F},
    {"RGBX32UI", NVN_FORMAT_RGBX32UI},
    {"RGBX32I", NVN_FORMAT_RGBX32I},
    {"RGBA_ASTC_4x4", NVN_FORMAT_RGBA_ASTC_4x4},
    {"RGBA_ASTC_5x4", NVN_FORMAT_RGBA_ASTC_5x4},
    {"RGBA_ASTC_5x5", NVN_FORMAT_RGBA_ASTC_5x5},
    {"RGBA_ASTC_6x5", NVN_FORMAT_RGBA_ASTC_6x5},
    {"RGBA_ASTC_6x6", NVN_FORMAT_RGBA_ASTC_6x6},
    {"RGBA_ASTC_8x5", NVN_FORMAT_RGBA_ASTC_8x5},
    {"RGBA_ASTC_8x6", NVN_FORMAT_RGBA_ASTC_8x6},
    {"RGBA_ASTC_8x8", NVN_FORMAT_RGBA_ASTC_8x8},
    {"RGBA_ASTC_10x5", NVN_FORMAT_RGBA_ASTC_10x5},
    {"RGBA_ASTC_10x6", NVN_FORMAT_RGBA_ASTC_10x6},
    {"RGBA_ASTC_10x8", NVN_FORMAT_RGBA_ASTC_10x8},
    {"RGBA_ASTC_10x10", NVN_FORMAT_RGBA_ASTC_10x10},
    {"RGBA_ASTC_12x10", NVN_FORMAT_RGBA_ASTC_12x10},
    {"RGBA_ASTC_12x12", NVN_FORMAT_RGBA_ASTC_12x12},
    {"RGBA_ASTC_4x4_SRGB", NVN_FORMAT_RGBA_ASTC_4x4_SRGB},
    {"RGBA_ASTC_5x4_SRGB", NVN_FORMAT_RGBA_ASTC_5x4_SRGB},
    {"RGBA_ASTC_5x5_SRGB", NVN_FORMAT_RGBA_ASTC_5x5_SRGB},
    {"RGBA_ASTC_6x5_SRGB", NVN_FORMAT_RGBA_ASTC_6x5_SRGB},
    {"RGBA_ASTC_6x6_SRGB", NVN_FORMAT_RGBA_ASTC_6x6_SRGB},
    {"RGBA_ASTC_8x5_SRGB", NVN_FORMAT_RGBA_ASTC_8x5_SRGB},
    {"RGBA_ASTC_8x6_SRGB", NVN_FORMAT_RGBA_ASTC_8x6_SRGB},
    {"RGBA_ASTC_8x8_SRGB", NVN_FORMAT_RGBA_ASTC_8x8_SRGB},
    {"RGBA_ASTC_10x5_SRGB", NVN_FORMAT_RGBA_ASTC_10x5_SRGB},
    {"RGBA_ASTC_10x6_SRGB", NVN_FORMAT_RGBA_ASTC_10x6_SRGB},
    {"RGBA_ASTC_10x8_SRGB", NVN_FORMAT_RGBA_ASTC_10x8_SRGB},
    {"RGBA_ASTC_10x10_SRGB", NVN_FORMAT_RGBA_ASTC_10x10_SRGB},
    {"RGBA_ASTC_12x10_SRGB", NVN_FORMAT_RGBA_ASTC_12x10_SRGB},
    {"RGBA_ASTC_12x12_SRGB", NVN_FORMAT_RGBA_ASTC_12x12_SRGB},
    {"BGR565", NVN_FORMAT_BGR565},
    {"BGR5", NVN_FORMAT_BGR5},
    {"BGR5A1", NVN_FORMAT_BGR5A1},
    {"A1BGR5", NVN_FORMAT_A1BGR5},
    {"BGRX8", NVN_FORMAT_BGRX8},
    {"BGRA8", NVN_FORMAT_BGRA8},
    {"BGRX8_SRGB", NVN_FORMAT_BGRX8_SRGB},
    {"BGRA8_SRGB", NVN_FORMAT_BGRA8_SRGB},
};

constexpr int kEnumCount = static_cast<int>(sizeof(kEnumerators) / sizeof(kEnumerators[0]));

const FormatInfo* by_name(const char* name) {
    for (int i = 0; i < format_count(); ++i) {
        const FormatInfo* f = format_at(i);
        if (f && std::strcmp(f->name, name) == 0) return f;
    }
    return nullptr;
}

bool has_suffix(const std::string& s, const char* suffix) {
    const size_t n = std::strlen(suffix);
    return s.size() >= n && s.compare(s.size() - n, n, suffix) == 0;
}

struct Spot {
    const char* name;
    uint8_t bytes, bw, bh, channels;
    uint8_t aspect;
    VkFormat vk;
    VkFormat fallback;
};

constexpr Spot kSpots[] = {
    {"R8", 1, 1, 1, 1, kAspectColor, VK_FORMAT_R8_UNORM, VK_FORMAT_UNDEFINED},
    {"RG8", 2, 1, 1, 2, kAspectColor, VK_FORMAT_R8G8_UNORM, VK_FORMAT_UNDEFINED},
    {"RGB8", 3, 1, 1, 3, kAspectColor, VK_FORMAT_R8G8B8_UNORM, VK_FORMAT_UNDEFINED},
    {"RGBA8", 4, 1, 1, 4, kAspectColor, VK_FORMAT_R8G8B8A8_UNORM, VK_FORMAT_UNDEFINED},
    {"RGBA16F", 8, 1, 1, 4, kAspectColor, VK_FORMAT_R16G16B16A16_SFLOAT, VK_FORMAT_UNDEFINED},
    {"RGBA32F", 16, 1, 1, 4, kAspectColor, VK_FORMAT_R32G32B32A32_SFLOAT, VK_FORMAT_UNDEFINED},
    {"R16", 2, 1, 1, 1, kAspectColor, VK_FORMAT_R16_UNORM, VK_FORMAT_UNDEFINED},
    {"RG32F", 8, 1, 1, 2, kAspectColor, VK_FORMAT_R32G32_SFLOAT, VK_FORMAT_UNDEFINED},
    {"RGBA8_SRGB", 4, 1, 1, 4, kAspectColor, VK_FORMAT_R8G8B8A8_SRGB, VK_FORMAT_UNDEFINED},
    {"RGBX8_SRGB", 4, 1, 1, 3, kAspectColor, VK_FORMAT_R8G8B8A8_SRGB, VK_FORMAT_UNDEFINED},
    {"RGB565", 2, 1, 1, 3, kAspectColor, VK_FORMAT_R5G6B5_UNORM_PACK16, VK_FORMAT_UNDEFINED},
    {"RGBA4", 2, 1, 1, 4, kAspectColor, VK_FORMAT_A4B4G4R4_UNORM_PACK16, VK_FORMAT_UNDEFINED},
    {"RGB10A2", 4, 1, 1, 4, kAspectColor, VK_FORMAT_A2B10G10R10_UNORM_PACK32, VK_FORMAT_UNDEFINED},
    {"R11G11B10F", 4, 1, 1, 3, kAspectColor, VK_FORMAT_B10G11R11_UFLOAT_PACK32, VK_FORMAT_R16G16B16A16_SFLOAT},
    {"STENCIL8", 1, 1, 1, 1, kAspectStencil, VK_FORMAT_S8_UINT, VK_FORMAT_UNDEFINED},
    {"DEPTH16", 2, 1, 1, 1, kAspectDepth, VK_FORMAT_D16_UNORM, VK_FORMAT_D32_SFLOAT},
    {"DEPTH24", 4, 1, 1, 1, kAspectDepth, VK_FORMAT_X8_D24_UNORM_PACK32, VK_FORMAT_D32_SFLOAT},
    {"DEPTH32F", 4, 1, 1, 1, kAspectDepth, VK_FORMAT_D32_SFLOAT, VK_FORMAT_UNDEFINED},
    {"DEPTH24_STENCIL8", 4, 1, 1, 2, kAspectDepthStencil, VK_FORMAT_D24_UNORM_S8_UINT, VK_FORMAT_D32_SFLOAT_S8_UINT},
    {"DEPTH32F_STENCIL8", 8, 1, 1, 2, kAspectDepthStencil, VK_FORMAT_D32_SFLOAT_S8_UINT, VK_FORMAT_UNDEFINED},
    {"RGB_DXT1", 8, 4, 4, 3, kAspectColor, VK_FORMAT_BC1_RGB_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_DXT1", 8, 4, 4, 4, kAspectColor, VK_FORMAT_BC1_RGBA_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_DXT3", 16, 4, 4, 4, kAspectColor, VK_FORMAT_BC2_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_DXT5", 16, 4, 4, 4, kAspectColor, VK_FORMAT_BC3_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_DXT1_SRGB", 8, 4, 4, 4, kAspectColor, VK_FORMAT_BC1_RGBA_SRGB_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGTC1_UNORM", 8, 4, 4, 1, kAspectColor, VK_FORMAT_BC4_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGTC2_SNORM", 16, 4, 4, 2, kAspectColor, VK_FORMAT_BC5_SNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"BPTC_UNORM", 16, 4, 4, 4, kAspectColor, VK_FORMAT_BC7_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"BPTC_UNORM_SRGB", 16, 4, 4, 4, kAspectColor, VK_FORMAT_BC7_SRGB_BLOCK, VK_FORMAT_UNDEFINED},
    {"BPTC_UFLOAT", 16, 4, 4, 3, kAspectColor, VK_FORMAT_BC6H_UFLOAT_BLOCK, VK_FORMAT_UNDEFINED},
    {"BPTC_SFLOAT", 16, 4, 4, 3, kAspectColor, VK_FORMAT_BC6H_SFLOAT_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_ASTC_4x4", 16, 4, 4, 4, kAspectColor, VK_FORMAT_ASTC_4x4_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_ASTC_5x4", 16, 5, 4, 4, kAspectColor, VK_FORMAT_ASTC_5x4_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_ASTC_5x5", 16, 5, 5, 4, kAspectColor, VK_FORMAT_ASTC_5x5_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_ASTC_8x5", 16, 8, 5, 4, kAspectColor, VK_FORMAT_ASTC_8x5_UNORM_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_ASTC_5x4_SRGB", 16, 5, 4, 4, kAspectColor, VK_FORMAT_ASTC_5x4_SRGB_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_ASTC_5x5_SRGB", 16, 5, 5, 4, kAspectColor, VK_FORMAT_ASTC_5x5_SRGB_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_ASTC_8x5_SRGB", 16, 8, 5, 4, kAspectColor, VK_FORMAT_ASTC_8x5_SRGB_BLOCK, VK_FORMAT_UNDEFINED},
    {"RGBA_ASTC_12x12_SRGB", 16, 12, 12, 4, kAspectColor, VK_FORMAT_ASTC_12x12_SRGB_BLOCK, VK_FORMAT_UNDEFINED},
    {"BGRA8", 4, 1, 1, 4, kAspectColor, VK_FORMAT_B8G8R8A8_UNORM, VK_FORMAT_UNDEFINED},
    {"BGRA8_SRGB", 4, 1, 1, 4, kAspectColor, VK_FORMAT_B8G8R8A8_SRGB, VK_FORMAT_UNDEFINED},
    {"BGR565", 2, 1, 1, 3, kAspectColor, VK_FORMAT_B5G6R5_UNORM_PACK16, VK_FORMAT_UNDEFINED},
    {"RG16_UI2F", 4, 1, 1, 2, kAspectColor, VK_FORMAT_R16G16_USCALED, VK_FORMAT_UNDEFINED},
    {"RGBA8_UI2F", 4, 1, 1, 4, kAspectColor, VK_FORMAT_R8G8B8A8_USCALED, VK_FORMAT_UNDEFINED},
    {"R32_UI2F", 4, 1, 1, 1, kAspectColor, VK_FORMAT_UNDEFINED, VK_FORMAT_UNDEFINED},
    {"RGB10A2_I2F", 4, 1, 1, 4, kAspectColor, VK_FORMAT_A2B10G10R10_SSCALED_PACK32, VK_FORMAT_UNDEFINED},
    {"RGB10A2SN", 4, 1, 1, 4, kAspectColor, VK_FORMAT_A2B10G10R10_SNORM_PACK32, VK_FORMAT_UNDEFINED},
    {"RGBX16F", 8, 1, 1, 3, kAspectColor, VK_FORMAT_R16G16B16A16_SFLOAT, VK_FORMAT_UNDEFINED},
};

}  // namespace

NXTEST(count_matches_enumerators) {
    CHECK_EQ(kEnumCount, 156);
    CHECK_EQ(format_count(), kEnumCount);
    for (int i = 0; i < format_count(); ++i) CHECK(format_at(i) != nullptr);
    CHECK(format_at(format_count()) == nullptr);
}

NXTEST(every_enumerator_resolves_once) {
    std::set<std::string> names;
    for (const Enumerator& e : kEnumerators) {
        const FormatInfo* f = format_info(e.value);
        CHECK_MSG(f != nullptr, e.name);
        if (!f) continue;
        CHECK_EQ(f->nvn, e.value);
        CHECK(f->name != nullptr && f->name[0] != '\0');
        CHECK_EQ(std::string(f->name), std::string(e.name));
        CHECK(names.insert(f->name).second);
    }
    CHECK_EQ(names.size(), static_cast<size_t>(kEnumCount));
}

NXTEST(row_invariants) {
    REQUIRE(format_count() == kEnumCount);
    for (int i = 0; i < format_count(); ++i) {
        const FormatInfo* f = format_at(i);
        REQUIRE(f != nullptr);
        const std::string n = f->name;
        CHECK_MSG(f->bytes > 0, n);
        CHECK_MSG(f->block_w >= 1 && f->block_h >= 1, n);
        CHECK_MSG(f->channels >= 1 && f->channels <= 4, n);
        CHECK_MSG(f->compressed || (f->block_w == 1 && f->block_h == 1), n);
        CHECK_MSG(!f->compressed || f->bytes == 8 || f->bytes == 16, n);
        CHECK_MSG(f->srgb == has_suffix(n, "_SRGB"), n);
        const bool bc_name = n.find("DXT") != std::string::npos || n.find("RGTC") != std::string::npos ||
                             n.find("BPTC") != std::string::npos;
        CHECK_MSG(f->bc == bc_name, n);
        CHECK_MSG(!f->bc || (f->block_w == 4 && f->block_h == 4), n);
        const bool astc_name = n.find("ASTC_") != std::string::npos;
        CHECK_MSG(f->astc == astc_name, n);
        CHECK_MSG(f->compressed == (bc_name || astc_name), n);
        if (astc_name) {
            int w = 0, h = 0;
            const char* p = std::strstr(n.c_str(), "ASTC_");
            CHECK_MSG(std::sscanf(p + 5, "%dx%d", &w, &h) == 2, n);
            CHECK_MSG(w == f->block_w && h == f->block_h, n);
        }
        const bool ds_name = n[0] == 'D' || n[0] == 'S';
        CHECK_MSG((f->aspect != kAspectColor) == ds_name, n);
        const int numeric = int(f->is_float) + int(f->is_snorm) + int(f->is_int) + int(f->is_uint);
        CHECK_MSG(numeric <= 1, n);
    }
}

NXTEST(spot_checks_exact_values) {
    for (const Spot& s : kSpots) {
        const FormatInfo* f = by_name(s.name);
        CHECK_MSG(f != nullptr, s.name);
        if (!f) continue;
        CHECK_MSG(f->bytes == s.bytes, s.name);
        CHECK_MSG(f->block_w == s.bw && f->block_h == s.bh, s.name);
        CHECK_MSG(f->channels == s.channels, s.name);
        CHECK_MSG(f->aspect == s.aspect, s.name);
        CHECK_MSG(f->vk == s.vk, s.name);
        CHECK_MSG(f->vk_fallback == s.fallback, s.name);
    }
}

NXTEST(srgb_variants_differ_from_unorm_twin) {
    int pairs = 0;
    for (int i = 0; i < format_count(); ++i) {
        const FormatInfo* f = format_at(i);
        REQUIRE(f != nullptr);
        if (!f->srgb) continue;
        const std::string n = f->name;
        CHECK_MSG(has_suffix(n, "_SRGB"), n);
        const std::string twin = n.substr(0, n.size() - 5);
        const FormatInfo* t = by_name(twin.c_str());
        CHECK_MSG(t != nullptr, n);
        if (!t) continue;
        CHECK_MSG(!t->srgb, n);
        CHECK_MSG(f->vk != t->vk, n);
        ++pairs;
    }
    CHECK_EQ(pairs, 23);
}

NXTEST(out_of_range_lookups_are_null) {
    CHECK(format_info(0) == nullptr);
    CHECK(format_info(-1) == nullptr);
    CHECK(format_info(9999) == nullptr);
    CHECK(format_info(NVN_FORMAT_NONE) == nullptr);
    CHECK(format_info(NVN_FORMAT_LARGE) == nullptr);
    CHECK(format_info(0x9D) == nullptr);
    CHECK(format_at(-1) == nullptr);
    CHECK(format_at(format_count()) == nullptr);
}

NXTEST(vertex_format_cases) {
    CHECK_EQ(vertex_format(NVN_FORMAT_R8), VK_FORMAT_R8_UNORM);
    CHECK_EQ(vertex_format(NVN_FORMAT_R8SN), VK_FORMAT_R8_SNORM);
    CHECK_EQ(vertex_format(NVN_FORMAT_R8UI), VK_FORMAT_R8_UINT);
    CHECK_EQ(vertex_format(NVN_FORMAT_R8I), VK_FORMAT_R8_SINT);
    CHECK_EQ(vertex_format(NVN_FORMAT_R32F), VK_FORMAT_R32_SFLOAT);
    CHECK_EQ(vertex_format(NVN_FORMAT_RG16F), VK_FORMAT_R16G16_SFLOAT);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGB32F), VK_FORMAT_R32G32B32_SFLOAT);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGBA32F), VK_FORMAT_R32G32B32A32_SFLOAT);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGBX8), VK_FORMAT_R8G8B8A8_UNORM);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGB10A2), VK_FORMAT_A2B10G10R10_UNORM_PACK32);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGB10A2_I2F), VK_FORMAT_A2B10G10R10_SSCALED_PACK32);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGBA8_UI2F), VK_FORMAT_R8G8B8A8_USCALED);
    CHECK_EQ(vertex_format(NVN_FORMAT_R32_UI2F), VK_FORMAT_UNDEFINED);
    CHECK_EQ(vertex_format(NVN_FORMAT_DEPTH32F), VK_FORMAT_UNDEFINED);
    CHECK_EQ(vertex_format(NVN_FORMAT_DEPTH24_STENCIL8), VK_FORMAT_UNDEFINED);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGBA8_SRGB), VK_FORMAT_UNDEFINED);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGBA_ASTC_4x4), VK_FORMAT_UNDEFINED);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGB_DXT1), VK_FORMAT_UNDEFINED);
    CHECK_EQ(vertex_format(NVN_FORMAT_RGBA4), VK_FORMAT_UNDEFINED);
    CHECK_EQ(vertex_format(NVN_FORMAT_NONE), VK_FORMAT_UNDEFINED);
    CHECK_EQ(vertex_format(9999), VK_FORMAT_UNDEFINED);
}
