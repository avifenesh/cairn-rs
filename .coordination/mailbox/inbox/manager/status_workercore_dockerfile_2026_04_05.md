# Status Update — Worker Core

## Task: Dockerfile (deployment packaging)
- **Files created**: Dockerfile, .dockerignore
- **Files changed**: none
- **Issues**: none

## Design decisions
- Multi-stage build: rust:1.82-bookworm builder → debian:bookworm-slim runtime
- Dependency cache layer: manifests copied first, stubs compiled, then real sources override — typical Rust Docker pattern
- SQLX_OFFLINE=true to avoid DB at build time (sqlx compile-time query checking)
- Non-root user (cairn) for defence in depth
- ca-certificates + libssl3 installed for TLS outbound LLM calls (axum-server uses tls-rustls)
- ENTRYPOINT uses --addr 0.0.0.0 --port 3000 to override default 127.0.0.1 bind
- CMD --mode local overrideable at docker run time (e.g. --mode team)
- .dockerignore excludes target/ .git/ .coordination/ .env* and IDE files

## Usage
  docker build -t cairn-rs .
  docker run -p 3000:3000 cairn-rs
  # team mode:
  docker run -p 3000:3000 -e CAIRN_ADMIN_TOKEN=<token> cairn-rs --mode team
