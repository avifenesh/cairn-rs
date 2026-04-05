# Workspace Health Check — 2026-04-04

## 1. Library Test Count (cargo test --workspace --lib)

All 11 crates compile and pass. **741 lib tests, 0 failures.**

| Crate | Passed | Failed |
|---|---|---|
| cairn-domain | 148 | 0 |
| cairn-store | 21 | 0 |
| cairn-runtime | 175 | 0 |
| cairn-api | 113 | 0 |
| cairn-evals | 39 | 0 |
| cairn-tools | 114 | 0 |
| cairn-memory | 89 | 0 |
| cairn-graph | 21 | 0 |
| cairn-signal | 7 | 0 |
| cairn-channels | 7 | 0 |
| cairn-plugin-proto | 7 | 0 |
| **Total** | **741** | **0** |

## 2. Integration Tests

### cairn-store cross_backend_parity (SQLite)
```
test result: ok. 16 passed; 0 failed
```
✅ All 16 parity tests pass.

### cairn-runtime lifecycle_integration
```
test result: ok. 10 passed; 0 failed
```
✅ All 10 lifecycle integration tests pass.

## 3. cairn-app Build Status

**Does not compile** — 40 errors (pre-existing, ahead-of-implementation stubs).

Error breakdown:
- **28× E0432** — unresolved imports: types/traits not yet implemented in dependency crates
- **4× E0433** — failed module resolutions (same root cause)
- **3× E0422** — struct constructors for undefined types
- **1× E0412** — undefined type reference

Top unresolved imports (representative sample):
- `cairn_api::CriticalEventSummary`, `cairn_api::http::ApiError`
- `cairn_runtime::InMemoryServices`, `cairn_runtime::ProviderConnectionPoolService`, `cairn_runtime::ResourceSharingService`
- `cairn_store::projections::SnapshotReadModel`, `cairn_store::projections::WorkspaceMembershipReadModel`
- `cairn_memory::import_service_impl`, `cairn_memory::export_service_impl`

**Root cause:** `cairn-app/src/lib.rs` was written ahead of implementation — it references 50+ service traits, store projections, and API types that are partially or fully unimplemented in their respective crates. This is a known pre-existing condition; cairn-app is the integration layer that will compile once all P2 service gaps (see `cairn-diff-gaps.md`) are filled.

**Priority next steps to unblock cairn-app:**
1. Implement `InMemoryServices` aggregator struct in cairn-runtime
2. Add `SnapshotReadModel`, `WorkspaceMembershipReadModel` to cairn-store
3. Wire `cairn_api::http::ApiError` and `CriticalEventSummary`

## 4. Integration Test E043x Errors (workspace --test "*")

Multiple integration test binaries also fail to compile due to the same ahead-of-implementation issue. Unique E0432 unresolved imports across all integration tests:

```
cairn_api::CriticalEventSummary
cairn_api::http::ApiError
cairn_domain::providers::RoutePolicyRule
cairn_memory::api_impl::SourceTagsApiImpl
cairn_memory::export_service_impl
cairn_memory::graph_expansion::related_documents_for
cairn_memory::import_service_impl
cairn_memory::in_memory::SourceSummary
cairn_memory::ingest::DocumentVersionReadModel
cairn_memory::pipeline::compute_chunk_quality
```

These all originate from integration test files that import from `cairn-app` or use advanced service layer types not yet implemented. The core unit test suite (741 tests) is unaffected.

## Summary

| Check | Status |
|---|---|
| `--workspace --lib` | ✅ 741/741 passing |
| `cross_backend_parity --features sqlite` | ✅ 16/16 passing |
| `lifecycle_integration` | ✅ 10/10 passing |
| `cairn-app` build | ❌ 40 errors (pre-existing, ahead-of-impl) |
| `--workspace --test "*"` E0432 errors | ❌ ~28 unique (same root cause) |

**Core implementation is healthy. cairn-app and integration test blockers are all ahead-of-implementation stubs, not regressions.**
