/// SQLite schema DDL for local-mode.
///
/// Single string applied in one transaction. Mirrors the Postgres
/// migrations but uses SQLite-compatible types.
pub const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS _cairn_migrations (
    version     INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    applied_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS event_log (
    position       INTEGER PRIMARY KEY AUTOINCREMENT,
    event_id       TEXT NOT NULL UNIQUE,
    source_type    TEXT NOT NULL,
    source_meta    TEXT NOT NULL DEFAULT '{}',
    ownership      TEXT NOT NULL,
    causation_id   TEXT,
    correlation_id TEXT,
    payload        TEXT NOT NULL,
    stored_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
    session_id   TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id   TEXT NOT NULL,
    state        TEXT NOT NULL DEFAULT 'open',
    version      INTEGER NOT NULL DEFAULT 1,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS runs (
    run_id        TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL REFERENCES sessions(session_id),
    parent_run_id TEXT REFERENCES runs(run_id),
    tenant_id     TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    project_id    TEXT NOT NULL,
    state         TEXT NOT NULL DEFAULT 'pending',
    failure_class TEXT,
    version       INTEGER NOT NULL DEFAULT 1,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tasks (
    task_id        TEXT PRIMARY KEY,
    tenant_id      TEXT NOT NULL,
    workspace_id   TEXT NOT NULL,
    project_id     TEXT NOT NULL,
    parent_run_id  TEXT REFERENCES runs(run_id),
    parent_task_id TEXT REFERENCES tasks(task_id),
    -- Session binding populated from TaskCreated.session_id at insert time.
    session_id     TEXT REFERENCES sessions(session_id),
    state          TEXT NOT NULL DEFAULT 'queued',
    failure_class  TEXT,
    lease_owner    TEXT,
    lease_expires_at INTEGER,
    lease_version  INTEGER NOT NULL DEFAULT 0,
    title          TEXT,
    description    TEXT,
    version        INTEGER NOT NULL DEFAULT 1,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_tasks_session_id ON tasks(session_id)
    WHERE session_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS approvals (
    approval_id  TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id   TEXT NOT NULL,
    run_id       TEXT REFERENCES runs(run_id),
    task_id      TEXT REFERENCES tasks(task_id),
    requirement  TEXT NOT NULL DEFAULT 'required',
    decision     TEXT,
    title        TEXT,
    description  TEXT,
    version      INTEGER NOT NULL DEFAULT 1,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    tenant_id     TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    project_id    TEXT NOT NULL,
    run_id        TEXT NOT NULL REFERENCES runs(run_id),
    disposition   TEXT NOT NULL DEFAULT 'latest',
    version       INTEGER NOT NULL DEFAULT 1,
    created_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mailbox_messages (
    message_id   TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id   TEXT NOT NULL,
    run_id       TEXT REFERENCES runs(run_id),
    task_id      TEXT REFERENCES tasks(task_id),
    version      INTEGER NOT NULL DEFAULT 1,
    created_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS tool_invocations (
    invocation_id   TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    project_id      TEXT NOT NULL,
    session_id      TEXT REFERENCES sessions(session_id),
    run_id          TEXT REFERENCES runs(run_id),
    task_id         TEXT REFERENCES tasks(task_id),
    target          TEXT NOT NULL,
    execution_class TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'requested',
    outcome         TEXT,
    error_message   TEXT,
    version         INTEGER NOT NULL DEFAULT 1,
    requested_at_ms INTEGER NOT NULL,
    started_at_ms   INTEGER,
    finished_at_ms  INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS documents (
    document_id   TEXT PRIMARY KEY,
    source_id     TEXT NOT NULL,
    tenant_id     TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    project_id    TEXT NOT NULL,
    source_type   TEXT NOT NULL,
    title         TEXT,
    ingest_status TEXT NOT NULL DEFAULT 'pending',
    version       INTEGER NOT NULL DEFAULT 1,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS chunks (
    chunk_id     TEXT PRIMARY KEY,
    document_id  TEXT NOT NULL REFERENCES documents(document_id),
    source_id    TEXT NOT NULL,
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id   TEXT NOT NULL,
    source_type  TEXT NOT NULL,
    text         TEXT NOT NULL,
    position     INTEGER NOT NULL,
    embedding    BLOB,
    created_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS graph_nodes (
    node_id      TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    tenant_id    TEXT,
    workspace_id TEXT,
    project_id   TEXT,
    metadata     TEXT NOT NULL DEFAULT '{}',
    created_at   INTEGER NOT NULL
);

-- FTS5 virtual table for lexical retrieval in local-mode (RFC 003).
-- FTS sync is handled in application code (SqliteDocumentStore.insert_chunks)
-- rather than triggers, because semicolons in trigger bodies break the
-- simple statement-split migration runner.
CREATE VIRTUAL TABLE IF NOT EXISTS chunks_fts USING fts5(
    chunk_id,
    text
);

CREATE TABLE IF NOT EXISTS graph_edges (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    source_node_id  TEXT NOT NULL REFERENCES graph_nodes(node_id),
    target_node_id  TEXT NOT NULL REFERENCES graph_nodes(node_id),
    kind            TEXT NOT NULL,
    metadata        TEXT NOT NULL DEFAULT '{}',
    created_at      INTEGER NOT NULL,
    UNIQUE(source_node_id, target_node_id, kind)
);

CREATE TABLE IF NOT EXISTS ff_lease_history_cursors (
    partition_id    TEXT NOT NULL,
    execution_id    TEXT NOT NULL,
    last_stream_id  TEXT NOT NULL,
    updated_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (partition_id, execution_id)
);
CREATE INDEX IF NOT EXISTS idx_ff_lease_history_cursors_partition
    ON ff_lease_history_cursors(partition_id);

-- ── Organization hierarchy (mirrors V017 Postgres migration) ─────────────
-- RFC 008 requires durable tenant/workspace/project reads in team-mode.

CREATE TABLE IF NOT EXISTS tenants (
    tenant_id   TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS workspaces (
    workspace_id TEXT PRIMARY KEY,
    tenant_id    TEXT NOT NULL REFERENCES tenants(tenant_id),
    name         TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_workspaces_tenant
    ON workspaces (tenant_id, created_at, workspace_id);

CREATE TABLE IF NOT EXISTS projects (
    project_id    TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES workspaces(workspace_id),
    tenant_id     TEXT NOT NULL REFERENCES tenants(tenant_id),
    name          TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_projects_workspace
    ON projects (tenant_id, workspace_id, created_at, project_id);

-- ── Workspace membership (mirrors V019 Postgres migration) ──────────────
-- RFC 008 RBAC enforcement.

CREATE TABLE IF NOT EXISTS workspace_members (
    workspace_id TEXT    NOT NULL,
    operator_id  TEXT    NOT NULL,
    role         TEXT    NOT NULL,
    added_at_ms  INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, operator_id)
);
CREATE INDEX IF NOT EXISTS idx_workspace_members_lookup
    ON workspace_members (workspace_id, operator_id);
CREATE INDEX IF NOT EXISTS idx_workspace_members_by_operator
    ON workspace_members (operator_id, workspace_id);

-- ── Prompt registry (mirrors V016 Postgres migration, prompt_* tables) ──

CREATE TABLE IF NOT EXISTS prompt_assets (
    prompt_asset_id TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    project_id      TEXT NOT NULL,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL,
    scope           TEXT,
    status          TEXT,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER
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
    created_at        INTEGER NOT NULL
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
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_prompt_releases_project
    ON prompt_releases (tenant_id, workspace_id, project_id, created_at, prompt_release_id);
CREATE INDEX IF NOT EXISTS idx_prompt_releases_selector
    ON prompt_releases (tenant_id, workspace_id, project_id, prompt_asset_id, state, rollout_target);

-- ── Routing/provider state (mirrors V016 route_decisions + provider_calls) ──
-- selector_context uses TEXT (JSON string) instead of Postgres JSONB —
-- the column is only written and read wholesale, never queried with
-- JSONB operators, so this is a portable substitution.

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
    selector_context              TEXT,
    attempt_count                 INTEGER NOT NULL DEFAULT 0,
    fallback_used                 INTEGER NOT NULL DEFAULT 0,
    final_status                  TEXT NOT NULL,
    created_at                    INTEGER NOT NULL
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
    latency_ms              INTEGER,
    input_tokens            INTEGER,
    output_tokens           INTEGER,
    cost_micros             INTEGER,
    error_class             TEXT,
    raw_error_message       TEXT,
    retry_count             INTEGER NOT NULL DEFAULT 0,
    created_at              INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_provider_calls_decision
    ON provider_calls (route_decision_id, created_at, provider_call_id);
"#;
