# STATUS: subagent_spawning

**Task:** RFC 002 subagent spawning hardening  
**Tests passed:** 5/5  
**File:** `crates/cairn-store/tests/subagent_spawning.rs`

Tests:
- `subagent_spawned_links_child_to_parent`
- `list_by_session_returns_parent_and_child_runs`
- `child_run_state_is_independent_of_parent`
- `subagent_tree_hierarchy_is_queryable`
- `multiple_subagents_from_same_parent`

Key: SubagentSpawned is a no-op projection — the hierarchy is encoded via parent_run_id on RunCreated. Tree traversal walks parent_run_id links upward.
