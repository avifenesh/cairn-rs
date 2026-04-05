# STATUS: permission_decision_audit

**Task:** RFC 002 permission decision audit hardening  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/permission_decision_audit.rs`

**Fix:** Linter had replaced `| RuntimeEvent::CheckpointStrategySet(_)` with a comment `// CheckpointStrategySet handled below`, breaking store compilation. Restored correct pattern arm.

Tests:
- `permission_allowed_persists_in_log`
- `permission_denied_is_stored_and_distinguishable`
- `read_by_entity_scoping_for_tool_invocation`
- `invocation_id_links_permission_to_tool_call`
- `cross_project_isolation_via_principal_filtering`
- `permission_audit_fields_are_fully_preserved`

Key: PermissionDecisionRecorded has no project field (returns _system sentinel) and no entity ref — uses read_stream filtered by invocation_id/principal. Cross-project isolation via principal prefix filtering.
