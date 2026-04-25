-- F29 CD-2: durable projections for session / project / workspace cost
-- rollups. Each row is upserted by the `SessionCostUpdated` projection on
-- Postgres (see `pg/projections.rs`). Project and workspace totals are
-- folded in the same transaction as the per-session row so the three
-- tables stay mutually consistent — `project_costs.total_cost_micros`
-- equals the sum of `session_costs.total_cost_micros` across rows in the
-- same (tenant_id, workspace_id, project_id).
--
-- Values are lifetime totals in v1. A daily-bucket table for time-range
-- slicing is tracked as follow-up; callers that need a window filter
-- today fall back to the per-session list in `session_costs`.

CREATE TABLE IF NOT EXISTS session_costs (
    session_id         TEXT PRIMARY KEY,
    tenant_id          TEXT NOT NULL,
    workspace_id       TEXT NOT NULL,
    project_id         TEXT NOT NULL,
    total_cost_micros  BIGINT NOT NULL DEFAULT 0,
    total_tokens_in    BIGINT NOT NULL DEFAULT 0,
    total_tokens_out   BIGINT NOT NULL DEFAULT 0,
    provider_calls     BIGINT NOT NULL DEFAULT 0,
    updated_at_ms      BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_costs_project
    ON session_costs (tenant_id, workspace_id, project_id);
CREATE INDEX IF NOT EXISTS idx_session_costs_tenant
    ON session_costs (tenant_id, updated_at_ms);

CREATE TABLE IF NOT EXISTS project_costs (
    tenant_id          TEXT NOT NULL,
    workspace_id       TEXT NOT NULL,
    project_id         TEXT NOT NULL,
    total_cost_micros  BIGINT NOT NULL DEFAULT 0,
    total_tokens_in    BIGINT NOT NULL DEFAULT 0,
    total_tokens_out   BIGINT NOT NULL DEFAULT 0,
    provider_calls     BIGINT NOT NULL DEFAULT 0,
    updated_at_ms      BIGINT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id, project_id)
);
CREATE INDEX IF NOT EXISTS idx_project_costs_workspace
    ON project_costs (tenant_id, workspace_id);

CREATE TABLE IF NOT EXISTS workspace_costs (
    tenant_id          TEXT NOT NULL,
    workspace_id       TEXT NOT NULL,
    total_cost_micros  BIGINT NOT NULL DEFAULT 0,
    total_tokens_in    BIGINT NOT NULL DEFAULT 0,
    total_tokens_out   BIGINT NOT NULL DEFAULT 0,
    provider_calls     BIGINT NOT NULL DEFAULT 0,
    updated_at_ms      BIGINT NOT NULL,
    PRIMARY KEY (tenant_id, workspace_id)
);
