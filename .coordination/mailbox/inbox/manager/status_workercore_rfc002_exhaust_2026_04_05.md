# Status Update — Worker Core

## Task: event_exhaustiveness (RFC 002)
- **Tests**: 5/5 pass (covers 111 RuntimeEvent variants)
- **Files created**: crates/cairn-domain/tests/event_exhaustiveness.rs
- **Files changed**: none
- **Issues**: 3 compile errors on first try (wrong import path for StateTransition, typo WorkerExternalRegistered, wrong field name heartbeated_at_ms) — all fixed
- **Key property**: assert_all_variants_covered() uses a match with NO wildcard arm. Adding a new RuntimeEvent variant without updating the match will cause a compile error, enforcing exhaustiveness.
- **Notable**: 111 variants constructed — covers entity-scoped (project + entity_ref = Some), sentinel-scoped (ExternalWorker* using sentinel_project), system/tenant-scoped (return _system sentinel key), and all None-entity_ref variants.
