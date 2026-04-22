# Health, Metrics, and Service Metadata

Liveness, readiness, Prometheus scrape targets, OpenAPI/Swagger JSON, version, system info, and other unauthenticated or meta-surface endpoints. These routes are the minimum operator-facing surface for ops tooling.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 23**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/.well-known/agent.json` | Preserve | A2A Agent Card (RFC 021) |
| `GET` | `/docs` | Preserve |  |
| `GET` | `/health` | Preserve | { ok: boolean } |
| `GET` | `/health/ready` | Preserve |  |
| `GET` | `/healthz` | Preserve |  |
| `GET` | `/metrics` | Preserve |  |
| `GET` | `/openapi.json` | Preserve |  |
| `GET` | `/ready` | Preserve |  |
| `GET` | `/v1/changelog` | Preserve |  |
| `GET` | `/v1/costs` | Preserve | cost summary payload |
| `GET` | `/v1/db/status` | Preserve |  |
| `GET` | `/v1/docs` | Preserve |  |
| `GET` | `/v1/health/detailed` | Preserve |  |
| `GET` | `/v1/metrics` | Preserve | metrics read model |
| `GET` | `/v1/metrics/prometheus` | Preserve |  |
| `GET` | `/v1/openapi.json` | Preserve |  |
| `GET` | `/v1/rate-limit` | Preserve |  |
| `GET` | `/v1/stats` | Preserve |  |
| `GET` | `/v1/status` | Preserve | runtime/system status |
| `GET` | `/v1/system/info` | Preserve |  |
| `GET` | `/v1/system/role` | Preserve |  |
| `GET` | `/v1/telemetry/usage` | Preserve |  |
| `GET` | `/version` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
