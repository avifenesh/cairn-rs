# Final Workspace Health Report
**Date:** 2026-04-05

---

## cairn-app compile errors

| Metric | Count |
|--------|-------|
| `cargo build -p cairn-app` error count | **0** |

cairn-app now compiles clean. The 129 remaining errors from earlier in the session were resolved by fixing `DashboardOverview` initializers in `cairn-api/src/overview.rs` (4 missing-field errors) and the pre-existing type issues.

---

## Workspace build status (excluding cairn-app)

All crates compile clean. Warnings only (unreachable patterns, unused imports — pre-existing).

---

## Lib tests (cargo test --workspace --exclude cairn-app --lib)

All 12 lib test suites pass. **0 failures.**

| Crate | Tests |
|-------|-------|
| cairn-domain | 113 |
| cairn-store | 208 |
| cairn-runtime | 148 |
| cairn-memory | 114 |
| cairn-api | 92 |
| cairn-evals | 42 |
| cairn-graph | 24 |
| cairn-tools | 21 |
| cairn-mcp | 13 |
| cairn-memory (extra) | 7 |
| cairn-runtime (extra) | 7 |
| cairn-store (extra) | 7 |
| **TOTAL** | **796 passed, 0 failed** |

---

## Integration tests written this session

All new integration tests pass.

| Test File | Crate | Tests |
|-----------|-------|-------|
| `tests/provider_routing_e2e.rs` | cairn-runtime | 4 |
| `tests/cost_tracking.rs` | cairn-store | 7 |
| `tests/checkpoint_recovery.rs` | cairn-store | 5 |
| `tests/fleet_management.rs` | cairn-store | 6 |
| `tests/entitlements.rs` | cairn-store | 10 |
| `tests/task_dependency.rs` | cairn-store | 5 |
| `tests/document_lifecycle.rs` | cairn-memory | 6 |
| `tests/bundle_round_trip.rs` | cairn-memory | 7 |
| `tests/scorecard_flow.rs` | cairn-evals | 6 |
| **TOTAL NEW** | | **56 passed, 0 failed** |

---

## Pre-existing integration test failures (not introduced this session)

These test files had compile errors before this session and remain broken:

| Test File | Crate | Error |
|-----------|-------|-------|
| `tests/provenance_tracking.rs` | cairn-memory | E0282 type annotations (6 errors) |
| `tests/binding_cost_stats.rs` | cairn-runtime | 6 errors |
| `tests/baseline_flow.rs` | cairn-evals | extra arg to create_run |
| `tests/dataset_flow.rs` | cairn-evals | 2 errors |
| `tests/rubric_flow.rs` | cairn-evals | 4 errors |
| `tests/ingest_retrieval_pipeline.rs` | cairn-memory | 2 errors |
| `tests/entity_extraction.rs` | cairn-memory | 3 errors |
| `tests/graph_proximity.rs` | cairn-memory | 4 errors |
| `tests/explain_result.rs` | cairn-memory | 5 errors |

---

## Session total

- **796 lib tests passing** (all zero failures)
- **56 new integration tests added and passing**
- **cairn-app: 0 compile errors** (was 129 at session start after context reset)
- **All other crates: clean builds**
