# cairn-rs production Dockerfile
#
# Multi-stage build:
#   builder — compiles the full Rust workspace in release mode
#   runtime — minimal Debian image with only the binary
#
# Usage:
#   docker build -t cairn-rs .
#   docker run -p 3000:3000 cairn-rs
#
# With a persistent SQLite store:
#   docker run -p 3000:3000 -e CAIRN_DB=sqlite -v /data:/data cairn-rs
#
# Team mode (requires auth token):
#   docker run -p 3000:3000 -e CAIRN_ADMIN_TOKEN=<token> -e CAIRN_MODE=team cairn-rs

# ── Stage 1: builder ──────────────────────────────────────────────────────────
FROM rust:1.82-bookworm AS builder

WORKDIR /build

# Cache dependency compilation separately from source changes.
# Copy manifests first so the cargo fetch/build layer is reused when only
# src files change.
COPY Cargo.toml Cargo.lock ./
COPY crates/cairn-domain/Cargo.toml       crates/cairn-domain/Cargo.toml
COPY crates/cairn-store/Cargo.toml        crates/cairn-store/Cargo.toml
COPY crates/cairn-runtime/Cargo.toml      crates/cairn-runtime/Cargo.toml
COPY crates/cairn-tools/Cargo.toml        crates/cairn-tools/Cargo.toml
COPY crates/cairn-memory/Cargo.toml       crates/cairn-memory/Cargo.toml
COPY crates/cairn-graph/Cargo.toml        crates/cairn-graph/Cargo.toml
COPY crates/cairn-agent/Cargo.toml        crates/cairn-agent/Cargo.toml
COPY crates/cairn-evals/Cargo.toml        crates/cairn-evals/Cargo.toml
COPY crates/cairn-signal/Cargo.toml       crates/cairn-signal/Cargo.toml
COPY crates/cairn-channels/Cargo.toml     crates/cairn-channels/Cargo.toml
COPY crates/cairn-api/Cargo.toml          crates/cairn-api/Cargo.toml
COPY crates/cairn-plugin-proto/Cargo.toml crates/cairn-plugin-proto/Cargo.toml
COPY crates/cairn-app/Cargo.toml          crates/cairn-app/Cargo.toml

# Stub all lib/main entry-points so cargo can resolve and fetch deps without
# needing the full source tree. Each stub is replaced when the real src is
# copied below.
RUN for d in cairn-domain cairn-store cairn-runtime cairn-tools cairn-memory \
             cairn-graph cairn-agent cairn-evals cairn-signal cairn-channels \
             cairn-api cairn-plugin-proto; do \
      mkdir -p crates/$d/src && echo "// stub" > crates/$d/src/lib.rs; \
    done && \
    mkdir -p crates/cairn-app/src && \
    echo "fn main() {}" > crates/cairn-app/src/main.rs

# Pre-fetch and compile all dependencies (cached layer).
# SQLX_OFFLINE=true avoids database connectivity at build time.
ENV SQLX_OFFLINE=true
RUN cargo build --release --bin cairn-app 2>/dev/null || true

# Now copy the real sources and rebuild. Only changed crates are recompiled.
COPY crates/ crates/

# Touch main.rs so Cargo sees the binary source changed.
RUN touch crates/cairn-app/src/main.rs

RUN cargo build --release --bin cairn-app

# ── Stage 2: runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

LABEL org.opencontainers.image.title="Cairn" \
      org.opencontainers.image.description="Self-hostable control plane for production AI agents" \
      org.opencontainers.image.source="https://github.com/avifenesh/cairn-rs"

# Install runtime dependencies:
#   ca-certificates — TLS outbound calls to LLM providers
#   libssl3         — rustls/openssl-sys runtime requirement
#   curl            — HEALTHCHECK probe
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 \
        curl && \
    rm -rf /var/lib/apt/lists/*

# Non-root user for defence in depth.
RUN useradd --system --no-create-home --shell /usr/sbin/nologin cairn

WORKDIR /app

# Copy only the release binary — no debug artifacts, no build cache.
COPY --from=builder /build/target/release/cairn-app /app/cairn-app

USER cairn

# ── Configuration ─────────────────────────────────────────────────────────────

# CAIRN_PORT        — HTTP listen port.
ENV CAIRN_PORT=3000

# CAIRN_ADDR        — Bind address. 0.0.0.0 binds all interfaces (Docker default).
ENV CAIRN_ADDR=0.0.0.0

# CAIRN_MODE        — Startup mode: "local" (no auth) or "team" (requires CAIRN_ADMIN_TOKEN).
ENV CAIRN_MODE=local

# CAIRN_ADMIN_TOKEN — Bearer token for the admin account. Required in team mode.
ENV CAIRN_ADMIN_TOKEN=dev-admin-token

# CAIRN_DB          — Storage backend: "memory" (default) or path to SQLite file.
ENV CAIRN_DB=memory

EXPOSE 3000

# Health check against the liveness endpoint.
# --start-period gives the process time to initialise before failures count.
HEALTHCHECK --interval=30s --timeout=5s --start-period=15s --retries=3 \
    CMD curl -fsS "http://localhost:${CAIRN_PORT}/health" || exit 1

# Shell-form ENTRYPOINT so ENV vars expand into the command.
# Users can append flags via CMD: docker run cairn-rs --mode team --port 8080
ENTRYPOINT ["/bin/sh", "-c", \
    "exec /app/cairn-app --addr \"$CAIRN_ADDR\" --port \"$CAIRN_PORT\" --mode \"$CAIRN_MODE\" \"$@\"", \
    "--"]

# Default CMD: empty (all defaults come from ENV above).
CMD []
