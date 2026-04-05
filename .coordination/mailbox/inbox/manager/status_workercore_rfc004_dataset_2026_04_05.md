# Status Update — Worker Core

## Task: eval_dataset_lifecycle (RFC 004)
- **Tests**: 10/10 pass
- **Files created**: crates/cairn-store/tests/eval_dataset_lifecycle.rs
- **Files modified**: crates/cairn-store/src/in_memory.rs
  - Added eval_datasets: HashMap field to State struct
  - Added projection handlers for EvalDatasetCreated and EvalDatasetEntryAdded
  - Added impl EvalDatasetReadModel for InMemoryStore
  - Removed shadowing no-op arms from both OR groups in apply_projection
- **Issues**: First run failed — EvalDatasetCreated/EntryAdded were in two no-op OR groups (lines 926 and 1726) that appeared BEFORE the new handlers, causing Rust to match the no-op arms first. Fixed by removing both no-op entries.
- **Adaptation**: EvalDatasetCreated has no tenant_id or subject_kind fields. Projection stores with empty sentinel tenant_id. list_by_tenant("") returns all. Dataset isolation tested by dataset_id. EvalDatasetEntryAdded has no payload — stores minimal entry with entry_id as tag for deduplication.
