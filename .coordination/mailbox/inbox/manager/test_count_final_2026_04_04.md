# Final Comprehensive Test Report — 2026-04-04

**From:** Worker-1  
**Subject:** Complete lib test totals + cross_backend_parity

---

## Lib Test Results

| Crate | Tests Passing | Failures | Status |
|---|---|---|---|
| cairn-domain | **148** | 0 | ✅ |
| cairn-store | **21** | 0 | ✅ |
| cairn-evals | **42** | 0 | ✅ |
| cairn-memory | **92** | 0 | ✅ |
| cairn-graph | **21** | 0 | ✅ |
| cairn-tools | **114** | 0 | ✅ |
| cairn-runtime | **208** | 0 | ✅ |
| cairn-api | — | — | lib test compilation fails (pre-existing `BootstrapProvenance` field errors in `#[cfg(test)]` blocks; Worker-1 is fixing) |

**TOTAL: 646 lib tests passing, 0 failures** (7 of 8 crates)

---

## Integration Tests

### `cairn-store --test cross_backend_parity --features sqlite`
```
test result: ok. 16 passed; 0 failed
```
✅ **16/16 passing**

Fix applied: `mailbox_list_ordering_is_deterministic` — sorted `list_by_run` results by `message_id` alphabetically (InMemoryStore was returning insertion order; test expects deterministic alphabetical order).

---

## Session Summary

### Tests gained this session (cairn-runtime: 177→208)
- +31 tests from implementing real InMemoryStore projections and read models for all new event types
- Resource sharing projection/read model fixed (ResourceShared/ResourceShareRevoked)
- Mailbox ordering determinism fixed

### New constructors added (for cairn-app compatibility)
- `EvalRunService::with_graph_and_event_log<G, S>(graph, store) -> Self` (stub)
- `EvalRunService::with_memory_diagnostics<D>(self, diagnostics) -> Self` (stub)
- `InMemoryRetrieval::with_diagnostics(store, diagnostics) -> Self` (stub)
- `InMemoryRetrieval::with_graph(self, graph) -> Self` (stub)

### cairn-app error reduction
- 611 → 185 errors (70% reduction)
- Primary fix: `InMemoryServices` populated with 40 real service fields

---

## cairn-app Remaining Error Summary (185 errors)

| Category | Count | Nature |
|---|---|---|
| E0599 | 57 | Missing methods on service impls (`spawn_subagent`, `set_priority`, `register_source`, etc.) |
| E0560 | 36 | Wrong struct fields in cairn-app initializers |
| E0308 | 28 | Type mismatches |
| E0609 | 25 | Missing fields on domain structs |
| E0061 | 20 | Wrong argument counts |

All remaining errors are structural cairn-app issues requiring deeper service method implementations.
