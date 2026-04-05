# cairn-store Final Integration Test Count — 2026-04-05

## Summary
- **Integration test files**: 65 (64 standard + 1 sqlite-feature variant)
- **Standard integration passing**: 639
- **SQLite integration passing**: +16 (cargo test --features sqlite)
- **Total integration**: 655
- **Lib tests**: 21
- **GRAND TOTAL cairn-store: 676 passing, 0 failing, 0 regressions**

---

## cargo build --workspace
CLEAN — 0 errors, 5 warnings (cairn-app binary only, pre-existing)

---

## Failures: NONE — all 65 test files green

## Per-file results (65 files)

| File | Tests | Status |
|---|---|---|
| approval_blocking | 6 | ok |
| approval_policy_lifecycle | 13 | ok |
| approval_workflow | 7 | ok |
| bootstrap_smoke | 6 | ok |
| channel_message_lifecycle | 16 | ok |
| checkpoint_recovery | 5 | ok |
| checkpoint_strategy | 9 | ok |
| cost_aggregation_accuracy | 6 | ok |
| cost_tracking | 7 | ok |
| credential_rotation | 6 | ok |
| cross_backend_parity (standard) | 0 | ok (sqlite feature disabled) |
| cross_backend_parity (--features sqlite) | 16 | ok |
| default_settings | 13 | ok |
| entitlements | 10 | ok |
| entity_scoped_reads | 6 | ok |
| eval_dataset_lifecycle | 10 | ok |
| eval_rubric_lifecycle | 13 | ok |
| eval_run_lifecycle | 6 | ok |
| event_correlation_chains | 9 | ok |
| event_log_compaction | 6 | ok |
| event_persistence_contract | 18 | ok |
| external_worker_lifecycle | 11 | ok |
| feature_gate_enforcement | 7 | ok |
| fleet_management | 6 | ok |
| global_ordering | 10 | ok |
| idempotency | 7 | ok |
| ingest_job_errors | 6 | ok |
| ingest_job_lifecycle | 9 | ok |
| license_activation | 17 | ok |
| mailbox_messaging | 10 | ok |
| notification_preference | 6 | ok |
| permission_decision_audit | 6 | ok |
| projection_rebuild | 8 | ok |
| prompt_asset_scoping | 7 | ok |
| prompt_lifecycle | 7 | ok |
| prompt_release_governance | 15 | ok |
| prompt_release_rollout | 16 | ok |
| prompt_version_diff | 10 | ok |
| provider_binding_lifecycle | 11 | ok |
| provider_budget_tracking | 9 | ok |
| provider_call_audit | 11 | ok |
| provider_call_status | 10 | ok |
| provider_connection_lifecycle | 6 | ok |
| provider_health_schedule | 17 | ok |
| recovery_escalation | 6 | ok |
| resource_sharing_lifecycle | 14 | ok |
| retention_window | 5 | ok |
| rfc_compliance_summary | 6 | ok |
| route_decision_persistence | 11 | ok |
| route_policy_lifecycle | 17 | ok |
| run_state_machine | 14 | ok |
| session_state_machine | 12 | ok |
| signal_ingestion_contract | 11 | ok |
| signal_routing | 8 | ok |
| snapshot_lifecycle | 13 | ok |
| sse_replay | 8 | ok |
| subagent_spawning | 5 | ok |
| task_dependency | 5 | ok |
| task_lease_lifecycle | 6 | ok |
| task_state_machine | 12 | ok |
| tenant_org_lifecycle | 8 | ok |
| tenant_quota_enforcement | 18 | ok |
| tenant_rbac | 7 | ok |
| tool_invocation_lifecycle | 8 | ok |
| user_message_lifecycle | 5 | ok |
| worker_health_reporting | 6 | ok |
| workspace_rbac_enforcement | 16 | ok |
| workspace_role_hierarchy | 24 | ok |
