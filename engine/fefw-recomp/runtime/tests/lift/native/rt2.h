/* SPDX-License-Identifier: GPL-3.0-or-later
 * Native (x64, hosted) stand-in for guest/rt2.h: the test programs run here with the host's own C++ exception implementation, and
 * the results[] they leave behind are the expected output of the lifted AArch64 run (tests/lift/CMakeLists.txt). The program text is
 * the same; only this header differs. Programs must not rely on anything rt2.h defines beyond what this file defines. */
#ifndef NX_RT2_H
#define NX_RT2_H
#include "t.h"
#define NX_NATIVE 1   /* programs may skip actions that end the process */
#endif
