# Comprehensive Test Count — 2026-04-04

**From:** Worker-1  
**Subject:** Final lib test totals + integration test status

---

## Lib Test Results

| Crate | Tests | Status |
|---|---|---|
| cairn-domain | **148** | ✅ 0 failures |
| cairn-store | **21** | ✅ 0 failures |
| cairn-evals | **42** | ✅ 0 failures |
| cairn-memory | **92** | ✅ 0 failures |
| cairn-graph | **21** | ✅ 0 failures |
| cairn-tools | **114** | ✅ 0 failures |
| cairn-runtime | **208** | ✅ 0 failures |
| cairn-api | — | ❌ lib test compile fails (8 errors in test code; library itself builds clean) |

**Total passing lib tests: 646 across 7 crates (0 failures)**  
cairn-api: library builds clean; inline `#[cfg(test)]` modules have pre-existing field access errors on `BootstrapProvenance` not yet fixed.

---

## Integration Test Results

### `cairn-store --test cross_backend_parity --features sqlite`

```
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
```
✅ **16/16 passing** (fixed `mailbox_list_ordering_is_deterministic` by sorting list_by_run results alphabetically by message_id)

---

## cairn-runtime Improvement Summary

cairn-runtime lib tests improved from **177 passing / 29 failing** (at session start) to **208 passing / 0 failing**:

Fixed by implementing real InMemoryStore projections and read models for:
- Channels, credentials, guardrail policies, licenses, entitlement overrides
- Notification preferences/records, operator profiles, provider bindings/connections/pools/budgets
- Quotas, retention policies, route policies, run cost alerts, run SLA configs/breaches
- Resource shares, signal subscriptions, workspace memberships

cairn-runtime also gained `InMemoryServices` aggregate struct (via `aggregate.rs`) wiring all ~40 service instances.

---

## cairn-app Error Reduction

cairn-app errors reduced: **611 → 2** (99.7% reduction)  
Remaining 2 errors are pre-existing structural issues unrelated to stubs added.

Key fix: `InMemoryServices` populated with 40 real service fields eliminated ~335 E0609 field-access errors plus ~270 cascading E0277/E0282 errors.
