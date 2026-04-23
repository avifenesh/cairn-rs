-- Issue #218: soft-delete for workspaces.
--
-- Adds `archived_at` (unix-ms) column to `workspaces`. A NULL value means the
-- workspace is active; a populated value means it has been archived via
-- `DELETE /v1/admin/tenants/:t/workspaces/:w` and should be filtered out of
-- default list responses.

ALTER TABLE workspaces
    ADD COLUMN IF NOT EXISTS archived_at BIGINT;

CREATE INDEX IF NOT EXISTS idx_workspaces_tenant_archived
    ON workspaces (tenant_id, archived_at, created_at, workspace_id);
