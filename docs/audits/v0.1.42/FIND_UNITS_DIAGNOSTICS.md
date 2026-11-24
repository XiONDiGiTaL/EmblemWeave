# Find Units Cheat Diagnostics

> **Historical working note - not release evidence (reviewed 2026-11-24).**
> The authoritative status is `docs/audits/V0.1.42_RELEASE_READINESS.md`. #76 is **Implemented - unverified**: parallel
> scan, cancellation and progress reporting exist, but there is no in-battle measurement. The install-relative paths
> below are local examples, not release artifacts. The 120-second session cited in this note is one FCC report, not #77's
> required 10+ menu/title cycles.

## Overview
Memory scanning cheat for "Find Units" functionality with performance optimization.

## Configuration
- Save data: Profile 1, slot 000 at `<install>/data/save` (local example)
- Thread priority: BELOW_NORMAL for background scanning
- Parallel workers with cancellation tokens and progress reporting

## Implementation Details

### Memory Scanning
- Parallel worker threads for scanning
- Cancellation token support for responsive UI
- Progress reporting for long scans
- Thread priority set to BELOW_NORMAL to avoid impacting gameplay

### Performance Considerations
- Scan runs on background threads
- Results cached for repeated searches
- Incremental scanning support

## Field Testing Checklist
- [ ] Activate Find Units cheat from menu
- [ ] Measure scan time for initial search
- [ ] Measure scan time for subsequent/refined searches
- [ ] Verify results accuracy (correct units found)
- [ ] Test cancellation during scan
- [ ] Verify no gameplay stutter during scan (thread priority BELOW_NORMAL)
- [ ] Test with Profile 1, slot 000 save data
- [ ] Measure memory usage during scan

## Expected Performance (targets, unverified)
- Initial scan: < 5 seconds for full memory range
- Refined scan: < 1 second
- Zero impact on gameplay FPS (background thread at BELOW_NORMAL)

These are targets, not measured results. The readiness audit records no in-battle measurement for #76.

## Log Indicators
- Look for memory scan start/complete messages
- Thread priority verification in logs
- No "slow frame" warnings during scan

## Save Data Location
```
<install>/data/save/Profile1/slot_000/
```

This is a local example, not a canonical save path.

## Current Status (v0.1.42)
- **Infrastructure**: **Implemented - unverified.** Parallel workers, cancellation, progress reporting and thread
  priority exist; there is no in-battle measurement.
- **Runtime evidence**: one FCC 120-second session is the only runtime report. It is not the required 10+ menu/title
  cycles (#77) and does not establish scan performance or absence of audio/visual impact.
- **Field Test**: Pending - requires reaching cheat menu in gameplay
- **Commit**: 59d948c added timeout (30s), memory limit (2GB), thread cap (8), BELOW_NORMAL priority; this is a
  code-level change, not an in-battle validation.