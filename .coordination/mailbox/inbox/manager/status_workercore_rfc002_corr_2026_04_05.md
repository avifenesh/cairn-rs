# Status Update — Worker Core

## Task: event_correlation_chains (RFC 002)
- **Tests**: 9/9 pass
- **Files created**: crates/cairn-store/tests/event_correlation_chains.rs
- **Files changed**: none
- **Issues**: none
- **Notable**:
  - correlation_id is a plain String field on EventEnvelope — there is no index, so correlation queries require read_stream + client-side filter. This is by design (RFC 002 tracing is at-read-time, not at-write-time).
  - causation_id is a CommandId that maps to the command that produced the event. find_by_causation_id() scans the log linearly and returns the first matching position.
  - Both fields can coexist on the same event (test 5 verifies this).
  - Test 8 proves that correlation chains are visible in partial SSE replays (only events after a given position).
