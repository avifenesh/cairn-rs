# Status Update — Worker Core

## Task: docker-compose.yml
- **Files created**: docker-compose.yml
- **Files changed**: none
- **Issues**: none
- **Notable**:
  - cairn-app uses --db postgres://... CLI flag (not env var) for storage config. docker-compose uses command: override to pass this. The CAIRN_DB env var in Dockerfile is documentation-only.
  - depends_on with condition: service_healthy waits for pg_isready before starting cairn.
  - healthcheck uses curl -sf on /health (not /healthz — the real path per bootstrap.rs:904).
  - postgres port 5432 exposed for local inspection with note to remove in production.
  - CAIRN_ADMIN_TOKEN env var set to changeme-in-production as an explicit reminder.
  - postgres_data named volume survives docker compose down (requires -v to wipe).
