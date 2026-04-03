-- Session current-state table (synchronous projection).
-- Updated transactionally with event_log inserts.

CREATE TABLE sessions (
    session_id  TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id  TEXT NOT NULL,
    state       TEXT NOT NULL DEFAULT 'open',
    version     BIGINT NOT NULL DEFAULT 1,
    created_at  BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL
);

CREATE INDEX idx_sessions_project ON sessions (tenant_id, workspace_id, project_id);
CREATE INDEX idx_sessions_state ON sessions (state);
