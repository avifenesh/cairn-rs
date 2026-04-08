# Cairn Quick-Start Guide

Get a working Cairn instance in under 5 minutes.

## Prerequisites

- **Rust 1.82+** — install via [rustup.rs](https://rustup.rs)
- **Node.js 18+** — for the dashboard UI build

---

## 1 — Clone and build

```bash
git clone https://github.com/avifenesh/cairn-rs.git
cd cairn-rs

# Build the UI first (embedded in the binary)
cd ui && npm install && npm run build && cd ..

# Build the server
cargo build -p cairn-app
```

First build takes a few minutes. Subsequent builds are incremental.

---

## 2 — Start the server

```bash
CAIRN_ADMIN_TOKEN=dev-admin-token cargo run -p cairn-app
```

The server starts at **`http://localhost:3000`**. Dashboard, API, and Swagger docs all on one port.

Cairn is a platform — it doesn't bundle or require any specific LLM provider. Connect your own via the Providers page in the dashboard or via environment variables.

| Env var | Default | Description |
|---|---|---|
| `CAIRN_ADMIN_TOKEN` | *(required)* | Bearer token for API access |
| `CAIRN_BRAIN_URL` | — | OpenAI-compatible endpoint for heavy generation |
| `CAIRN_WORKER_URL` | — | OpenAI-compatible endpoint for everyday tasks + embeddings |
| `OPENAI_COMPAT_API_KEY` | — | API key for the OpenAI-compatible endpoints above |
| `OPENROUTER_API_KEY` | — | OpenRouter API key |
| `OLLAMA_HOST` | — | Ollama endpoint |
| `CAIRN_PORT` | `3000` | HTTP port |

Any OpenAI-compatible endpoint works: OpenAI, Anthropic (via proxy), Bedrock, Vertex AI, OpenRouter, Ollama, Groq, Together, etc.

---

## 3 — Open the dashboard

Go to `http://localhost:3000` in your browser. You'll see a login page — enter your admin token (`dev-admin-token`).

The dashboard has 30 operator pages: runs, tasks, approvals, memory, evals, providers, and more.

---

## 4 — Connect a provider

Navigate to the **Providers** page in the dashboard, or set env vars before starting the server:

```bash
# Example: any OpenAI-compatible endpoint
CAIRN_BRAIN_URL=https://your-provider.com/v1 \
OPENAI_COMPAT_API_KEY=your-key \
CAIRN_ADMIN_TOKEN=dev-admin-token \
cargo run -p cairn-app
```

You can also configure providers at runtime via `POST /v1/providers/connections`.

---

## 5 — First workflow: session → run → task (2 minutes)

```bash
TOKEN=dev-admin-token

# Create a session
curl -s -X POST http://localhost:3000/v1/sessions \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id":"default","workspace_id":"default","project_id":"default","session_id":"my-first-session"}'

# Start a run in that session
curl -s -X POST http://localhost:3000/v1/runs \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id":"default","workspace_id":"default","project_id":"default","session_id":"my-first-session","run_id":"my-first-run"}'

# Create a task via event append
curl -s -X POST http://localhost:3000/v1/events/append \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '[{"event_id":"evt_task_1","source":{"source_type":"runtime"},"ownership":{"scope":"project","tenant_id":"default","workspace_id":"default","project_id":"default"},"causation_id":null,"correlation_id":null,"payload":{"event":"task_created","project":{"tenant_id":"default","workspace_id":"default","project_id":"default"},"task_id":"my-first-task","parent_run_id":"my-first-run","parent_task_id":null,"prompt_release_id":null}}]'

# See it in the dashboard
curl -s http://localhost:3000/v1/dashboard \
  -H "Authorization: Bearer $TOKEN" \
  -d 'tenant_id=default&workspace_id=default&project_id=default' | python3 -m json.tool
```

Refresh the dashboard — you'll see active runs and tasks.

---

## 6 — Approval workflow (2 minutes)

```bash
# Request approval
curl -s -X POST http://localhost:3000/v1/events/append \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '[{"event_id":"evt_appr_1","source":{"source_type":"runtime"},"ownership":{"scope":"project","tenant_id":"default","workspace_id":"default","project_id":"default"},"causation_id":null,"correlation_id":null,"payload":{"event":"approval_requested","project":{"tenant_id":"default","workspace_id":"default","project_id":"default"},"approval_id":"appr-1","run_id":"my-first-run","task_id":null,"requirement":"required"}}]'

# Check pending approvals (also visible in dashboard)
curl -s http://localhost:3000/v1/approvals/pending \
  -H "Authorization: Bearer $TOKEN"

# Approve it
curl -s -X POST http://localhost:3000/v1/approvals/appr-1/resolve \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"decision":"approved","reason":"looks good"}'
```

---

## 7 — Memory ingestion + search (1 minute)

```bash
# Ingest a document
curl -s -X POST http://localhost:3000/v1/memory/ingest \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"source_id":"docs","document_id":"doc1","content":"Cairn is an all-in-one control plane for AI agent deployments. It provides session management, approval workflows, cost tracking, and prompt versioning.","tenant_id":"default","workspace_id":"default","project_id":"default"}'

# Search it
curl -s "http://localhost:3000/v1/memory/search?query_text=agent+control+plane&tenant_id=default&workspace_id=default&project_id=default&limit=5" \
  -H "Authorization: Bearer $TOKEN" | python3 -m json.tool
```

---

## 8 — LLM orchestration (requires a connected provider)

```bash
curl -s -X POST "http://localhost:3000/v1/runs/my-first-run/orchestrate" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"goal":"Summarize what Cairn does based on ingested memory.","max_iterations":2,"timeout_ms":30000}'
```

After orchestration, check LLM traces:

```bash
curl -s "http://localhost:3000/v1/sessions/my-first-session/llm-traces" \
  -H "Authorization: Bearer $TOKEN" | python3 -m json.tool
```

---

## 9 — Run the smoke test

```bash
CAIRN_TOKEN=dev-admin-token ./scripts/smoke-test.sh
```

Runs 90+ checks against all API endpoints. Takes ~5 seconds.

---

## 10 — SDK example

A complete Python SDK example is at `examples/basic-agent.py`:

```bash
python3 examples/basic-agent.py
```

Exercises the full lifecycle: session, run, task, approval, eval, cancel, observability.

---

## Common gotchas

| Problem | Fix |
|---------|-----|
| **401 Unauthorized** | Set `CAIRN_ADMIN_TOKEN` and pass it as `Authorization: Bearer <token>` |
| **In-memory store loses data on restart** | Expected — use `--db cairn.db` for SQLite persistence or `--db postgres://...` for Postgres |
| **Orchestrate returns 503** | No LLM provider configured — connect one via Providers page or env vars |
| **Dashboard shows "disconnected"** | Check the SSE stream at `/v1/streams/runtime?token=<your-token>` |

---

## Next steps

- Browse the [API reference](./api-quick-reference.md) for the full route catalog
- Read `CLAUDE.md` at the repo root for architecture details
- Run `cargo test --workspace` for the full test suite (~2700 tests)
- Switch to team mode: `cargo run -p cairn-app -- --mode team --db postgres://user:pass@host/db`
