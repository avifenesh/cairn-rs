-- Durable projection for `ToolInvocationCacheHit` events. One row per
-- cache-hit keyed by `invocation_id`. Operators query cache activity
-- (counts per run, recent hits, served_at_ms latencies) through this
-- read-model instead of scanning `event_log`.
--
-- Portable surface only: no JSONB, no arrays, no Postgres-specific
-- operators. Mirrors the SQLite schema in
-- `crates/cairn-store/src/sqlite/schema.rs`.

CREATE TABLE IF NOT EXISTS tool_invocation_cache_hits (
    invocation_id            TEXT   PRIMARY KEY,
    tenant_id                TEXT   NOT NULL,
    workspace_id             TEXT   NOT NULL,
    project_id               TEXT   NOT NULL,
    run_id                   TEXT,
    task_id                  TEXT,
    tool_name                TEXT   NOT NULL,
    tool_call_id             TEXT   NOT NULL,
    original_completed_at_ms BIGINT NOT NULL,
    served_at_ms             BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_tool_invocation_cache_hits_run
    ON tool_invocation_cache_hits (run_id) WHERE run_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_tool_invocation_cache_hits_tool_call
    ON tool_invocation_cache_hits (tool_call_id);
CREATE INDEX IF NOT EXISTS idx_tool_invocation_cache_hits_served_at
    ON tool_invocation_cache_hits (served_at_ms);
