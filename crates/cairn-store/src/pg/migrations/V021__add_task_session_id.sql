-- Adds the task → session binding column to the tasks projection.
--
-- Nullable: solo (session-less) tasks carry NULL. Callers must fall
-- back to walking parent_run_id → RunRecord.session_id when NULL.

ALTER TABLE tasks ADD COLUMN IF NOT EXISTS session_id TEXT;

-- Index used by the hot session-scope lookup path.
CREATE INDEX IF NOT EXISTS idx_tasks_session_id ON tasks (session_id)
    WHERE session_id IS NOT NULL;
