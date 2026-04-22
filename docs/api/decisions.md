# Decisions & Policies

RFC 019 decision pipeline: evaluate, cache inspection, invalidate (by id / bulk / by rule), and the policy engine (`/v1/policies*`).

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 10**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/decisions` | Preserve | query: scope?, kind?, outcome?; { items } |
| `GET` | `/v1/decisions/:id` | Preserve | full decision with reasoning chain |
| `POST` | `/v1/decisions/:id/invalidate` | Preserve | body: { reason }; invalidate one cached decision |
| `GET` | `/v1/decisions/cache` | Preserve | { items } active cached decisions |
| `POST` | `/v1/decisions/evaluate` | Preserve | body: { kind, principal?, subject?, tenant_id?, workspace_id?, project_id?, correlation_id? }; { decision_id, outcome, source, cached, cache_hit, original_decision_id } via RFC 019 pipeline |
| `POST` | `/v1/decisions/invalidate` | Preserve | body: { scope, kind }; bulk invalidation |
| `POST` | `/v1/decisions/invalidate-by-rule` | Preserve | body: { rule_id }; selective invalidation via rule |
| `POST` | `/v1/policies` | Preserve |  |
| `GET` | `/v1/policies/decisions` | Preserve | query: limit?; { items } |
| `POST` | `/v1/policies/evaluate` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
