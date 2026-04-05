# STATUS: worker_health_reporting

**Task:** RFC 011 external worker health reporting hardening  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/worker_health_reporting.rs`

**Store fixes:** Multiple linter corruptions to apply_projection no-op arm repaired:
- EvalBaselineLocked, EvalBaselineSet, EvalRubricCreated duplicated/removed incorrectly
- RecoveryEscalated `|` prefix stripped in first no-op arm (syntax error)
- All fixed; cairn-store builds clean

Tests:
- `worker_registered_with_default_health`
- `heartbeat_updates_health_fields`
- `current_task_id_assigned_and_cleared`
- `failed_outcome_clears_current_task_id`
- `stale_detection_workers_without_heartbeat`
- `list_by_tenant_health_ordering`
