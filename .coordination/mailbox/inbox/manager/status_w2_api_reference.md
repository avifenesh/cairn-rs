# STATUS: api-reference.md

**Task:** Write operator API reference documentation  
**File:** `docs/api-reference.md`  
**Lines:** ~500

Covers all 15 routes extracted from crates/cairn-app/src/main.rs:
- GET /health (public)
- GET /v1/stream (public, SSE, RFC 002 replay)
- GET /v1/status
- GET /v1/dashboard
- GET /v1/runs + /v1/runs/:id
- GET /v1/sessions
- GET /v1/approvals/pending + POST /v1/approvals/:id/resolve
- GET /v1/prompts/assets + /v1/prompts/releases
- GET /v1/costs
- GET /v1/providers
- GET /v1/events + POST /v1/events/append (idempotency explained)

Each endpoint: method, path, description, query params, request body,
response example, error cases, curl examples.
Includes: auth guide, error response table, server config reference,
route summary table.
