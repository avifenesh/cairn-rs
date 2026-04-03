-- Approval request current-state table (synchronous projection).

CREATE TABLE approvals (
    approval_id TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id  TEXT NOT NULL,
    run_id      TEXT REFERENCES runs(run_id),
    task_id     TEXT REFERENCES tasks(task_id),
    requirement TEXT NOT NULL DEFAULT 'required',
    decision    TEXT,
    version     BIGINT NOT NULL DEFAULT 1,
    created_at  BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL
);

CREATE INDEX idx_approvals_project ON approvals (tenant_id, workspace_id, project_id);
CREATE INDEX idx_approvals_pending ON approvals (decision) WHERE decision IS NULL;
