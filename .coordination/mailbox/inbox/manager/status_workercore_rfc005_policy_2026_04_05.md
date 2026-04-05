# Status Update — Worker Core

## Task: approval_policy_lifecycle (RFC 005)
- **Tests**: 13/13 pass
- **Files created**: crates/cairn-store/tests/approval_policy_lifecycle.rs
- **Files modified**: crates/cairn-store/src/in_memory.rs — added InMemoryStore::attach_release_to_policy() non-trait method
- **Issues**: none
- **Notable**:
  - attached_release_ids is always empty from ApprovalPolicyCreated (no domain event exists to update it — it is a governance-layer concern). Added attach_release_to_policy() as a non-trait InMemoryStore method to enable the update test.
  - attach_release_to_policy() is idempotent (deduplicates on insert).
  - list_by_tenant sorts by policy_id string (lexicographic), not by created_at.
  - Cross-tenant isolation verified: same policy name in two tenants, each scoped correctly.
