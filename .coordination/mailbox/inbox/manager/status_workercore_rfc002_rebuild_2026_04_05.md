# Status Update — Worker Core

## Task: projection_rebuild (RFC 002)
- **Tests**: 8/8 pass
- **Files created**: crates/cairn-store/tests/projection_rebuild.rs
- **Files changed**: none
- **Issues**: 3 unused import warnings (EventPosition, TaskStateChanged, Arc) — cosmetic only
- **Design note**: stored_at=now_millis() means created_at/updated_at differ between replay runs — timestamps are explicitly excluded from parity comparison. Tests compare (entity_id, state, version, relationships) which ARE deterministic.
- **Notable**:
  - 20-event fixture covers sessions, runs (including subagent with parent_run_id), tasks (including child task), approvals (approved and rejected paths), run transitions (WaitingApproval, Failed+failure_class), task transitions
  - Test 5 proves upsert semantics: re-applying same events to populated store preserves logical state (last-write-wins) but doubles the event log length (40 events)
  - Cross-project isolation verified: project_a and project_b sessions stay separate after replay

## Updated Grand Total: 1,202 passing tests (+8)
