// SPDX-License-Identifier: GPL-3.0-or-later
// Pattern from smo-recomp runtime/core/hostrt.c crash_filter (RtlVirtualUnwind loop).
#include "stackwalk.h"

namespace nxrt::detail {

size_t capture_stack(const CONTEXT* ctx, uint64_t* frames, size_t max) {
    CONTEXT c = *ctx;
    size_t n = 0;
    while (n < max && c.Rip) {
        frames[n++] = c.Rip;
        DWORD64 image_base = 0;
        RUNTIME_FUNCTION* rf = RtlLookupFunctionEntry(c.Rip, &image_base, nullptr);
        if (!rf) {  // leaf function without unwind info: the return address is at [rsp]
            if (c.Rsp < 0x10000) break;
            c.Rip = *reinterpret_cast<DWORD64*>(c.Rsp);
            c.Rsp += 8;
            if (n > 1) break;   // only the first frame may be a leaf; anything else means we lost the chain
            continue;
        }
        void* handler_data = nullptr;
        DWORD64 establisher = 0;
        RtlVirtualUnwind(UNW_FLAG_NHANDLER, image_base, c.Rip, rf, &c, &handler_data, &establisher, nullptr);
    }
    return n;
}

}  // namespace nxrt::detail
