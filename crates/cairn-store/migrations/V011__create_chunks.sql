-- Document chunks with provenance for owned retrieval (RFC 003).
-- Each chunk retains source document ID, ownership, and position.

CREATE TABLE chunks (
    chunk_id     TEXT PRIMARY KEY,
    document_id  TEXT NOT NULL REFERENCES documents(document_id),
    source_id    TEXT NOT NULL,
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id   TEXT NOT NULL,
    source_type  TEXT NOT NULL,
    text         TEXT NOT NULL,
    position     INTEGER NOT NULL,
    embedding    BYTEA,
    created_at   BIGINT NOT NULL
);

CREATE INDEX idx_chunks_document ON chunks (document_id);
CREATE INDEX idx_chunks_project ON chunks (tenant_id, workspace_id, project_id);
CREATE INDEX idx_chunks_source ON chunks (source_id);
