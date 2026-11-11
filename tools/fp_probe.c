/* Floating point probe for the differential campaign.
 *
 * The point is to test the lifter against real clang instruction selection,
 * not against hand written assembly. One function performs every scalar
 * floating point operation the lifter models, and each result is written to a
 * volatile sink so the optimiser cannot fold or delete it. No library call is
 * emitted: every operation here lowers to a hardware instruction, which is
 * exactly what the guest code contains.
 *
 * tools/verify_fp.sh compiles this file for AArch64, lifts the .text segment and
 * compares the emitted C against the reference interpreter register by register.
 */

static volatile float  g_float;
static volatile double g_double;
static volatile int    g_int32;
static volatile unsigned g_uint32;

float fp_sweep(float a, float b, int i, unsigned u, double p, double q)
{
    /* Two source arithmetic, both precisions. */
    g_float = a + b;
    g_float = a - b;
    g_float = a * b;
    g_float = a / b;
    g_double = p + q;
    g_double = p - q;
    g_double = p * q;
    g_double = p / q;

    /* One source: sign, magnitude, square root. */
    g_float = -a;
    g_float = __builtin_fabsf(a);
    g_float = __builtin_sqrtf(a);
    g_double = -p;
    g_double = __builtin_fabs(p);
    g_double = __builtin_sqrt(p);

    /* Three source: multiply accumulate and its negated forms. */
    g_float = a * b + a;
    g_float = -(a * b) - b;
    g_double = p * q + p;
    g_double = -(p * q) - q;

    /* Maximum and minimum, which compile to a compare plus select. */
    g_float = a > b ? a : b;
    g_float = a < b ? a : b;
    g_double = p > q ? p : q;
    g_double = p < q ? p : q;

    /* The rounding family. */
    g_float = __builtin_roundf(a);
    g_float = __builtin_floorf(a);
    g_float = __builtin_ceilf(a);
    g_float = __builtin_truncf(a);
    g_float = __builtin_rintf(a);
    g_double = __builtin_round(p);
    g_double = __builtin_floor(p);
    g_double = __builtin_ceil(p);
    g_double = __builtin_trunc(p);
    g_double = __builtin_rint(p);

    /* Precision conversion in both directions. */
    g_double = (double)a;
    g_float = (float)p;

    /* Floating point to integer, both widths. The unsigned forms are absent on
     * purpose: clang lowers them through a SIMD scalar rounding instruction
     * (`fcvtzs s0, s1`) that this decoder does not model yet, and a probe that
     * silently traps would not be a probe. */
    g_int32 = (int)a;
    g_int32 = (int)p;
    g_int32 = (int)__builtin_rintf(a);
    g_int32 = (int)__builtin_rint(p);

    /* Integer to floating point, signed and unsigned, both widths. */
    g_float = (float)i;
    g_float = (float)u;
    g_double = (double)i;
    g_double = (double)u;

    /* Short immediates. */
    g_float = 1.0f;
    g_float = 0.5f;
    g_float = -2.0f;
    g_float = 3.0f;
    g_double = 1.0;
    g_double = 2.0;

    /* Movement between the register files without conversion. */
    g_float = __builtin_bit_cast(float, (unsigned)i);
    g_int32 = __builtin_bit_cast(int, a);
    g_double = __builtin_bit_cast(double, (unsigned long long)u);
    g_int32 = (int)__builtin_bit_cast(unsigned long long, p);

    return g_float + (float)g_double + (float)g_int32 + (float)g_uint32;
}
