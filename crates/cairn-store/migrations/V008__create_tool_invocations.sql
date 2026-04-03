-- Tool invocation current-state table (synchronous projection).
-- Every tool call emits structured runtime facts (RFC 002).
-- Aligned with cairn_domain::tool_invocation::ToolInvocationRecord.

CREATE TABLE tool_invocations (
    invocation_id   TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    project_id      TEXT NOT NULL,
    session_id      TEXT REFERENCES sessions(session_id),
    run_id          TEXT REFERENCES runs(run_id),
    task_id         TEXT REFERENCES tasks(task_id),
    target          JSONB NOT NULL,
    execution_class TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'requested',
    outcome         TEXT,
    error_message   TEXT,
    version         BIGINT NOT NULL DEFAULT 1,
    requested_at_ms BIGINT NOT NULL,
    started_at_ms   BIGINT,
    finished_at_ms  BIGINT,
    created_at      BIGINT NOT NULL,
    updated_at      BIGINT NOT NULL
);

CREATE INDEX idx_tool_invocations_run ON tool_invocations (run_id) WHERE run_id IS NOT NULL;
CREATE INDEX idx_tool_invocations_state ON tool_invocations (state);
