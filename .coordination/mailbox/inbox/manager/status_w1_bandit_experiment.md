# STATUS: bandit_experiment

**Task:** GAP-013 bandit experimentation integration test  
**Tests passed:** 8/8  
**File:** `crates/cairn-evals/tests/bandit_experiment.rs`  
**Note:** Added `cairn-runtime` as dev-dependency to `cairn-evals/Cargo.toml`

Tests:
- `create_experiment_with_three_arms`
- `record_wins_updates_arm_state`
- `epsilon_greedy_exploit_selects_best_arm_overwhelmingly`
- `epsilon_greedy_real_rng_selects_best_arm_majority`
- `ucb1_selects_unexplored_arms_first`
- `ucb1_always_prefers_unpulled_arm`
- `win_rates_return_correct_ratios`
- `experiment_list_is_tenant_scoped`

Session total integration tests: 64 (56 previous + 8 new)
Lib tests: 796 passing
cairn-app: 0 compile errors
