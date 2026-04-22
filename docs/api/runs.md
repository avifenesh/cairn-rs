# Runs

Agent run lifecycle: create/list/detail, orchestration, checkpoint, resume, cancel, SLA, cost alerts, intervention, replay, diagnose, and queue visibility (stalled / escalated / resume-due / sla-breached).

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 40**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/runs` | Preserve | query: session_id?, limit?; { items } |
| `POST` | `/v1/runs` | Preserve |  |
| `GET` | `/v1/runs/:id` | Preserve | path param: id; run detail |
| `GET` | `/v1/runs/:id/approvals` | Preserve |  |
| `POST` | `/v1/runs/:id/approve` | Preserve | body: { reviewer_comments? }; approve plan run |
| `GET` | `/v1/runs/:id/audit` | Preserve |  |
| `POST` | `/v1/runs/:id/cancel` | Preserve |  |
| `POST` | `/v1/runs/:id/checkpoint` | Preserve |  |
| `GET` | `/v1/runs/:id/checkpoint-strategy` | Preserve |  |
| `POST` | `/v1/runs/:id/checkpoint-strategy` | Preserve |  |
| `GET` | `/v1/runs/:id/children` | Preserve |  |
| `POST` | `/v1/runs/:id/claim` | Preserve |  |
| `GET` | `/v1/runs/:id/cost` | Preserve |  |
| `POST` | `/v1/runs/:id/cost-alert` | Preserve |  |
| `POST` | `/v1/runs/:id/diagnose` | Preserve |  |
| `GET` | `/v1/runs/:id/events` | Preserve |  |
| `GET` | `/v1/runs/:id/export` | Preserve |  |
| `POST` | `/v1/runs/:id/intervene` | Preserve |  |
| `GET` | `/v1/runs/:id/interventions` | Preserve |  |
| `POST` | `/v1/runs/:id/orchestrate` | Preserve |  |
| `POST` | `/v1/runs/:id/pause` | Preserve |  |
| `POST` | `/v1/runs/:id/recover` | Preserve |  |
| `POST` | `/v1/runs/:id/reject` | Preserve | body: { reason }; reject plan run |
| `GET` | `/v1/runs/:id/replay` | Preserve |  |
| `POST` | `/v1/runs/:id/replay-to-checkpoint` | Preserve |  |
| `POST` | `/v1/runs/:id/resume` | Preserve |  |
| `POST` | `/v1/runs/:id/revise` | Preserve | body: { reviewer_comments }; create revision plan run |
| `GET` | `/v1/runs/:id/sla` | Preserve |  |
| `POST` | `/v1/runs/:id/sla` | Preserve |  |
| `POST` | `/v1/runs/:id/spawn` | Preserve |  |
| `GET` | `/v1/runs/:id/tasks` | Preserve |  |
| `POST` | `/v1/runs/:id/tasks` | Preserve |  |
| `GET` | `/v1/runs/:id/tool-invocations` | Preserve |  |
| `POST` | `/v1/runs/batch` | Preserve |  |
| `GET` | `/v1/runs/cost-alerts` | Preserve | { items } |
| `GET` | `/v1/runs/escalated` | Preserve | { items } |
| `POST` | `/v1/runs/process-scheduled-resumes` | Preserve |  |
| `GET` | `/v1/runs/resume-due` | Preserve | { items } |
| `GET` | `/v1/runs/sla-breached` | Preserve | { items } |
| `GET` | `/v1/runs/stalled` | Preserve | { items } |

<!-- TODO: contract bodies (tracked as follow-up) -->
