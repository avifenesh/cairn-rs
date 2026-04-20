-- Cursor table for the FF lease_history subscriber.
--
-- The subscriber tails FF's `{p:N}:<eid>:lease_history` streams so cairn
-- learns about FF-initiated state changes (lease expiry, reclaim) that
-- never flow through a cairn service call. One row per execution; the
-- cursor is the last XREAD stream id we've successfully consumed and
-- emitted a BridgeEvent for. On restart, we resume from here.
--
-- partition_id + execution_id together uniquely identify a stream.
-- They're stored as TEXT because FF's hash-tag encoding (`{p:42}` etc.)
-- is opaque to us and the subscriber just echoes the values back.

CREATE TABLE IF NOT EXISTS ff_lease_history_cursors (
    partition_id    TEXT NOT NULL,
    execution_id    TEXT NOT NULL,
    last_stream_id  TEXT NOT NULL,
    updated_at_ms   BIGINT NOT NULL,
    PRIMARY KEY (partition_id, execution_id)
);

CREATE INDEX IF NOT EXISTS idx_ff_lease_history_cursors_partition
    ON ff_lease_history_cursors (partition_id);
