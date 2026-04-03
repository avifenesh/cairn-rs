-- Graph edges for typed directed relationships (RFC 004).
-- Supports the 6 v1 query families: execution trace, dependency path,
-- prompt provenance, retrieval provenance, decision involvement, eval lineage.

CREATE TABLE graph_edges (
    id              BIGSERIAL PRIMARY KEY,
    source_node_id  TEXT NOT NULL REFERENCES graph_nodes(node_id),
    target_node_id  TEXT NOT NULL REFERENCES graph_nodes(node_id),
    kind            TEXT NOT NULL,
    metadata        JSONB NOT NULL DEFAULT '{}',
    created_at      BIGINT NOT NULL
);

CREATE INDEX idx_graph_edges_source ON graph_edges (source_node_id);
CREATE INDEX idx_graph_edges_target ON graph_edges (target_node_id);
CREATE INDEX idx_graph_edges_kind ON graph_edges (kind);
CREATE UNIQUE INDEX idx_graph_edges_unique ON graph_edges (source_node_id, target_node_id, kind);
