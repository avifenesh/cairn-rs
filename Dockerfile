# cairn-rs — multi-stage production build
#
# Stages:
#   ui-builder  — Node 22 slim: npm ci && npm run build
#   builder     — Rust 1.82 slim: cargo build --release (embeds ui/dist via rust-embed)
#   runtime     — Debian bookworm-slim: minimal image with only the final binary
#
# Usage:
#   docker build -t cairn-rs .
#   docker run -p 3000:3000 -e CAIRN_ADMIN_TOKEN=secret cairn-rs
#
# With SQLite persistence:
#   docker run -p 3000:3000 -v /data:/data cairn-rs --db /data/cairn.db
#
# Connect your LLM provider (any OpenAI-compatible endpoint):
#   docker run -p 3000:3000 \
#     -e CAIRN_BRAIN_URL=https://your-provider/v1 \
#     -e OPENAI_COMPAT_API_KEY=your-key \
#     cairn-rs

# ── Stage 0: UI builder ───────────────────────────────────────────────────────
FROM node:22-slim AS ui-builder

WORKDIR /ui

# Install dependencies — separate layer so it's cached when only sources change.
COPY ui/package.json ui/package-lock.json* ./
RUN npm ci --prefer-offline

# Build the SPA.
COPY ui/ ./
RUN npm run build

# ── Stage 1: Rust builder ─────────────────────────────────────────────────────
FROM rust:1.82-slim AS builder

# Build dependencies needed by sqlx, openssl-sys, and rustls.
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# ── Dependency cache layer ────────────────────────────────────────────────────
# Copy workspace manifests first.  Cargo stubs each crate's entry-point so
# the dependency graph can be resolved and pre-compiled without the real source.
# This layer is reused on source-only changes.
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

# Stub all crate entry-points so cargo can resolve and fetch deps.
RUN for d in cairn-domain cairn-store cairn-runtime cairn-tools cairn-memory \
             cairn-graph cairn-agent cairn-evals cairn-signal cairn-channels \
             cairn-api cairn-plugin-proto; do \
      mkdir -p crates/$d/src && echo "// stub" > crates/$d/src/lib.rs; \
    done && \
    mkdir -p crates/cairn-app/src && \
    echo "fn main() {}" > crates/cairn-app/src/main.rs

# Stub ui/dist so rust-embed doesn't abort the deps build.
RUN mkdir -p ui/dist && touch ui/dist/.keep

# Pre-compile dependencies (failures are expected due to stubs — suppressed).
ENV SQLX_OFFLINE=true
RUN cargo build --release --bin cairn-app 2>/dev/null || true

# ── Real source build ─────────────────────────────────────────────────────────
COPY crates/ crates/

# Inject the production UI assets so rust-embed bakes them into the binary.
COPY --from=ui-builder /ui/dist /build/ui/dist

# Touch binary crate source so Cargo knows it changed.
RUN touch crates/cairn-app/src/main.rs

RUN cargo build --release --bin cairn-app

# ── Stage 2: runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

LABEL org.opencontainers.image.title="Cairn" \
      org.opencontainers.image.description="Self-hostable control plane for production AI agents" \
      org.opencontainers.image.source="https://github.com/avifenesh/cairn-rs" \
      org.opencontainers.image.version="0.1.0" \
      maintainer="Avi Fenesh <avifenesh@users.noreply.github.com>"

# Runtime dependencies:
#   ca-certificates — TLS for outbound LLM provider calls
#   curl            — HEALTHCHECK probe
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        curl && \
    rm -rf /var/lib/apt/lists/*

# Non-root user for defence-in-depth.
RUN useradd --system --no-create-home --shell /usr/sbin/nologin cairn

WORKDIR /app

COPY --from=builder /build/target/release/cairn-app /app/cairn-app

USER cairn

# ── Environment defaults ──────────────────────────────────────────────────────

# HTTP bind address and port.
ENV CAIRN_ADDR=0.0.0.0
ENV CAIRN_PORT=3000

# Deployment mode: "local" (no auth) or "team" (requires CAIRN_ADMIN_TOKEN).
ENV CAIRN_MODE=local

# Admin bearer token — always override in production.
ENV CAIRN_ADMIN_TOKEN=dev-admin-token

# LLM provider — Cairn is provider-agnostic. Connect any endpoint you prefer.
# All are optional; configure via env vars or POST /v1/providers/connections.
ENV CAIRN_BRAIN_URL=
ENV CAIRN_WORKER_URL=
ENV OPENAI_COMPAT_BASE_URL=
ENV OPENAI_COMPAT_API_KEY=
ENV OPENROUTER_API_KEY=
ENV OLLAMA_HOST=

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS "http://localhost:${CAIRN_PORT}/health" || exit 1

# Shell ENTRYPOINT so ENV vars expand; extra args are forwarded from CMD.
ENTRYPOINT ["/bin/sh", "-c", \
    "exec /app/cairn-app --addr \"$CAIRN_ADDR\" --port \"$CAIRN_PORT\" --mode \"$CAIRN_MODE\" \"$@\"", \
    "--"]

CMD []
