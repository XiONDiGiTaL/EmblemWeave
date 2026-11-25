// SPDX-License-Identifier: MIT
// Synthetic, game-independent programs exercise the real Ampere -> SPIR-V path.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using Nx.Shaders.Ampere;
using Ryujinx.Graphics.Shader;
using Ryujinx.Graphics.Shader.Translation;
using O = Nx.Shaders.Ampere.Operand;

namespace SpirvHarness
{
    static class WarpTests
    {
        static O R(int n) => new(OperandKind.Reg, Reg:n);
        static O P(int n) => new(OperandKind.Pred, Reg:n);
        static O U(int n) => new(OperandKind.UReg, Reg:n);
        static O I(uint n) => new(OperandKind.Imm, ImmBits:n);
        static O SR(string n) => new(OperandKind.Special, Text:n);
        // The SASS expansion has to stay bit-exact against the hardware abs, including the boundaries (INT_MIN abs stays INT_MIN).
        static void CheckIntegerAbsFormula()
        {
            foreach (int x in new[]{0,1,-1,2,-2,int.MaxValue,int.MinValue})
            {
                int expected = x < 0 ? unchecked(-x) : x;
                int got = unchecked(x * ((x >> 31) | 1));
                if (got != expected) throw new Exception($"integer abs formula: {x} -> {got}, expected {expected}");
            }
        }
        public static int Run(string output)
        {
            CheckIntegerAbsFormula();
            Directory.CreateDirectory(output);
            int count=0;
            void Test(string name, Action<Action<string,string,O[]>> emit)
            {
                var instructions=new List<SassInst>();
                void Add(string op,string mods,params O[] operands) => instructions.Add(new(
                    instructions.Count*16,0,0,7,false,false,op,mods.Split('.',StringSplitOptions.RemoveEmptyEntries),operands,1,0));
                for(int r=0;r<4;r++) Add("MOV","",R(r),I(0));
                Add("S2R","",R(4),SR("SR_LANEID"));
                Add("ISETP","LT.U32.AND",P(0),P(7),R(4),I(16),P(7));
                emit((op,mods,args)=>Add(op,mods,args));
                Add("EXIT","");
                byte[] header=new byte[0x100];
                BitConverter.GetBytes(5u<<10).CopyTo(header,0);
                BitConverter.GetBytes(15u).CopyTo(header,18*4);
                var gpu=new StaticGpuAccessor(header,0,0,ShaderStage.Fragment,null);
                var ctx=AmpereTranslator.CreateContext(gpu,new TranslationOptions(TargetLanguage.Spirv,TargetApi.Vulkan,TranslationFlags.None),instructions,out var program);
                var sp=ctx.Translate();
                if(program.Unsupported.Count!=0 || sp.BinaryCode==null) throw new Exception(name+": "+string.Join(";",program.Unsupported));
                // Verify that the exercised operation survived optimisation into the module.
                var ops = new HashSet<uint>();
                var ext = new HashSet<uint>();
                uint[] words = new uint[sp.BinaryCode.Length/4];
                Buffer.BlockCopy(sp.BinaryCode,0,words,0,sp.BinaryCode.Length);
                for(int i=5;i<words.Length;i+=(int)(words[i]>>16)) {
                    int wc=(int)(words[i]>>16);
                    if(wc==0) throw new Exception(name+": malformed SPIR-V");
                    uint opcode=words[i]&65535;
                    ops.Add(opcode);
                    if(opcode==12 && wc>=5) ext.Add(words[i+4]);   // OpExtInst: the instruction literal is the first operand
                }
                if ((name.StartsWith("shfl_") || name.StartsWith("redux_") || name.StartsWith("match")) && !ops.Contains(345))
                    throw new Exception(name+": missing subgroup shuffle");
                if ((name.StartsWith("vote_") || name.StartsWith("redux_") || name.StartsWith("match")) && !ops.Contains(339))
                    throw new Exception(name+": missing subgroup ballot");
                if (name.StartsWith("iabs") || name=="i2f_abs")
                {
                    if (AmpereTranslator.NativeIAbs)
                    {
                        if (!ext.Contains(5)) throw new Exception(name+": --native-iabs did not emit SAbs");
                        if (ops.Contains(132)) throw new Exception(name+": --native-iabs still emitted the multiply expansion");
                    }
                    else
                    {
                        if (ext.Contains(5)) throw new Exception(name+": default mode emitted SAbs");
                        if (!ops.Contains(132) || !ops.Contains(195) || !ops.Contains(197))
                            throw new Exception(name+": integer abs was not emitted as x * ((x >> 31) | 1)");
                    }
                }
                File.WriteAllBytes(Path.Combine(output,name+".spv"),sp.BinaryCode);
                count++;
            }
            foreach(string sr in new[]{"LANEID","EQMASK","LTMASK","LEMASK","GTMASK","GEMASK"})
                Test(sr,a=>a("S2R","",new[]{R(0),SR("SR_"+sr)}));
            foreach(string mode in new[]{"IDX","UP","DOWN","BFLY"})
                foreach(uint c in new uint[]{0,7,31,0x1800,0x1807,0x181f})
                    Test("shfl_"+mode+"_"+c,a=>{
                        a("SHFL",mode,new[]{P(1),R(0),R(4),I(39),I(c)});
                        a("SEL","",new[]{R(1),I(1),I(0),P(1)});
                    });
            foreach(string mode in new[]{"ANY","ALL","EQ"})
                Test("vote_"+mode,a=>{
                    a("VOTE",mode,new[]{R(0),P(1),P(0)});
                    a("SEL","",new[]{R(1),I(1),I(0),P(1)});
                });
            foreach(string mode in new[]{"SUM","AND","OR","XOR","MIN","MAX","MIN.S32","MAX.S32"})
                Test("redux_"+mode,a=>{a("REDUX",mode,new[]{U(0),R(4)});a("MOV","",new[]{R(0),U(0)});});
            Test("match",a=>{a("MATCH","ALL",new[]{P(1),R(0),R(4)});a("SEL","",new[]{R(1),I(1),I(0),P(1)});});
            Test("match64",a=>{a("MOV","",new[]{R(5),R(4)});a("MATCH","ALL.U64",new[]{P(1),R(0),R(4)});a("SEL","",new[]{R(1),I(1),I(0),P(1)});});
            Test("matchany",a=>a("MATCH","ANY",new[]{R(0),R(4)}));
            Test("matchany64",a=>{a("MOV","",new[]{R(5),R(4)});a("MATCH","ANY.U64",new[]{R(0),R(4)});});
            Test("iabs",a=>a("IABS","",new[]{R(0),R(4)}));
            Test("iabs_src_abs",a=>a("IABS","",new[]{R(0),new O(OperandKind.Reg, Reg:4, Abs:true)}));
            Test("i2f_abs",a=>a("I2F","",new[]{R(0),new O(OperandKind.Reg, Reg:4, Abs:true)}));
            Console.WriteLine($"PASS: {count} synthetic Ampere warp programs translated to {output}");
            return 0;
        }
    }
}
