// SPDX-License-Identifier: GPL-3.0-or-later
// SPIR-V of tests/gfx/test_nvn_shaderint.cpp, written for these tests (not game data): assembled with SPIRV-Tools v2026.4
// (spirv-as --target-env vulkan1.3) from the assembly in the comment above each array, and checked with spirv-val. Bindings and
// names follow the harness's scheme (tools/shaders, notes/shaders.md), so the runtime reads them as it reads translated shaders.
#pragma once
#include <cstdint>

/*
; full-screen triangle from gl_VertexIndex: (-1,-1) (3,-1) (-1,3)
               OpCapability Shader
               OpMemoryModel Logical GLSL450
               OpEntryPoint Vertex %main "main" %vid %pos
               OpDecorate %vid BuiltIn VertexIndex
               OpDecorate %pos BuiltIn Position
       %void = OpTypeVoid
         %fn = OpTypeFunction %void
        %int = OpTypeInt 32 1
      %float = OpTypeFloat 32
    %v4float = OpTypeVector %float 4
     %in_int = OpTypePointer Input %int
     %out_v4 = OpTypePointer Output %v4float
        %vid = OpVariable %in_int Input
        %pos = OpVariable %out_v4 Output
      %int_1 = OpConstant %int 1
      %int_2 = OpConstant %int 2
    %float_0 = OpConstant %float 0
    %float_1 = OpConstant %float 1
    %float_2 = OpConstant %float 2
       %main = OpFunction %void None %fn
          %l = OpLabel
          %i = OpLoad %int %vid
          %a = OpShiftLeftLogical %int %i %int_1
          %b = OpBitwiseAnd %int %a %int_2
          %c = OpBitwiseAnd %int %i %int_2
         %fx = OpConvertSToF %float %b
         %fy = OpConvertSToF %float %c
         %x2 = OpFMul %float %fx %float_2
          %x = OpFSub %float %x2 %float_1
         %y2 = OpFMul %float %fy %float_2
          %y = OpFSub %float %y2 %float_1
          %p = OpCompositeConstruct %v4float %x %y %float_0 %float_1
               OpStore %pos %p
               OpReturn
               OpFunctionEnd
*/
static const uint32_t kSpvFullscreenVs[] = {
    0x07230203, 0x00010600, 0x00070000, 0x0000001c, 0x00000000, 0x00020011, 0x00000001, 0x0003000e,
    0x00000000, 0x00000001, 0x0007000f, 0x00000000, 0x00000001, 0x6e69616d, 0x00000000, 0x00000002,
    0x00000003, 0x00040047, 0x00000002, 0x0000000b, 0x0000002a, 0x00040047, 0x00000003, 0x0000000b,
    0x00000000, 0x00020013, 0x00000004, 0x00030021, 0x00000005, 0x00000004, 0x00040015, 0x00000006,
    0x00000020, 0x00000001, 0x00030016, 0x00000007, 0x00000020, 0x00040017, 0x00000008, 0x00000007,
    0x00000004, 0x00040020, 0x00000009, 0x00000001, 0x00000006, 0x00040020, 0x0000000a, 0x00000003,
    0x00000008, 0x0004003b, 0x00000009, 0x00000002, 0x00000001, 0x0004003b, 0x0000000a, 0x00000003,
    0x00000003, 0x0004002b, 0x00000006, 0x0000000b, 0x00000001, 0x0004002b, 0x00000006, 0x0000000c,
    0x00000002, 0x0004002b, 0x00000007, 0x0000000d, 0x00000000, 0x0004002b, 0x00000007, 0x0000000e,
    0x3f800000, 0x0004002b, 0x00000007, 0x0000000f, 0x40000000, 0x00050036, 0x00000004, 0x00000001,
    0x00000000, 0x00000005, 0x000200f8, 0x00000010, 0x0004003d, 0x00000006, 0x00000011, 0x00000002,
    0x000500c4, 0x00000006, 0x00000012, 0x00000011, 0x0000000b, 0x000500c7, 0x00000006, 0x00000013,
    0x00000012, 0x0000000c, 0x000500c7, 0x00000006, 0x00000014, 0x00000011, 0x0000000c, 0x0004006f,
    0x00000007, 0x00000015, 0x00000013, 0x0004006f, 0x00000007, 0x00000016, 0x00000014, 0x00050085,
    0x00000007, 0x00000017, 0x00000015, 0x0000000f, 0x00050083, 0x00000007, 0x00000018, 0x00000017,
    0x0000000e, 0x00050085, 0x00000007, 0x00000019, 0x00000016, 0x0000000f, 0x00050083, 0x00000007,
    0x0000001a, 0x00000019, 0x0000000e, 0x00070050, 0x00000008, 0x0000001b, 0x00000018, 0x0000001a,
    0x0000000d, 0x0000000e, 0x0003003e, 0x00000003, 0x0000001b, 0x000100fd, 0x00010038,
};

/*
; colour = c[0x18] row 1 (bytes 0x10..0x1f): NVN2 graphics push constant slot 0 as the translator names it
               OpCapability Shader
               OpMemoryModel Logical GLSL450
               OpEntryPoint Fragment %main "main" %out0 %fp_c24
               OpExecutionMode %main OriginUpperLeft
               OpName %fp_c24 "fp_c24"
               OpDecorate %out0 Location 0
               OpDecorate %arr ArrayStride 16
               OpMemberDecorate %blk 0 Offset 0
               OpDecorate %blk Block
               OpDecorate %fp_c24 DescriptorSet 0
               OpDecorate %fp_c24 Binding 111
       %void = OpTypeVoid
         %fn = OpTypeFunction %void
        %int = OpTypeInt 32 1
       %uint = OpTypeInt 32 0
      %float = OpTypeFloat 32
    %v4float = OpTypeVector %float 4
    %uint_16 = OpConstant %uint 16
        %arr = OpTypeArray %v4float %uint_16
        %blk = OpTypeStruct %arr
      %u_blk = OpTypePointer Uniform %blk
       %u_v4 = OpTypePointer Uniform %v4float
     %out_v4 = OpTypePointer Output %v4float
     %fp_c24 = OpVariable %u_blk Uniform
       %out0 = OpVariable %out_v4 Output
      %int_0 = OpConstant %int 0
      %int_1 = OpConstant %int 1
       %main = OpFunction %void None %fn
          %l = OpLabel
          %p = OpAccessChain %u_v4 %fp_c24 %int_0 %int_1
          %v = OpLoad %v4float %p
               OpStore %out0 %v
               OpReturn
               OpFunctionEnd
*/
static const uint32_t kSpvPushFs[] = {
    0x07230203, 0x00010600, 0x00070000, 0x00000015, 0x00000000, 0x00020011, 0x00000001, 0x0003000e,
    0x00000000, 0x00000001, 0x0007000f, 0x00000004, 0x00000001, 0x6e69616d, 0x00000000, 0x00000002,
    0x00000003, 0x00030010, 0x00000001, 0x00000007, 0x00040005, 0x00000003, 0x635f7066, 0x00003432,
    0x00040047, 0x00000002, 0x0000001e, 0x00000000, 0x00040047, 0x00000004, 0x00000006, 0x00000010,
    0x00050048, 0x00000005, 0x00000000, 0x00000023, 0x00000000, 0x00030047, 0x00000005, 0x00000002,
    0x00040047, 0x00000003, 0x00000022, 0x00000000, 0x00040047, 0x00000003, 0x00000021, 0x0000006f,
    0x00020013, 0x00000006, 0x00030021, 0x00000007, 0x00000006, 0x00040015, 0x00000008, 0x00000020,
    0x00000001, 0x00040015, 0x00000009, 0x00000020, 0x00000000, 0x00030016, 0x0000000a, 0x00000020,
    0x00040017, 0x0000000b, 0x0000000a, 0x00000004, 0x0004002b, 0x00000009, 0x0000000c, 0x00000010,
    0x0004001c, 0x00000004, 0x0000000b, 0x0000000c, 0x0003001e, 0x00000005, 0x00000004, 0x00040020,
    0x0000000d, 0x00000002, 0x00000005, 0x00040020, 0x0000000e, 0x00000002, 0x0000000b, 0x00040020,
    0x0000000f, 0x00000003, 0x0000000b, 0x0004003b, 0x0000000d, 0x00000003, 0x00000002, 0x0004003b,
    0x0000000f, 0x00000002, 0x00000003, 0x0004002b, 0x00000008, 0x00000010, 0x00000000, 0x0004002b,
    0x00000008, 0x00000011, 0x00000001, 0x00050036, 0x00000006, 0x00000001, 0x00000000, 0x00000007,
    0x000200f8, 0x00000012, 0x00060041, 0x0000000e, 0x00000013, 0x00000003, 0x00000010, 0x00000011,
    0x0004003d, 0x0000000b, 0x00000014, 0x00000013, 0x0003003e, 0x00000002, 0x00000014, 0x000100fd,
    0x00010038,
};

/*
; colour = 2D pool texture (handle c[3] word 0) + cube pool texture (handle c[3] word 1), as the translator's pool lookups do it
               OpCapability Shader
               OpCapability SampledImageArrayNonUniformIndexing
               OpExtension "SPV_EXT_descriptor_indexing"
               OpMemoryModel Logical GLSL450
               OpEntryPoint Fragment %main "main" %out0 %fp_c3 %fp_t2d %fp_s %fp_tcube
               OpExecutionMode %main OriginUpperLeft
               OpName %fp_c3 "fp_c3"
               OpName %fp_t2d "fp_t2d_cb0_40000000"
               OpName %fp_s "fp_s_cb0_40000000"
               OpName %fp_tcube "fp_tcube_cb0_40000000"
               OpDecorate %out0 Location 0
               OpDecorate %arr ArrayStride 16
               OpMemberDecorate %blk 0 Offset 0
               OpDecorate %blk Block
               OpDecorate %fp_c3 DescriptorSet 0
               OpDecorate %fp_c3 Binding 22
               OpDecorate %fp_t2d DescriptorSet 2
               OpDecorate %fp_t2d Binding 640
               OpDecorate %fp_s DescriptorSet 2
               OpDecorate %fp_s Binding 641
               OpDecorate %fp_tcube DescriptorSet 2
               OpDecorate %fp_tcube Binding 642
       %void = OpTypeVoid
         %fn = OpTypeFunction %void
        %int = OpTypeInt 32 1
       %uint = OpTypeInt 32 0
      %float = OpTypeFloat 32
    %v2float = OpTypeVector %float 2
    %v3float = OpTypeVector %float 3
    %v4float = OpTypeVector %float 4
     %v4uint = OpTypeVector %uint 4
    %uint_16 = OpConstant %uint 16
 %uint_65536 = OpConstant %uint 65536
  %uint_4096 = OpConstant %uint 4096
    %uint_20 = OpConstant %uint 20
  %uint_mask = OpConstant %uint 1048575
        %arr = OpTypeArray %v4uint %uint_16
        %blk = OpTypeStruct %arr
      %u_blk = OpTypePointer Uniform %blk
     %u_uint = OpTypePointer Uniform %uint
      %img2d = OpTypeImage %float 2D 0 0 0 1 Unknown
    %imgcube = OpTypeImage %float Cube 0 0 0 1 Unknown
        %smp = OpTypeSampler
     %si2d = OpTypeSampledImage %img2d
   %sicube = OpTypeSampledImage %imgcube
     %arr2d = OpTypeArray %img2d %uint_65536
   %arrcube = OpTypeArray %imgcube %uint_65536
      %arrs = OpTypeArray %smp %uint_4096
   %uc_arr2d = OpTypePointer UniformConstant %arr2d
 %uc_arrcube = OpTypePointer UniformConstant %arrcube
    %uc_arrs = OpTypePointer UniformConstant %arrs
   %uc_img2d = OpTypePointer UniformConstant %img2d
 %uc_imgcube = OpTypePointer UniformConstant %imgcube
     %uc_smp = OpTypePointer UniformConstant %smp
     %out_v4 = OpTypePointer Output %v4float
      %fp_c3 = OpVariable %u_blk Uniform
     %fp_t2d = OpVariable %uc_arr2d UniformConstant
       %fp_s = OpVariable %uc_arrs UniformConstant
   %fp_tcube = OpVariable %uc_arrcube UniformConstant
       %out0 = OpVariable %out_v4 Output
      %int_0 = OpConstant %int 0
      %int_1 = OpConstant %int 1
    %float_0 = OpConstant %float 0
  %float_0_5 = OpConstant %float 0.5
    %float_1 = OpConstant %float 1
  %float_0_2 = OpConstant %float 0.2
  %float_0_3 = OpConstant %float 0.3
         %uv = OpConstantComposite %v2float %float_0_5 %float_0_5
        %dir = OpConstantComposite %v3float %float_1 %float_0_2 %float_0_3
       %main = OpFunction %void None %fn
          %l = OpLabel
        %hp0 = OpAccessChain %u_uint %fp_c3 %int_0 %int_0 %int_0
         %h0 = OpLoad %uint %hp0
        %hp1 = OpAccessChain %u_uint %fp_c3 %int_0 %int_0 %int_1
         %h1 = OpLoad %uint %hp1
        %t0i = OpBitwiseAnd %uint %h0 %uint_mask
        %s0i = OpShiftRightLogical %uint %h0 %uint_20
        %t1i = OpBitwiseAnd %uint %h1 %uint_mask
        %s1i = OpShiftRightLogical %uint %h1 %uint_20
        %t0p = OpAccessChain %uc_img2d %fp_t2d %t0i
         %t0 = OpLoad %img2d %t0p
        %s0p = OpAccessChain %uc_smp %fp_s %s0i
         %s0 = OpLoad %smp %s0p
        %si0 = OpSampledImage %si2d %t0 %s0
         %c0 = OpImageSampleExplicitLod %v4float %si0 %uv Lod %float_0
        %t1p = OpAccessChain %uc_imgcube %fp_tcube %t1i
         %t1 = OpLoad %imgcube %t1p
        %s1p = OpAccessChain %uc_smp %fp_s %s1i
         %s1 = OpLoad %smp %s1p
        %si1 = OpSampledImage %sicube %t1 %s1
         %c1 = OpImageSampleExplicitLod %v4float %si1 %dir Lod %float_0
          %c = OpFAdd %v4float %c0 %c1
               OpStore %out0 %c
               OpReturn
               OpFunctionEnd
*/
static const uint32_t kSpvPoolFs[] = {
    0x07230203, 0x00010600, 0x00070000, 0x00000047, 0x00000000, 0x00020011, 0x00000001, 0x00020011,
    0x000014bb, 0x0008000a, 0x5f565053, 0x5f545845, 0x63736564, 0x74706972, 0x695f726f, 0x7865646e,
    0x00676e69, 0x0003000e, 0x00000000, 0x00000001, 0x000a000f, 0x00000004, 0x00000001, 0x6e69616d,
    0x00000000, 0x00000002, 0x00000003, 0x00000004, 0x00000005, 0x00000006, 0x00030010, 0x00000001,
    0x00000007, 0x00040005, 0x00000003, 0x635f7066, 0x00000033, 0x00070005, 0x00000004, 0x745f7066,
    0x635f6432, 0x345f3062, 0x30303030, 0x00303030, 0x00070005, 0x00000005, 0x735f7066, 0x3062635f,
    0x3030345f, 0x30303030, 0x00000030, 0x00080005, 0x00000006, 0x745f7066, 0x65627563, 0x3062635f,
    0x3030345f, 0x30303030, 0x00000030, 0x00040047, 0x00000002, 0x0000001e, 0x00000000, 0x00040047,
    0x00000007, 0x00000006, 0x00000010, 0x00050048, 0x00000008, 0x00000000, 0x00000023, 0x00000000,
    0x00030047, 0x00000008, 0x00000002, 0x00040047, 0x00000003, 0x00000022, 0x00000000, 0x00040047,
    0x00000003, 0x00000021, 0x00000016, 0x00040047, 0x00000004, 0x00000022, 0x00000002, 0x00040047,
    0x00000004, 0x00000021, 0x00000280, 0x00040047, 0x00000005, 0x00000022, 0x00000002, 0x00040047,
    0x00000005, 0x00000021, 0x00000281, 0x00040047, 0x00000006, 0x00000022, 0x00000002, 0x00040047,
    0x00000006, 0x00000021, 0x00000282, 0x00020013, 0x00000009, 0x00030021, 0x0000000a, 0x00000009,
    0x00040015, 0x0000000b, 0x00000020, 0x00000001, 0x00040015, 0x0000000c, 0x00000020, 0x00000000,
    0x00030016, 0x0000000d, 0x00000020, 0x00040017, 0x0000000e, 0x0000000d, 0x00000002, 0x00040017,
    0x0000000f, 0x0000000d, 0x00000003, 0x00040017, 0x00000010, 0x0000000d, 0x00000004, 0x00040017,
    0x00000011, 0x0000000c, 0x00000004, 0x0004002b, 0x0000000c, 0x00000012, 0x00000010, 0x0004002b,
    0x0000000c, 0x00000013, 0x00010000, 0x0004002b, 0x0000000c, 0x00000014, 0x00001000, 0x0004002b,
    0x0000000c, 0x00000015, 0x00000014, 0x0004002b, 0x0000000c, 0x00000016, 0x000fffff, 0x0004001c,
    0x00000007, 0x00000011, 0x00000012, 0x0003001e, 0x00000008, 0x00000007, 0x00040020, 0x00000017,
    0x00000002, 0x00000008, 0x00040020, 0x00000018, 0x00000002, 0x0000000c, 0x00090019, 0x00000019,
    0x0000000d, 0x00000001, 0x00000000, 0x00000000, 0x00000000, 0x00000001, 0x00000000, 0x00090019,
    0x0000001a, 0x0000000d, 0x00000003, 0x00000000, 0x00000000, 0x00000000, 0x00000001, 0x00000000,
    0x0002001a, 0x0000001b, 0x0003001b, 0x0000001c, 0x00000019, 0x0003001b, 0x0000001d, 0x0000001a,
    0x0004001c, 0x0000001e, 0x00000019, 0x00000013, 0x0004001c, 0x0000001f, 0x0000001a, 0x00000013,
    0x0004001c, 0x00000020, 0x0000001b, 0x00000014, 0x00040020, 0x00000021, 0x00000000, 0x0000001e,
    0x00040020, 0x00000022, 0x00000000, 0x0000001f, 0x00040020, 0x00000023, 0x00000000, 0x00000020,
    0x00040020, 0x00000024, 0x00000000, 0x00000019, 0x00040020, 0x00000025, 0x00000000, 0x0000001a,
    0x00040020, 0x00000026, 0x00000000, 0x0000001b, 0x00040020, 0x00000027, 0x00000003, 0x00000010,
    0x0004003b, 0x00000017, 0x00000003, 0x00000002, 0x0004003b, 0x00000021, 0x00000004, 0x00000000,
    0x0004003b, 0x00000023, 0x00000005, 0x00000000, 0x0004003b, 0x00000022, 0x00000006, 0x00000000,
    0x0004003b, 0x00000027, 0x00000002, 0x00000003, 0x0004002b, 0x0000000b, 0x00000028, 0x00000000,
    0x0004002b, 0x0000000b, 0x00000029, 0x00000001, 0x0004002b, 0x0000000d, 0x0000002a, 0x00000000,
    0x0004002b, 0x0000000d, 0x0000002b, 0x3f000000, 0x0004002b, 0x0000000d, 0x0000002c, 0x3f800000,
    0x0004002b, 0x0000000d, 0x0000002d, 0x3e4ccccd, 0x0004002b, 0x0000000d, 0x0000002e, 0x3e99999a,
    0x0005002c, 0x0000000e, 0x0000002f, 0x0000002b, 0x0000002b, 0x0006002c, 0x0000000f, 0x00000030,
    0x0000002c, 0x0000002d, 0x0000002e, 0x00050036, 0x00000009, 0x00000001, 0x00000000, 0x0000000a,
    0x000200f8, 0x00000031, 0x00070041, 0x00000018, 0x00000032, 0x00000003, 0x00000028, 0x00000028,
    0x00000028, 0x0004003d, 0x0000000c, 0x00000033, 0x00000032, 0x00070041, 0x00000018, 0x00000034,
    0x00000003, 0x00000028, 0x00000028, 0x00000029, 0x0004003d, 0x0000000c, 0x00000035, 0x00000034,
    0x000500c7, 0x0000000c, 0x00000036, 0x00000033, 0x00000016, 0x000500c2, 0x0000000c, 0x00000037,
    0x00000033, 0x00000015, 0x000500c7, 0x0000000c, 0x00000038, 0x00000035, 0x00000016, 0x000500c2,
    0x0000000c, 0x00000039, 0x00000035, 0x00000015, 0x00050041, 0x00000024, 0x0000003a, 0x00000004,
    0x00000036, 0x0004003d, 0x00000019, 0x0000003b, 0x0000003a, 0x00050041, 0x00000026, 0x0000003c,
    0x00000005, 0x00000037, 0x0004003d, 0x0000001b, 0x0000003d, 0x0000003c, 0x00050056, 0x0000001c,
    0x0000003e, 0x0000003b, 0x0000003d, 0x00070058, 0x00000010, 0x0000003f, 0x0000003e, 0x0000002f,
    0x00000002, 0x0000002a, 0x00050041, 0x00000025, 0x00000040, 0x00000006, 0x00000038, 0x0004003d,
    0x0000001a, 0x00000041, 0x00000040, 0x00050041, 0x00000026, 0x00000042, 0x00000005, 0x00000039,
    0x0004003d, 0x0000001b, 0x00000043, 0x00000042, 0x00050056, 0x0000001d, 0x00000044, 0x00000041,
    0x00000043, 0x00070058, 0x00000010, 0x00000045, 0x00000044, 0x00000030, 0x00000002, 0x0000002a,
    0x00050081, 0x00000010, 0x00000046, 0x0000003f, 0x00000045, 0x0003003e, 0x00000002, 0x00000046,
    0x000100fd, 0x00010038,
};

/*
; d[0..1] = c[0] words 4, 5 (push words 0, 1), d[2..4] = c[0] words 1..3 (work-group counts), d[5] = c[0] word 31 (push word 27)
               OpCapability Shader
               OpMemoryModel Logical GLSL450
               OpEntryPoint GLCompute %main "main" %cp_c0 %cp_s0
               OpExecutionMode %main LocalSize 1 1 1
               OpName %cp_c0 "cp_c0"
               OpName %cp_s0 "cp_s0"
               OpDecorate %arr ArrayStride 16
               OpMemberDecorate %blk 0 Offset 0
               OpDecorate %blk Block
               OpDecorate %cp_c0 DescriptorSet 0
               OpDecorate %cp_c0 Binding 1
               OpDecorate %rarr ArrayStride 4
               OpMemberDecorate %sblk 0 Offset 0
               OpDecorate %sblk Block
               OpDecorate %cp_s0 DescriptorSet 1
               OpDecorate %cp_s0 Binding 0
       %void = OpTypeVoid
         %fn = OpTypeFunction %void
        %int = OpTypeInt 32 1
       %uint = OpTypeInt 32 0
     %v4uint = OpTypeVector %uint 4
    %uint_64 = OpConstant %uint 64
        %arr = OpTypeArray %v4uint %uint_64
        %blk = OpTypeStruct %arr
      %u_blk = OpTypePointer Uniform %blk
     %u_uint = OpTypePointer Uniform %uint
       %rarr = OpTypeRuntimeArray %uint
       %sblk = OpTypeStruct %rarr
     %sb_blk = OpTypePointer StorageBuffer %sblk
    %sb_uint = OpTypePointer StorageBuffer %uint
      %cp_c0 = OpVariable %u_blk Uniform
      %cp_s0 = OpVariable %sb_blk StorageBuffer
      %int_0 = OpConstant %int 0
      %int_1 = OpConstant %int 1
      %int_2 = OpConstant %int 2
      %int_3 = OpConstant %int 3
      %int_4 = OpConstant %int 4
      %int_5 = OpConstant %int 5
      %int_7 = OpConstant %int 7
       %main = OpFunction %void None %fn
          %l = OpLabel
         %p0 = OpAccessChain %u_uint %cp_c0 %int_0 %int_1 %int_0
         %v0 = OpLoad %uint %p0
         %q0 = OpAccessChain %sb_uint %cp_s0 %int_0 %int_0
               OpStore %q0 %v0
         %p1 = OpAccessChain %u_uint %cp_c0 %int_0 %int_1 %int_1
         %v1 = OpLoad %uint %p1
         %q1 = OpAccessChain %sb_uint %cp_s0 %int_0 %int_1
               OpStore %q1 %v1
         %p2 = OpAccessChain %u_uint %cp_c0 %int_0 %int_0 %int_1
         %v2 = OpLoad %uint %p2
         %q2 = OpAccessChain %sb_uint %cp_s0 %int_0 %int_2
               OpStore %q2 %v2
         %p3 = OpAccessChain %u_uint %cp_c0 %int_0 %int_0 %int_2
         %v3 = OpLoad %uint %p3
         %q3 = OpAccessChain %sb_uint %cp_s0 %int_0 %int_3
               OpStore %q3 %v3
         %p4 = OpAccessChain %u_uint %cp_c0 %int_0 %int_0 %int_3
         %v4 = OpLoad %uint %p4
         %q4 = OpAccessChain %sb_uint %cp_s0 %int_0 %int_4
               OpStore %q4 %v4
         %p5 = OpAccessChain %u_uint %cp_c0 %int_0 %int_7 %int_3
         %v5 = OpLoad %uint %p5
         %q5 = OpAccessChain %sb_uint %cp_s0 %int_0 %int_5
               OpStore %q5 %v5
               OpReturn
               OpFunctionEnd
*/
static const uint32_t kSpvPushCs[] = {
    0x07230203, 0x00010600, 0x00070000, 0x0000002c, 0x00000000, 0x00020011, 0x00000001, 0x0003000e,
    0x00000000, 0x00000001, 0x0007000f, 0x00000005, 0x00000001, 0x6e69616d, 0x00000000, 0x00000002,
    0x00000003, 0x00060010, 0x00000001, 0x00000011, 0x00000001, 0x00000001, 0x00000001, 0x00040005,
    0x00000002, 0x635f7063, 0x00000030, 0x00040005, 0x00000003, 0x735f7063, 0x00000030, 0x00040047,
    0x00000004, 0x00000006, 0x00000010, 0x00050048, 0x00000005, 0x00000000, 0x00000023, 0x00000000,
    0x00030047, 0x00000005, 0x00000002, 0x00040047, 0x00000002, 0x00000022, 0x00000000, 0x00040047,
    0x00000002, 0x00000021, 0x00000001, 0x00040047, 0x00000006, 0x00000006, 0x00000004, 0x00050048,
    0x00000007, 0x00000000, 0x00000023, 0x00000000, 0x00030047, 0x00000007, 0x00000002, 0x00040047,
    0x00000003, 0x00000022, 0x00000001, 0x00040047, 0x00000003, 0x00000021, 0x00000000, 0x00020013,
    0x00000008, 0x00030021, 0x00000009, 0x00000008, 0x00040015, 0x0000000a, 0x00000020, 0x00000001,
    0x00040015, 0x0000000b, 0x00000020, 0x00000000, 0x00040017, 0x0000000c, 0x0000000b, 0x00000004,
    0x0004002b, 0x0000000b, 0x0000000d, 0x00000040, 0x0004001c, 0x00000004, 0x0000000c, 0x0000000d,
    0x0003001e, 0x00000005, 0x00000004, 0x00040020, 0x0000000e, 0x00000002, 0x00000005, 0x00040020,
    0x0000000f, 0x00000002, 0x0000000b, 0x0003001d, 0x00000006, 0x0000000b, 0x0003001e, 0x00000007,
    0x00000006, 0x00040020, 0x00000010, 0x0000000c, 0x00000007, 0x00040020, 0x00000011, 0x0000000c,
    0x0000000b, 0x0004003b, 0x0000000e, 0x00000002, 0x00000002, 0x0004003b, 0x00000010, 0x00000003,
    0x0000000c, 0x0004002b, 0x0000000a, 0x00000012, 0x00000000, 0x0004002b, 0x0000000a, 0x00000013,
    0x00000001, 0x0004002b, 0x0000000a, 0x00000014, 0x00000002, 0x0004002b, 0x0000000a, 0x00000015,
    0x00000003, 0x0004002b, 0x0000000a, 0x00000016, 0x00000004, 0x0004002b, 0x0000000a, 0x00000017,
    0x00000005, 0x0004002b, 0x0000000a, 0x00000018, 0x00000007, 0x00050036, 0x00000008, 0x00000001,
    0x00000000, 0x00000009, 0x000200f8, 0x00000019, 0x00070041, 0x0000000f, 0x0000001a, 0x00000002,
    0x00000012, 0x00000013, 0x00000012, 0x0004003d, 0x0000000b, 0x0000001b, 0x0000001a, 0x00060041,
    0x00000011, 0x0000001c, 0x00000003, 0x00000012, 0x00000012, 0x0003003e, 0x0000001c, 0x0000001b,
    0x00070041, 0x0000000f, 0x0000001d, 0x00000002, 0x00000012, 0x00000013, 0x00000013, 0x0004003d,
    0x0000000b, 0x0000001e, 0x0000001d, 0x00060041, 0x00000011, 0x0000001f, 0x00000003, 0x00000012,
    0x00000013, 0x0003003e, 0x0000001f, 0x0000001e, 0x00070041, 0x0000000f, 0x00000020, 0x00000002,
    0x00000012, 0x00000012, 0x00000013, 0x0004003d, 0x0000000b, 0x00000021, 0x00000020, 0x00060041,
    0x00000011, 0x00000022, 0x00000003, 0x00000012, 0x00000014, 0x0003003e, 0x00000022, 0x00000021,
    0x00070041, 0x0000000f, 0x00000023, 0x00000002, 0x00000012, 0x00000012, 0x00000014, 0x0004003d,
    0x0000000b, 0x00000024, 0x00000023, 0x00060041, 0x00000011, 0x00000025, 0x00000003, 0x00000012,
    0x00000015, 0x0003003e, 0x00000025, 0x00000024, 0x00070041, 0x0000000f, 0x00000026, 0x00000002,
    0x00000012, 0x00000012, 0x00000015, 0x0004003d, 0x0000000b, 0x00000027, 0x00000026, 0x00060041,
    0x00000011, 0x00000028, 0x00000003, 0x00000012, 0x00000016, 0x0003003e, 0x00000028, 0x00000027,
    0x00070041, 0x0000000f, 0x00000029, 0x00000002, 0x00000012, 0x00000018, 0x00000015, 0x0004003d,
    0x0000000b, 0x0000002a, 0x00000029, 0x00060041, 0x00000011, 0x0000002b, 0x00000003, 0x00000012,
    0x00000017, 0x0003003e, 0x0000002b, 0x0000002a, 0x000100fd, 0x00010038,
};

/*
; d[0..1] = texel (0, 0) red and green (float bits) of the texture whose handle is c[0] word 0x24: with 28 push words and one storage
; buffer, that is image slot 0 (the shader reads the image bound with nvnCommandBufferBindImage)
               OpCapability Shader
               OpMemoryModel Logical GLSL450
               OpEntryPoint GLCompute %main "main" %cp_s0 %cp_t
               OpExecutionMode %main LocalSize 1 1 1
               OpName %cp_s0 "cp_s0"
               OpName %cp_t "cp_t_cb0_24"
               OpDecorate %rarr ArrayStride 4
               OpMemberDecorate %sblk 0 Offset 0
               OpDecorate %sblk Block
               OpDecorate %cp_s0 DescriptorSet 1
               OpDecorate %cp_s0 Binding 0
               OpDecorate %cp_t DescriptorSet 2
               OpDecorate %cp_t Binding 0
       %void = OpTypeVoid
         %fn = OpTypeFunction %void
        %int = OpTypeInt 32 1
       %uint = OpTypeInt 32 0
      %float = OpTypeFloat 32
    %v2int = OpTypeVector %int 2
    %v4float = OpTypeVector %float 4
       %rarr = OpTypeRuntimeArray %uint
       %sblk = OpTypeStruct %rarr
     %sb_blk = OpTypePointer StorageBuffer %sblk
    %sb_uint = OpTypePointer StorageBuffer %uint
        %img = OpTypeImage %float 2D 0 0 0 1 Unknown
       %simg = OpTypeSampledImage %img
    %uc_simg = OpTypePointer UniformConstant %simg
      %cp_s0 = OpVariable %sb_blk StorageBuffer
       %cp_t = OpVariable %uc_simg UniformConstant
      %int_0 = OpConstant %int 0
      %int_1 = OpConstant %int 1
     %origin = OpConstantComposite %v2int %int_0 %int_0
       %main = OpFunction %void None %fn
          %l = OpLabel
         %si = OpLoad %simg %cp_t
         %im = OpImage %img %si
          %c = OpImageFetch %v4float %im %origin Lod %int_0
          %r = OpCompositeExtract %float %c 0
          %g = OpCompositeExtract %float %c 1
         %ru = OpBitcast %uint %r
         %gu = OpBitcast %uint %g
         %q0 = OpAccessChain %sb_uint %cp_s0 %int_0 %int_0
               OpStore %q0 %ru
         %q1 = OpAccessChain %sb_uint %cp_s0 %int_0 %int_1
               OpStore %q1 %gu
               OpReturn
               OpFunctionEnd
*/
static const uint32_t kSpvImageTexCs[] = {
    0x07230203, 0x00010600, 0x00070000, 0x0000001f, 0x00000000, 0x00020011, 0x00000001, 0x0003000e,
    0x00000000, 0x00000001, 0x0007000f, 0x00000005, 0x00000001, 0x6e69616d, 0x00000000, 0x00000002,
    0x00000003, 0x00060010, 0x00000001, 0x00000011, 0x00000001, 0x00000001, 0x00000001, 0x00040005,
    0x00000002, 0x735f7063, 0x00000030, 0x00050005, 0x00000003, 0x745f7063, 0x3062635f, 0x0034325f,
    0x00040047, 0x00000004, 0x00000006, 0x00000004, 0x00050048, 0x00000005, 0x00000000, 0x00000023,
    0x00000000, 0x00030047, 0x00000005, 0x00000002, 0x00040047, 0x00000002, 0x00000022, 0x00000001,
    0x00040047, 0x00000002, 0x00000021, 0x00000000, 0x00040047, 0x00000003, 0x00000022, 0x00000002,
    0x00040047, 0x00000003, 0x00000021, 0x00000000, 0x00020013, 0x00000006, 0x00030021, 0x00000007,
    0x00000006, 0x00040015, 0x00000008, 0x00000020, 0x00000001, 0x00040015, 0x00000009, 0x00000020,
    0x00000000, 0x00030016, 0x0000000a, 0x00000020, 0x00040017, 0x0000000b, 0x00000008, 0x00000002,
    0x00040017, 0x0000000c, 0x0000000a, 0x00000004, 0x0003001d, 0x00000004, 0x00000009, 0x0003001e,
    0x00000005, 0x00000004, 0x00040020, 0x0000000d, 0x0000000c, 0x00000005, 0x00040020, 0x0000000e,
    0x0000000c, 0x00000009, 0x00090019, 0x0000000f, 0x0000000a, 0x00000001, 0x00000000, 0x00000000,
    0x00000000, 0x00000001, 0x00000000, 0x0003001b, 0x00000010, 0x0000000f, 0x00040020, 0x00000011,
    0x00000000, 0x00000010, 0x0004003b, 0x0000000d, 0x00000002, 0x0000000c, 0x0004003b, 0x00000011,
    0x00000003, 0x00000000, 0x0004002b, 0x00000008, 0x00000012, 0x00000000, 0x0004002b, 0x00000008,
    0x00000013, 0x00000001, 0x0005002c, 0x0000000b, 0x00000014, 0x00000012, 0x00000012, 0x00050036,
    0x00000006, 0x00000001, 0x00000000, 0x00000007, 0x000200f8, 0x00000015, 0x0004003d, 0x00000010,
    0x00000016, 0x00000003, 0x00040064, 0x0000000f, 0x00000017, 0x00000016, 0x0007005f, 0x0000000c,
    0x00000018, 0x00000017, 0x00000014, 0x00000002, 0x00000012, 0x00050051, 0x0000000a, 0x00000019,
    0x00000018, 0x00000000, 0x00050051, 0x0000000a, 0x0000001a, 0x00000018, 0x00000001, 0x0004007c,
    0x00000009, 0x0000001b, 0x00000019, 0x0004007c, 0x00000009, 0x0000001c, 0x0000001a, 0x00060041,
    0x0000000e, 0x0000001d, 0x00000002, 0x00000012, 0x00000012, 0x0003003e, 0x0000001d, 0x0000001b,
    0x00060041, 0x0000000e, 0x0000001e, 0x00000002, 0x00000012, 0x00000013, 0x0003003e, 0x0000001e,
    0x0000001c, 0x000100fd, 0x00010038,
};
