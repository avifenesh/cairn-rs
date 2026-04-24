# Approvals

Human-in-the-loop approval queue: list pending, approve/deny/reject/delegate/resolve, and policy management. Backed by the approvals handler (`crates/cairn-app/src/handlers/approvals.rs`) and approval policy engine.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 15**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/approval-policies` | Preserve | { items } |
| `POST` | `/v1/approval-policies` | Preserve |  |
| `GET` | `/v1/approvals` | Preserve | query: status?; { items, hasMore } |
| `POST` | `/v1/approvals` | Preserve |  |
| `POST` | `/v1/approvals/:id/approve` | Preserve | path param: id; { ok } |
| `POST` | `/v1/approvals/:id/delegate` | Preserve |  |
| `POST` | `/v1/approvals/:id/deny` | Preserve | path param: id; { ok } |
| `POST` | `/v1/approvals/:id/reject` | Preserve |  |
| `POST` | `/v1/approvals/:id/resolve` | Preserve |  |
| `GET` | `/v1/approvals/pending` | Preserve |  |
| `GET` | `/v1/tool-call-approvals` | Preserve | BP-v2 tool-call inbox; filter by run_id / session_id / project triple + state; limit/offset + hasMore |
| `GET` | `/v1/tool-call-approvals/:call_id` | Preserve | single record; 404 on cross-tenant |
| `POST` | `/v1/tool-call-approvals/:call_id/approve` | Preserve | body: `{ scope, approved_tool_args? }`; scope is `{type: once \| session{match_policy?}}`; 400 on operator_id impersonation |
| `POST` | `/v1/tool-call-approvals/:call_id/reject` | Preserve | body: `{ reason? }` |
| `PATCH` | `/v1/tool-call-approvals/:call_id/amend` | Preserve | non-resolving preview edit; 403 `self_amend_forbidden` on `tool_name=amend_approval` |

<!-- TODO: contract bodies (tracked as follow-up) -->
