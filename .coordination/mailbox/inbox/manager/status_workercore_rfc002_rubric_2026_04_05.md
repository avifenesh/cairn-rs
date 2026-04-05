# Status Update — Worker Core

## Task: eval_rubric_lifecycle (RFC 002)
- **Tests**: 13/13 pass
- **Files created**: crates/cairn-store/tests/eval_rubric_lifecycle.rs
- **Files modified**: crates/cairn-store/src/in_memory.rs
  - Added eval_rubrics/eval_baselines HashMap fields to State
  - Added EvalRubricCreated projection handler
  - Added EvalBaselineSet projection handler (upsert; respects locked flag)
  - Added EvalBaselineLocked projection handler (sets locked=true)
  - Added impl EvalRubricReadModel for InMemoryStore
  - Added impl EvalBaselineReadModel for InMemoryStore
- **Schema gaps**: EvalRubricCreated/EvalBaselineSet have no tenant_id or full domain fields. Stored with sentinel tenant_id "". list_by_tenant("") returns all.
- **Locked immutability**: projection enforces at store layer — EvalBaselineSet is ignored when locked=true.
