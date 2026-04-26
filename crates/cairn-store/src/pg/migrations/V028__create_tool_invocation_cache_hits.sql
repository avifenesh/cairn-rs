-- F52: durable projection for `ToolInvocationCacheHit` events.
--
-- The Phase 2 dogfood run surfaced a WARN "pg projection stub: event
-- committed to event_log but no projection table updated" for the
-- `ToolInvocationCacheHit` variant. F39 was supposed to close this
-- projection-coverage class, but the CacheHit variant was missed
-- because Track 3 originally classified it as audit-only.
--
-- Operators now need to see cache-hit activity on the REST surface
-- (not just via event-log replay). This table records one row per hit
-- so the read side can report counts, recent hits per run, and
-- served_at_ms latencies without scanning `event_log`.
--
-- Portable surface only: no JSONB, no arrays, no partial indexes
-- outside Postgres+SQLite (both supported here). See the
-- `no-DB-specific-features` project memory for the full rubric.

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
