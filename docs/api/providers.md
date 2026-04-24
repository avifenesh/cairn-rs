# Providers

LLM provider connections, bindings, pools, policies, budget, health scheduling, Ollama adapter, and cost-ranking. Covers both tenant-configurable provider state and operator-observable provider health.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 37**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `POST` | `/v1/providers/:id/health-check` | Preserve |  |
| `POST` | `/v1/providers/:id/recover` | Preserve |  |
| `GET` | `/v1/providers/bindings` | Preserve | query: limit?; { items } |
| `POST` | `/v1/providers/bindings` | Preserve |  |
| `GET` | `/v1/providers/bindings/:id/cost-stats` | Preserve |  |
| `GET` | `/v1/providers/bindings/cost-ranking` | Preserve | { items } |
| `GET` | `/v1/providers/budget` | Preserve | budget summary |
| `POST` | `/v1/providers/budget` | Preserve |  |
| `GET` | `/v1/providers/connections` | Preserve | query: limit?; { items } |
| `POST` | `/v1/providers/connections` | Preserve |  |
| `DELETE` | `/v1/providers/connections/:id` | Preserve |  |
| `PUT` | `/v1/providers/connections/:id` | Preserve |  |
| `GET` | `/v1/providers/connections/:id/discover-models` | Preserve |  |
| `POST` | `/v1/providers/connections/discover-preview` | Preserve | ad-hoc discover BEFORE registration |
| `GET` | `/v1/providers/connections/:id/health-schedule` | Preserve |  |
| `POST` | `/v1/providers/connections/:id/health-schedule` | Preserve |  |
| `GET` | `/v1/providers/connections/:id/models` | Preserve |  |
| `POST` | `/v1/providers/connections/:id/models` | Preserve |  |
| `GET` | `/v1/providers/connections/:id/resolve-key` | Preserve |  |
| `PUT` | `/v1/providers/connections/:id/retry-policy` | Preserve |  |
| `GET` | `/v1/providers/connections/:id/test` | Preserve |  |
| `GET` | `/v1/providers/health` | Preserve | provider connection health |
| `POST` | `/v1/providers/ollama/delete` | Preserve |  |
| `POST` | `/v1/providers/ollama/generate` | Preserve |  |
| `GET` | `/v1/providers/ollama/models` | Preserve |  |
| `GET` | `/v1/providers/ollama/models/:name/info` | Preserve |  |
| `POST` | `/v1/providers/ollama/pull` | Preserve |  |
| `POST` | `/v1/providers/ollama/stream` | Preserve |  |
| `GET` | `/v1/providers/policies` | Preserve | query: limit?; { items } |
| `POST` | `/v1/providers/policies` | Preserve |  |
| `GET` | `/v1/providers/pools` | Preserve | { items } |
| `POST` | `/v1/providers/pools` | Preserve |  |
| `POST` | `/v1/providers/pools/:id/connections` | Preserve |  |
| `DELETE` | `/v1/providers/pools/:id/connections/:conn_id` | Preserve |  |
| `GET` | `/v1/providers/registry` | Preserve |  |
| `POST` | `/v1/providers/run-health-checks` | Preserve |  |
| `GET` | `/v1/models/catalog` | Preserve | bundled LiteLLM catalog; query: provider?, tier?, search?, supports_tools?, supports_json_mode?, reasoning?, max_cost_per_1m?, free_only?, limit?, offset?; { items, total, hasMore } |
| `GET` | `/v1/models/catalog/providers` | Preserve | unique providers in catalog with counts; cached |

<!-- TODO: contract bodies (tracked as follow-up) -->
