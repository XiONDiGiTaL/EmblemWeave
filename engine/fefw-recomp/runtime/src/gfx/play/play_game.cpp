// SPDX-License-Identifier: GPL-3.0-or-later
// Play mode's half of the game boundary: see play_game.h.
#include "play_game.h"

#include "nxrt/game.h"

namespace nxrt::play {

const GamePlay* game_play() { return game::info().play; }

}  // namespace nxrt::play
