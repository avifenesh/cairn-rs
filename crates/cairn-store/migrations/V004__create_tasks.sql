-- Task current-state table (synchronous projection).
-- Tasks are the only leased execution entity in v1 (RFC 005).

CREATE TABLE tasks (
    task_id         TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    project_id      TEXT NOT NULL,
    parent_run_id   TEXT REFERENCES runs(run_id),
    parent_task_id  TEXT REFERENCES tasks(task_id),
    state           TEXT NOT NULL DEFAULT 'queued',
    failure_class   TEXT,
    lease_owner     TEXT,
    lease_expires_at BIGINT,
    lease_version   BIGINT NOT NULL DEFAULT 0,
    version         BIGINT NOT NULL DEFAULT 1,
    created_at      BIGINT NOT NULL,
    updated_at      BIGINT NOT NULL
);

CREATE INDEX idx_tasks_project ON tasks (tenant_id, workspace_id, project_id);
CREATE INDEX idx_tasks_state ON tasks (state);
CREATE INDEX idx_tasks_lease_expiry ON tasks (lease_expires_at) WHERE lease_expires_at IS NOT NULL;
CREATE INDEX idx_tasks_parent_run ON tasks (parent_run_id) WHERE parent_run_id IS NOT NULL;
