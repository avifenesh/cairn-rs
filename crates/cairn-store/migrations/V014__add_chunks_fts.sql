-- Add full-text search support to chunks for lexical retrieval (RFC 003).
-- Postgres-specific: uses tsvector and GIN index.

ALTER TABLE chunks ADD COLUMN IF NOT EXISTS tsv tsvector;

CREATE INDEX IF NOT EXISTS idx_chunks_fts ON chunks USING GIN (tsv);

-- Trigger to auto-update tsvector on insert/update.
CREATE OR REPLACE FUNCTION chunks_tsv_trigger() RETURNS trigger AS $$
BEGIN
    NEW.tsv := to_tsvector('english', NEW.text);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_chunks_tsv
    BEFORE INSERT OR UPDATE ON chunks
    FOR EACH ROW EXECUTE FUNCTION chunks_tsv_trigger();
