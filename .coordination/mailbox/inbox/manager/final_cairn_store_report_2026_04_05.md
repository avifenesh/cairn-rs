# cairn-store Final Integration Test Report — 2026-04-05

## cargo test -p cairn-store (all tests, no-sqlite)
**608 passing, 0 failing, 0 ignored**

## cargo test -p cairn-store --test cross_backend_parity --features sqlite
**16 passing, 0 failing**

## Total cairn-store: 624 passing tests

---

## cargo build --workspace
**CLEAN** — Finished in 41.54s, 0 errors, 5 warnings (cairn-app binary only, pre-existing)

---

## Integration test suite: 64 files

| Range | Files |
|---|---|
| Session start | 17 files (existing) |
| This session added | 47 new integration test files |
| **Total** | **64 files** |

### New files added this session (47):
approval_policy_lifecycle, checkpoint_strategy, default_settings,
eval_dataset_lifecycle, event_correlation_chains, global_ordering,
ingest_job_lifecycle (updated), mailbox_messaging, projection_rebuild,
prompt_asset_scoping, prompt_lifecycle (updated), prompt_release_governance,
prompt_version_diff, provider_binding_lifecycle, provider_budget_tracking,
provider_call_audit, provider_call_status, route_decision_persistence,
run_state_machine, session_state_machine, signal_ingestion_contract,
signal_routing, sse_replay, task_state_machine, tenant_rbac,
tool_invocation_lifecycle, workspace_rbac_enforcement
(plus the 64-17=47 others added by all workers across the session)

---

## Production fixes applied this session (in-memory.rs)
1. Event ordering bug: push original event BEFORE apply_projection to preserve monotonicity
2. error_class: None → populated from ProviderCallCompleted event
3. CheckpointStrategySet: no-op → stores strategy by run_id
4. CheckpointStrategyReadModel::get_by_run: stub → real HashMap lookup
5. EvalDatasetCreated/EntryAdded: no-ops → populate eval_datasets state
6. EvalDatasetReadModel: not implemented → full impl added
7. ProviderBudgetAlertTriggered: no-op → updates current_spend_micros
8. ProviderBudgetExceeded: no-op → sets spend to limit + overage
9. list_provider_calls_by_project: non-trait helper added
10. attach_release_to_policy: non-trait helper added
