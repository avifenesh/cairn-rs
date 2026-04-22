-- Prompt schema hardening: enforce invariants that code already relies on.
--
-- Context: PR #102 ported prompt_* tables to SQLite and reviewers noted
-- that both backends were missing defensive constraints the application
-- code relies on (version allocation via MAX+1, updated_at always written
-- at insert time). The original PR kept changes as a pure port; this
-- migration ships the hardening symmetrically across Postgres and
-- SQLite so the two backends stay shape-identical.
--
-- Three changes:
--   1. prompt_versions: UNIQUE(prompt_asset_id, version_number)
--   2. prompt_versions.version_number NOT NULL
--   3. prompt_assets.updated_at NOT NULL
--
-- Backfill strategy runs first for every constraint, then the constraint
-- is applied. Backfill is defensive: production data is not expected to
-- violate these invariants (the projection code writes every field), but
-- we refuse to apply a constraint that would fail on live data.

-- ── (1) prompt_assets.updated_at NOT NULL ────────────────────────────

-- Backfill any row where updated_at is NULL using created_at. The
-- PromptAssetCreated projection writes both timestamps at insert time,
-- so NULL rows can only exist from pre-projection data or from a bug
-- that no longer exists.
UPDATE prompt_assets
   SET updated_at = created_at
 WHERE updated_at IS NULL;

ALTER TABLE prompt_assets
    ALTER COLUMN updated_at SET NOT NULL;

-- ── (2) prompt_versions.version_number NOT NULL ──────────────────────

-- The projection allocates version_number via COALESCE(MAX+1) at insert
-- time, so NULL rows are not expected. Attempt a conservative backfill
-- by re-deriving sequence numbers ordered by created_at within each
-- asset. If any remain NULL afterwards (e.g. ties on created_at with
-- gaps in the existing numbering) we raise loudly rather than paper
-- over the anomaly.
WITH ranked AS (
    SELECT prompt_version_id,
           ROW_NUMBER() OVER (
               PARTITION BY prompt_asset_id
               ORDER BY created_at, prompt_version_id
           ) AS rn
      FROM prompt_versions
     WHERE version_number IS NULL
)
UPDATE prompt_versions pv
   SET version_number = ranked.rn
  FROM ranked
 WHERE pv.prompt_version_id = ranked.prompt_version_id
   AND pv.version_number IS NULL;

DO $$
DECLARE
    null_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO null_count
      FROM prompt_versions
     WHERE version_number IS NULL;
    IF null_count > 0 THEN
        RAISE EXCEPTION
            'V023 backfill incomplete: % prompt_versions rows still have NULL version_number',
            null_count;
    END IF;
END $$;

ALTER TABLE prompt_versions
    ALTER COLUMN version_number SET NOT NULL;

-- ── (3) prompt_versions UNIQUE(prompt_asset_id, version_number) ──────

-- Dedup defensively: if duplicate (asset, version_number) pairs exist
-- (they should not — the projection serializes allocation), keep the
-- oldest row by created_at (ties broken by prompt_version_id) and
-- delete the rest. We raise loudly first so the operator sees what
-- was cleaned up in the migration log.
DO $$
DECLARE
    dup_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO dup_count FROM (
        SELECT 1
          FROM prompt_versions
         GROUP BY prompt_asset_id, version_number
        HAVING COUNT(*) > 1
    ) d;
    IF dup_count > 0 THEN
        RAISE NOTICE
            'V023 dedup: found % duplicate (prompt_asset_id, version_number) groups; keeping oldest row per group',
            dup_count;
    END IF;
END $$;

DELETE FROM prompt_versions pv
 USING (
    SELECT prompt_version_id,
           ROW_NUMBER() OVER (
               PARTITION BY prompt_asset_id, version_number
               ORDER BY created_at, prompt_version_id
           ) AS rn
      FROM prompt_versions
 ) ranked
 WHERE pv.prompt_version_id = ranked.prompt_version_id
   AND ranked.rn > 1;

ALTER TABLE prompt_versions
    ADD CONSTRAINT prompt_versions_asset_version_unique
    UNIQUE (prompt_asset_id, version_number);
