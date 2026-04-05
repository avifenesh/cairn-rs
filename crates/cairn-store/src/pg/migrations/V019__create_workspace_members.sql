-- Workspace membership: current-state table for RFC 008 RBAC enforcement.
-- Maps to the WorkspaceMemberRecord projection and
-- WorkspaceMemberAdded / WorkspaceMemberRemoved events.
-- Each row represents one operator's active membership in a workspace.

CREATE TABLE IF NOT EXISTS workspace_members (
    workspace_id TEXT    NOT NULL,
    operator_id  TEXT    NOT NULL,
    -- WorkspaceRole enum persisted as snake_case text: owner|admin|member|viewer
    role         TEXT    NOT NULL,
    -- Unix ms timestamp of when the membership was established.
    added_at_ms  BIGINT  NOT NULL,

    PRIMARY KEY (workspace_id, operator_id)
);

-- RFC 008 role-gate hot-path: look up a specific operator's role in a workspace.
CREATE INDEX IF NOT EXISTS idx_workspace_members_lookup
    ON workspace_members (workspace_id, operator_id);

-- Operator fleet view: list all workspaces an operator belongs to.
CREATE INDEX IF NOT EXISTS idx_workspace_members_by_operator
    ON workspace_members (operator_id, workspace_id);
