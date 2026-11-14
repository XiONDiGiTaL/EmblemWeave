// SPDX-License-Identifier: GPL-3.0-or-later
#include "nxrt/gfx_tuning.h"

namespace nxrt::gfx {

Tuning& tuning() {
    static Tuning t;
    return t;
}

}  // namespace nxrt::gfx
