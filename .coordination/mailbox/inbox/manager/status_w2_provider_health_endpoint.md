# STATUS: GET /v1/providers/health

**Task:** Wire provider health endpoint  
**Tests passed:** 4 new (95 total, 0 regressions)

## Changes

### Import: `ProviderHealthReadModel` added to projections imports

### Handler: `provider_health_handler`
- `GET /v1/providers/health` (no pagination — snapshot of all connections)
- Derives tenant IDs from `list_all_provider_bindings`
- Queries `ProviderHealthReadModel::list_by_tenant` per tenant
- Returns `Vec<ProviderHealthEntry>`: connection_id, status, healthy, last_checked_at, consecutive_failures, error_message

### Route wired
`.route("/v1/providers/health", get(provider_health_handler))`
after `/v1/providers`

### Test module: `provider_health_tests`
- `provider_health_empty_with_no_providers` — empty list when no providers
- `provider_health_shows_healthy_after_health_check` — after ProviderHealthChecked: healthy=true, last_checked_at set
- `provider_health_shows_degraded_after_mark_degraded` — after ProviderMarkedDegraded: healthy=false, error_message set
- `provider_health_requires_auth` — 401 without token
