// SPDX-License-Identifier: GPL-3.0-or-later
// Link check of the Fortune's Weave module libraries (target fefw_link_check): the six libraries plus stubs for every function ctx.h
// declares as runtime-provided. Linking proves that each f_<addr> / u_<addr> the generated code references is defined exactly once;
// running it prints each module descriptor and checks its tables against the bases of config.py. It runs no game code.
#include <nxrecomp/ctx.h>

#include <cstdio>
#include <cstdlib>
#include <cstring>

extern "C" {
void call_indirect(Ctx *, uint64_t) { std::abort(); }
uint32_t *nx_fn_flat = nullptr;
uint64_t nx_fn_flat_n = 0;
uintptr_t nx_fn_flat_host = 0;
void host_import(Ctx *, uint32_t) { std::abort(); }
void guest_trap(const char *, uint64_t) { std::abort(); }
void guest_unimpl(uint64_t, uint32_t, const char *) { std::abort(); }
void guest_svc(Ctx *, uint32_t) { std::abort(); }
uint64_t guest_mrs(Ctx *, uint32_t) { std::abort(); }
void guest_msr(Ctx *, uint32_t, uint64_t) { std::abort(); }
void nx_site_inline(Ctx *, uint64_t) { std::abort(); }
int nx_site_midhook(Ctx *, uint64_t) { std::abort(); }
int nx_hook_entry(Ctx *, uint64_t) { std::abort(); }
void stx_diag(Ctx *, uint64_t, uint64_t, uint64_t, uint64_t, int) {}
void nx_catch_call(Ctx *, void (*)(Ctx *, uint64_t)) { std::abort(); }

extern const NxModule nxmod_main, nxmod_sdk, nxmod_subsdk0, nxmod_subsdk1, nxmod_subsdk2, nxmod_rtld;
extern const uint32_t nxmod_main_abi, nxmod_sdk_abi, nxmod_subsdk0_abi, nxmod_subsdk1_abi, nxmod_subsdk2_abi, nxmod_rtld_abi;
}

namespace {

struct Expected {
    const NxModule *nx;
    const uint32_t *abi;
    const char *name;
    uint32_t index;
    uint64_t base;
};

const Expected kModules[] = {
    {&nxmod_main, &nxmod_main_abi, "main", 0, 0x7100000000ull},
    {&nxmod_sdk, &nxmod_sdk_abi, "sdk", 1, 0x7500000000ull},
    {&nxmod_subsdk0, &nxmod_subsdk0_abi, "subsdk0", 2, 0x7600000000ull},
    {&nxmod_subsdk1, &nxmod_subsdk1_abi, "subsdk1", 3, 0x7640000000ull},
    {&nxmod_subsdk2, &nxmod_subsdk2_abi, "subsdk2", 4, 0x7680000000ull},
    {&nxmod_rtld, &nxmod_rtld_abi, "rtld", 5, 0x7700000000ull},
};

}  // namespace

int main() {
    int bad = 0;
    for (const Expected &e : kModules) {
        const NxModule &m = *e.nx;
        bool sorted = true, inside = true;
        for (int i = 0; i < m.nfns; i++) {
            if (i && m.fns[i - 1].addr >= m.fns[i].addr) sorted = false;
            if (m.fns[i].addr < m.base || m.fns[i].addr >= m.text_end || !m.fns[i].fn) inside = false;
        }
        int defined = 0;
        for (int i = 0; i < m.nsyms; i++) defined += m.syms[i].defined ? 1 : 0;
        const bool ok = *e.abi == NXRECOMP_ABI_VERSION && m.index == e.index && m.base == e.base && sorted && inside && m.nfns > 0 &&
                        std::strcmp(m.name, e.name) == 0;
        std::printf("%-8s index %u base 0x%llx text_end 0x%llx functions %d dynsyms %d (defined %d) resume %d fdes %d abi %u: %s\n",
                    m.name, m.index, (unsigned long long)m.base, (unsigned long long)m.text_end, m.nfns, m.nsyms, defined, m.nresume,
                    m.eh ? m.eh->nfdes : 0, *e.abi, ok ? "ok" : "MISMATCH");
        bad += ok ? 0 : 1;
    }
    return bad ? 1 : 0;
}
