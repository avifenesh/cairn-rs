-- F55: persist tool args and output preview on the tool_invocations
-- projection so operators can see "what cairn ran and what it got back"
-- via GET /v1/tool-invocations and the RunDetailPage telemetry panel.
--
-- args_json: structured tool arguments captured at dispatch time.
--            JSONB because the rest of this table already uses JSONB for
--            `target` — Postgres is the v1 target and SQLite mirrors this
--            column with TEXT-over-JSON per the portability policy.
-- output_preview: UTF-8 safe truncation (~8 KiB) of the tool's captured
--                 output. TEXT because we never run server-side JSON
--                 operators on it — it is a human-readable preview.
--
-- Both columns are nullable: pre-F55 rows get no backfill (we do not
-- replay the event log at migration time) and future "started" events
-- that have no captured output yet leave `output_preview` NULL until
-- the completion event lands.

ALTER TABLE tool_invocations
    ADD COLUMN IF NOT EXISTS args_json      JSONB,
    ADD COLUMN IF NOT EXISTS output_preview TEXT;
