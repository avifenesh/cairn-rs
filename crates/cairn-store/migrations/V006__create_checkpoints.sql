-- Checkpoint current-state table (synchronous projection).
-- Checkpoints are immutable recovery records (RFC 005).

CREATE TABLE checkpoints (
    checkpoint_id TEXT PRIMARY KEY,
    tenant_id     TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    project_id    TEXT NOT NULL,
    run_id        TEXT NOT NULL REFERENCES runs(run_id),
    disposition   TEXT NOT NULL DEFAULT 'latest',
    version       BIGINT NOT NULL DEFAULT 1,
    created_at    BIGINT NOT NULL
);

CREATE INDEX idx_checkpoints_run ON checkpoints (run_id);
CREATE INDEX idx_checkpoints_latest ON checkpoints (run_id, disposition) WHERE disposition = 'latest';
