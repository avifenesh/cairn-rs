# cairn-rs — multi-stage production build
#
# Stages:
#   ui-builder  — Node 22: npm install && npm run build
#   builder     — Rust 1.88: cargo build --release (clean, no stubs)
#   runtime     — Debian bookworm-slim: minimal final image
#
# Usage:
#   docker build -t cairn-rs .
#   docker run -p 3000:3000 -e CAIRN_ADMIN_TOKEN=secret cairn-rs
#
# With Postgres (default in docker-compose):
#   DATABASE_URL=postgres://user:pass@host/db docker run ... cairn-rs

# ── Stage 0: UI builder ───────────────────────────────────────────────────────
FROM node:22-slim AS ui-builder

WORKDIR /ui
COPY ui/package.json ui/package-lock.json* ./
RUN npm install
COPY ui/ ./
RUN npm run build

# ── Stage 1: Rust builder (single clean build, no stubs) ─────────────────────
FROM rust:1.88-bookworm AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        pkg-config \
        libssl-dev \
        ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy everything needed for the build.
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Inject the production UI assets so rust-embed bakes them into the binary.
COPY --from=ui-builder /ui/dist /build/ui/dist

# Single clean release build — no stubs, no cached artifacts, no surprises.
ENV SQLX_OFFLINE=true
RUN cargo build --release --bin cairn-app

# ── Stage 2: Runtime ──────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

LABEL org.opencontainers.image.title="Cairn" \
      org.opencontainers.image.description="Self-hostable control plane for production AI agents" \
      org.opencontainers.image.source="https://github.com/avifenesh/cairn-rs" \
      org.opencontainers.image.version="0.1.0" \
      maintainer="Avi Fenesh <avifenesh@users.noreply.github.com>"

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        curl && \
    rm -rf /var/lib/apt/lists/*

RUN useradd --system --no-create-home --shell /usr/sbin/nologin cairn

WORKDIR /app
COPY --from=builder /build/target/release/cairn-app /app/cairn-app
USER cairn

ENV CAIRN_ADDR=0.0.0.0
ENV CAIRN_PORT=3000
ENV CAIRN_MODE=local
ENV CAIRN_ADMIN_TOKEN=dev-admin-token
ENV CAIRN_BRAIN_URL=
ENV CAIRN_WORKER_URL=
ENV OPENAI_COMPAT_BASE_URL=
ENV OPENAI_COMPAT_API_KEY=
ENV OPENROUTER_API_KEY=
ENV OLLAMA_HOST=

EXPOSE 3000

HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS "http://localhost:${CAIRN_PORT}/health" || exit 1

ENTRYPOINT ["/bin/sh", "-c", \
    "exec /app/cairn-app --addr \"$CAIRN_ADDR\" --port \"$CAIRN_PORT\" --mode \"$CAIRN_MODE\" \"$@\"", \
    "--"]

CMD []
