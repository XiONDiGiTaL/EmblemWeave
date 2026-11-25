// SPDX-License-Identifier: GPL-3.0-or-later
// [fefw] cloth_gravity = match: the gravity factor that preserves the 30 fps settled position at s steps per 1/30 s frame.
#include "test_framework.h"

#include "../games/fefw/fefw_cloth.h"

#include <cmath>

NXTEST_GROUP(fefw_cloth)

namespace {

constexpr float kEps = 1e-5f;

bool near(float a, float b) {
    const float scale = std::max(1.0f, std::max(std::fabs(a), std::fabs(b)));
    return std::fabs(a - b) <= kEps * scale;   // the settled offset is ~1/k, so compare relatively
}

}  // namespace

NXTEST(match_keeps_the_30fps_settled_offset) {
    for (float s : {0.125f, 0.25f, 0.5f, 0.75f}) {
        for (float k : {0.01f, 0.05f, 0.2f, 0.5f, 0.7f}) {
            const float q = 1.0f - k;
            const float qs = std::pow(q, s);
            const float f = nxrt::games::fefw::cloth_force_factor(k, s, qs);
            const float settled = f * qs / (1.0f - qs);
            const float at30 = q / (1.0f - q);
            CHECK_MSG(near(settled, at30), "s=" + std::to_string(s) + " k=" + std::to_string(k));
        }
    }
}

NXTEST(match_is_clamped_and_bounded) {
    for (float s : {0.125f, 0.25f, 0.5f, 0.75f}) {
        for (float k : {0.0f, 0.001f, 0.5f, 0.9f, 0.999f, 1.0f}) {
            const float qs = std::pow(1.0f - k, s);
            const float f = nxrt::games::fefw::cloth_force_factor(k, s, qs);
            CHECK(f >= s * s - kEps);
            CHECK(f <= 1.0f + kEps);
        }
    }
    // A strong turn would ask for less than s^2 (0.2403 at k=0.9, s=0.5); the clamp keeps s^2.
    CHECK(near(nxrt::games::fefw::cloth_force_factor(0.9f, 0.5f, std::pow(0.1f, 0.5f)), 0.25f));
    // A weak turn is close to s, a free point (k -> 0) to s^2.
    CHECK(near(nxrt::games::fefw::cloth_force_factor(0.5f, 0.5f, std::pow(0.5f, 0.5f)), 0.41421356f));
    CHECK(near(nxrt::games::fefw::cloth_force_factor(0.0f, 0.5f, 0.5f), 0.25f));
    CHECK(near(nxrt::games::fefw::cloth_force_factor(1.0f, 0.5f, 0.0f), 0.25f));
}

NXTEST(match_is_identity_at_a_whole_30fps_step) {
    for (float k : {0.01f, 0.1f, 0.5f, 0.9f}) {
        CHECK(near(nxrt::games::fefw::cloth_force_factor(k, 1.0f, 1.0f - k), 1.0f));
    }
}
