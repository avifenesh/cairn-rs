# Approvals

Human-in-the-loop approval queue: list pending, approve/deny/reject/delegate/resolve, and policy management. Backed by the approvals handler (`crates/cairn-app/src/handlers/approvals.rs`) and approval policy engine.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 17**

F45 unified the approval surface: the canonical family is `/v1/approvals/*`, returning a discriminated union keyed by `kind` (`plan` | `tool_call`). The pre-F45 `/v1/tool-call-approvals/*` paths are kept live for zero-downtime migration but 308-redirect here (preserving method + body) with `Deprecation: true` + `Link: <...>; rel="successor-version"` headers.

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/approval-policies` | Preserve | { items } |
| `POST` | `/v1/approval-policies` | Preserve |  |
| `GET` | `/v1/approvals` | Preserve | F45 unified list — merges plan + tool-call rows, each tagged with `kind`. Filters: `kind? run_id? session_id? state? tenant_id? workspace_id? project_id? limit? offset?`; returns `{ items, hasMore }`. |
| `GET` | `/v1/approvals/:id` | Preserve | F45 unified fetch — resolves tool-call first then plan; response carries `kind` discriminator. |
| `POST` | `/v1/approvals` | Preserve |  |
| `POST` | `/v1/approvals/:id/approve` | Preserve | F45 kind-aware. Plan: body ignored. Tool-call: `{ scope: once \| session{match_policy?}, approved_tool_args? }`. 400 on operator_id impersonation. |
| `PATCH` | `/v1/approvals/:id/amend` | Preserve | F45 — tool-call only; `{ new_tool_args }`. 422 on plan-approval id; 403 `self_amend_forbidden` on `tool_name=amend_approval`. |
| `POST` | `/v1/approvals/:id/delegate` | Preserve | Stub — 501 `not_implemented`. |
| `POST` | `/v1/approvals/:id/deny` | Preserve | Alias of `/reject`. |
| `POST` | `/v1/approvals/:id/reject` | Preserve | F45 kind-aware. Body `{ reason? }` applied to both kinds. |
| `POST` | `/v1/approvals/:id/resolve` | Preserve |  |
| `GET` | `/v1/approvals/pending` | Preserve |  |
| `GET` | `/v1/tool-call-approvals` | Deprecated | F45 — 308-redirects to `/v1/approvals?kind=tool_call` (query forwarded). |
| `GET` | `/v1/tool-call-approvals/:call_id` | Deprecated | F45 — 308-redirects to `/v1/approvals/:id`. |
| `POST` | `/v1/tool-call-approvals/:call_id/approve` | Deprecated | F45 — 308-redirects to `/v1/approvals/:id/approve`. |
| `POST` | `/v1/tool-call-approvals/:call_id/reject` | Deprecated | F45 — 308-redirects to `/v1/approvals/:id/reject`. |
| `PATCH` | `/v1/tool-call-approvals/:call_id/amend` | Deprecated | F45 — 308-redirects to `/v1/approvals/:id/amend`. |

<!-- TODO: contract bodies (tracked as follow-up) -->
