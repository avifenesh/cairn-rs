-- Graph nodes for provenance, execution, and knowledge relationships (RFC 004).
-- Stored in the product-owned store, not a separate graph database.

CREATE TABLE graph_nodes (
    node_id    TEXT PRIMARY KEY,
    kind       TEXT NOT NULL,
    tenant_id  TEXT,
    workspace_id TEXT,
    project_id TEXT,
    metadata   JSONB NOT NULL DEFAULT '{}',
    created_at BIGINT NOT NULL
);

CREATE INDEX idx_graph_nodes_kind ON graph_nodes (kind);
CREATE INDEX idx_graph_nodes_project ON graph_nodes (tenant_id, workspace_id, project_id)
    WHERE project_id IS NOT NULL;
