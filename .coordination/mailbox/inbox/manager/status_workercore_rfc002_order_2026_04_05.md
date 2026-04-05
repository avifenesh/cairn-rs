# Status Update — Worker Core

## Task: global_ordering (RFC 002)
- **Tests**: 10/10 pass
- **Files created**: crates/cairn-store/tests/global_ordering.rs
- **Files modified**: crates/cairn-store/src/in_memory.rs — REAL BUG FIXED

## Bug found and fixed
The test caught a real ordering bug in InMemoryStore::append():

Before:
  1. assign position N to original event
  2. apply_projection() → derived events get positions N+1, N+2, pushed to state.events
  3. state.events.push(original at N) — AFTER derived events

Result: log order [..., N+1, N+2, N] — NOT monotonic!

After:
  1. assign position N to original event
  2. state.events.push(original at N) FIRST
  3. apply_projection() → derived events get N+1, N+2, pushed after original

Result: log order [..., N, N+1, N+2] — correct monotonic order.

cairn-store lib tests: 21/21 still pass after the fix.
