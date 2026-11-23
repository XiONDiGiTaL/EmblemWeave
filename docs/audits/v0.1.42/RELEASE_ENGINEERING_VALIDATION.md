# Release Engineering Validation - v0.1.42

## Version Metadata
- **Version**: 0.1.42
- **Build Date**: 2026-11-22
- **Commit**: 22a380c901 (nxrt) + local fixes
- **Launcher**: EmblemWeave.exe
- **Runtime**: fefw.exe

## Build Artifacts
| Artifact | Path | SHA256 |
|----------|------|--------|
| Launcher | E:\EmblemWeave\EmblemWeave.exe | 52da8612ba53ddd64c0bd2a8b5688ecb7517f79d1c2cb407008c5db29508b2fc |
| Runtime | E:\EmblemWeave\fefw.exe | f4af373fd8f1f597288607482d31267fa5353f9b917a0f029bbf7ab25fe7450e |
| Config | E:\EmblemWeave\data\play\fefw.ini | ee073abfd30e5f4fb21db053e458f20faba95af4d72c49031ae8e473f7e460dd |
| Overlay | work/romfs-overlay (28.9 GB) | TBD (large directory) |

## Test Results Summary

### Issue #77 - Crash Returning to Title (CRITICAL - FIXED)
| Test | Result | Notes |
|------|--------|-------|
| Write fault crash at ~4s | FIXED | ImportFreeze installs fault handler |
| Read fault crash at ~6.3s | FIXED | Broad fallback catches PAGE_NOACCESS |
| "Second exception" crash at ~8s | FIXED | protect() failures handled gracefully |
| Stable runtime | **120+ seconds** | 4000+ frames, no crashes |
| 10+ menu-title cycles | PENDING | Field test required |
| Retired page race | FIXED | ImportFreeze installs fault handler + heap_decommit clears retired bit |

### Issue #75 - Cinematic Playback
| Test | Result | Notes |
|------|--------|-------|
| AV1 decode (dav1d) | READY | 24 imports routed |
| Opus decode (hwopus) | READY | FFmpeg dynamic DLLs |
| WebM AV1CodecConfigurationRecord | READY | Variable-length OBU |
| MP4 av1C fallback | READY | Implemented |
| D3D11VA hardware accel | CONFIGURED | 8 threads, 30 frames ahead |

### Issue #76 - Find Units Cheat
| Test | Result | Notes |
|------|--------|-------|
| Parallel scanning | IMPLEMENTED | BELOW_NORMAL priority |
| Cancellation tokens | IMPLEMENTED | |
| Progress reporting | IMPLEMENTED | |
| Profile 1, slot 000 | CONFIGURED | Save data ready |

### Issue #78 - HDR State
| Test | Result | Notes |
|------|--------|-------|
| Fullscreen HDR toggle | CONFIGURED | present_guard = auto |
| Borderless HDR toggle | CONFIGURED | residency = auto |
| Exit restores HDR | PENDING | Field test required |

## Performance Metrics
- **Frame Rate**: Mean 150+ fps, 1% low 17+ fps
- **Pool Imports**: Background prefetch working
- **Page Faults**: ~20k initial, ~130k during heavy scene
- **Memory**: 12 GB heap, 16 GB limit
- **Engine Tests**: All pass (1000+ tests)

## Known Issues
1. RomFS file count mismatch: 252695 vs 252686 expected (9 extra files)
2. FSR3 frame gen: vkCmdWriteBufferMarkerAMD not available on device
3. No fefw.sites file for frame-fix hook validation
4. **New Issue #79**: Save files not being read (reported Nov 22, 2026)

## Compatibility
- **OS**: Windows 11 Pro 10.0.26100
- **GPU**: AMD Radeon RX 7900 XT (Vulkan 1.4.359)
- **CPU**: 8 physical cores, console core model active
- **FFmpeg**: N-123619-g44ad73031d-20260325

## Sign-off Checklist
- [x] Core runtime crash fixed (Issue #77)
- [x] Engine tests all pass (1000+ tests)
- [x] Version metadata updated to 0.1.42
- [x] SHA256 hashes computed for launcher, runtime, config
- [ ] All four issues verified independently (field tests pending)
- [ ] Cross-issue regression testing complete
- [ ] Release notes written
- [ ] Changelog updated
- [ ] GitHub issues closed with references
- [ ] Freebuff coordination complete (merge commit 20396fc validated)
- [ ] Investigate Issue #79 (save files not being read)