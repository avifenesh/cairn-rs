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
"#;
