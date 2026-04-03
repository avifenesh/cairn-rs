-- Add product-level title and description to tasks and approvals
-- for SSE enrichment and operator surfaces.

ALTER TABLE tasks ADD COLUMN IF NOT EXISTS title TEXT;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS description TEXT;

ALTER TABLE approvals ADD COLUMN IF NOT EXISTS title TEXT;
ALTER TABLE approvals ADD COLUMN IF NOT EXISTS description TEXT;
