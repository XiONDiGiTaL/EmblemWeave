// SPDX-License-Identifier: MIT
// nvdisasm-style text of a decoded instruction (SassInst.ToSassText). The spelling rules (operand forms, immediates,
// float formatting) were read from nvdisasm's own output over the corpus (see docs/AMPERE.md, section 6).
using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text;

namespace Nx.Shaders.Ampere
{
    public static class SassText
    {
        public static string Format(SassInst i)
        {
            var sb = new StringBuilder(64);
            if (!(i.Pred == 7 && !i.PredNeg))
            {
                sb.Append('@');
                if (i.PredNeg) sb.Append('!');
                sb.Append(PredName(i.Pred, i.PredIsUniform)).Append(' ');
            }
            sb.Append(i.Opcode);
            foreach (var m in i.Mods) sb.Append('.').Append(m);
            for (int k = 0; k < i.Operands.Count; k++)
            {
                sb.Append(k == 0 ? " " : ", ");
                AppendOperand(sb, i.Operands[k]);
            }
            sb.Append(" ;");
            return sb.ToString();
        }

        public static string RegName(int r) => r == 255 ? "RZ" : "R" + r.ToString(CultureInfo.InvariantCulture);
        public static string URegName(int r) => r == 63 ? "URZ" : "UR" + r.ToString(CultureInfo.InvariantCulture);
        public static string PredName(int p, bool uniform) => uniform ? (p == 7 ? "UPT" : "UP" + p) : (p == 7 ? "PT" : "P" + p);

        public static string Hex(long v) => v < 0 ? "-0x" + ((ulong)(-v)).ToString("x", CultureInfo.InvariantCulture) : "0x" + v.ToString("x", CultureInfo.InvariantCulture);

        public static string OperandText(Operand o)
        {
            var sb = new StringBuilder();
            AppendOperand(sb, o);
            return sb.ToString();
        }

        static void AppendOperand(StringBuilder sb, Operand o)
        {
            string core;
            switch (o.Kind)
            {
                case OperandKind.Reg: core = RegName(o.Reg); break;
                case OperandKind.UReg: core = URegName(o.Reg); break;
                case OperandKind.Pred: core = PredName(o.Reg, false); break;
                case OperandKind.UPred: core = PredName(o.Reg, true); break;
                case OperandKind.CBuf: core = CBufText(o); break;
                case OperandKind.Label: core = Hex(o.Offset); break;
                case OperandKind.Attr:
                    core = o.Text.Length > 0 ? o.Text : "a[" + (o.IndexReg >= 0 ? RegName(o.IndexReg) + (o.Offset != 0 ? "+" + Hex(o.Offset) : "") : Hex(o.Offset)) + "]";
                    break;
                default: core = o.Text; break; // Imm, FImm, Mem, Special, Other: spelled by the decoder
            }
            if (o.Not) sb.Append(o.Kind is OperandKind.Pred or OperandKind.UPred ? '!' : '~');
            if (o.Neg) sb.Append('-');
            if (o.Abs) sb.Append('|');
            sb.Append(core);
            // scalar sub-word selectors (.H1, .B2) are printed inside |..|, packed ones (.H0_H0) and .reuse outside [oracle]
            if (o.Suffixes != null)
                foreach (var s in o.Suffixes) if (IsScalarSel(s)) sb.Append(s);
            if (o.Abs) sb.Append('|');
            if (o.Suffixes != null)
                foreach (var s in o.Suffixes) if (!IsScalarSel(s)) sb.Append(s);
        }

        static bool IsScalarSel(string s) => s.Length == 3 && s[0] == '.' && (s[1] == 'H' || s[1] == 'B') && char.IsDigit(s[2]);

        static string CBufText(Operand o)
        {
            if (o.Text.Length > 0) return o.Text;
            string bank = o.BankIsUReg ? "cx[" + URegName(o.Bank) + "]" : "c[" + Hex(o.Bank) + "]";
            string idx;
            if (o.IndexReg >= 0)
            {
                string r = o.IndexIsUReg ? URegName(o.IndexReg) : RegName(o.IndexReg);
                idx = o.Offset != 0 ? r + "+" + Hex(o.Offset) : r;
            }
            else idx = Hex(o.Offset);
            return bank + "[" + idx + "]";
        }

        // ---- float immediates -------------------------------------------------------------------------------------

        /// nvdisasm spelling of an fp32 immediate: C "%.20g" of the exact value, +INF/-INF, +QNAN/-QNAN.
        public static string F32(uint bits)
        {
            if ((bits & 0x7f800000) == 0x7f800000)
            {
                bool neg = (bits >> 31) != 0;
                if ((bits & 0x7fffff) == 0) return neg ? "-INF " : "+INF ";
                return ((bits & 0x400000) != 0 ? (neg ? "-QNAN " : "+QNAN ") : (neg ? "-SNAN " : "+SNAN "));
            }
            return G20(BitConverter.Int32BitsToSingle((int)bits));
        }

        public static string F64(ulong bits)
        {
            if ((bits & 0x7ff0000000000000UL) == 0x7ff0000000000000UL)
            {
                bool neg = (bits >> 63) != 0;
                if ((bits & 0xfffffffffffffUL) == 0) return neg ? "-INF " : "+INF ";
                return (bits & 0x8000000000000UL) != 0 ? (neg ? "-QNAN " : "+QNAN ") : (neg ? "-SNAN " : "+SNAN ");
            }
            return G20(BitConverter.Int64BitsToDouble((long)bits));
        }

        public static string F16(uint bits)
        {
            bits &= 0xffff;
            if ((bits & 0x7c00) == 0x7c00)
            {
                bool neg = (bits >> 15) != 0;
                if ((bits & 0x3ff) == 0) return neg ? "-INF " : "+INF ";
                return (bits & 0x200) != 0 ? (neg ? "-QNAN " : "+QNAN ") : (neg ? "-SNAN " : "+SNAN ");
            }
            return G20((double)BitConverter.UInt16BitsToHalf((ushort)bits));
        }

        /// C printf("%.20g", v), except |v| >= 2^63, which nvdisasm prints as "%.20e" [oracle].
        public static string G20(double v)
        {
            if (v == 0) return BitConverter.DoubleToInt64Bits(v) < 0 ? "-0" : "0";
            if (Math.Abs(v) >= 9223372036854775808.0)
            {
                string x = v.ToString("E20", CultureInfo.InvariantCulture);
                int p = x.IndexOf('E');
                int ex = int.Parse(x.AsSpan(p + 1), NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture);
                return x.Substring(0, p) + "e" + (ex < 0 ? "-" : "+") + Math.Abs(ex).ToString("00", CultureInfo.InvariantCulture);
            }
            string e = v.ToString("E19", CultureInfo.InvariantCulture); // d.ddddddddddddddddddde+xxx, 20 significant digits
            int ePos = e.IndexOf('E');
            int exp = int.Parse(e.AsSpan(ePos + 1), NumberStyles.AllowLeadingSign, CultureInfo.InvariantCulture);
            bool neg = e[0] == '-';
            string digits = e.Substring(neg ? 1 : 0, ePos - (neg ? 1 : 0)).Replace(".", "");
            var sb = new StringBuilder();
            if (neg) sb.Append('-');
            if (exp < -4 || exp >= 20)
            {
                string m = digits.Substring(0, 1) + "." + digits.Substring(1);
                m = m.TrimEnd('0').TrimEnd('.');
                sb.Append(m).Append('e').Append(exp < 0 ? '-' : '+').Append(Math.Abs(exp).ToString("00", CultureInfo.InvariantCulture));
            }
            else if (exp >= 0)
            {
                string ip = digits.Substring(0, exp + 1), fp = digits.Substring(exp + 1).TrimEnd('0');
                sb.Append(ip);
                if (fp.Length > 0) sb.Append('.').Append(fp);
            }
            else
            {
                string fp = (new string('0', -exp - 1) + digits).TrimEnd('0');
                sb.Append("0.").Append(fp);
            }
            return sb.ToString();
        }
    }
}
