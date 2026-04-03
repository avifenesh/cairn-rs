-- Knowledge documents for owned retrieval (RFC 003).
-- Canonical storage for ingested documents with provenance.

CREATE TABLE documents (
    document_id  TEXT PRIMARY KEY,
    source_id    TEXT NOT NULL,
    tenant_id    TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    project_id   TEXT NOT NULL,
    source_type  TEXT NOT NULL,
    title        TEXT,
    ingest_status TEXT NOT NULL DEFAULT 'pending',
    version      BIGINT NOT NULL DEFAULT 1,
    created_at   BIGINT NOT NULL,
    updated_at   BIGINT NOT NULL
);

CREATE INDEX idx_documents_project ON documents (tenant_id, workspace_id, project_id);
CREATE INDEX idx_documents_source ON documents (source_id);
CREATE INDEX idx_documents_status ON documents (ingest_status);
