/* mx_batch.c - every .mxs stage file of a folder through the Maxwell translator (mx_translate.c) and then the Vulkan
 * GLSL compiler (glslang) and SPIR-V validator (spirv-val), offline, several at a time.
 *
 * Derived from smo-recomp tests/mx_all.c: same stage file format, same translator settings (target 1, binding_base 40,
 * ssbo_base 60, tex_base 40, image_base 72, wmask_binding 78), same geometry variants (points, lines, triangles).
 * Differences: the compiler is the Vulkan SDK's glslang.exe (not smo_glslang.dll), every result is also checked with
 * spirv-val, the stages run in parallel, and a results table is written.
 *
 * Copyright (C) the smo-recomp authors and contributors. This file is part of a GPL-3.0 work (smo-recomp, see
 * tools/shaders/licenses/LICENSE.smo-recomp.GPL-3.0). It is free software: you can redistribute it and/or modify it under
 * the terms of the GNU General Public License as published by the Free Software Foundation, either version 3 of the
 * License, or (at your option) any later version. It is distributed WITHOUT ANY WARRANTY; see the license for details.
 *
 *   mx_batch.exe <folder of .mxs> --out <dir> [--glslang <path>] [--spirv-val <path>] [--jobs N]
 *
 * For each <name>.mxs: <out>\<name>.glsl (geometry: <name>.points.glsl, <name>.lines.glsl, <name>.triangles.glsl),
 * <out>\<name>.spv (same suffixes), and one row in <out>\results.tsv: name, stage, status, instructions, spv_bytes, error.
 * status: ok | not-translated | compile-failed | invalid | skipped (tessellation, the renderer has none) | unreadable.
 * Exit status: 0 when every stage that is not skipped is ok, 1 otherwise, 2 on a usage error.
 *
 * Stage file (little endian): "MXS1", u32 stage (0 vertex, 1 fragment, 2 geometry, 3 tess control, 4 tess evaluation,
 * 5 compute), u32 code bytes, u32 constant data bytes, u32 work group x, y, z, u32 shared bytes, u32 local bytes, the code,
 * the constant data. Written by scripts/corpus_to_mxs.py (or check_all_shaders.py in smo-recomp).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <windows.h>
#include "mx_translate.h"

#define DEF_GLSLANG   "C:\\VulkanSDK\\1.4.363.0\\Bin\\glslang.exe"
#define DEF_SPIRV_VAL "C:\\VulkanSDK\\1.4.363.0\\Bin\\spirv-val.exe"
#define TARGET_ENV    "vulkan1.3"
#define NSTAGE        6
#define MAX_JOBS      64

static const char *STAGE_NAMES[NSTAGE] = {"vertex", "fragment", "geometry", "tess_control", "tess_eval", "compute"};
static const char *STAGE_FLAG[NSTAGE] = {"vert", "frag", "geom", "tesc", "tese", "comp"};
static const char *GEOM_SUFFIX[3] = {"points", "lines", "triangles"};
static const char *STATUS_NAMES[6] = {"ok", "not-translated", "compile-failed", "invalid", "skipped", "unreadable"};

typedef struct {
    char name[MAX_PATH];        /* file name without .mxs */
    char stage[16];             /* STAGE_NAMES entry, or "?" when unreadable */
    char status[24];            /* STATUS_NAMES entry */
    int insns;
    unsigned long long spv_bytes;
    char error[320];            /* first line of the reason, tabs and newlines removed */
} Res;

typedef struct {
    char key[400];              /* status + reason */
    char status[24];
    int count;
    char example[MAX_PATH];
} Group;

static char g_folder[MAX_PATH], g_out[MAX_PATH];
static char g_glslang[MAX_PATH] = DEF_GLSLANG, g_spirv_val[MAX_PATH] = DEF_SPIRV_VAL;
static char **g_files;
static int g_nfiles;
static Res *g_res;
static volatile LONG g_next, g_done;
static CRITICAL_SECTION g_print_cs, g_spawn_cs;
static HANDLE g_nul;

static void set_text(char *dst, size_t cap, const char *s) {
    size_t i;
    for (i = 0; s[i] && i + 1 < cap; i++) {
        char c = s[i];
        dst[i] = (c == '\t' || c == '\r' || c == '\n') ? ' ' : c;
    }
    dst[i] = 0;
}

/* the first line of a tool's output that names an error, else its first non-empty line */
static void first_error(const char *text, char *dst, size_t cap) {
    char line[400], pick[400];
    const char *q = text;
    int have = 0;
    pick[0] = 0;
    while (*q) {
        const char *e = strchr(q, '\n');
        size_t n = e ? (size_t)(e - q) : strlen(q);
        if (n >= sizeof line) n = sizeof line - 1;
        memcpy(line, q, n);
        line[n] = 0;
        char *s = line;
        while (*s == ' ' || *s == '\t' || *s == '\r') s++;
        size_t l = strlen(s);
        while (l && (s[l - 1] == ' ' || s[l - 1] == '\t' || s[l - 1] == '\r')) s[--l] = 0;
        if (l) {
            if (!have) { snprintf(pick, sizeof pick, "%s", s); have = 1; }
            if (strstr(s, "ERROR") || strstr(s, "error")) { snprintf(pick, sizeof pick, "%s", s); break; }
        }
        if (!e) break;
        q = e + 1;
    }
    if (!have) snprintf(pick, sizeof pick, "(no message)");
    set_text(dst, cap, pick);
}

/* runs a command line with stdout and stderr captured into text (cap bytes); returns its exit code, -1 if it did not start */
static int run_capture(char *cmd, char *text, size_t cap) {
    SECURITY_ATTRIBUTES sa;
    STARTUPINFOA si;
    PROCESS_INFORMATION pi;
    HANDLE rd = NULL, wr = NULL;
    DWORD rc = (DWORD)-1, got = 0;
    size_t used = 0;
    BOOL ok = FALSE;
    text[0] = 0;
    memset(&sa, 0, sizeof sa);
    sa.nLength = sizeof sa;
    sa.bInheritHandle = TRUE;
    memset(&pi, 0, sizeof pi);
    /* the pipe's write end is inherited by the child only: spawning is serialised so no other child inherits it */
    EnterCriticalSection(&g_spawn_cs);
    if (CreatePipe(&rd, &wr, &sa, 0)) {
        SetHandleInformation(rd, HANDLE_FLAG_INHERIT, 0);
        memset(&si, 0, sizeof si);
        si.cb = sizeof si;
        si.dwFlags = STARTF_USESTDHANDLES;
        si.hStdInput = g_nul;
        si.hStdOutput = wr;
        si.hStdError = wr;
        ok = CreateProcessA(NULL, cmd, NULL, NULL, TRUE, CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
        CloseHandle(wr);
        wr = NULL;
    }
    LeaveCriticalSection(&g_spawn_cs);
    if (!ok) {
        if (rd) CloseHandle(rd);
        return -1;
    }
    for (;;) {
        char buf[4096];
        if (!ReadFile(rd, buf, sizeof buf, &got, NULL) || got == 0) break;
        if (used + 1 < cap) {
            size_t take = got;
            if (used + take + 1 > cap) take = cap - 1 - used;
            memcpy(text + used, buf, take);
            used += take;
            text[used] = 0;
        }
    }
    CloseHandle(rd);
    WaitForSingleObject(pi.hProcess, INFINITE);
    GetExitCodeProcess(pi.hProcess, &rc);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    return (int)rc;
}

static int file_size(const char *path, unsigned long long *size) {
    WIN32_FILE_ATTRIBUTE_DATA fa;
    if (!GetFileAttributesExA(path, GetFileExInfoStandard, &fa)) return 0;
    *size = ((unsigned long long)fa.nFileSizeHigh << 32) | fa.nFileSizeLow;
    return 1;
}

static void do_job(int idx) {
    Res *r = &g_res[idx];
    char path[MAX_PATH * 2], base[MAX_PATH], text[4096], g[MAX_PATH * 2], spv[MAX_PATH * 2], cmd[MAX_PATH * 4];
    FILE *f;
    uint32_t hd[9];
    uint8_t *code = NULL, *cdata = NULL;
    int good, stage, variants, var, bad = 0;

    size_t bl = strlen(g_files[idx]) - 4;                 /* drop ".mxs" */
    memcpy(base, g_files[idx], bl);
    base[bl] = 0;
    set_text(r->name, sizeof r->name, base);
    snprintf(path, sizeof path, "%s\\%s", g_folder, g_files[idx]);
    snprintf(r->status, sizeof r->status, "%s", "ok");
    snprintf(r->stage, sizeof r->stage, "%s", "?");

    f = fopen(path, "rb");
    good = f && fread(hd, 4, 9, f) == 9 && hd[0] == 0x3153584d && hd[1] < NSTAGE && hd[2] && hd[2] < (64u << 20) && hd[3] < (16u << 20);
    if (good) {
        code = malloc(hd[2]);
        cdata = hd[3] ? malloc(hd[3]) : NULL;
        good = code && fread(code, 1, hd[2], f) == hd[2] && (!hd[3] || (cdata && fread(cdata, 1, hd[3], f) == hd[3]));
    }
    if (f) fclose(f);
    if (!good) {
        snprintf(r->status, sizeof r->status, "%s", "unreadable");
        set_text(r->error, sizeof r->error, "could not read stage file");
        free(code);
        free(cdata);
        return;
    }
    stage = (int)hd[1];
    snprintf(r->stage, sizeof r->stage, "%s", STAGE_NAMES[stage]);
    if (stage == 3 || stage == 4) {
        snprintf(r->status, sizeof r->status, "%s", "skipped");
        set_text(r->error, sizeof r->error, "tessellation stage: the renderer has none");
        free(code);
        free(cdata);
        return;
    }
    /* a geometry stage takes the draw's input primitive: translated and compiled for points, lines and triangles */
    variants = stage == 2 ? 3 : 1;
    for (var = 0; var < variants && !bad; var++) {
        MxIn in;
        MxOut out;
        memset(&in, 0, sizeof in);
        memset(&out, 0, sizeof out);
        in.stage = stage; in.code = code; in.code_len = (int)hd[2]; in.cdata = cdata; in.cdata_size = hd[3];
        in.target = 1; in.binding_base = 40; in.ssbo_base = 60; in.tex_base = 40; in.image_base = 72; in.wmask_binding = 78;
        in.local_size[0] = hd[4]; in.local_size[1] = hd[5]; in.local_size[2] = hd[6];
        in.smem_bytes = hd[7]; in.lmem_bytes = hd[8];
        in.gs_in_prim = var;
        if (!mx_translate(&in, &out) || !out.glsl) {
            char why[300];
            if (out.op[0] && out.why[0] && strcmp(out.op, out.why))
                snprintf(why, sizeof why, "not translated: %s (%s)", out.op, out.why);   /* summary groups by the part before " (" */
            else
                snprintf(why, sizeof why, "not translated: %s", out.op[0] ? out.op : out.why);
            snprintf(r->status, sizeof r->status, "%s", "not-translated");
            set_text(r->error, sizeof r->error, why);
            free(out.glsl);
            bad = 1;
            break;
        }
        if (var == 0) r->insns = out.insns;
        if (variants == 1) {
            snprintf(g, sizeof g, "%s\\%s.glsl", g_out, base);
            snprintf(spv, sizeof spv, "%s\\%s.spv", g_out, base);
        } else {
            snprintf(g, sizeof g, "%s\\%s.%s.glsl", g_out, base, GEOM_SUFFIX[var]);
            snprintf(spv, sizeof spv, "%s\\%s.%s.spv", g_out, base, GEOM_SUFFIX[var]);
        }
        f = fopen(g, "wb");
        if (!f || fwrite(out.glsl, 1, strlen(out.glsl), f) != strlen(out.glsl)) {
            if (f) fclose(f);
            free(out.glsl);
            snprintf(r->status, sizeof r->status, "%s", "compile-failed");
            set_text(r->error, sizeof r->error, "could not write the GLSL file");
            bad = 1;
            break;
        }
        fclose(f);
        free(out.glsl);
        DeleteFileA(spv);
        snprintf(cmd, sizeof cmd, "\"%s\" -V --target-env %s -S %s -o \"%s\" \"%s\"", g_glslang, TARGET_ENV, STAGE_FLAG[stage], spv, g);
        /* the GLSL file was just written: a virus scanner can still hold it open, so "unable to open" is retried */
        int rc = -1, tries;
        unsigned long long sz = 0;
        for (tries = 0; tries < 4; tries++) {
            rc = run_capture(cmd, text, sizeof text);
            if (rc != 0 && rc != -1 && strstr(text, "unable to open input file")) { Sleep(300); continue; }
            break;
        }
        if (rc != 0) {
            snprintf(r->status, sizeof r->status, "%s", "compile-failed");
            if (rc == -1) set_text(r->error, sizeof r->error, "could not start glslang.exe");
            else first_error(text, r->error, sizeof r->error);
            bad = 1;
            break;
        }
        if (!file_size(spv, &sz) || sz == 0) {
            snprintf(r->status, sizeof r->status, "%s", "compile-failed");
            set_text(r->error, sizeof r->error, "glslang wrote no SPIR-V");
            bad = 1;
            break;
        }
        snprintf(cmd, sizeof cmd, "\"%s\" --target-env %s \"%s\"", g_spirv_val, TARGET_ENV, spv);
        rc = run_capture(cmd, text, sizeof text);
        if (rc != 0) {
            snprintf(r->status, sizeof r->status, "%s", "invalid");
            if (rc == -1) set_text(r->error, sizeof r->error, "could not start spirv-val.exe");
            else first_error(text, r->error, sizeof r->error);
            bad = 1;
            break;
        }
        r->spv_bytes += sz;
    }
    free(code);
    free(cdata);
}

static DWORD WINAPI worker(LPVOID arg) {
    (void)arg;
    for (;;) {
        LONG k = InterlockedIncrement(&g_next) - 1;
        if (k >= g_nfiles) break;
        do_job((int)k);
        LONG d = InterlockedIncrement(&g_done);
        EnterCriticalSection(&g_print_cs);
        printf("[%ld/%d] %-64s %s%s%s\n", (long)d, g_nfiles, g_res[k].name, g_res[k].status,
               g_res[k].error[0] ? "  " : "", g_res[k].error);
        LeaveCriticalSection(&g_print_cs);
    }
    return 0;
}

static void mkdirs(const char *p) {
    char tmp[MAX_PATH * 2];
    char *s;
    snprintf(tmp, sizeof tmp, "%s", p);
    for (s = tmp + 1; *s; s++) {
        if (*s == '\\' || *s == '/') {
            char c = *s;
            *s = 0;
            CreateDirectoryA(tmp, NULL);
            *s = c;
        }
    }
    CreateDirectoryA(tmp, NULL);
}

static int status_index(const char *s) {
    for (int i = 0; i < 6; i++) if (!strcmp(s, STATUS_NAMES[i])) return i;
    return 5;
}

static int stage_index(const char *s) {
    for (int i = 0; i < NSTAGE; i++) if (!strcmp(s, STAGE_NAMES[i])) return i;
    return NSTAGE;                                      /* "?" */
}

static int cmp_names(const void *a, const void *b) { return strcmp(*(char *const *)a, *(char *const *)b); }

static int cmp_groups(const void *a, const void *b) {
    const Group *x = a, *y = b;
    return y->count != x->count ? y->count - x->count : strcmp(x->key, y->key);
}

static void usage(void) {
    printf("usage: mx_batch.exe <folder of .mxs> --out <dir> [--glslang <path>] [--spirv-val <path>] [--jobs N]\n");
}

int main(int argc, char **argv) {
    int jobs = 0, have_folder = 0, have_out = 0;
    SECURITY_ATTRIBUTES sa;
    WIN32_FIND_DATAA fd;
    HANDLE h;
    char pat[MAX_PATH * 2];
    int cap = 0;
    ULONGLONG t0 = GetTickCount64();
    static Group groups[512];
    int ngroups = 0;
    int by_stage[NSTAGE + 1][6];
    int total_by_stage[NSTAGE + 1];
    int status_tot[6];
    int nth, i, failures = 0;
    HANDLE th[MAX_JOBS];

    setvbuf(stdout, NULL, _IONBF, 0);
    for (i = 1; i < argc; i++) {
        if (!strcmp(argv[i], "--out") && i + 1 < argc) { snprintf(g_out, sizeof g_out, "%s", argv[++i]); have_out = 1; }
        else if (!strcmp(argv[i], "--glslang") && i + 1 < argc) snprintf(g_glslang, sizeof g_glslang, "%s", argv[++i]);
        else if (!strcmp(argv[i], "--spirv-val") && i + 1 < argc) snprintf(g_spirv_val, sizeof g_spirv_val, "%s", argv[++i]);
        else if (!strcmp(argv[i], "--jobs") && i + 1 < argc) jobs = atoi(argv[++i]);
        else if ((!strcmp(argv[i], "--help") || !strcmp(argv[i], "-h"))) { usage(); return 0; }
        else if (argv[i][0] != '-' && !have_folder) { snprintf(g_folder, sizeof g_folder, "%s", argv[i]); have_folder = 1; }
        else { usage(); fprintf(stderr, "unknown argument: %s\n", argv[i]); return 2; }
    }
    if (!have_folder || !have_out) { usage(); return 2; }
    /* mx_translate.c has a test hook that breaks translation when this is set; a run with it set would be wrong */
    if (getenv("SMO_MX_MUTATE")) {
        fprintf(stderr, "refusing to run: SMO_MX_MUTATE is set (planted-bug test hook in mx_translate.c)\n");
        return 2;
    }
    if (getenv("SMO_MX_NOSTRUCT")) printf("warning: SMO_MX_NOSTRUCT is set; structured control flow is off for this run\n");

    memset(&sa, 0, sizeof sa);
    sa.nLength = sizeof sa;
    sa.bInheritHandle = TRUE;
    g_nul = CreateFileA("NUL", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, &sa, OPEN_EXISTING, 0, NULL);
    if (g_nul == INVALID_HANDLE_VALUE) { fprintf(stderr, "cannot open NUL\n"); return 2; }
    InitializeCriticalSection(&g_print_cs);
    InitializeCriticalSection(&g_spawn_cs);

    snprintf(pat, sizeof pat, "%s\\*.mxs", g_folder);
    h = FindFirstFileA(pat, &fd);
    if (h == INVALID_HANDLE_VALUE) { printf("no .mxs files in %s\n", g_folder); return 2; }
    do {
        if (g_nfiles == cap) {
            cap = cap ? cap * 2 : 256;
            g_files = realloc(g_files, cap * sizeof *g_files);
        }
        g_files[g_nfiles++] = _strdup(fd.cFileName);
    } while (FindNextFileA(h, &fd));
    FindClose(h);
    qsort(g_files, g_nfiles, sizeof *g_files, cmp_names);
    g_res = calloc(g_nfiles, sizeof *g_res);
    mkdirs(g_out);

    if (jobs <= 0) {
        SYSTEM_INFO si;
        GetSystemInfo(&si);
        jobs = (int)si.dwNumberOfProcessors;
    }
    nth = jobs;
    if (nth > MAX_JOBS) nth = MAX_JOBS;
    if (nth > g_nfiles) nth = g_nfiles;
    printf("mx_batch: %d stage files, %d parallel jobs, glslang %s\n", g_nfiles, nth, g_glslang);
    for (i = 0; i < nth; i++) th[i] = CreateThread(NULL, 0, worker, NULL, 0, NULL);
    WaitForMultipleObjects(nth, th, TRUE, INFINITE);
    for (i = 0; i < nth; i++) CloseHandle(th[i]);

    /* results table */
    {
        char rp[MAX_PATH * 2];
        FILE *tf;
        snprintf(rp, sizeof rp, "%s\\results.tsv", g_out);
        tf = fopen(rp, "wb");
        if (!tf) { fprintf(stderr, "cannot write %s\n", rp); return 2; }
        fprintf(tf, "name\tstage\tstatus\tinstructions\tspv_bytes\terror\n");
        for (i = 0; i < g_nfiles; i++) {
            Res *r = &g_res[i];
            fprintf(tf, "%s\t%s\t%s\t%d\t%llu\t%s\n", r->name, r->stage, r->status, r->insns, r->spv_bytes, r->error);
        }
        fclose(tf);
    }

    /* summary */
    memset(by_stage, 0, sizeof by_stage);
    memset(total_by_stage, 0, sizeof total_by_stage);
    memset(status_tot, 0, sizeof status_tot);
    for (i = 0; i < g_nfiles; i++) {
        Res *r = &g_res[i];
        int s = stage_index(r->stage), st = status_index(r->status);
        total_by_stage[s]++;
        by_stage[s][st]++;
        status_tot[st]++;
        if (st == 4) continue;                          /* skipped: not a failure */
        if (st != 0) {
            failures++;
            char key[400];
            snprintf(key, sizeof key, "%s: %s", r->status, r->error);
            char *par = strstr(key, " (");                 /* group by the reason; the detail in parentheses stays in results.tsv */
            if (par) *par = 0;
            int k;
            for (k = 0; k < ngroups && strcmp(groups[k].key, key); k++) {}
            if (k == ngroups && ngroups < 512) {
                snprintf(groups[k].key, sizeof groups[k].key, "%s", key);
                snprintf(groups[k].status, sizeof groups[k].status, "%s", r->status);
                snprintf(groups[k].example, sizeof groups[k].example, "%s", r->name);
                ngroups++;
            }
            if (k < ngroups) groups[k].count++;
        }
    }
    printf("\nper stage (total / ok / not-translated / compile-failed / invalid / skipped / unreadable):\n");
    for (i = 0; i <= NSTAGE; i++) {
        if (!total_by_stage[i]) continue;
        printf("  %-14s %4d / %4d / %4d / %4d / %4d / %4d / %4d\n", i < NSTAGE ? STAGE_NAMES[i] : "unknown", total_by_stage[i],
               by_stage[i][0], by_stage[i][1], by_stage[i][2], by_stage[i][3], by_stage[i][4], by_stage[i][5]);
    }
    printf("per status:\n");
    for (i = 0; i < 6; i++) printf("  %-14s %d\n", STATUS_NAMES[i], status_tot[i]);
    if (ngroups) {
        qsort(groups, ngroups, sizeof groups[0], cmp_groups);
        printf("failure reasons (count, then the reason verbatim, then one stage name):\n");
        for (i = 0; i < ngroups && i < 60; i++)
            printf("  %5d  %s   e.g. %s\n", groups[i].count, groups[i].key, groups[i].example);
        if (ngroups > 60) printf("  ... %d more reasons in results.tsv\n", ngroups - 60);
    }
    printf("\nresults: %s\\results.tsv\n", g_out);
    printf("time: %.1f s\n", (GetTickCount64() - t0) / 1000.0);
    return failures ? 1 : 0;
}
