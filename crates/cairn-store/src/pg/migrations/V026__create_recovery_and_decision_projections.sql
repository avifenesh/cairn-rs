-- F39: durable projections for RFC 002 / RFC 020 recovery events and
-- RFC 019 / RFC 020 decision-cache events.
--
-- Prior to this migration the PgSyncProjection emitted a `log_stub`
-- warning for each of these event_variants: the envelope landed in
-- `event_log` (source of truth preserved) but no projection table was
-- written, so any query that tried to read recovery attempts or
-- recorded decisions returned empty. Five variants are covered:
--
--   RecoveryAttempted       → recovery_attempts
--   RecoveryCompleted       → recovery_completions
--   RecoverySummaryEmitted  → recovery_summaries (one row per boot)
--   DecisionRecorded        → decision_records
--   DecisionCacheWarmup     → decision_cache_warmups (one row per boot replay)
--
-- Each table keys on an event-intrinsic identifier (envelope event_id
-- for the append-style audits, `decision_id` for the RFC 019 entry,
-- `boot_id` for once-per-boot summaries, `warmed_at` for the warmup)
-- and uses `ON CONFLICT … DO NOTHING` so replay is idempotent.

CREATE TABLE IF NOT EXISTS recovery_attempts (
    event_id        TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    project_id      TEXT NOT NULL,
    run_id          TEXT,
    task_id         TEXT,
    reason          TEXT NOT NULL,
    boot_id         TEXT,
    recorded_at_ms  BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_recovery_attempts_project
    ON recovery_attempts (tenant_id, workspace_id, project_id, recorded_at_ms);
CREATE INDEX IF NOT EXISTS idx_recovery_attempts_boot
    ON recovery_attempts (boot_id)
    WHERE boot_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_recovery_attempts_run
    ON recovery_attempts (run_id)
    WHERE run_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_recovery_attempts_task
    ON recovery_attempts (task_id)
    WHERE task_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS recovery_completions (
    event_id        TEXT PRIMARY KEY,
    tenant_id       TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    project_id      TEXT NOT NULL,
    run_id          TEXT,
    task_id         TEXT,
    recovered       BOOLEAN NOT NULL,
    boot_id         TEXT,
    recorded_at_ms  BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_recovery_completions_project
    ON recovery_completions (tenant_id, workspace_id, project_id, recorded_at_ms);
CREATE INDEX IF NOT EXISTS idx_recovery_completions_boot
    ON recovery_completions (boot_id)
    WHERE boot_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_recovery_completions_run
    ON recovery_completions (run_id)
    WHERE run_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_recovery_completions_task
    ON recovery_completions (task_id)
    WHERE task_id IS NOT NULL;

-- One row per cairn-app boot. `boot_id` is the natural primary key —
-- the emitter contract in cairn-runtime guarantees at most one
-- `RecoverySummaryEmitted` per boot, so ON CONFLICT DO NOTHING is
-- sufficient for replay idempotency.
CREATE TABLE IF NOT EXISTS recovery_summaries (
    boot_id                     TEXT PRIMARY KEY,
    tenant_id                   TEXT NOT NULL,
    workspace_id                TEXT NOT NULL,
    project_id                  TEXT NOT NULL,
    recovered_runs              BIGINT NOT NULL,
    recovered_tasks             BIGINT NOT NULL,
    recovered_sandboxes         BIGINT NOT NULL,
    preserved_sandboxes         BIGINT NOT NULL,
    orphaned_sandboxes_cleaned  BIGINT NOT NULL,
    decision_cache_entries      BIGINT NOT NULL,
    stale_pending_cleared       BIGINT NOT NULL,
    tool_result_cache_entries   BIGINT NOT NULL,
    memory_projection_entries   BIGINT NOT NULL,
    graph_nodes_recovered       BIGINT NOT NULL,
    graph_edges_recovered       BIGINT NOT NULL,
    webhook_dedup_entries       BIGINT NOT NULL,
    trigger_projections         BIGINT NOT NULL,
    startup_ms                  BIGINT NOT NULL,
    summary_at_ms               BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_recovery_summaries_tenant
    ON recovery_summaries (tenant_id, summary_at_ms);

-- RFC 019 decision cache projection. `event_json` preserves the full
-- `DecisionEvent::DecisionRecorded` payload so operator tooling can
-- reconstruct the reasoning chain after restart without re-reading the
-- event log. `decision_key_json` is the `DecisionKey` struct serialized
-- as JSON text (TEXT for pg/sqlite portability — no JSONB operators).
CREATE TABLE IF NOT EXISTS decision_records (
    decision_id        TEXT PRIMARY KEY,
    tenant_id          TEXT NOT NULL,
    workspace_id       TEXT NOT NULL,
    project_id         TEXT NOT NULL,
    decision_key_json  TEXT NOT NULL,
    outcome_kind       TEXT NOT NULL,
    cached             BOOLEAN NOT NULL,
    expires_at         BIGINT NOT NULL,
    decided_at         BIGINT NOT NULL,
    event_json         TEXT NOT NULL,
    recorded_at_ms     BIGINT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_decision_records_project
    ON decision_records (tenant_id, workspace_id, project_id, decided_at);
CREATE INDEX IF NOT EXISTS idx_decision_records_cached
    ON decision_records (cached, expires_at)
    WHERE cached = TRUE;

-- One row per boot replay. `warmed_at` is a unix-ms timestamp unique
-- per boot; on the remote chance two boots land on the exact same ms,
-- ON CONFLICT DO NOTHING keeps the earlier row (the event log is still
-- the durable source of truth for the warmup fact).
CREATE TABLE IF NOT EXISTS decision_cache_warmups (
    warmed_at            BIGINT PRIMARY KEY,
    cached               BIGINT NOT NULL,
    expired_and_dropped  BIGINT NOT NULL
);
