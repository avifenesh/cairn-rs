# Events, SSE, and A2A

Server-sent event streams (`/v1/stream`, `/v1/streams/runtime`, `/v1/sqeq/events`), raw event log reads and append, WebSocket upgrade, SQ/EQ command plane (RFC 021), and A2A task submission/status (RFC 021).

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 11**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `POST` | `/v1/a2a/tasks` | Preserve | body: A2A task shape; A2A task submission |
| `GET` | `/v1/a2a/tasks/:id` | Preserve | A2A task status |
| `GET` | `/v1/events` | Preserve |  |
| `POST` | `/v1/events/append` | Preserve |  |
| `GET` | `/v1/events/recent` | Preserve |  |
| `GET` | `/v1/sqeq/events` | Preserve | query: sqeq_session_id; SSE event stream (RFC 021) |
| `POST` | `/v1/sqeq/initialize` | Preserve | body: { protocol_versions, scope, subscriptions }; SQ/EQ session init |
| `POST` | `/v1/sqeq/submit` | Preserve | body: { method, correlation_id, params }; SQ/EQ command submission |
| `GET` | `/v1/stream` | Preserve | query: token?, lastEventId?; SSE stream with replay support |
| `GET` | `/v1/streams/runtime` | Preserve | query: token?; SSE runtime stream |
| `GET` | `/v1/ws` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
