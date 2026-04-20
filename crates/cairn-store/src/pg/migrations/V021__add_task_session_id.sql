-- RFC-011 Phase 3: persist the task → session binding directly on the
-- tasks projection. Phase 2 derived it by walking parent_run_id → run
-- at resolve time; Phase 3 stores it at TaskCreated time so the hot
-- path never needs a second read-model lookup.
--
-- Nullable: solo (session-less) tasks carry None, and legacy rows
-- created before V018 are migrated with NULL. Callers must still fall
-- back to walking parent_run_id → RunRecord.session_id when this
-- column is NULL on a legacy row.

ALTER TABLE tasks ADD COLUMN IF NOT EXISTS session_id TEXT;

-- Index used by the hot session-scope lookup path.
CREATE INDEX IF NOT EXISTS idx_tasks_session_id ON tasks (session_id)
    WHERE session_id IS NOT NULL;
