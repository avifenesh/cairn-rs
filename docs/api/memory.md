# Memory, Sources, and Ingestion

Knowledge-plane surface: memory documents, vector search, provenance, versions, deep-search, embed, feedback, ingest jobs, source connections, chunks, quality, refresh scheduling, and the UI-facing `/v1/memories` collection.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 30**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/ingest/jobs` | Preserve | query: limit?; { items } |
| `POST` | `/v1/ingest/jobs` | Preserve |  |
| `GET` | `/v1/ingest/jobs/:id` | Preserve |  |
| `POST` | `/v1/ingest/jobs/:id/complete` | Preserve |  |
| `POST` | `/v1/ingest/jobs/:id/fail` | Preserve |  |
| `GET` | `/v1/memories` | Preserve | query: status?, category?; { items, hasMore } |
| `POST` | `/v1/memories` | Preserve | body: { content, category }; create memory response object |
| `POST` | `/v1/memories/:id/accept` | Preserve | path param: id; { ok } |
| `POST` | `/v1/memories/:id/reject` | Preserve | path param: id; { ok } |
| `GET` | `/v1/memories/search` | Preserve | query: q required, limit?; { items } |
| `POST` | `/v1/memory/deep-search` | Preserve |  |
| `GET` | `/v1/memory/diagnostics` | Preserve | index status |
| `GET` | `/v1/memory/documents/:id` | Preserve |  |
| `GET` | `/v1/memory/documents/:id/versions` | Preserve |  |
| `POST` | `/v1/memory/embed` | Preserve |  |
| `POST` | `/v1/memory/feedback` | Preserve |  |
| `POST` | `/v1/memory/ingest` | Preserve |  |
| `GET` | `/v1/memory/provenance/:document_id` | Preserve |  |
| `GET` | `/v1/memory/related/:document_id` | Preserve |  |
| `GET` | `/v1/memory/search` | Preserve | query: q, limit?; { results } |
| `GET` | `/v1/sources` | Preserve | source connection health |
| `POST` | `/v1/sources` | Preserve |  |
| `DELETE` | `/v1/sources/:id` | Preserve |  |
| `GET` | `/v1/sources/:id` | Preserve |  |
| `PUT` | `/v1/sources/:id` | Preserve |  |
| `GET` | `/v1/sources/:id/chunks` | Preserve |  |
| `GET` | `/v1/sources/:id/quality` | Preserve |  |
| `GET` | `/v1/sources/:id/refresh-schedule` | Preserve |  |
| `POST` | `/v1/sources/:id/refresh-schedule` | Preserve |  |
| `POST` | `/v1/sources/process-refresh` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
