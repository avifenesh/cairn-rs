# Final Test Count Report — 2026-04-05

## Grand Total: 1,072 passing tests (0 failures in counted tests)

---

## Per-Crate Breakdown

| Crate | Lib | Integration | Total |
|---|---|---|---|
| cairn-domain | 148 | 25 | **173** |
| cairn-store | 21 | 145 (129 standard + 16 sqlite) | **166** |
| cairn-runtime | 208 | 68 | **276** |
| cairn-api | 113 | 36 | **149** |
| cairn-evals | 42 | 18 | **60** |
| cairn-tools | 114 | 0 | **114** |
| cairn-memory | 92 | 0 | **92** |
| cairn-graph | 21 | 0 | **21** |
| cairn-signal | 7 | 0 | **7** |
| cairn-channels | 7 | 0 | **7** |
| cairn-plugin-proto | 7 | 0 | **7** |
| **TOTAL** | **780** | **292** | **1,072** |

---

## cairn-store Integration Tests (17 files)

| File | Tests |
|---|---|
| approval_workflow | 7 |
| bootstrap_smoke | 6 |
| checkpoint_recovery | 5 |
| cost_tracking | 7 |
| cross_backend_parity (no-sqlite) | 0 |
| cross_backend_parity (--features sqlite) | 16 |
| entitlements | 10 |
| fleet_management | 6 |
| ingest_job_lifecycle | 9 |
| mailbox_messaging | 10 |
| prompt_lifecycle | 7 |
| run_state_machine | 14 |
| session_state_machine | 12 |
| signal_routing | 8 |
| sse_replay | 8 |
| task_dependency | 5 |
| tenant_rbac | 7 |
| tool_invocation_lifecycle | 8 |

## cairn-runtime Integration Tests (passing, 15/23 files compile)

| File | Tests |
|---|---|
| config_store | 10 |
| enrichment_integration | 6 |
| lifecycle_integration | 10 |
| mailbox_delivery | 4 |
| prompt_approval_flow | 5 |
| prompt_rollout | 5 |
| provider_routing_e2e | 4 |
| recovery_integration | 4 |
| replay_guard | 1 |
| run_cost_integration | 1 |
| seam_protection | 6 |
| session_cost_integration | 1 |
| task_dependency_integration | 1 |
| week3_integration | 7 |
| week4_e2e | 3 |
| auto_checkpoint_integration | **broken** |
| binding_cost_stats | **broken** |
| event_log_compaction | **broken** |
| llm_observability | **broken** |
| provider_model_registry | **broken** |
| prompt_version_diff | **broken** |
| snapshot | **broken** |
| sqlite_integration | not counted (needs --features sqlite) |

## Issues Found (pre-existing, not introduced by worker-core)

- **cairn-evals/tests/baseline_flow.rs**: create_run called with 9 args, signature expects 8
- **cairn-evals/tests/dataset_flow.rs**: similar wrong arg count
- **cairn-evals/tests/rubric_flow.rs**: 4 compile errors (wrong arg count)
- **cairn-api/tests/http_boundary_alignment.rs**: compile error (not counted)
- **cairn-api/tests/llm_traces_route.rs**: compile error (not counted)
- **cairn-api/tests/product_surface_composition.rs**: compile error (not counted)
- **cairn-api/tests/compat_catalog_sync.rs**: 3 pass, 1 FAIL (not counted in passing total)
- **cairn-runtime**: 7 integration test files do not compile

## Worker-Core Tests Delivered (this session)

| File | Tests |
|---|---|
| cairn-store/tests/bootstrap_smoke.rs | 6 |
| cairn-store/tests/sse_replay.rs | 8 |
| cairn-store/tests/prompt_lifecycle.rs | 7 |
| cairn-store/tests/tenant_rbac.rs | 7 |
| cairn-store/tests/approval_workflow.rs | 7 |
| cairn-store/tests/tool_invocation_lifecycle.rs | 8 |
| cairn-store/tests/signal_routing.rs | 8 |
| cairn-evals/tests/eval_pipeline.rs | 13 |
| cairn-store/tests/mailbox_messaging.rs | 10 |
| cairn-store/tests/ingest_job_lifecycle.rs | 9 |
| cairn-store/tests/session_state_machine.rs | 12 |
| cairn-store/tests/run_state_machine.rs | 14 |
| cairn-domain/tests/model_catalog.rs | 25 |
| **Worker-core subtotal** | **134** |
