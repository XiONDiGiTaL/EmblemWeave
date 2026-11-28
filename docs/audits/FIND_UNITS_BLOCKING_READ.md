# Find Units (#76): blocked memory reads and cancellation

**Date:** 2026-11-28
**Code:** `apps/emblemweave/src/cheats.rs`, `apps/emblemweave/src/pages/cheats.rs`
**Classification:** investigated; boundary cancellation and loss detection implemented and tested with test doubles; a read already inside `ReadProcessMemory` is not interruptible.

## What the API allows

The scanner reads through `Memory::read`, whose Windows implementation calls `ReadProcessMemory`:

```rust
fn read(&self, addr: u64, buf: &mut [u8]) -> bool {
    let mut n = 0usize;
    unsafe {
        ReadProcessMemory(self.handle, addr as _, buf.as_mut_ptr() as _, buf.len(), &mut n) != 0
            && n == buf.len()
    }
}
```

Microsoft's [ReadProcessMemory documentation](https://learn.microsoft.com/en-us/windows/win32/api/memoryapi/nf-memoryapi-readprocessmemory)
defines no timeout, cancellation token, overlapped form or asynchronous completion. It is a synchronous request to the
memory manager. The documented failure cases concern inaccessible memory and invalid parameters, not a deadline. The
function has no length limit that makes a call bounded in time: a page fault, a target that is suspended, or a paging
path can keep the calling thread inside the call.

`CancelSynchronousIo` is not a solution for this call. Microsoft documents it for pending **I/O** operations on a
thread; `ReadProcessMemory` is a memory-manager operation, not a file/device read. There is no returned cancellation
status to classify for it, and no documented guarantee that it would abort the call.

Consequences, stated precisely:

- The 30-second scan deadline is observed **between** reads.
- A cancel request is observed **between** reads.
- The UI thread is not blocked, because the scan runs on a worker thread, but the worker cannot be reclaimed while
  the read is in flight.
- The timeout does **not** guarantee that the button resolves within 30 seconds.

## What changed

The scanner now checks cancellation, the deadline and target availability both before and after every read. If the
read returns after a cancel or timeout, its buffer is discarded and no addresses from it are accepted. A process
that disappeared while a read was in flight is reported as `device_lost`, not as a timeout or an empty result.
Progress is published for every piece, not every sixteenth piece, and records `cancelled`, `timed_out` and
`device_lost`.

`scan_outcome` is the single application-level decision about partial results. It rejects cancelled, timed-out and
disconnected scans, so a late completion cannot replace the unit list with partial addresses.

The page updates its progress state as soon as Cancel is clicked and says “Cancelling; waiting for the current
memory read to return.” It does not claim the read was interrupted. The Cancel button is disabled after the request.

The scoped worker threads are joined before `scan_units` returns. The implementation does not detach a thread,
force-terminate a thread, or leave a callback pointing at dropped state.

## Tests

The new tests use a controlled memory target, not a game process:

| Test | What it establishes |
| --- | --- |
| `scan_normal_completion_publishes_final_progress` | Normal completion, final progress, no cancellation |
| `scan_cancelled_between_reads_stops_before_the_next_piece` | Cancellation after one read prevents the next piece |
| `scan_cancelled_while_a_read_is_blocked_only_stops_when_the_read_returns` | A blocked read keeps the worker in the call; the page thread can still lock state and cancel; the returned buffer is discarded |
| `scan_stops_when_the_game_disappears_during_a_read` | Process loss is detected and stops the scan |
| `repeated_cancellation_is_idempotent` | Eight cancel calls and three scans behave the same |
| `scoped_scan_workers_release_their_state_after_return` | Eight scans leave no retained `Arc` state; scoped workers are joined |
| `late_or_partial_scan_results_are_not_accepted` | Cancelled, timed-out and disconnected results are rejected |

This is **tested with simulation or test doubles**. No real blocked `ReadProcessMemory` was reproduced, and no
in-game Find Units frame-time measurement was made. The test deliberately proves deferral of cancellation during a
blocked read rather than pretending it can be interrupted.

## Options considered

1. **Smaller reads.** Less data per call, but no bound on a single call and more calls. Rejected as a fix; the
   current 64 MiB pieces are unchanged.
2. **CancelSynchronousIo.** Rejected: no documented applicability to `ReadProcessMemory`, and no completion status
   the caller can rely on.
3. **Abandon the worker thread after a timeout.** Rejected: a blocked thread still owns the process handle and
   buffers, can complete after the scan state is dropped, and cannot be reclaimed safely.
4. **Force-terminate the thread.** Rejected: unsafe shutdown and use-after-free risk.
5. **Helper process.** The safest architectural solution if a real indefinite block is observed. A single-use
   `EmblemWeave.exe --scan-units-helper --pid <pid>` process would own the ReadProcessMemory calls, emit progress
   and results over a pipe, and be terminated as a whole on cancel or timeout. Windows then releases its handles and
   partially copied buffers. The helper would be read-only against the game, so terminating it cannot corrupt the
   target. A Job Object with kill-on-close and a bounded result queue would make cleanup deterministic. This was not
   implemented because no real block has been reproduced and adding a second process to the release would be a
   larger change than the evidence supports.

## What remains

- Reproduce the actual Find Units stall in a disposable game session and record where the worker is waiting.
- Measure Find Units frame time in battle.
- If the stall is inside `ReadProcessMemory`, implement the single-use helper process and test cancel, timeout,
  process exit and repeated scans against the real game.
