# Status Update — Worker Core

## Task: checkpoint_strategy (RFC 002)
- **Tests**: 9/9 pass
- **Files created**: crates/cairn-store/tests/checkpoint_strategy.rs
- **Files modified**: crates/cairn-store/src/in_memory.rs
  - Added checkpoint_strategies: HashMap<String, CheckpointStrategy> to State
  - Added projection handler for CheckpointStrategySet (was no-op)
  - Fixed get_by_run from stub (Ok(None)) to actual HashMap lookup
- **Adaptation**: CheckpointStrategySet has strategy_id/description/set_at_ms/run_id but no project/interval_ms/max_checkpoints. Stored with sentinel project and defaults. list_by_run does not exist on trait (get_by_run is the only method) — sequential update tests substitute for ordering tests.
- **CheckpointDisposition**: Latest/Superseded (not Historical as manager said) — covered by tests 5, 6, 7.
