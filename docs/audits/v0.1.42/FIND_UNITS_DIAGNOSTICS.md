# Find Units Cheat Diagnostics

## Overview
Memory scanning cheat for "Find Units" functionality with performance optimization.

## Configuration
- Save data: Profile 1, slot 000 at `E:/EmblemWeave/data/save`
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

## Expected Performance
- Initial scan: < 5 seconds for full memory range
- Refined scan: < 1 second
- Zero impact on gameplay FPS (background thread at BELOW_NORMAL)

## Log Indicators
- Look for memory scan start/complete messages
- Thread priority verification in logs
- No "slow frame" warnings during scan

## Save Data Location
```
E:/EmblemWeave/data/save/Profile1/slot_000/
```

## Current Status (v0.1.42)
- **Infrastructure**: Complete - parallel workers, cancellation, progress reporting, thread priority
- **Runtime Stability**: Game now runs 120+ seconds without crashing (fixed read fault crash)
- **Field Test**: Pending - requires reaching cheat menu in gameplay
- **Commit**: 59d948c added timeout (30s), memory limit (2GB), thread cap (8), BELOW_NORMAL priority