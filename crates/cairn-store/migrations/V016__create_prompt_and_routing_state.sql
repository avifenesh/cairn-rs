-- Prompt registry + routing/provider current-state tables.
-- These are synchronous projections from runtime events so durable
-- backends can materialize the same records the in-memory store keeps.

CREATE TABLE IF NOT EXISTS prompt_assets (
    prompt_asset_id TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    project_id      TEXT NOT NULL,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL,
    scope           TEXT,
    status          TEXT,
    created_at      BIGINT NOT NULL,
    updated_at      BIGINT
);

CREATE INDEX IF NOT EXISTS idx_prompt_assets_project
    ON prompt_assets (tenant_id, workspace_id, project_id);

CREATE TABLE IF NOT EXISTS prompt_versions (
    prompt_version_id TEXT PRIMARY KEY,
    prompt_asset_id   TEXT NOT NULL REFERENCES prompt_assets(prompt_asset_id),
    tenant_id         TEXT NOT NULL,
    workspace_id      TEXT NOT NULL,
    project_id        TEXT NOT NULL,
    version_number    INTEGER,
    content_hash      TEXT NOT NULL,
    content           TEXT,
    format            TEXT,
    created_by        TEXT,
    created_at        BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_prompt_versions_asset
    ON prompt_versions (prompt_asset_id, created_at, prompt_version_id);

CREATE TABLE IF NOT EXISTS prompt_releases (
    prompt_release_id TEXT PRIMARY KEY,
    prompt_asset_id   TEXT NOT NULL REFERENCES prompt_assets(prompt_asset_id),
    prompt_version_id TEXT NOT NULL REFERENCES prompt_versions(prompt_version_id),
    tenant_id         TEXT NOT NULL,
    workspace_id      TEXT NOT NULL,
    project_id        TEXT NOT NULL,
    release_tag       TEXT,
    state             TEXT NOT NULL DEFAULT 'draft',
    rollout_target    TEXT,
    created_at        BIGINT NOT NULL,
    updated_at        BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_prompt_releases_project
    ON prompt_releases (tenant_id, workspace_id, project_id, created_at, prompt_release_id);

CREATE INDEX IF NOT EXISTS idx_prompt_releases_selector
    ON prompt_releases (tenant_id, workspace_id, project_id, prompt_asset_id, state, rollout_target);

CREATE TABLE IF NOT EXISTS route_decisions (
    route_decision_id             TEXT PRIMARY KEY,
    tenant_id                     TEXT NOT NULL,
    workspace_id                  TEXT NOT NULL,
    project_id                    TEXT NOT NULL,
    operation_kind                TEXT NOT NULL,
    route_policy_id               TEXT,
    terminal_route_attempt_id     TEXT,
    selected_provider_binding_id  TEXT,
    selected_route_attempt_id     TEXT,
    selector_context              JSONB,
    attempt_count                 INTEGER NOT NULL DEFAULT 0,
    fallback_used                 BOOLEAN NOT NULL DEFAULT FALSE,
    final_status                  TEXT NOT NULL,
    created_at                    BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_route_decisions_project
    ON route_decisions (tenant_id, workspace_id, project_id, created_at, route_decision_id);

CREATE TABLE IF NOT EXISTS provider_calls (
    provider_call_id        TEXT PRIMARY KEY,
    route_decision_id       TEXT NOT NULL REFERENCES route_decisions(route_decision_id),
    route_attempt_id        TEXT NOT NULL,
    tenant_id               TEXT NOT NULL,
    workspace_id            TEXT NOT NULL,
    project_id              TEXT NOT NULL,
    operation_kind          TEXT NOT NULL,
    provider_binding_id     TEXT NOT NULL,
    provider_connection_id  TEXT NOT NULL,
    provider_adapter        TEXT NOT NULL DEFAULT '',
    provider_model_id       TEXT NOT NULL,
    task_id                 TEXT,
    run_id                  TEXT,
    prompt_release_id       TEXT,
    fallback_position       INTEGER NOT NULL DEFAULT 0,
    status                  TEXT NOT NULL,
    latency_ms              BIGINT,
    input_tokens            INTEGER,
    output_tokens           INTEGER,
    cost_micros             BIGINT,
    error_class             TEXT,
    raw_error_message       TEXT,
    retry_count             INTEGER NOT NULL DEFAULT 0,
    created_at              BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_provider_calls_decision
    ON provider_calls (route_decision_id, created_at, provider_call_id);
