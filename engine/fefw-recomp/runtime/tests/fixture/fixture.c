/*
 * NXRT loader test fixture.
 * Synthetic AArch64 PIE image, linked at VA 0, shaped like an nx2elf output.
 * It is never executed on the host: only its layout, symbols and relocations
 * are tested. Built by make_fixture.cmd in this folder.
 * Freestanding C: no system headers.
 */

#define NOINL __attribute__((noinline, used))

/* ---- imports, resolved by the host at run time ---- */
extern long hle_data_import;
extern int hle_add(int, int);
extern int hle_mul(int, int);
extern void weak_missing(void) __attribute__((weak));

/* ---- exported code ---- */
NOINL int fn_a(void) { return 11; }
NOINL int fn_b(void) { return 22; }
NOINL int fn_c(void) { return 33; }

int init_flags;

NOINL void fixture_ctor_a(void) { init_flags |= 1; }
NOINL void fixture_ctor_b(void) { init_flags |= 2; }
NOINL __attribute__((destructor)) void fixture_dtor_a(void) { init_flags |= 4; }

/* lands in .init_array; lld should emit DT_INIT_ARRAY for it */
__attribute__((section(".init_array"), used))
void (*const init_array_entries[2])(void) = { fixture_ctor_a, fixture_ctor_b };

static volatile int entry_sink;
NOINL void fixture_entry(void) { entry_sink = fn_a(); }

/* ---- exported data ---- */
int values[4] = { 1, 2, 3, 4 };
int *ptr_to_values = &values[2];
int (*fn_table[3])(void) = { fn_a, fn_b, fn_c };
int (*const fn_table_ro[3])(void) = { fn_c, fn_b, fn_a };
const char fixture_msg[] = "NXRT-FIXTURE";
int bss_block[64];

/* ---- imports used from code and data ---- */
long *ptr_import = &hle_data_import;
NOINL long read_import(void) { return hle_data_import; }
NOINL int call_hle_add(int a, int b) { return hle_add(a, b); }
/* Address taken through a volatile pointer: clang emits a GOT load (GLOB_DAT) and an indirect call, not a PLT call. */
NOINL int call_hle_mul(int a, int b) { int (*volatile mul_fp)(int, int) = hle_mul; return mul_fp(a, b); }
void (*weak_ptr)(void) = weak_missing;

/* ---- thread-local storage (PT_TLS) ---- */
__thread int tls_a = 7;
__thread int tls_b[4] __attribute__((aligned(16))) = { 1, 2, 3, 4 };
__thread int tls_zero[8];
NOINL int get_tls_a(void) { return tls_a; }

/* ---- padding so that .text spans more than one 4 KiB page ---- */
#define STEP(k) s = s * (unsigned)((k) * 2654435761u + 1u) + (unsigned)x; s ^= s >> (((k) % 13) + 1);
#define BODY8(n) STEP(n + 0) STEP(n + 1) STEP(n + 2) STEP(n + 3) STEP(n + 4) STEP(n + 5) STEP(n + 6) STEP(n + 7)
#define PAD(n) NOINL int fn_pad_##n(int x) { unsigned s = (unsigned)x; BODY8(n) return (int)s; }

PAD(0)  PAD(1)  PAD(2)  PAD(3)  PAD(4)  PAD(5)  PAD(6)  PAD(7)  PAD(8)  PAD(9)
PAD(10) PAD(11) PAD(12) PAD(13) PAD(14) PAD(15) PAD(16) PAD(17) PAD(18) PAD(19)
PAD(20) PAD(21) PAD(22) PAD(23) PAD(24) PAD(25) PAD(26) PAD(27) PAD(28) PAD(29)
PAD(30) PAD(31) PAD(32) PAD(33) PAD(34) PAD(35) PAD(36) PAD(37) PAD(38) PAD(39)
PAD(40)
