// SPDX-License-Identifier: GPL-3.0-or-later
// Port of Ryujinx.Audio/Renderer/Dsp/Effect/ExponentialMovingAverage.cs (Ryubing fork, MIT).
#pragma once
#include "../../audren_common.h"

namespace nxrt::audren {

struct ExponentialMovingAverage {
  private:
    float _mean = 0.0f;

  public:
    ExponentialMovingAverage() = default;
    ExponentialMovingAverage(float mean) : _mean(mean) {}

    float Read() const { return _mean; }

    float Update(float value, float alpha) {
        _mean += alpha * (value - _mean);
        return _mean;
    }
};

}  // namespace nxrt::audren
