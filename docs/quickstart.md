# Cairn Quick-Start Guide

## Prerequisites

- **Rust 1.82+** — install via [rustup.rs](https://rustup.rs)
- **Docker + Docker Compose** (optional, for the containerised path)

---

## 1 — Clone and build

```bash
git clone https://github.com/avifenesh/cairn-rs.git
cd cairn-rs
cargo build -p cairn-app
```

The first build fetches all dependencies and takes a few minutes. Subsequent builds are incremental.

---

## 2 — Start the server

```bash
CAIRN_ADMIN_TOKEN=your-token cargo run -p cairn-app
```

The server listens on **`http://localhost:3000`** by default.

| Env var | Default | Description |
|---|---|---|
| `CAIRN_ADMIN_TOKEN` | *(required)* | Bearer token for operator API access |
| `CAIRN_PORT` | `3000` | HTTP port |
| `CAIRN_LOG` | `info` | Log level (`trace`/`debug`/`info`/`warn`/`error`) |

## 3 — Verify the server is up

```bash
curl http://localhost:3000/health   # → {"ok":true}
```

## 4 — First API call

```bash
curl -H 'Authorization: Bearer your-token' http://localhost:3000/v1/dashboard
```

Returns the operator dashboard payload with live run/task counts.

## 5 — Common endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Liveness check |
| `GET` | `/v1/overview` | Operator overview |
| `GET` | `/v1/runs` | List active runs |
| `GET` | `/v1/tasks` | List tasks |
| `GET` | `/v1/approvals` | Approval inbox |
| `GET` | `/v1/streams/runtime` | SSE real-time stream |

See [`docs/api-quick-reference.md`](./api-quick-reference.md) for the full route list.

---

## 6 — Docker (recommended for production)

```bash
docker compose up
```

This starts the server with a Postgres backend. The admin token is set in
`docker-compose.yml` — change it before deploying.

---

## Next steps

- Browse the [API reference](./api-quick-reference.md)
- Read the [session changelog](../CHANGELOG-session.md) for recent RFC fixes
- Run the test suite: `cargo test --workspace`
