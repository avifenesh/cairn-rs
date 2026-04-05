# STATUS: tenant_org_lifecycle

**Task:** RFC 008 tenant organization hierarchy test  
**Tests passed:** 8/8  
**File:** `crates/cairn-store/tests/tenant_org_lifecycle.rs`

Tests:
- `tenant_created_stores_record`
- `tenant_not_created_returns_none`
- `workspace_created_stores_record`
- `full_hierarchy_tenant_workspace_project`
- `list_by_tenant_returns_all_workspaces_for_tenant`
- `workspace_scoping_is_cascade_boundary`
- `tenant_list_returns_all_tenants_in_order`
- `cross_tenant_org_isolation`

Note: InMemoryStore has no delete events (append-only). "Cascade" is proven via workspace-scoped query isolation — projects don't leak across workspaces; non-existent workspace returns empty.
