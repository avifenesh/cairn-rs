-- F47 PR2: event-sourced persistence of run completion annotation
-- (summary + CompletionVerification sidecar).
--
-- PR1 (cc8f90d5) made the CompletionVerification sidecar visible on the
-- SSE `orchestrate_finished` frame so live operators could cross-check
-- the LLM's free-text summary against warning/error lines extracted
-- from tool_result frames. After a refresh that block disappeared
-- because nothing persisted it. PR2 closes that gap: a new
-- `RunCompletionAnnotated` domain event lands on the event log after a
-- run terminates via the normal completion path; this projection stores
-- it on the `runs` row so the same evidence is available on any run at
-- any time via `GET /v1/runs/:id`.
--
-- All three columns are nullable. Event logs written before F47 PR2
-- simply have no `RunCompletionAnnotated` entries, so the projection
-- applier never writes them and the existing rows stay at `NULL` —
-- the REST surface then returns `completion: null` without crashing.
-- This is the replay-safety contract.
--
-- `completion_verification_json` is TEXT (not JSONB) deliberately.
-- Per the no-DB-specific-features memory, we use only SQL surface that
-- the common backends share — the value is written and read wholesale,
-- never queried with pg JSONB operators, so TEXT serde-JSON is the
-- portable choice (matches `route_policies.rules` on SQLite, etc.).

ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS completion_summary            TEXT NULL;

ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS completion_verification_json  TEXT NULL;

ALTER TABLE runs
    ADD COLUMN IF NOT EXISTS completion_annotated_at_ms    BIGINT NULL;
