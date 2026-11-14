// SPDX-License-Identifier: GPL-3.0-or-later
// Test-only C++ exception runtime of the end-to-end oracle (see eh_rt.cpp and progs/rt.h for the guest side).
#pragma once
#include <nxrecomp/ctx.h>
void eh_rt_init(const NxModule *m);
// Called by call_indirect for every indirect call: handles the host-service addresses of progs/rt.h, returns false
// when addr is not one of them.
bool eh_host_call(Ctx *c, uint64_t addr);
