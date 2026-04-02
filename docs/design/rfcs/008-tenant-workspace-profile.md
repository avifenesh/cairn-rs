# RFC 008: Tenant, Workspace, Profile Separation

Status: stub  
Owner: architecture lead  
Depends on: [RFC 001](./001-product-boundary.md)

## Purpose

Define the ownership and scoping model for:

- tenant
- workspace
- project
- user/operator profile
- defaults versus runtime data

## Must Decide

1. Minimum tenancy model for v1
2. Which entities are tenant-scoped versus workspace-scoped versus project-scoped
3. Where profile data lives
4. How defaults differ from user data
5. Which schemas must be tenancy-aware in Phase 1

## Blocking Reason

Storage, permissions, retrieval, graph, and evals all depend on this.
