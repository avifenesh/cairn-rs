# STATUS: CHANGELOG.md

**Task:** Write CHANGELOG.md for 0.1.0 release  
**File:** `CHANGELOG.md`  
**Lines:** 178 lines

Sections under ## [0.1.0] — 2026-04-05:

### Added
- Runtime/domain: 111 RuntimeEvent variants, RFC 002/005/006/007/008/009/013/014
- Storage: InMemoryStore (51 read models), PgEventLog, PgAdapter (7 models), PgSyncProjection, PgMigrationRunner (17 migrations)
- HTTP server: 16 routes, bearer token auth, SSE with replay, Postgres dual-write, CLI flags
- Knowledge pipeline: ingest, scoring, graph proximity (implemented), explain_result (implemented)
- Eval system: rubrics, baselines, bandit experimentation, binding cost stats (real implementation)
- Docs: api-reference.md (769 lines), deployment.md

### Architecture
- 12 crates, event log + synchronous projections, RFC 002-014 compliance tests

### Test suite table
- 796 lib + ~230 integration + 33 fixed = ~1059 total, 0 failures
- 40+ integration suites with notable ones listed

### Fixed
- 9 pre-existing test failures (root causes documented)
- DashboardOverview initializers, PgSyncProjection non-exhaustive patterns
