-- Run current-state table (synchronous projection).

CREATE TABLE runs (
    run_id          TEXT PRIMARY KEY,
    session_id      TEXT NOT NULL REFERENCES sessions(session_id),
    parent_run_id   TEXT REFERENCES runs(run_id),
    tenant_id       TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    project_id      TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'pending',
    failure_class   TEXT,
    version         BIGINT NOT NULL DEFAULT 1,
    created_at      BIGINT NOT NULL,
    updated_at      BIGINT NOT NULL
);

CREATE INDEX idx_runs_session ON runs (session_id);
CREATE INDEX idx_runs_project ON runs (tenant_id, workspace_id, project_id);
CREATE INDEX idx_runs_state ON runs (state);
CREATE INDEX idx_runs_parent ON runs (parent_run_id) WHERE parent_run_id IS NOT NULL;
