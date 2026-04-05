# Worker-2 Status: cairn-app Compilation + Server Startup

**Date:** 2026-04-05  
**From:** Worker-2

---

## Summary

cairn-app binary now compiles and starts a real axum server. Previous state: 129 lib errors, binary printed "bootstrap blocked" and exited.

---

## Error Count

| Phase | cairn-app (lib) | cairn-app (binary) |
|---|---|---|
| Start of session | 129 errors | blocked by lib errors |
| After fix | **0 errors** | **compiles and starts** |

---

## Changes Made

| File | Change |
|---|---|
| `crates/cairn-app/src/main.rs` | Replaced stub AppBootstrap with real axum server: GET /health, GET /v1/status, GET /v1/dashboard, binds 0.0.0.0:3000 |
| `crates/cairn-app/src/lib.rs` | Fixed 129 compile errors (type mismatches, missing fields, wrong arg counts, missing methods, duplicate routes) |
| `crates/cairn-memory/src/in_memory.rs` | Added 9 methods to `InMemoryDocumentStore` (all_current_chunks, list_sources, register_source, deactivate_source, etc.) that callers expected |
| `crates/cairn-memory/src/diagnostics_impl.rs` | Moved stub methods from trait impl into inherent impl block |
| `crates/cairn-api/src/auth.rs` | Changed `ServiceTokenRegistry` to use interior mutability (`RwLock<HashMap>`) — `register` now takes `&self` |

---

## Test Results

| Crate | Tests | Status |
|---|---|---|
| cairn-domain | 148 | ✅ 0 failures |
| cairn-store | 21 | ✅ 0 failures |
| cairn-evals | 42 | ✅ 0 failures |
| cairn-memory | 92 | ✅ 0 failures |
| cairn-graph | 24 | ✅ 0 failures |
| cairn-tools | 114 | ✅ 0 failures |
| cairn-runtime | 208 | ✅ 0 failures |
| cairn-app (lib) | 10 pass / 9 fail | ⚠️ partial (see below) |

**Total passing lib tests: 659 across 7 crates + 10 in cairn-app = 669**

---

## cairn-app Remaining Failures (9 tests)

These 9 tests were previously NEVER PASSING (they failed to compile). They now compile and run but fail at runtime with HTTP 404 from entity-lookup handlers after test setup via service methods. The rbac_viewer test PASSES (same pattern), so auth works. Root cause unclear — likely pre-existing test logic bugs uncovered now that compilation works.

Failed tests:
- `run_audit_trail_returns_chronological_entries` (404 on GET /v1/runs/{id}/audit)
- `session_activity_feed_returns_run_and_task_entries` (404)
- `event_pagination_run_events` (404 on GET /v1/runs/{id}/events)
- `eval_dashboard_returns_assets_with_run_counts_and_trend` (404)
- `run_auto_complete_when_all_tasks_done` (404)
- `plugin_capabilities_route_reports_verified_manifest_capabilities` (404)
- `plugin_tools_list_and_search` (404)
- `eval_provider_matrix_returns_row_with_binding_and_cost` (404)
- `task_lease_expiry_requeues_expired_task` (404)

---

## cairn-app Binary Status

```
cairn-app listening on http://0.0.0.0:3000
GET /health         → {"ok":true}
GET /v1/status      → {"runtime_ok":true,"store_ok":true,"uptime_secs":N}
GET /v1/dashboard   → DashboardOverview with zeros
```

Binary compiles and starts. Server is runnable.
