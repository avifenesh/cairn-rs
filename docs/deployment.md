# cairn-rs Deployment Guide

## Quick start

```bash
git clone https://github.com/your-org/cairn-rs
cd cairn-rs
docker compose up --build
```

The control plane starts at **http://localhost:3000**.  
Health check: `curl http://localhost:3000/health` → `{"status":"ok"}`

---

## Docker Compose (recommended)

```bash
# Start (foreground)
docker compose up --build

# Start (background)
docker compose up -d --build

# Stop, keep data
docker compose down

# Stop and wipe Postgres data
docker compose down -v
```

Services started:
| Service | Port | Notes |
|---|---|---|
| `cairn` | 3000 | Control-plane HTTP API |
| `postgres` | 5432 | PostgreSQL 16 (host-accessible for inspection) |

---

## Production setup with external Postgres

Skip the bundled Postgres and point cairn at your own database:

```bash
docker run -d \
  --name cairn \
  -p 3000:3000 \
  -e CAIRN_ADMIN_TOKEN="$(openssl rand -hex 32)" \
  cairn-rs \
  --mode team \
  --addr 0.0.0.0 \
  --port 3000 \
  --db "postgres://cairn:password@db.internal:5432/cairn"
```

The database is created and migrated automatically on first start.

---

## Environment variables

| Variable | Default | Description |
|---|---|---|
| `CAIRN_ADMIN_TOKEN` | random (local) / **required** (team) | Bearer token for operator API auth. Set to a random 32-byte hex string in production. |
| `CAIRN_PORT` | `3000` | HTTP listen port (also settable with `--port`). |
| `CAIRN_DB` | `memory` | Documentation-only. Pass the storage backend via `--db` flag. |
| `CAIRN_MODE` | `local` | Documentation-only. Pass via `--mode local\|team`. |

> **Note:** cairn-app reads storage configuration from CLI flags (`--db`, `--mode`), not environment variables. The env vars above are listed for documentation completeness.

### CLI flags reference

```
cairn-app [flags]

  --mode   local|team    Deployment mode (default: local)
  --addr   <ip>          Bind address (default: 127.0.0.1; use 0.0.0.0 for Docker)
  --port   <port>        HTTP listen port (default: 3000)
  --db     <dsn>         Storage backend:
                           postgres://user:pass@host:5432/db  — PostgreSQL
                           /path/to/data.db                   — SQLite
                           (omit for in-memory, local dev only)
  --tls-cert <path>      Path to TLS certificate file (PEM)
  --tls-key  <path>      Path to TLS private key file (PEM)
```

---

## Health check

```
GET /health
```

Returns `200 OK` with `{"status":"ok"}`. No authentication required.  
Safe to use as a load-balancer health probe and liveness check.

```bash
curl -sf http://localhost:3000/health
```

---

## TLS setup

Provide PEM-encoded certificate and key files:

```bash
cairn-app \
  --mode team \
  --addr 0.0.0.0 \
  --port 443 \
  --tls-cert /etc/cairn/tls/cert.pem \
  --tls-key  /etc/cairn/tls/key.pem \
  --db "postgres://cairn:password@localhost:5432/cairn"
```

With Docker:

```bash
docker run -d \
  -p 443:443 \
  -v /etc/cairn/tls:/tls:ro \
  -e CAIRN_ADMIN_TOKEN="..." \
  cairn-rs \
  --mode team \
  --addr 0.0.0.0 \
  --port 443 \
  --tls-cert /tls/cert.pem \
  --tls-key  /tls/key.pem \
  --db "postgres://cairn:password@db.internal:5432/cairn"
```

> **TLS is required in `--mode team`.** cairn-app will refuse to start in team mode without a TLS certificate.

Let's Encrypt with Certbot:

```bash
certbot certonly --standalone -d cairn.example.com
# Certificates written to /etc/letsencrypt/live/cairn.example.com/
--tls-cert /etc/letsencrypt/live/cairn.example.com/fullchain.pem
--tls-key  /etc/letsencrypt/live/cairn.example.com/privkey.pem
```

---

## Systemd service (non-Docker)

Install the binary:

```bash
cp target/release/cairn-app /usr/local/bin/cairn-app
chmod 755 /usr/local/bin/cairn-app
```

Create `/etc/systemd/system/cairn.service`:

```ini
[Unit]
Description=cairn control-plane
Documentation=https://github.com/your-org/cairn-rs
After=network.target postgresql.service
Requires=postgresql.service

[Service]
Type=simple
User=cairn
Group=cairn

# Admin token — store in /etc/cairn/env or use systemd-creds
EnvironmentFile=/etc/cairn/env

ExecStart=/usr/local/bin/cairn-app \
  --mode team \
  --addr 0.0.0.0 \
  --port 3000 \
  --tls-cert /etc/cairn/tls/cert.pem \
  --tls-key  /etc/cairn/tls/key.pem \
  --db "postgres://cairn:password@localhost:5432/cairn"

Restart=on-failure
RestartSec=5s
TimeoutStopSec=30s

# Harden the process.
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
ReadWritePaths=/var/lib/cairn

[Install]
WantedBy=multi-user.target
```

`/etc/cairn/env`:

```bash
CAIRN_ADMIN_TOKEN=your-32-byte-hex-token-here
```

Enable and start:

```bash
# Create the cairn system user
useradd --system --no-create-home --shell /usr/sbin/nologin cairn

# Create data directory
install -d -o cairn -g cairn /var/lib/cairn

sudo systemctl daemon-reload
sudo systemctl enable cairn
sudo systemctl start cairn
sudo systemctl status cairn

# Tail logs
journalctl -u cairn -f
```

---

## Upgrading

```bash
# Docker Compose
docker compose pull
docker compose up -d --build

# Systemd
systemctl stop cairn
cp target/release/cairn-app /usr/local/bin/cairn-app
systemctl start cairn
```

Migrations run automatically on startup.
