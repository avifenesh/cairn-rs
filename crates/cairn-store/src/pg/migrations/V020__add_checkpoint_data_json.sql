-- RFC 005: preserve checkpoint recovery payload.
-- The data_json column stores the serialized serde_json::Value from
-- CheckpointRecorded.data so that durable recovery can reconstruct
-- the full agent state without replaying the entire event log.

ALTER TABLE checkpoints ADD COLUMN IF NOT EXISTS data_json TEXT;
