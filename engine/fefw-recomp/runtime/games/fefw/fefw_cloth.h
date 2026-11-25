// SPDX-License-Identifier: GPL-3.0-or-later
// [fefw] cloth_gravity = match: the factor that keeps the cloth step's settled position at 30 fps when the frame is s steps long.
//
// Each point is moved by (velocity kept + gravity + wind), then turned a share k of the way back to its parent bone's direction.
// The turn acts on the position, so a point settles about gravity * (1 - k) / k from its rest pose. A substep turns it by
// 1 - (1 - k)^s instead of k and applies gravity * F. Settling after s substeps gives F * g * Q^s / (1 - Q^s), Q = 1 - k; matching
// g * Q / (1 - Q) at 30 fps yields F = Q (1 - Q^s) / ((1 - Q) Q^s). F is about s for a weak turn and tends to s^2 as k -> 0.
#pragma once
#include <algorithm>
#include <cmath>

namespace nxrt::games::fefw {

// qs is (1 - k)^s, passed in so the game path can use its per-thread power cache. k outside (0, 1) means no turn back: the settled
// position is unbounded, so gravity is a plain acceleration and the factor is s^2. The clamp keeps a strong turn (where the exact
// factor would be below s^2) from under-scaling.
inline float cloth_force_factor(float k, float s, float qs) {
    float f = s * s;
    if (k > 1e-4f && k < 1.0f && qs > 0.0f) {
        const float q = 1.0f - k;
        f = std::clamp(q * (1.0f - qs) / (k * qs), s * s, 1.0f);
    }
    return f;
}

}  // namespace nxrt::games::fefw
