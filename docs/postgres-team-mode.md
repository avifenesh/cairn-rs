# Postgres + Team Mode

How to transition from local/in-memory to persistent team mode with Postgres.

## Quick start (Docker)

```bash
# One command — starts Cairn + Postgres (connect your own LLM provider separately)
docker compose -f docker-compose.postgres.yml up --build

# Override the admin token
echo 'CAIRN_ADMIN_TOKEN=my-production-token' > .env
docker compose -f docker-compose.postgres.yml up -d

# Verify
./scripts/docker-health-check.sh postgres
```

## Manual setup (no Docker)

### 1. Create the database

```bash
createdb cairn
# Or with a specific user:
psql -c "CREATE USER cairn WITH PASSWORD 'your-password';"
psql -c "CREATE DATABASE cairn OWNER cairn;"
```

### 2. Start Cairn in team mode

```bash
CAIRN_ADMIN_TOKEN=your-token \
cargo run -p cairn-app -- \
  --mode team \
  --db postgres://cairn:your-password@localhost:5432/cairn
```

Migrations run automatically on first boot. No manual schema setup needed.

### 3. Verify

```bash
# Health check
curl http://localhost:3000/health

# Check store backend
curl -H "Authorization: Bearer your-token" http://localhost:3000/v1/settings
# → {"store_backend":"postgres","deployment_mode":"self_hosted_team",...}
```

## What changes in team mode

| Feature | Local mode | Team mode |
|---------|-----------|-----------|
| Auth | Token optional | Token required on every request |
| Store | In-memory (ephemeral) | Postgres (persistent) |
| Multi-tenant | Single default tenant | Multiple tenants via admin API |
| Restart | All data lost | All data persists |
| Operator tokens | Not available | Create per-operator tokens |
| Audit log | In-memory ring buffer | Persisted to Postgres |

## Creating tenants and operators

```bash
TOKEN=your-admin-token

# Create a tenant
curl -X POST http://localhost:3000/v1/admin/tenants \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"tenant_id":"acme","name":"Acme Corp"}'

# Create a workspace
curl -X POST http://localhost:3000/v1/admin/tenants/acme/workspaces \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"workspace_id":"engineering","name":"Engineering"}'

# Create a project
curl -X POST http://localhost:3000/v1/admin/tenants/acme/workspaces/engineering/projects \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"project_id":"agent-v1","name":"Agent v1"}'
```

## Data migration

Data from in-memory or SQLite mode does **not** automatically migrate to Postgres. To migrate:

1. Export from the running instance: `GET /v1/bundles/export?tenant_id=default&workspace_id=default&project_id=default`
2. Start the Postgres instance
3. Import: `POST /v1/bundles/import` with the exported bundle

## Verifying multi-tenant isolation

```bash
# Create two tenants
curl -X POST localhost:3000/v1/admin/tenants \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"tenant_id":"team-a","name":"Team A"}'

curl -X POST localhost:3000/v1/admin/tenants \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"tenant_id":"team-b","name":"Team B"}'

# Create a session for each
curl -X POST localhost:3000/v1/sessions \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"tenant_id":"team-a","workspace_id":"default","project_id":"default","session_id":"sess-a"}'

curl -X POST localhost:3000/v1/sessions \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"tenant_id":"team-b","workspace_id":"default","project_id":"default","session_id":"sess-b"}'

# Query — each tenant sees only their own data
curl "localhost:3000/v1/sessions?tenant_id=team-a&workspace_id=default&project_id=default" \
  -H "Authorization: Bearer $TOKEN"
# → only sess-a

curl "localhost:3000/v1/sessions?tenant_id=team-b&workspace_id=default&project_id=default" \
  -H "Authorization: Bearer $TOKEN"
# → only sess-b
```

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `connection refused` | Postgres not running or wrong host/port |
| `password authentication failed` | Check `POSTGRES_PASSWORD` matches the connection string |
| `database "cairn" does not exist` | Run `createdb cairn` or check Docker volume |
| `401 Unauthorized` | Team mode requires `CAIRN_ADMIN_TOKEN` on every request |
| Restart loses data | Verify `--db postgres://...` flag is set (not falling back to in-memory) |
