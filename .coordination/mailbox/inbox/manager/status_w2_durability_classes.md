# STATUS: durability_classes

**Task:** RFC 002 durability class contract  
**Tests passed:** 14/14  
**File:** `crates/cairn-domain/tests/durability_classes.rs`

**Code added:**
- `cairn_domain::errors::EntityDurabilityClass` enum (FullHistory, CurrentStatePlusAudit)  
  with serde snake_case + `RuntimeEntityKind::durability_class()` method
- Exported via `pub use errors::*` from cairn-domain

Tests:
- `durability_class_serde_round_trip`
- `durability_class_serializes_to_snake_case`
- `durability_classes_are_distinct`
- `session_run_task_are_full_history`
- `approval_checkpoint_are_current_state_plus_audit`
- `other_entity_kinds_are_current_state_plus_audit`
- `entity_ref_kind_mapping_is_correct` (all 13 variants)
- `entity_ref_durability_class_via_kind`
- `primary_entity_ref_is_some_for_operational_events`
- `primary_entity_ref_session_carries_correct_id`
- `primary_entity_ref_run_carries_correct_id`
- `primary_entity_ref_task_carries_correct_id`
- `project_extraction_matches_for_all_event_families`
- `full_history_event_projects_are_accessible`
