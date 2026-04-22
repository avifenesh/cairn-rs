# Graph & Traces

Provenance and execution-graph queries: dependency-path, execution-trace, multi-hop, prompt-provenance, retrieval-provenance, subgraph trace, and raw request traces.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 10**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/graph/dependency-path/:run_id` | Preserve |  |
| `GET` | `/v1/graph/execution-trace/:run_id` | Preserve |  |
| `GET` | `/v1/graph/multi-hop/:node_id` | Preserve |  |
| `GET` | `/v1/graph/prompt-provenance/:release_id` | Preserve |  |
| `GET` | `/v1/graph/provenance/:node_id` | Preserve |  |
| `GET` | `/v1/graph/retrieval-provenance/:run_id` | Preserve |  |
| `GET` | `/v1/graph/trace` | Preserve | query: root_id, kind; subgraph |
| `GET` | `/v1/trace/:trace_id` | Preserve |  |
| `GET` | `/v1/traces` | Preserve |  |
| `GET` | `/v1/traces/export` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
