-- PR BP-2: projection for tool-call approval events (ToolCallProposed,
-- ToolCallAmended, ToolCallApproved, ToolCallRejected).
--
-- One row per proposed tool call. Rows are inserted by
-- ToolCallProposed and mutated in place by the other three events.
-- The `state` column follows a simple state machine:
--     pending -> approved
--     pending -> rejected
--     pending -> timeout (reserved for PR BP-3)
-- Amendments do NOT transition state; they only update the
-- amended_tool_args / last_amended_at_ms fields.
--
-- `match_policy` and `scope` are stored as JSONB so the discriminated
-- union shape (`{"kind": "exact"}`, `{"kind": "session", ...}`, etc.)
-- round-trips through the projection without bespoke encoding.

CREATE TABLE IF NOT EXISTS tool_call_approvals (
    call_id              TEXT PRIMARY KEY,
    session_id           TEXT NOT NULL,
    run_id               TEXT NOT NULL,
    tenant_id            TEXT NOT NULL,
    workspace_id         TEXT NOT NULL,
    project_id           TEXT NOT NULL,
    tool_name            TEXT NOT NULL,
    original_tool_args   JSONB NOT NULL,
    amended_tool_args    JSONB,
    approved_tool_args   JSONB,
    display_summary      TEXT,
    match_policy         JSONB NOT NULL,
    state                TEXT NOT NULL DEFAULT 'pending',
    operator_id          TEXT,
    scope                JSONB,
    reason               TEXT,
    proposed_at_ms       BIGINT NOT NULL,
    approved_at_ms       BIGINT,
    rejected_at_ms       BIGINT,
    last_amended_at_ms   BIGINT,
    version              BIGINT NOT NULL DEFAULT 1,
    created_at           BIGINT NOT NULL,
    updated_at           BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tool_call_approvals_project_state
    ON tool_call_approvals (tenant_id, workspace_id, project_id, state);
CREATE INDEX IF NOT EXISTS idx_tool_call_approvals_session
    ON tool_call_approvals (session_id);
CREATE INDEX IF NOT EXISTS idx_tool_call_approvals_run
    ON tool_call_approvals (run_id);
CREATE INDEX IF NOT EXISTS idx_tool_call_approvals_pending
    ON tool_call_approvals (tenant_id, workspace_id, project_id, proposed_at_ms)
    WHERE state = 'pending';
