// SPDX-License-Identifier: GPL-3.0-or-later
// ASTC through the transcoder (the astc2bc7 library of tools/textures, as nvn_texdata.cpp uses it) against known blocks. Each row of kKnown is a
// 23x19 image encoded by ARM astcenc 5.7.0 (thorough) at one footprint, with the FNV-1a 64 hash of astcenc's own RGBA8 decode of those blocks
// in the linear and in the sRGB profile (the same blocks decode differently in the two; generated offline). The image has gradients, a hard
// edge, noise, and alpha below 255 in its right half; 23 and 19 are prime, so every footprint leaves partial blocks at two edges.
//   * the library's decode (threaded and not) hashes to astcenc's;
//   * every ASTC format of the NVN format table (14 footprints, linear and sRGB) goes through prepare_texture as a block-linear texture: the
//     RGBA8 decode hashes to astcenc's, the BC7 transcode is BC7_SRGB / BC7_UNORM as the format, stays close to the decode (PSNR with the
//     reference BC7 decoder of bc7enc_rdo) and keeps alpha exactly 255 in the 4x4 blocks that are opaque;
//   * ASTC 8x8 sRGB (the format of nearly all the game's ASTC textures) as a 2D array with a mip chain: every level and layer lands in its place.
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include <nxrt/config.h>

#include "astc2bc7.h"
#include "nvn_core.h"
#include "nvn_format.h"
#include "nvn_texdata.h"
#include "nvn_texture_layout.h"
#ifdef NXRT_HAVE_BC7_REF
#include "bc7decomp.h"
#endif
#include "tegra_tiling.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_astc)

using namespace nxrt;
using namespace nxrt::nvn;
using namespace nxrt::nvn::be;

namespace {

constexpr uint32_t kW = 23, kH = 19;

struct Known {
    uint32_t bw, bh;
    uint64_t hash_linear, hash_srgb;
    const char* hex;   // the blocks, row by row
};

const Known kKnown[] = {
    {4, 4, 0xcc979432e24f56a2ull, 0x4f98eb12b7e4b25aull,
     "5e070114403f536155d0baa1e8c050014204d5e520a5ea467d3b5e194e0a4c08de05b1f2045620f740bc68b5a14d4003de856f3f4081f1a24532dab813ca0032"
     "3284975390add8747763b79ed49c13265f6926782e8122a5ad41189512288bff330401a4c162567f7b0dba63053c0a01de05599a58b8582f41fee8b7502d4813"
     "02062d7c6adcbd57fb53a307f454a40442e89d67b5468c9733c48e1d222c1f9fde856333c64ec23c73716fb89d3403200286ef8d309434d9b4b439bf4848212f"
     "42048151fadde65c7f3b6e1a6e284c08bd0795e4b8bdea26e27ac1e8160cc10322502f30662731f332662c0b0f7b940032e863f8bc7ec6d1b75d8057f131232d"
     "4284b786b7c4f2d66cfdac79a8b5283132842b39ac87a8a1f8509b7cdc1980a4330401a49463567b7b0dbe63053a0a01ce111b67df341266bbee714d811e701c"
     "5128fc09fc7c6ccc5eaf9d7a0015014642849faf65c238e3ecf5e8b9e8f9e071ad87c1f23d11e6630d2408c08a030689328477debd6922a1fa8ccb2f941180b4"
     "ce9105cd2556b0737e8073d719ba8e2d51d049fb2f936658ffae3d08481a97c2cf057dec2f09482f29da25ce0b42811542845dfe86c2bcc2647d647d24352039"
     "de857d9cb68428bed3541e9547ca82b051847fb8713d6229dd4c5af633a9271d"},
    {5, 4, 0x6ec522d9c6158a3bull, 0x76708a6e5135a064ull,
     "b204a104443c37f95f1b4cb328881304b2044d1c44dc5ff15f794c331988130493844d2e683250fe35544bd6031482e1cf85cb0dd09467b2082bd5f513ec4be2"
     "ce297c782e112235ad4108b932b00e57de05015866ba905941fc68b5914d480333041d6cc15251fb5b15791a073c2b0133849b534ab744c7fbfd71d2e53483f1"
     "c2281378ce48bbcdd04957879a1abaf0ad876396b992e5c92c928972d90b002cb204a104d33c37f1af3a4aab38cc1b14b2044d1cf7d85ff1af3a4ebb39881304"
     "924829b81ebf22561c7f4eef06f0cc3fde852b596c8bb27dcf543eab4baaa2b0ad875733231106d22c53c938d409000cb204a1447f3b37f15f3a4aab38cc1304"
     "d2080970ef64224b1fff166dd95c2550ad8749f0841784fe2d23c9528b033008ad8749f08495e4830d2689c086070408cf85dbbfe58246b8212831a61b310030"
     "2290054d16178658178173d709dad02cd1047d75ab41b2936c5e6c5e00aa4e22d1847dafccc2acc25cb35cb3581a68922284fff13b4b10aaffcdecfdecf528b9"
     "ce297cd8bb8caab83bdf1b1533300c3f"},
    {5, 5, 0xa78baf0bfee2a919ull, 0xb5799a099c69f8bdull,
     "de0501580668905941fce8b5a14d4003b2044d1c64da5ff1af794ab31888130402861b0758dcbd4287e7bbf784ecb0f832841b55908de0387332ae9dd49a1326"
     "ce0920784e5122a5ad0108b9303006d733048105fae255fb5b1dbe6b19380a014e0799b12eff5b5ff70ff10ef68af8047148a9e7d6e4ecfb94ece41b7f3f2600"
     "ad87dd14ea16a57b0d2689c08a0b0408de856337d8a3a820f8503ea18ad002a03304810574e355fb3a1d792a19380a1183f014300d5fa4e614d5061755e847ac"
     "22902c673d978c775c3afc5fa8f4f8700286f7f76786f0cf707675bf80028987ad87cf106c9523d20c92c9408903000cce113a4d165784511b817bc709754d1e"
     "e150c9b1f8477c7c78ad2d12004af187d1845dfe86d2aac25ab35cb368924812de85bffcb7802abecf543e958fb282a0ee8541eed58f45daccfccc4f78440b30"},
    {6, 5, 0x94ee20d7670665e3ull, 0xa0cd9bd46ccb9c0bull,
     "b2040126643a77f1ae1b4ab3188813044204a17c20a69c577f3b5d196e2a4c080286cf1758c8fb468befd7fbc8e054f481863506e690031928068f2408343908"
     "fe050126e63a77f1a3df86b5204d40034204a17c508f9c577f3b6d296e2a4c0812090978ac49e617a751ac5ea0fa20cd0286cfd5ba9444feb074b97fc048c14f"
     "ae0701e773e354ffe30ff1110ef4e00cce7104cb8bdedd7fd8f7e721237f8d610286d7df658668e270ba75bf404a454f0286d7d7e784c0de30b439bfc048c147"
     "61e9830943b73dc157715315e038671cf104afef04843afe3afe2559ed9b891042845ffe0c845cc3e4fde47da8b92031ee85410ed6912522cdfeec8f76a40332"},
    {6, 6, 0x84a6f9a8e3e5f680ull, 0x84ff8184552b423dull,
     "1206012680397749449e4621a03828010206a17c20999c57f35bab07f050a000bd87e70358e1bf2c821e59f1374ee6b01285171110cea05cff505d970f4084a1"
     "7d07016722e354448293d1e8011e80002e90096bdde3bf63456d5772dbc66110fd2940b81dbb2a4edee3cf6516fc2e210286930eb59444fe7074f5bfc0c849cf"
     "f188a001e8e9dba19f9902390f2ff807fe692390d4bfbffdf26750caf74ea94102867baf8d945ae3b07abdbf4082454f02867baf8d94c6fab074b9bf80844147"
     "12698889d46ee73bc78d433471a3dc280303e1e0c6e3277a00d180c310071c0c03857d670cd25c8381dabf255ae5fd321285f99ca2a42865fb0054170a4075a1"},
    {8, 5, 0x06ea2902600a243eull, 0xce2d14c6533a7541ull,
     "0306011498b71346bcba300018160c188186290366c5fb89c0e1892441a682850286cb0c9844b1760d2713f308c850f8fe05a108e63a5bf1a3dd86b5204d4003"
     "020909b83fba2a7a3622c6de06560fd60286cfddb89444d6b07cb9bf4044894743040171b964aa45c2f3c0565b2060013131cee15e43dc63326e7f3c41b81558"
     "2384fbff27944692a1e8259a8f05a210828992c1656f2c4c1b1e63bbb05b235806845ffe44c4b8c26c7d7f772031333323845f360d94c2b2a1f8259eaf05aa10"},
    {8, 6, 0x6c2654170c649c42ull, 0xb430ea43b89e7445ull,
     "ad0701ae0688902145e33cf20807c33002863706e0fa4b428beb7bf708e470f80286cb0c6004b57683af1bfb00c850f80306011477b753c4b8b8e08039360c1a"
     "22e9c3e7ef4a8c9d45bd0d90f06f3fd60286930e759444d6b07c75bfc04449cf4e88a0812239172eb907fffff7db18284d104f4b37e468f51f7e594ca81111ee"
     "02867baf179446f2f07c757f804c494f15289a89d46ee73bc78d4332410af36106847d670cd2bcc264bdbfb364bdbfb31385bdcf848af518a6e7e58f617a3efe"},
    {8, 8, 0x0b15c1e2dd6a95f7ull, 0x8b1b4e84c7aafcfdull,
     "4204412520dbe6467f3b5d196e2a4c0842844f1e6ae6db4273ff59dd6aee48cc918657034a9ad4c06091890881181c0833040353936747fb351e76641a300402"
     "62e8e1e5c8f252d156841ffdf5566a8091869bb5a982f4d0e4958b0a89543c2806306750b54571662409ed0bb36042fd06841da70ec2bcc2647dbf73a43db7bb"
     "cf855de74c94c2b2612861a420972810"},
    {10, 5, 0x24ec0419717e18b3ull, 0x06f0b9710c7f5d60ull,
     "420401b020e647577f3b5d196e2a4c0802864b0f58e665430b6737f78068d4f07d874701898490d9efb8ff65cb943429420401b030ef475f7f3b6d196e2a4c08"
     "028938b87572aa7c34d78e6c07d88fda3185133be222690437f6528e240b0010320907109bfa7bd1df9e825dd14bf405128573dd7fc6a07cc1581b8f0f244ca0"
     "b1845f77ff67142711da12762499001082f13f0b6d5fe65c7c805b19d26d8bd81285bfda7788aabec1d80b8f0f244ca0e14896aff85dc4af27a7299a81ffc103"},
    {10, 6, 0x54f32d5e0c8cbab9ull, 0x9d9c65385d29df16ull,
     "420401b020f8475f7f3b5d196e2a4c088186b303c840f789e0e0fd24002657811f84470182849079ce664c12c7807516420481b1f2ed474f7f3b5d196e2a4c08"
     "9249a967b350e45e0f4d0e1fd096ab7882852779a9a3a861d8ccccec0f000030957084016cb767f77fae6f24d807e80d9384c9eeb49504f48debfdff0342701c"
     "31857b6e5b95482c3bf6528e240b001015299a49e2fbf1c7b631c612f461f4618684bdcb8ce4c68862d9fb83281ebdb71681e1e0d4d9515c96de000010050028"},
    {10, 8, 0x6aed6a2caa4dd54aull, 0x757b249b09bd1b3eull,
     "420401b020eb47577f3b5d196e2a4c089186a705a3d74685866eab0800c181b08186b705ab94c338280a85240000004ab204011d7b2c69f1f53a72ca18082104"
     "8186296cf890f71938ba893460703c0881862b6cb790d378381a83244110100c613167d76f7324831b369c1dbaaeebeba284bdade61106fc4dda7764fd071233"
     "61853f8ed61945e2aceaaaaeeaaa0e40"},
    {10, 10, 0x644470901505680bull, 0xccef13ec56008965ull,
     "420401b020fe47577f3b5d196e2a4c08a38453a15c0d29432eefcacf2722708381863705ae94c338180a85240000000c159165d0f9b55e5e40c18b19ad75800d"
     "9186bff9a4c2c6c1a11389081038140806841bfe858442f8eeeecefd00000032"},
    {12, 10, 0xa7c7a965daa8246cull, 0x7b024232ddcb14ebull,
     "4204413c20be87577f3b5d196e2a4c081385d747074665e7753fef0f801082e1149065d0c56ddf1b91f168737ff8a00d42841bfe859432e3ecfd6475a8b92031"},
    {12, 12, 0xa30fc76ca3794144ull, 0x63099bb2798f3026ull,
     "df05853410e34aff75b455a14a020a01ad876532200d4bf145e1f9d2080fc0b045047d05fb33d55355f555f555fd80aaae878ffb248ac4707e7fb03ebf804e8f"},
};

uint64_t fnv1a(const uint8_t* p, size_t n) {
    uint64_t h = 0xcbf29ce484222325ull;
    for (size_t i = 0; i < n; ++i) {
        h ^= p[i];
        h *= 0x100000001b3ull;
    }
    return h;
}

std::vector<uint8_t> blocks_of(const Known& k) {
    std::vector<uint8_t> v(std::strlen(k.hex) / 2);
    for (size_t i = 0; i < v.size(); ++i) v[i] = static_cast<uint8_t>(std::strtoul(std::string(k.hex + 2 * i, 2).c_str(), nullptr, 16));
    return v;
}

const Known* known_for(uint32_t bw, uint32_t bh) {
    for (const Known& k : kKnown)
        if (k.bw == bw && k.bh == bh) return &k;
    return nullptr;
}

bool have_transcoder() {
    if (a2b_bc7_encoder(nullptr) != A2B_ENCODER_NONE) return true;
    nxtest::skip("no ASTC transcoder in this build (NXRT_ASTC2BC7=OFF)");
    return false;
}

void no_cache() {
    _putenv_s("NXRT_NVN_BC7_CACHE", "");
    config_set("nvn", "bc7_cache", "");
}

// A block-linear texture whose untiled image is `lin` (levels, layers, tight rows of blocks: the layout's linear buffer).
struct Tex {
    TexLayout L;
    std::vector<uint8_t> lin, mem;
    Ref<Texture> t;
};

bool make_tex(Tex& x, int target, uint32_t w, uint32_t h, uint32_t depth, uint32_t levels, int format) {
    TextureBuilderInfo b;
    b.target = target;
    b.width = static_cast<int>(w);
    b.height = static_cast<int>(h);
    b.depth = static_cast<int>(depth);
    b.levels = static_cast<int>(levels);
    b.format = format;
    x.L = compute_texture_layout(b);
    if (!x.L.valid || !x.L.block_linear) return false;
    x.lin.assign(static_cast<size_t>(x.L.layout.linear_size), 0);
    x.t = Ref<Texture>(new Texture());
    x.t->b = b;
    x.t->fmt = x.L.fmt;
    x.t->storage_size = x.L.storage_size;
    x.t->block_linear = true;
    x.t->levels = x.L.levels;
    x.t->layers = x.L.layers;
    x.t->block_height_gobs = x.L.layout.levels[0].gob_blocks_y;
    return true;
}

void tile(Tex& x) {
    x.mem.assign(static_cast<size_t>(x.L.storage_size), 0);
    tegra::tile(x.L.desc, x.L.layout, x.lin.data(), x.mem.data());
    x.t->mem = x.mem.data();
}

#ifdef NXRT_HAVE_BC7_REF
// RGBA PSNR of the BC7 blocks (w x h texels, tight rows of blocks) against `ref`, and the texels whose alpha is not 255 in the 4x4 blocks
// where `ref` is opaque.
double bc7_psnr(const uint8_t* bc7, const uint8_t* ref, uint32_t w, uint32_t h, uint32_t* alpha_bad) {
    const uint32_t bx = (w + 3) / 4, by = (h + 3) / 4;
    double se = 0;
    *alpha_bad = 0;
    for (uint32_t y = 0; y < by; ++y)
        for (uint32_t x = 0; x < bx; ++x) {
            bc7decomp::color_rgba c[16];
            bc7decomp_ref::unpack_bc7(bc7 + (static_cast<size_t>(y) * bx + x) * 16, c);
            bool opaque = true;
            for (uint32_t i = 0; i < 16; ++i) {
                const uint32_t tx = std::min(x * 4 + (i & 3), w - 1), ty = std::min(y * 4 + (i >> 2), h - 1);
                opaque &= ref[(static_cast<size_t>(ty) * w + tx) * 4 + 3] == 255;
            }
            for (uint32_t i = 0; i < 16; ++i) {
                const uint32_t tx = x * 4 + (i & 3), ty = y * 4 + (i >> 2);
                if (opaque && c[i].a != 255) ++*alpha_bad;
                if (tx >= w || ty >= h) continue;
                const uint8_t* r = ref + (static_cast<size_t>(ty) * w + tx) * 4;
                const double d[4] = {double(c[i].r) - r[0], double(c[i].g) - r[1], double(c[i].b) - r[2], double(c[i].a) - r[3]};
                se += d[0] * d[0] + d[1] * d[1] + d[2] * d[2] + d[3] * d[3];
            }
        }
    const double mse = se / (static_cast<double>(w) * h * 4);
    return mse > 0 ? 10.0 * std::log10(255.0 * 255.0 / mse) : 99.0;
}
#endif

}  // namespace

// The library's decode of every row, in both profiles, on all threads and on one, hashes to astcenc's own decode.
NXTEST(known_blocks_decode_as_astcenc) {
    if (!have_transcoder()) return;
    const char* name = "?";
    a2b_bc7_encoder(&name);
    std::printf("    encoder: %s\n", name);
    std::vector<uint8_t> px(static_cast<size_t>(kW) * kH * 4);
    for (const Known& k : kKnown) {
        const std::vector<uint8_t> blocks = blocks_of(k);
        CHECK_EQ(blocks.size(), static_cast<size_t>(a2b_astc_size(kW, kH, k.bw, k.bh)));
        for (int srgb = 0; srgb < 2; ++srgb)
            for (int threads : {0, 1}) {
                const a2b_input in{srgb ? A2B_ASTC_SRGB : A2B_ASTC_UNORM, kW, kH, k.bw, k.bh};
                std::fill(px.begin(), px.end(), 0);
                CHECK_EQ(a2b_decode_astc_rgba8(&in, blocks.data(), blocks.size(), threads, px.data(), nullptr), A2B_OK);
                const uint64_t want = srgb ? k.hash_srgb : k.hash_linear;
                CHECK_MSG(fnv1a(px.data(), px.size()) == want, std::to_string(k.bw) + "x" + std::to_string(k.bh) + (srgb ? " sRGB" : " linear") +
                                                                  ": decode differs from astcenc's");
            }
    }
}

// Every ASTC format of the NVN format table through prepare_texture: RGBA8 decode and BC7 transcode of the known blocks.
NXTEST(every_astc_format_through_prepare_texture) {
    if (!have_transcoder()) return;
    no_cache();
    int formats = 0;
    double worst = 99;
    for (int i = 0; i < format_count(); ++i) {
        const FormatInfo* f = format_at(i);
        if (!f || !f->astc) continue;
        ++formats;
        const Known* k = known_for(f->block_w, f->block_h);
        REQUIRE(k != nullptr);
        Tex x;
        REQUIRE(make_tex(x, NVN_TEXTURE_TARGET_2D, kW, kH, 1, 1, f->nvn));
        const std::vector<uint8_t> blocks = blocks_of(*k);
        REQUIRE(blocks.size() == x.L.layout.levels[0].linear_slice_size);
        std::memcpy(x.lin.data(), blocks.data(), blocks.size());
        tile(x);

        TexCaps rgba;
        rgba.bc7_transcode = false;
        PreparedTexture d;
        CHECK_MSG(prepare_texture(*x.t, rgba, d), d.error);
        CHECK_EQ(d.format, f->srgb ? VK_FORMAT_R8G8B8A8_SRGB : VK_FORMAT_R8G8B8A8_UNORM);
        CHECK(d.transcoded);
        REQUIRE(d.data.size() == static_cast<size_t>(kW) * kH * 4);
        CHECK_MSG(fnv1a(d.data.data(), d.data.size()) == (f->srgb ? k->hash_srgb : k->hash_linear), std::string(f->name) + ": RGBA8 differs from astcenc's");

        PreparedTexture b;
        CHECK_MSG(prepare_texture(*x.t, TexCaps(), b), b.error);
        CHECK_EQ(b.format, f->srgb ? VK_FORMAT_BC7_SRGB_BLOCK : VK_FORMAT_BC7_UNORM_BLOCK);
        CHECK_EQ(b.level[0].row_blocks, (kW + 3) / 4);
        REQUIRE(b.data.size() == static_cast<size_t>((kW + 3) / 4) * ((kH + 3) / 4) * 16);
#ifdef NXRT_HAVE_BC7_REF
        uint32_t alpha_bad = 0;
        const double psnr = bc7_psnr(b.data.data(), d.data.data(), kW, kH, &alpha_bad);
        worst = std::min(worst, psnr);
        const double bound = bc7_fast_preset() == 0 ? 31.0 : 34.0;   // the test image is harsh: presets 1..3 give 36 to 46 dB
        CHECK_MSG(psnr >= bound, std::string(f->name) + ": BC7 PSNR " + std::to_string(psnr));
        CHECK_MSG(alpha_bad == 0, std::string(f->name) + ": alpha not 255 in opaque blocks on " + std::to_string(alpha_bad) + " texels");
#endif
    }
    CHECK_EQ(formats, 28);
    std::printf("    %d ASTC formats; worst BC7 PSNR %.2f dB at preset %d\n", formats, worst, bc7_fast_preset());
}

// ASTC 8x8 sRGB, 61x37, 4 levels, 2 layers, every level and layer different: each lands in its place in the RGBA8 decode and in the BC7 data.
NXTEST(astc_8x8_srgb_array_mip_chain) {
    if (!have_transcoder()) return;
    no_cache();
    Tex x;
    REQUIRE(make_tex(x, NVN_TEXTURE_TARGET_2D_ARRAY, 61, 37, 2, 4, NVN_FORMAT_RGBA_ASTC_8x8_SRGB));
    REQUIRE(x.L.levels == 4 && x.L.layers == 2);
    const std::vector<uint8_t> src = blocks_of(*known_for(8, 8));
    const size_t nsrc = src.size() / 16;
    size_t next = 0;
    for (const tegra::LevelLayout& lv : x.L.layout.levels)
        for (uint32_t layer = 0; layer < 2; ++layer)
            for (uint64_t off = 0; off < lv.linear_slice_size; off += 16, next += 5)   // 5 and 9 are coprime: every slice gets another sequence
                std::memcpy(x.lin.data() + lv.linear_offset + layer * lv.linear_slice_size + off, src.data() + (next % nsrc) * 16, 16);
    tile(x);

    TexCaps rgba;
    rgba.bc7_transcode = false;
    PreparedTexture d, b;
    CHECK_MSG(prepare_texture(*x.t, rgba, d), d.error);
    CHECK_MSG(prepare_texture(*x.t, TexCaps(), b), b.error);
    REQUIRE(d.level.size() == 4 && b.level.size() == 4);
    CHECK_EQ(b.format, VK_FORMAT_BC7_SRGB_BLOCK);
    a2b_options opt{};
    opt.quality = bc7_fast_preset();
    for (size_t i = 0; i < 4; ++i) {
        const tegra::LevelLayout& lv = x.L.layout.levels[i];
        CHECK_EQ(d.level[i].width, lv.width);
        CHECK_EQ(d.level[i].height, lv.height);
        const a2b_input in{A2B_ASTC_SRGB, lv.width, lv.height, 8, 8};
        const size_t px_bytes = static_cast<size_t>(lv.width) * lv.height * 4, bc7_bytes = static_cast<size_t>(a2b_output_size(&in));
        CHECK_EQ(d.level[i].size, px_bytes * 2);
        CHECK_EQ(b.level[i].size, bc7_bytes * 2);
        for (uint32_t layer = 0; layer < 2; ++layer) {
            const uint8_t* slice = x.lin.data() + lv.linear_offset + layer * lv.linear_slice_size;
            std::vector<uint8_t> want(px_bytes), want_bc7(bc7_bytes);
            CHECK_EQ(a2b_decode_astc_rgba8(&in, slice, static_cast<size_t>(lv.linear_slice_size), 1, want.data(), nullptr), A2B_OK);
            CHECK_MSG(std::memcmp(d.data.data() + d.level[i].offset + layer * px_bytes, want.data(), px_bytes) == 0,
                      "RGBA8 of level " + std::to_string(i) + " layer " + std::to_string(layer) + " is not in its place");
            a2b_format of{};
            CHECK_EQ(a2b_transcode(&in, slice, static_cast<size_t>(lv.linear_slice_size), &opt, &of, want_bc7.data(), want_bc7.size(), nullptr), A2B_OK);
            CHECK_EQ(of, A2B_BC7_SRGB);
            CHECK_MSG(std::memcmp(b.data.data() + b.level[i].offset + layer * bc7_bytes, want_bc7.data(), bc7_bytes) == 0,
                      "BC7 of level " + std::to_string(i) + " layer " + std::to_string(layer) + " is not in its place");
        }
    }
}
