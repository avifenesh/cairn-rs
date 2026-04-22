# Prompts

Prompt asset registry, versioning, render, release lifecycle (rollout / activate / rollback / transition / approval), and history. Governed by the `cairn-evals` release state machine.

Source of truth: [`tests/compat/http_routes.tsv`](../../tests/compat/http_routes.tsv). Drift from this table against the live router is enforced by `cargo test -p cairn-api --test compat_catalog_sync`.

**Routes: 16**

| Method | Path | Classification | Notes |
|---|---|---|---|
| `GET` | `/v1/prompts/assets` | Preserve | query: limit?; { items } |
| `POST` | `/v1/prompts/assets` | Preserve |  |
| `GET` | `/v1/prompts/assets/:id/versions` | Preserve |  |
| `POST` | `/v1/prompts/assets/:id/versions` | Preserve |  |
| `GET` | `/v1/prompts/assets/:id/versions/:version_id/diff` | Preserve |  |
| `POST` | `/v1/prompts/assets/:id/versions/:version_id/render` | Preserve |  |
| `GET` | `/v1/prompts/assets/:id/versions/:version_id/template-vars` | Preserve |  |
| `GET` | `/v1/prompts/releases` | Preserve | query: limit?; { items } |
| `POST` | `/v1/prompts/releases` | Preserve |  |
| `POST` | `/v1/prompts/releases/:id/activate` | Preserve |  |
| `GET` | `/v1/prompts/releases/:id/history` | Preserve |  |
| `POST` | `/v1/prompts/releases/:id/request-approval` | Preserve |  |
| `POST` | `/v1/prompts/releases/:id/rollback` | Preserve |  |
| `POST` | `/v1/prompts/releases/:id/rollout` | Preserve |  |
| `POST` | `/v1/prompts/releases/:id/transition` | Preserve |  |
| `POST` | `/v1/prompts/releases/compare` | Preserve |  |

<!-- TODO: contract bodies (tracked as follow-up) -->
