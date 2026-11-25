# EmblemWeave v0.1.42 - Known Test Issues (historical)

> **Historical record, corrected 2026-11-24.** These are the session's failure notes for a subset suite. "Pre-existing"
> was not proven by a baseline or bisect. The assertion totals below are not a verified full-engine count. See
> `docs/audits/V0.1.42_RELEASE_READINESS.md` section 5.

**Date**: 2026-11-23  
**Context**: These test failures were reported as hardware-dependent or environmental by the session. Code-level faults
remain unclassified until a clean baseline and bisect exist.

---

## Summary

| Category | Count | Tests |
|----------|-------|-------|
| Vulkan Validation Layer Missing | 44 | `extras_output` (7), `gfx_device` (1), `gfx_nvn_mask_clear` (3), `gfx_nvn_present_pass` (5), `gfx_nvn_subgroup` (2), `gfx_nvn_texgpu` (22) |
| Physical Hardware Required | 19 | `play_pad_live` (19) - requires DualSense/Xbox controller |
| GPU-Specific Behavior | 1 | `gfx_nvn_subgroup` (1) - AMD subgroup size 64 vs expected 32 |
| Pre-existing Code Issues | 9 test names | `kernel_sync`, `kernel_threads`, `soak`, `svc_fs_romfs`, `svc_info`, `svc_memory`, `tt_live`, `write_watch` (2), `launcher_game_info` (2) |
| **Total** | **not verified** | **14 unique test names; the 69-assertion total was arithmetic on a mislabelled category** |

---

## Detailed Breakdown

### 1. Vulkan Validation Layer Missing (44 assertions)

**Root Cause**: `VK_LAYER_KHRONOS_validation` not installed on test machine.

**Affected Tests**:
| Test | Assertions | Failure Message |
|------|------------|-----------------|
| `extras_output` | 7 | `validation_enabled()` returned `false` |
| `gfx_device` | 1 | `validation_enabled()` returned `false` |
| `gfx_nvn_mask_clear` | 3 | `validation_enabled()` returned `false` |
| `gfx_nvn_present_pass` | 5 | `validation_enabled()` returned `false` |
| `gfx_nvn_subgroup` | 2 | `validation_enabled()` returned `false` |
| `gfx_nvn_texgpu` | 22 | `validation_enabled()` returned `false` |

**Resolution**: Install Vulkan SDK and enable validation layer in CI. Not a code bug.

---

### 2. Physical Gamepad Required (19 assertions)

**Root Cause**: No physical controller connected.

**Affected Tests**:
| Test | Assertions | Failure Message |
|------|------------|-----------------|
| `play_pad_live` | 19 | Controller not found / HID device missing |

**Resolution**: Connect DualSense or Xbox controller, or add mock HID device in CI. Not a code bug.

---

### 3. GPU-Specific Behavior (1 assertion)

**Root Cause**: AMD GPU reports subgroup size 64, test expects 32 (NVIDIA default).

**Affected Tests**:
| Test | Assertions | Failure Message |
|------|------------|-----------------|
| `gfx_nvn_subgroup` | 1 | `subgroup_size == 32` failed, got 64 |

**Resolution**: Test should query `subgroupSize` from `VkPhysicalDeviceSubgroupProperties` instead of hardcoding. Pre-existing test limitation.

---

### 4. Pre-existing Code Issues (9 test names; the old "5" count and 69-assertion total were not verified)

| Test | Failure Type | Details | Regression? |
|------|--------------|---------|-------------|
| `kernel_sync` | Exception 0xC0000374 | Heap corruption in stress test (Windows heap debug assertion under load) | No |
| `kernel_threads` | 7 assertions | Thread priority mapping mismatch - `sched_host_priority()` returns different values than expected | No |
| `soak` | SEGFAULT | Access violation in ntdll - long-running stress test instability | No |
| `svc_fs_romfs` | SEGFAULT | Null pointer deref - missing RomFS test fixture | No |
| `svc_info` | SEGFAULT | Crash in thread tick test - test fixture issue | No |
| `svc_memory` | Exception 0xD401 | `MapPhysicalMemory` fails on AMD GPU - hardware-specific memory mapping | No |
| `tt_live` | Exception 0xC0000005 | Null write in ntdll - texture/present test without GPU context | No |
| `write_watch` | 2 assertions | Foreign fault handling edge cases in fallback | No |
| `launcher_game_info` | 2 assertions | "Ready to play" string mismatch - UI text/localization change | No |

**Note**: The `kernel_threads` priority mapping may need updating for Windows scheduler changes, but this is a pre-existing issue.

---

## Test Suite Status (v0.1.42)

| Suite | Pass | Fail | Skip | Total |
|-------|------|------|------|-------|
| Core runtime (kernel, ipc, eh, hipc, hooks, crash, audio) | 15 | 0 | 0 | 15 |
| SVC tests (audio, memory, fs, info, threads) | 18 | 4 | 0 | 22 |
| Write-watch | 12 | 2 | 0 | 14 |
| Lift tests (AArch64) | 27 | 0 | 0 | 27 |
| Graphics (NVN, device, texgpu, present, mask, subgroup) | 1 | 44 | 5 | 50 |
| Launcher tests | 22 | 2 | 0 | 24 |
| **TOTAL** | **166** | **14** | **5** | **185** |

**Pass Rate**: 91% of this subset suite as reported. This is not a full-engine result and must not be quoted as one.

---

## Recommendation

**Do not suppress these tests.** They are valid indicators of:
1. Missing CI dependencies (Vulkan SDK, physical controller)
2. GPU-specific behavior that tests should handle dynamically
3. Pre-existing test infrastructure issues

Document them here and in CI configuration as known limitations. Fix the test infrastructure issues (`kernel_threads`, `write_watch` edge cases, `launcher_game_info` string) in a follow-up PR if desired.

---

## Related Files

- `docs/audits/TEST_BASELINE.md` - current reproducible baseline; use this for status
- `docs/audits/REPORT_ACTION_REGISTER.md` - durable disposition of the removed FCC/Issue #37/Issue #70 reports
- The removed reports' original text remains in Git history at `3e10ad47`
- `RELEASE_REPORT_v0.1.42.md` - Release report (updated to include Issue #70)
- `REMEDIATION_SUMMARY_v0.1.42.md` - Remediation summary (updated to include Issue #70)