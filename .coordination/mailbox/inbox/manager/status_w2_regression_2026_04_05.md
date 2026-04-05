# Worker-2 Regression Check — 2026-04-05

**From:** Worker-2

---

## Results

| Test suite | Result |
|---|---|
| `cargo test -p cairn-store --test cross_backend_parity --features sqlite` | ✅ **16/16 passed** |
| `cargo test -p cairn-runtime --test lifecycle_integration` | ✅ **10/10 passed** |

Zero failures. Foundational integration tests are clean.

---

## Session summary

New tests added this session:

| File | Tests | RFC |
|---|---|---|
| `cairn-store/tests/event_persistence_contract.rs` | 18 | RFC 002 |
| `cairn-store/tests/workspace_role_hierarchy.rs` | 24 | RFC 008 |
| `cairn-store/tests/provider_health_schedule.rs` | 17 | RFC 009 |
| `cairn-app/src/main.rs` (bin tests) | 52 | RFC 002/005/006/007/008/009/010 |

**Total new tests: 111 — all passing.**

Full cairn-store test count: 21 lib + 18 persistence + 24 role hierarchy + 17 health schedule + 16 cross-backend parity = **96 cairn-store tests passing**.

