# Portable NVIDIA warps

The offline GPU accessor reports 128 to select Ryujinx's generic four-word
ballot and virtual-lane lowering. This is an upper bound for lowering, not a
hardware subgroup requirement. A single module works at Vulkan subgroup sizes
32, 64 and 128. The runtime still requests 32 where available.

* Guest lane: subgroup invocation ID & 31. Guest warp: invocation ID >> 5.
* Physical shuffle source: guest source | (invocation ID & ~31).
* Ballots and all five lane masks select the guest warp's uvec4 word.
* ANY/ALL/EQ use that ballot, comparing ALL against ballot(true), so inactive
  lanes and fragment helpers do not become false votes. A size <=32 select
  preserves native votes. REDUX and MATCH use these same ballot/shuffle helpers.
* MATCH.ANY (32/64-bit values) now builds the per-invocation equality mask;
  MATCH.ALL retains its predicate and all-active-mask-or-zero result.
* SHFL masks b to five bits, honours UP's clamp as well as the segment, and
  returns the original value and false predicate when invalid. Wider subgroups
  shuffle from the invocation itself for invalid sources. The native-32 source
  path is preserved. See NVIDIA's [SHFL semantics](https://docs.nvidia.com/cuda/parallel-thread-execution/#data-movement-and-conversion-instructions-shfl-sync).

Explicit gl_SubgroupSize <=32 selects let the driver discard warp base / mask
word selection / lane masking and the emulated vote result on native warp32.
This is a compiler-folding design, not a measured NVIDIA performance guarantee.
No variants or runtime specialization-constant plumbing are needed.

Subgroups smaller than 32 preserve the previous per-subgroup behavior: this
cannot reconstruct a guest warp spread across subgroups. Vulkan's uvec4 ballot
covers the supported 32/64/128 sizes. Undefined reads from inactive source lanes
remain undefined; active masks are never replaced with 0xffffffff. Fragment
helpers are governed by Vulkan subgroup participation rules. Derivatives remain
OpDPdx/OpDPdy, and swizzle-add uses lane & 3, which is independent of warp base.
WARPSYNC remains the existing reconvergence no-op in this translator; this
change does not add cross-invocation barriers or broaden participation.

## Cache and release path

The packaged tools/build/fefw_build.py comes from game-build/release/fefw_build.py;
it builds game code, not shaders. launcher/src-tauri/src/setup.rs runs shader
extraction and the packaged self-contained spirv_harness on the player's PC.
Its shader setup stamp now requires translator_version=2. Old/missing stamps
force full retranslation, and the source launcher rebuilds its harness first.
make-release.ps1 always publishes the current harness, including when reusing
an output staging directory. Runtime preload-pack entries are keyed by their
results.tsv line; changed translations invalidate those entries too.

## Verification

Build the translator, then run:

```
dotnet tools/shaders/build/bin/Release/net10.0/spirv_harness.dll --warp-tests work/warp/unit
python runtime/tests/gfx/generate_subgroup_probe.py --glslang PATH/TO/glslangValidator.exe
runtime/build/release/nxrt_gfx_tests.exe gfx_nvn_subgroup
```

The synthetic tests translate 45 game-independent Ampere programs through the
real frontend/backend and check that subgroup instructions survive optimization.
They cover special registers, every shuffle mode and segmented/clamped forms,
three votes, eight REDUX forms, and MATCH ALL/ANY with 32/64-bit inputs. They are
translation/structure tests, not GPU numerical tests of those emitted modules.
The authored GLSL GPU probe independently checks numerical warp isolation in
compute and fragment stages, exhaustive shuffle clamp cases, votes, ballots,
lane masks, reductions, matching, divergent masks and partial fragment coverage.
Required sizes unsupported by the device/stage are explicitly reported.

The local work/warp/ directory contains corpus comparisons, build/probe logs,
independent shader caches and copied-save game runs. Never copy these game
artifacts into the repository or release package.
