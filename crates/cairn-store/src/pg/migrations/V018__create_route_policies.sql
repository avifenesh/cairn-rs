-- Route policies: current-state table for provider routing rules (RFC 007).
-- Maps to the RoutePolicy domain type and RoutePolicyCreated/RoutePolicyUpdated events.
-- Each row represents one named routing policy for a tenant.

CREATE TABLE IF NOT EXISTS route_policies (
    policy_id   TEXT PRIMARY KEY,
    tenant_id   TEXT NOT NULL,
    name        TEXT NOT NULL,
    -- Serialised JSONB array of RoutePolicyRule objects.
    -- Kept as JSONB so the rule set can be extended without schema changes.
    rules       JSONB NOT NULL DEFAULT '[]',
    enabled     BOOLEAN NOT NULL DEFAULT TRUE,
    created_at  BIGINT NOT NULL,
    updated_at  BIGINT NOT NULL
);

-- Operator/fleet view: list all policies for a tenant ordered by creation time.
CREATE INDEX IF NOT EXISTS idx_route_policies_tenant
    ON route_policies (tenant_id, created_at, policy_id);

-- Provider routing hot-path: quickly find enabled policies for a tenant.
CREATE INDEX IF NOT EXISTS idx_route_policies_tenant_enabled
    ON route_policies (tenant_id, enabled, created_at);
