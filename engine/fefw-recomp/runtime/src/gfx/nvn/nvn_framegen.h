// SPDX-License-Identifier: GPL-3.0-or-later
// The HUD-less colour of DLSS Frame Generation ([dlss] frame_gen, notes/framegen.md). The game draws its UI into the window texture after
// the scene: the window texture just before the first blended draw into it, after the frame's DLSS evaluation, is the scene without the UI.
// It is copied there and, at the present, drawn by the present pass into an image of the backbuffer's size and format.
#pragma once
#include <functional>

#include "nvn_gpu_internal.h"

namespace nxrt::nvn::be::gpu {

bool fg_hudless_on();      // [dlss] frame_gen and [dlss] frame_gen_hudless (default 1), read once
void fg_note_eval();       // replay: this frame's DLSS evaluation fed frame generation
// replay, before a draw into colour target t (blended: blending on target 0): true = call fg_hudless_capture now, with no pass open
bool fg_hudless_candidate(Texture* t, bool blended);
void fg_hudless_capture(Frame& f, Texture* t);
// present, after the picture is recorded: draw(src, dst) records the picture from src (a view in SHADER_READ_ONLY_OPTIMAL of an image the
// size of the presented texture) into dst (COLOR_ATTACHMENT_OPTIMAL, the backbuffer's size and format) as the present does
void fg_hudless_present(Window* w, Texture* presented, Frame& f, VkExtent2D bb, VkFormat bb_format, bool again,
                        const std::function<void(VkImageView, VkImageView)>& draw);

}  // namespace nxrt::nvn::be::gpu
