-- Migration history tracking table.
-- Used by the MigrationRunner to record which migrations have been applied.

CREATE TABLE IF NOT EXISTS _cairn_migrations (
    version     INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    applied_at  BIGINT NOT NULL
);
