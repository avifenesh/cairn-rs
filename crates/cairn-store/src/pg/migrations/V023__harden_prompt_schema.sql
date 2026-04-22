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
-- Backfill runs first for every constraint; the constraint is applied
-- only after the data is known to satisfy it. Backfill is defensive:
-- production data is not expected to violate these invariants (the
-- projection code writes every field), but we refuse to apply a
-- constraint that would fail on live data.

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

-- ── (2) + (3) prompt_versions: NOT NULL + UNIQUE in one re-sequence ───

-- Re-sequence version_number across *all* rows per asset ordered by
-- (created_at, prompt_version_id). This is a single, idempotent
-- operation that:
--   - backfills any NULL version_number, and
--   - resolves any pre-existing duplicate (prompt_asset_id,
--     version_number) pair
-- without deleting any rows.
--
-- This is safe because version_number is a display field: relations
-- from prompt_releases (and anywhere else) use prompt_version_id (the
-- primary key), never the (asset, version_number) pair. The ordering
-- key (created_at, prompt_version_id) is stable and independent of the
-- current version_number, so running this migration on already-clean
-- data is a no-op.
--
-- Credit: this replaces an earlier NULL-only backfill that could have
-- collided with existing numbers and triggered silent row deletion in
-- the subsequent dedup step (gemini-code-assist review on PR #103).
UPDATE prompt_versions pv
   SET version_number = ranked.new_vn
  FROM (
    SELECT prompt_version_id,
           ROW_NUMBER() OVER (
               PARTITION BY prompt_asset_id
               ORDER BY created_at, prompt_version_id
           ) AS new_vn
      FROM prompt_versions
  ) ranked
 WHERE pv.prompt_version_id = ranked.prompt_version_id
   AND pv.version_number IS DISTINCT FROM ranked.new_vn;

-- Belt-and-braces: after re-sequencing, no NULLs and no duplicates
-- should remain. Raise loudly if either invariant is still violated
-- (impossible given the UPDATE above, but cheap to check).
DO $$
DECLARE
    null_count BIGINT;
    dup_count BIGINT;
BEGIN
    SELECT COUNT(*) INTO null_count
      FROM prompt_versions
     WHERE version_number IS NULL;
    IF null_count > 0 THEN
        RAISE EXCEPTION
            'V023 backfill incomplete: % prompt_versions rows still have NULL version_number',
            null_count;
    END IF;

    SELECT COUNT(*) INTO dup_count FROM (
        SELECT 1
          FROM prompt_versions
         GROUP BY prompt_asset_id, version_number
        HAVING COUNT(*) > 1
    ) d;
    IF dup_count > 0 THEN
        RAISE EXCEPTION
            'V023 re-sequence incomplete: % duplicate (prompt_asset_id, version_number) groups remain',
            dup_count;
    END IF;
END $$;

ALTER TABLE prompt_versions
    ALTER COLUMN version_number SET NOT NULL;

ALTER TABLE prompt_versions
    ADD CONSTRAINT prompt_versions_asset_version_unique
    UNIQUE (prompt_asset_id, version_number);
