// SPDX-License-Identifier: GPL-3.0-or-later
// Pipeline layouts and pipelines of the NVN back end (render thread only). A program (one translated shader per stage) has
// one pipeline layout of four descriptor sets with the Ryujinx binding scheme (set 0 uniform buffers, 1 storage buffers,
// 2 textures, 3 images; see docs/S4d.md). Graphics pipelines use dynamic rendering and as much dynamic state as Vulkan 1.3
// offers (viewports, scissors, cull mode, front face, topology, depth/stencil tests and ops, depth bias, stencil references
// and masks, blend constants, line width, vertex strides); everything else is part of the key.
#pragma once
#include <array>
#include <atomic>
#include <cstdint>
#include <functional>
#include <memory>
#include <shared_mutex>
#include <string>
#include <vector>

#include <volk.h>

#include "nxrt/gauge.h"
#include "nxrt/nvn_backend.h"

#include "nvn_core.h"
#include "nvn_shaders.h"
#include "nvn_spirv_patch.h"

namespace nxrt::nvn::be::gpu {

struct Frame;

struct BindingDesc {
    uint32_t set = 0, binding = 0;
    VkDescriptorType type = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER;
    VkShaderStageFlags stages = 0;
    const ShaderInfo* shader = nullptr;         // the stage module that declares it
    int index = -1;                             // into shader->refl.bindings
    int nvn_stage = 0;                          // NVNshaderStage of that module (0 vertex, 1 fragment, 2 geometry, 5 compute)
};

struct ProgLayout {
    VkDescriptorSetLayout sets[5] = {};         // 0..3 the program's own; kPoolSet (4) the shared pool arrays, when a stage samples through them
    VkPipelineLayout layout = VK_NULL_HANDLE;
    std::vector<BindingDesc> bindings;          // every used descriptor of every stage, sorted by (set, binding)
    bool has_set[5] = {};
    VkShaderStageFlags push_stages = 0;         // push constant range [0, push_bytes) of generated stages (ShaderInfo::push_bytes); 0 = none
    uint32_t push_bytes = 0;
    // The alpha-test variant of a program's layout (alpha_layout): the same sets and one more push constant range, FRAGMENT [0, 4) = the
    // reference of the game's alpha test (spirv_alpha_test reads it). Draws that run the alpha-test fragment variant use this layout.
    bool alpha_push = false;
    // Set 0 (the uniform buffers) is a push descriptor set ([nvn] push_uniforms): the replay pushes its buffers with each draw that changes
    // them (vkCmdPushDescriptorSetKHR) instead of finding or making a descriptor set for them. Every binding of it is a buffer.
    bool push_set0 = false;
    // Set 0 is pushed and nothing outside it depends on the uniform buffers (no texture handle read from a uniform block): a draw whose
    // only change is a uniform buffer binding pushes set 0 again and keeps the other sets it has bound ([nvn] ubo_only_rebind).
    bool ubo_only = false;
    // A per-program invariant of the replay: a storage buffer the program's shaders write (one that [nvn] gfx_ssbo_readonly proves read-only
    // does not count) or a storage image. A draw of this program then writes memory; worked out once here instead of over every binding a draw.
    bool writes_storage = false;

    // --- Invariant caches for fast descriptor set building (Phase 2.2) ---
    // Pre-computed VkWriteDescriptorSet templates for each set (0..3), static parts only.
    // The dynamic parts (pBufferInfo, pImageInfo, pTexelBufferView) are filled in at build time.
    struct WriteTemplate {
        VkWriteDescriptorSet write = {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET};
        uint32_t binding = 0;
        VkDescriptorType type = VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER;
        // Source classification for dynamic data lookup
        uint8_t source_class = 0;  // 0=uniform, 1=storage, 2=image, 3=sampler, 4=texel, 5=push_const, 6=driver_const, 7=support, 8=constant_data, 9=uniform_block
        int32_t source_index = -1; // index into the source array (ubo, ssbo, tex, img, etc.)
        int32_t nvn_stage = -1;    // NVN shader stage this binding comes from
    };
    std::array<std::vector<WriteTemplate>, 4> set_templates;  // templates for sets 0..3
    bool templates_built = false;
};

// [nvn] push_uniforms (default true): programs whose set 0 fits the device's push descriptor limit push it (ProgLayout::push_set0). False when the
// device has no VK_KHR_push_descriptor.
bool push_uniforms_on();

// Stages by NVN stage index (0 vertex, 1 fragment, 2 geometry, 5 compute); null = absent.
const ProgLayout* prog_layout(const ShaderInfo* const stages[6]);
// The layout of `base` with the alpha-test push constant (cached per base; `base` itself when it has push constants of its own).
const ProgLayout* alpha_layout(const ProgLayout& base);
// The same answer for the render thread's per-draw calls, through a thread-local direct-mapped cache ([nvn] alpha_cache): alpha_layout's
// mutex and map were taken for every alpha-tested draw. Layouts live until pipeline_shutdown.
const ProgLayout* alpha_layout_cached(const ProgLayout& base);
// [hdr] ui_nits, for the replay (render thread; each answer is worked out once per shader while the HDR scene runs, and not kept before):
// hdr_tonemap_shader: fs is the HDR tone-map shader. hdr_gain_shader: the scene-gain variant of fs fits (colour output 0's rgb times the
// push constant at offset 0, spirv_scale_output0; GraphicsKey::scene_gain selects it) and its words are ready for the pipeline.
bool hdr_tonemap_shader(const ShaderInfo* fs);
bool hdr_gain_shader(const ShaderInfo* fs);
// [hdr] grade_*: fs is the GT tone-map shader (extras::hdr_is_gt_tonemap), kept on the shader once the HDR scene runs.
bool hdr_gt_shader(const ShaderInfo* fs);
// [hdr] bloom: fs is the game's bloom composite (Fire Emblem Fortune's Weave, ImageProcessing.bfsha: bloom times a constant, added onto the
// scene before the tone map) and its output-scaled variant is ready. SDR and HDR.
bool bloom_shader(const ShaderInfo* fs);
// [hdr] sharpen: fs is the game's sharpen pass (Fire Emblem Fortune's Weave, filter.bfsha), replaced by kFefwSharpenFrag when it is not neutral.
bool sharpen_shader(const ShaderInfo* fs);
// [hdr] motion_blur, vignette, depth_of_field: fs is the game's motion blur (1), its vignette + gradation filter (2) or its depth-of-field
// circle-of-confusion pass (3) (Fire Emblem Fortune's Weave,
// ImageProcessing.bfsha), which then runs as its spirv_push_scale_cb variant (GraphicsKey::post); 0 for any other shader.
int post_scale_shader(const ShaderInfo* fs);

// Vertex input for the key.
struct KeyAttr {
    uint8_t location = 0, binding = 0;
    uint16_t offset = 0;
    uint32_t format = 0;                        // VkFormat
};

struct GraphicsKey {
    const ShaderInfo* stage[3] = {};            // vertex, fragment, geometry
    uint32_t color_format[8] = {};              // VkFormat per attachment slot (0 = unused slot)
    uint32_t color_count = 0;
    uint32_t depth_format = 0, stencil_format = 0;
    uint32_t samples = 1;
    uint32_t attr_count = 0;
    KeyAttr attr[16] = {};
    uint8_t binding_instance[16] = {};          // 1 = per-instance rate
    // blend, per colour slot
    uint8_t blend_enable[8] = {};
    uint8_t blend_src_rgb[8] = {}, blend_dst_rgb[8] = {}, blend_src_a[8] = {}, blend_dst_a[8] = {};   // VkBlendFactor
    uint8_t blend_op_rgb[8] = {}, blend_op_a[8] = {};                                                 // VkBlendOp
    uint8_t write_mask[8] = {};
    uint8_t logic_op_enable = 0, logic_op = 0;
    uint8_t topology = 0;                       // VkPrimitiveTopology (class; the exact topology is dynamic)
    uint8_t polygon_mode = 0;                   // VkPolygonMode
    uint8_t depth_clamp = 0, alpha_to_coverage = 0, depth_neg_one = 0;
    uint8_t patch_control_points = 0;
    uint8_t dummy_binding = 0;                  // 1 + the vertex binding that feeds unset attributes (stride 0, defaults buffer); 0 = none
    uint8_t xfb = 0;                            // the vertex shader runs with transform feedback (captured outputs, no fragment stage)
    uint8_t dual_src = 0;                       // a blend factor of an enabled attachment reads the second source (fragment output moved to Location 0, Index 1)
    uint8_t no_out = 0;                         // the pass has no colour attachment: the fragment shader's outputs become private variables
    uint8_t pad[0 + 1] = {};
    // Tessellation control and evaluation stages (null: none). Last, so that the prewarm key file keeps its record layout (it records the bytes
    // before these and never a tessellated key). A key with them has topology PATCH_LIST and patch_control_points = the NVN patch size.
    const ShaderInfo* tess[2] = {};
    // After the tessellation stages, so that the prewarm key file keeps its record layout (it records the bytes before them): the fixed-function
    // alpha test of the game ([nvn] alpha_test). alpha_func = NVNalphaFunc 1..7 (0: none / ALWAYS); the fragment shader then runs as a variant
    // that kills the invocation when the test fails (spirv_alpha_test); the reference is a push constant (ProgLayout::alpha_push).
    // alpha_layout = 1: built with the alpha-test layout (alpha_layout(): the program's sets plus the reference's push constant range). It is in
    // the key because begin_draw picks that layout before the pass is known: a draw whose pass then rules the variant out (no colour target 0,
    // dual-source blend) keeps alpha_func = 0 but still uses the other layout, and pipelines of incompatible layouts must not share a cache entry.
    // scene_gain = 1: [hdr] ui_nits, the fragment shader runs as its scene-gain variant (spirv_scale_output0), the factor in the same push
    // constant (with alpha_layout = 1). Exclusive with alpha_func.
    // grade = 1: the GT tone-map draw runs kFefwGtGradeFrag ([hdr] grade_*, highlight_saturation; extras/hdr_nvn.h), its settings in the
    // push constant (with alpha_layout = 1). Exclusive with alpha_func and scene_gain.
    // bloom = 1: the game's bloom composite runs as its output-scaled variant, the factor ([hdr] bloom) in the push constant (alpha_layout = 1).
    // sharpen = 1: the game's sharpen pass runs kFefwSharpenFrag ([hdr] sharpen, compare; the scene gain too), its settings in the push constant.
    // post = 1: the game's motion blur or vignette runs as its strength variant ([hdr] motion_blur, vignette), the strength in the push constant.
    uint8_t alpha_func = 0, alpha_layout = 0, scene_gain = 0, grade = 0, bloom = 0, sharpen = 0, post = 0, alpha_pad[1] = {};
    bool operator==(const GraphicsKey& o) const;
    uint64_t hash() const;
};

// The pipeline for a key, created on first use (logged; counters). Null on failure (the draw is skipped).
VkPipeline graphics_pipeline(const GraphicsKey& key, const ProgLayout& layout);
// The creation itself: no lookup, no bookkeeping; safe to call from any thread (the prewarm workers). Null on failure.
VkPipeline build_graphics_pipeline(const GraphicsKey& key, const ProgLayout& layout);

// The shader module each stage of a key runs (the translated one or a patched variant: varyings the next stage reads, integer attributes,
// stream-out, dual-source, private outputs, HDR tone map), by stage slot as in GraphicsKey::stage, then 3 tessellation control and 4 evaluation
// (GraphicsKey::tess); null = no such stage. False on a Vulkan error.
struct StageModules { VkShaderModule m[5] = {}; };
bool resolve_stage_modules(const GraphicsKey& key, StageModules* out);
// The vertex inputs the vertex shader declares as floats that this key feeds with integer attributes (the shader module is then a patched variant).
std::vector<SpvIntSlot> key_int_slots(const GraphicsKey& key);
// The dynamic state list every graphics pipeline and pipeline library uses.
const VkPipelineDynamicStateCreateInfo* pipeline_dynamic_state();
// The pNext of every translated shader stage: robust uniform buffer access ([nvn] robust_ubo), or null when off or not supported.
const void* ubo_robustness();
VkPipelineCache pipeline_cache_handle();
void note_pipeline_created(double ms, const char* what);   // counters and the dirty flag of the cache file

// Pipeline cache on disk ([nvn] pipeline_cache): written by a background thread while new pipelines appear and at shutdown.
bool pipeline_cache_dirty();
void pipeline_cache_save();
void pipeline_cache_flush();

// ---- prewarm (nvn_prewarm.cpp) ----------------------------------------------------------------------------------------------
// Every pipeline that had to be created at a draw is recorded in a key file ([nvn] pipeline_keys). The next run compiles the recorded
// pipelines of a program on worker threads as soon as the game sets the program's shaders, long before its first draw.
// Every key in the file is built from boot on the worker threads, in the order the keys were first used (the boot ones come first), with
// stand-in shaders from the shader cache (ShaderCache::load_stub: the same SPIR-V and reflection as the game's own), so nothing waits for the
// game to set a program's shaders. A draw finds the pipeline by content (shader hashes and key bytes), not by ShaderInfo pointer. The shader
// parts of every recorded key are also built ahead (nvn_gpl.cpp), so a draw whose pipeline is still being built links it from the parts.
struct PipeUpgrade;
void prewarm_init();                                                      // loads the key file, queues its keys, starts the cache saver (idempotent)
void prewarm_record(const GraphicsKey& key);                              // a draw used this key (built, linked or waiting for its parts)
// True when prewarm_record can store this key: no tessellation and none of the alpha-test / scene-gain fields the record does not hold.
bool prewarm_key_recordable(const GraphicsKey& key);
size_t prewarm_program(const ShaderInfo* const stages[6]);                // a program got its shaders: its recorded pipelines go first in line (how many)
// True with *out: the pipeline was built ahead. False: build it here, except that when a background build of it is under way and can_link()
// says the draw can link it from parts now, *later gets the whole pipeline that will replace the linked one (link with no upgrade of its own).
// busy (when given): the background build is under way and the draw cannot link it; instead of waiting for the build, false with *busy = true
// (the caller skips the draw and asks again at the next one).
bool prewarm_take(const GraphicsKey& key, VkPipeline* out, const std::function<bool()>& can_link, std::shared_ptr<PipeUpgrade>* later,
                  bool* busy = nullptr);
void prewarm_shutdown();                                                  // stops the workers (before the device is torn down)
struct PrewarmStats {
    uint64_t recorded = 0, loaded = 0, queued = 0, built = 0, taken = 0, waited = 0, claimed = 0, skipped_busy = 0;
    uint64_t failed = 0, promoted = 0, linked_meanwhile = 0;   // builds that failed; jobs moved ahead by a program set; draws linked while the whole build ran
    double wait_ms = 0;
    uint64_t bounded_drops = 0;    // keys dropped due to queue bound
    uint64_t metadata_lookups = 0; // metadata lookups performed
    uint64_t metadata_hits = 0;    // metadata hits (frequency > 1)
};
PrewarmStats prewarm_stats();
// Note a key request at draw time for prewarm metadata tracking (recency/frequency priority).
void prewarm_note_key_request(const GraphicsKey& key);
// Update prewarm frame counter and re-prioritize queue (called once per presented frame).
void prewarm_update_frame(uint64_t frame);
// `ints`: samplers that read integer textures (spirv_int_samplers); null when that variant cannot be made (the caller then uses the
// translated module with float views).
VkPipeline compute_pipeline(const ShaderInfo* cs, const ProgLayout& layout, const std::vector<SpvIntSampler>& ints = {});
// [nvn] compute_async (default prebuild): a compute program's pipeline is built on a background thread as soon as the game sets its shader, so
// the first dispatch (render thread) takes it ready instead of compiling it (tens to hundreds of ms on a cold driver cache). The dispatch waits
// for a build under way and builds one still queued itself; `skip` skips the dispatch instead while a build runs and frames are flowing (visual
// risk: a dispatch whose results later frames keep; for measurements). Any thread.
void compute_prebuild(const ShaderInfo* cs);
void compute_progress_log();                     // one log line when something changed since the last call
VkShaderModule shader_module(const ShaderInfo* s);
// A content id of a shader module (hash of the words it was made from): two modules made from the same words have the same id, so the shader
// parts built for the stand-in shaders of the key file are found again by the game's own. Any thread.
uint64_t module_content_id(VkShaderModule m);
uint64_t pipelines_built_at_draw();          // whole pipelines the render thread built itself at a draw (session)
// Bytes of one captured vertex of the vertex shader's transform feedback variant (0: none could be made).
uint32_t xfb_stride(const ShaderInfo* vs);

// Persistent buffers every draw may bind: the driver constants (c[0]) image and a shader's constant data (c[1]).
VkBuffer constant_data_buffer(const ShaderInfo* s, VkDeviceSize* range);
VkBuffer driver_constants_buffer(VkDeviceSize* range);

// ---- texture and sampler pools as descriptor arrays (kPoolSet, nvn_shaders.h) -------------------------------------------------------
// One set layout for every program that samples through the pools: kPoolSamplers (4096 samplers), kPoolTex2D and kPoolTexCube (65536
// sampled images each), partially bound and updated after bind. Null when the device cannot (descriptor indexing, its limits): those
// programs get no layout and their draws are skipped.
VkDescriptorSetLayout pool_set_layout();
// The set for a texture and sampler pool pair, brought up to date with the registrations since it was last current: only the ids changed since
// are written. A set the GPU may still be reading is left alone and another one of the pair's few is made current instead. Element i of
// the 2D / cube array is texture id i viewed as that shape (a black dummy when it is not one, or not registered); element j of the
// samplers is sampler id j. Null without a pool or a layout.
VkDescriptorSet pool_set(TexturePool* tp, SamplerPool* sp, Frame& f);
// The textures registered in tp that are not ready to be sampled (written by the CPU since the last upload, or not in their sampled
// layout), appended to `out`: the replay uploads and transitions them before a draw that samples through the pool. Cheap when nothing
// moved: it looks at the ids registered since the last call and the textures tex_transition moved since (pool_note_layout), and at every
// registered texture only when the CPU wrote textures since (g_tex_scan_epoch).
void pool_textures_to_prepare(TexturePool* tp, std::vector<Texture*>& out);
// The texture's first upload waits for a later frame ([nvn] pool_upload_ms): until this frame ends it is left out of the lists every draw looks
// through (its pool elements keep the dummy); the next frame's first look takes it up again. Render thread.
void pool_hold(Texture* t);
// How many textures the last frame that held any held back (the backlog the pool upload budget looks at).
size_t pool_held_backlog();
// Some registration, layout move or CPU write since the pair was last made current: the replay sets its bindings again. O(1).
bool pool_state_moved(TexturePool* tp, SamplerPool* sp);
void pool_sets_shutdown();                       // device idle

// ---- pipeline libraries (nvn_gpl.cpp, VK_EXT_graphics_pipeline_library) ------------------------------------------------------
// A pipeline is linked from four parts: vertex input, pre-rasterisation shaders (vertex/geometry code + raster state), fragment shader
// (fragment code), fragment output (blend and attachment formats). The two shader parts hold all the compile time and depend on very little
// state (the shaders of a program and three raster flags), so they are compiled by background threads when the game sets a program's
// shaders, long before the draw whose blend state and formats are only known then. The draw links the parts.
bool gpl_active();                                                   // enabled and supported (first call reads the settings)
// The optimised pipeline that replaces a linked one: made by a background thread, taken by the draw side on a later lookup (state 1 = ready;
// pipe may be null when the build failed). The linked pipeline stays in use until then.
struct PipeUpgrade {
    std::atomic<int> state{0};
    VkPipeline pipe = VK_NULL_HANDLE;
};
// null: not possible for this key, build it whole. *up (when given) gets the upgrade the linked pipeline will be replaced by (null: none queued).
VkPipeline gpl_link(const GraphicsKey& key, const ProgLayout& layout, std::shared_ptr<PipeUpgrade>* up = nullptr, bool* pending = nullptr);
// [nvn] gpl_async: a draw whose shader parts are not compiled yet gets no pipeline while frames are flowing (*pending = true: skip it and ask
// again later); the parts compile on the background threads ahead of everything else. While the game loads (no present for 100 ms) the draw
// builds them itself as before, so one-time renders of a loading screen are never lost.
void gpl_note_presents(uint64_t presents);   // the replay, once per submission: when presents were last seen moving
bool gpl_ab_whole();                                                 // measurement only ([nvn] gpl_ab): true in the windows that run the optimised pipelines
void gpl_take_upgrade(VkPipeline& current, VkPipeline& alt, std::shared_ptr<PipeUpgrade>& up);   // render thread: swaps in the finished optimised pipeline (the old one is destroyed once the GPU is done with it)
void gpl_program(const ShaderInfo* const stages[6], bool covered);   // a program got its shaders: queue its shader parts (covered: the key file has pipelines for it, skip)
void gpl_program_bound(const ShaderInfo* const stages[6]);           // a command buffer was recorded with the program bound: its parts go first
// The parts (all four) of a pipeline the key file holds, built on the background threads. `resolve` runs there and gives the key and layout
// (false: nothing to build). front = ahead of the other key-file jobs (the game just set one of its shaders).
void gpl_prebuild(std::function<bool(GraphicsKey&, const ProgLayout*&)> resolve, bool front);
// True when the draw can link this key now: eligible, and its two shader parts are built.
bool gpl_ready(const GraphicsKey& key, const ProgLayout& layout);
bool gpl_frames_flowing();                                           // a present within the last 100 ms (draws may be skipped rather than wait)
// A job for the pipeline-library threads (compute pipelines, compute_prebuild): ahead of the hot programs' parts. False when they are off.
bool gpl_queue_task(std::function<void()> fn);
void gpl_progress_log();                                             // one log line when something changed since the last call
void gpl_stop();                                                     // stops the workers (before modules and layouts go)
void gpl_shutdown();                                                 // destroys the parts (after the pipelines)
struct GplStats {
    uint64_t upgrades_queued = 0, upgrades_done = 0, upgrades_failed = 0, upgrades_taken = 0;
    double upgrade_ms = 0;
    uint64_t linked = 0, link_failed = 0, parts_ahead = 0, parts_at_draw = 0, parts_waited = 0, parts_hit = 0, jobs = 0, jobs_skipped = 0;
    uint64_t jobs_pending = 0;                       // shader-part jobs queued or running at this moment (0 once every job queued so far has finished)
    uint64_t prog_set = 0, prog_covered = 0, prog_new = 0, prog_bound = 0, prog_bound_new = 0, jobs_run = 0, jobs_no_layout = 0, jobs_no_modules = 0;
    uint64_t draws_deferred = 0, parts_async = 0;    // [nvn] gpl_async: draws skipped while their parts compiled, parts compiled for them
    uint64_t prog_deferred = 0;                      // programs the game set but never drew with, left to build at their first record ([nvn] gpl_cold = false)
    uint64_t prog_single = 0;                        // programs with only one of vertex and fragment shader (Pokopia sets them apart): nothing to queue per program
    uint64_t prebuild_jobs = 0, prebuild_run = 0;    // key-file keys whose parts were queued, run
    double link_ms = 0, draw_part_ms = 0, wait_ms = 0, worker_ms = 0;
    double paced_ms = 0;                             // time the background threads spent stepped aside for late frames ([nvn] compile_pace_ms)
};
GplStats gpl_stats();
void set_compile_thread_priority(bool background = true);                                  // [nvn] compile_priority for the threads that build pipelines in the background
// [nvn] compile_pace_ms / compile_pace_after_ms: called by a background compile thread after each speculative job (a cold part, a
// key-file build ahead, an upgrade). When the last presented frame (or the one in progress) is over compile_pace_after_ms, the thread
// sleeps 1 ms plus half the lateness, at most compile_pace_ms, so an area load's CPU work is not slowed by compiles nothing waits for.
// Never before the second presented frame (the boot prewarm runs at full speed); never after work a draw waits for or will soon (a
// part or key-file pipeline moved ahead because the game set its program), and never while such work is queued. Technique from
// EmblemWeave (moistman7).
void pace_shader_work();

// ---- shader preparation (nvn_gpl.cpp, S24) ---------------------------------------------------------------------------------
// Compiles the shader parts of every program in a list (setup writes it from the player's own game files: <shader cache>/../programs.bin)
// before the game runs, so the driver has them in its cache and the game's own requests are fast. The parts are thrown away as soon as they exist.
using PrepareOptions = nxrt::nvn::ShaderPrepareOptions;
using PrepareResult = nxrt::nvn::ShaderPrepareResult;
bool gpl_prepare(const PrepareOptions& o, PrepareResult* r);       // false: nothing could be done (see r->error)
void module_cache_clear();                                          // destroys every cached shader module and variant (nothing may be creating pipelines)

// create_module's fallback for a device without shaderFloat64: lowers a scalar-double module to 32-bit and logs the
// one-time outcome for `what`. False (lowered untouched) when the module has an unsupported double shape.
bool lower_f64_for_module(const std::vector<uint32_t>& words, const char* what, std::vector<uint32_t>& lowered);

void pipeline_shutdown();
size_t pipeline_count();

// Pipelines being compiled right now, on any thread (a frame that ran long while this was non-zero was waiting on, or competing with, a compile): the
// hitch log shows it. Every vkCreateGraphicsPipelines of the back end goes through the wrapper.
inline std::atomic<int>& compiles_in_flight() {
    static std::atomic<int> n{0};
    return n;
}
void configure_guest_subgroup(VkPipelineShaderStageCreateInfo& stage,
                              VkPipelineShaderStageRequiredSubgroupSizeCreateInfo& size);

// Intel's Windows driver (igvk64.dll) faulted inside vkGetPipelineCacheData on the cache-save thread while combat compiled new pipelines
// into the same cache (#124), although the spec makes the cache internally synchronized. With [nvn] pipeline_cache_lock on (auto: Intel
// GPUs only) compiles into the shared cache hold this shared and the save holds it exclusively; elsewhere it is not taken.
std::shared_mutex& pipeline_cache_mutex();
bool pipeline_cache_locked();

inline VkResult create_graphics_pipeline_tracked(VkDevice dev, VkPipelineCache cache, const VkGraphicsPipelineCreateInfo* gi, VkPipeline* out) {
    std::shared_lock<std::shared_mutex> cache_lk;
    if (cache != VK_NULL_HANDLE && pipeline_cache_locked()) cache_lk = std::shared_lock<std::shared_mutex>(pipeline_cache_mutex());
    compiles_in_flight().fetch_add(1, std::memory_order_relaxed);
    const uint64_t t0 = nxrt::hitch_ticks();
    // Centralised here so whole pipelines, GPL parts and prewarm workers agree.
    auto info = *gi;
    std::vector<VkPipelineShaderStageCreateInfo> stages;
    std::vector<VkPipelineShaderStageRequiredSubgroupSizeCreateInfo> sizes(gi->stageCount);
    if (gi->stageCount) stages.assign(gi->pStages, gi->pStages + gi->stageCount);
    for (size_t i = 0; i < stages.size(); ++i) configure_guest_subgroup(stages[i], sizes[i]);
    info.pStages = stages.data();
    const VkResult r = vkCreateGraphicsPipelines(dev, cache, 1, &info, nullptr, out);
    if (nxrt::hitch_on_render_thread()) nxrt::hitch_phase_add(nxrt::kHpRenderPipeline, nxrt::hitch_ticks() - t0);
    compiles_in_flight().fetch_sub(1, std::memory_order_relaxed);
    return r;
}

}  // namespace nxrt::nvn::be::gpu
