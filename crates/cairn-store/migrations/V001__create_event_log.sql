-- Append-only event log for full_history entities (RFC 002).
-- Every accepted runtime event is durably recorded here.
-- Synchronous projections are updated within the same transaction as inserts.

CREATE TABLE event_log (
    position    BIGSERIAL PRIMARY KEY,
    event_id    TEXT NOT NULL UNIQUE,
    source_type TEXT NOT NULL,
    source_meta JSONB NOT NULL DEFAULT '{}',
    ownership   JSONB NOT NULL,
    causation_id TEXT,
    correlation_id TEXT,
    payload     JSONB NOT NULL,
    stored_at   BIGINT NOT NULL
);

CREATE INDEX idx_event_log_event_id ON event_log (event_id);
CREATE INDEX idx_event_log_correlation ON event_log (correlation_id) WHERE correlation_id IS NOT NULL;
CREATE INDEX idx_event_log_stored_at ON event_log (stored_at);
