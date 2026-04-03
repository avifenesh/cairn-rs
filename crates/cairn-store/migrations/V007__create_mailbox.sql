-- Mailbox message current-state table (synchronous projection).
-- Mailbox durability belongs to the Rust runtime store (RFC 002).

CREATE TABLE mailbox_messages (
    message_id  TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id  TEXT NOT NULL,
    run_id      TEXT REFERENCES runs(run_id),
    task_id     TEXT REFERENCES tasks(task_id),
    version     BIGINT NOT NULL DEFAULT 1,
    created_at  BIGINT NOT NULL
);

CREATE INDEX idx_mailbox_project ON mailbox_messages (tenant_id, workspace_id, project_id);
CREATE INDEX idx_mailbox_run ON mailbox_messages (run_id) WHERE run_id IS NOT NULL;
CREATE INDEX idx_mailbox_task ON mailbox_messages (task_id) WHERE task_id IS NOT NULL;
