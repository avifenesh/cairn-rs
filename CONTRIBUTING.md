# Contributing to cairn-rs

Thank you for your interest in contributing. This document covers prerequisites,
development workflow, testing, and the pull-request process.

---

## Prerequisites

| Tool | Minimum version | Notes |
|------|-----------------|-------|
| Rust | 1.95 | `rustup update stable` — project pins via `rust-toolchain.toml` |
| Node.js | 22 | Required only for UI development |
| npm | 10 | Bundled with Node.js 22 |
| Ollama | any | Optional; enables local LLM tests |
| curl | any | Required for the smoke test (`scripts/smoke-test.sh`) |

Install Rust via [rustup](https://rustup.rs). Node.js via
[nvm](https://github.com/nvm-sh/nvm) or the official installer.

---

## Project structure

```
crates/
  cairn-domain/          Domain types, events, lifecycle rules, RFC contracts
  cairn-store/           Append-only event log + projections (InMemory / Postgres / SQLite)
  cairn-runtime/         Service implementations: sessions, runs, tasks, approvals, routing
  cairn-api/             HTTP route handlers (admin, evals, graph, memory, triggers)
  cairn-app/             Axum HTTP server, route handlers, embedded React UI (ui/dist/)
  cairn-memory/          Knowledge ingestion, chunking, retrieval pipeline
  cairn-graph/           Entity relationship graph
  cairn-evals/           Eval runs, rubrics, baselines, bandit experiments
  cairn-tools/           Tool invocation, stdio plugin host
  cairn-tools-derive/    Proc-macro helpers for built-in and plugin tool definitions
  cairn-agent/           Agent orchestration loop, ReAct / reflection / streaming
  cairn-orchestrator/    GATHER → DECIDE → EXECUTE loop, lifecycle control
  cairn-workspace/       Sandbox workspaces (OverlayFS / reflink-copy)
  cairn-integrations/    Integration plugin framework (GitHub, Linear, Notion, webhook)
  cairn-providers/       Unified LLM provider abstraction (13+ backends)
  cairn-github/          GitHub App client (JWT auth, webhook verification)
  cairn-plugin-catalog/  Plugin marketplace + catalog
  cairn-signal/          Signal routing
  cairn-channels/        Notification channels (Slack, email, webhook)
  cairn-plugin-proto/    Plugin wire protocol types
  cairn-fabric/          FlowFabric bridge — Valkey-native execution layer (feature-gated)

ui/                  React + TypeScript operator dashboard (Vite, Tailwind)
docs/
  design/rfcs/       RFC specifications
  design/            Architecture notes + FINALIZED runbook
  api-reference.md   Full endpoint reference
```

---

## Development workflow

### Run the server

```bash
# In-memory store, default token dev-admin-token, binds 127.0.0.1:3000
cargo run -p cairn-app

# With SQLite persistence
cargo run -p cairn-app -- --db cairn.db

# With Ollama for local LLM support
OLLAMA_HOST=http://localhost:11434 cargo run -p cairn-app

# With any OpenAI-compatible provider
OPENAI_COMPAT_BASE_URL=https://your-server/v1 \
OPENAI_COMPAT_API_KEY=your-key \
  cargo run -p cairn-app

# Bind all interfaces (for Docker / WSL access)
cargo run -p cairn-app -- --addr 0.0.0.0 --port 3000
```

After startup the operator dashboard is at **http://localhost:3000** and the
interactive API explorer is at **http://localhost:3000/v1/docs**.

### Incremental compile check

```bash
cargo check --workspace
```

### UI development

The UI is a Vite project under `ui/`. During development it runs its own dev
server on port 5173 and proxies `/v1/*` to the Rust server on port 3000.

```bash
cd ui
npm install
npm run dev          # Vite dev server — http://localhost:5173
```

When you want to embed the UI in the binary for a production build:

```bash
cd ui && npm run build   # outputs to ui/dist/
cargo build -p cairn-app # rust-embed picks up ui/dist/
```

---

## Testing

```bash
# Full workspace — 3 300+ tests (recommended before opening a PR)
cargo test --workspace

# A single crate
cargo test -p cairn-runtime

# End-to-end workflow tests (approval gates, bundles, prompts, evals)
cargo test -p cairn-app --test full_workspace_suite

# Smoke test against a running server (81 checks)
CAIRN_ADMIN_TOKEN=cairn-demo-token cargo run -p cairn-app &
CAIRN_TOKEN=cairn-demo-token ./scripts/smoke-test.sh

# UI type-check
cd ui && npx tsc --noEmit
```

The `full_workspace_suite` covers 6 end-to-end workflows (approval gate flow,
bundle import/export, prompt lifecycle, eval runs, operator setup, tool
invocations). The smoke test validates the real HTTP server binary across 20
sections including health, sessions, runs, tasks, approvals, events, memory,
SSE, admin, LLM generation, and more.

> **Adding integration tests?** CI runs `cargo test` with an explicit `-p`
> allow-list for crates that have non-empty `tests/` directories (see the
> `test` job in `.github/workflows/ci.yml`). If you add integration tests in
> a new crate, extend the allow-list in the same PR — otherwise your tests
> will never execute in CI.

---

## Code style

### Rust

Format all code before committing:

```bash
cargo fmt --all
```

Run Clippy and fix any warnings before opening a PR:

```bash
cargo clippy --workspace -- -D warnings
```

The CI pipeline enforces both; PRs with formatting or lint failures will not
be merged.

### TypeScript / React

The UI uses TypeScript's strict mode. Run a type-check after UI changes:

```bash
cd ui && npx tsc --noEmit
```

---

## RFC-driven development

cairn-rs features are specified by RFCs in `docs/design/rfcs/`. Each RFC
defines a contract (MUST / SHOULD requirements) and a corresponding
integration test in `crates/cairn-runtime/tests/` or
`crates/cairn-store/tests/` serves as the compliance proof.

If you are adding a significant new capability:

1. Write or update an RFC in `docs/design/rfcs/`.
2. Add an integration test that directly verifies the RFC's MUST clause.
3. Implement the feature.
4. Confirm the new test passes before submitting the PR.

Small fixes and improvements do not require an RFC.

---

## Pull request process

1. Fork the repository and create a branch from `main`.
2. Make your changes. Keep commits focused — one logical change per commit.
3. Ensure `cargo test --workspace` passes.
4. Ensure `cargo fmt --all` and `cargo clippy --workspace` produce no errors.
5. If you changed the UI, run `cd ui && npx tsc --noEmit` and rebuild the dist
   with `npm run build` if the change should be reflected in the embedded binary.
6. Open a pull request against `main`. Include a short description of what
   changed and why.

PRs are reviewed for correctness, test coverage, and RFC alignment. Larger
changes benefit from a brief design note in the PR description.

---

## License

By contributing you agree that your contributions will be licensed under the
[Business Source License 1.1](./LICENSE) that governs this repository.
