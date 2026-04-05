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
#   docker run -p 3000:3000 -e CAIRN_ADMIN_TOKEN=<token> cairn-rs --mode team

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

# Install ca-certificates (for TLS outbound calls to LLM providers) and
# libssl (required by rustls/openssl-sys at runtime on some builds).
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        ca-certificates \
        libssl3 && \
    rm -rf /var/lib/apt/lists/*

# Non-root user for defence in depth.
RUN useradd --system --no-create-home --shell /usr/sbin/nologin cairn

WORKDIR /app

COPY --from=builder /build/target/release/cairn-app /app/cairn-app

# Ensure the binary is executable.
RUN chmod +x /app/cairn-app

USER cairn

# ── Configuration ─────────────────────────────────────────────────────────────

# CAIRN_PORT   — HTTP listen port (cairn-app reads --port from CLI; this env
#                var is used by the ENTRYPOINT shell form below).
ENV CAIRN_PORT=3000

# CAIRN_DB     — Storage backend: "memory" (default) or "sqlite".
ENV CAIRN_DB=memory

# CAIRN_ADDR   — Bind address. 0.0.0.0 binds all interfaces (Docker default).
ENV CAIRN_ADDR=0.0.0.0

EXPOSE 3000

# Use shell form so ENV vars expand.  The app also supports --mode, --port,
# and --addr flags; pass them as CMD overrides:
#   docker run cairn-rs --mode team --port 8080
ENTRYPOINT ["/app/cairn-app", "--addr", "0.0.0.0", "--port", "3000"]

# Default CMD: local in-memory mode (no auth required).
CMD ["--mode", "local"]
