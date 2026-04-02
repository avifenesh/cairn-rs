# Compatibility and Break Matrix

Status: draft  
Purpose: classify inherited Cairn surfaces before parallel implementation begins

## Rule

Every inherited surface must be tagged:

- preserve
- intentionally break
- transitional

## Matrix

| Surface | Classification | Reason | Success Condition |
|---|---|---|---|
| Durable session/run/task/checkpoint concepts | Preserve | Core to product wedge | Same conceptual guarantees in Rust |
| Approvals, pause/resume, recovery | Preserve | Core runtime value | Equivalent or stronger operational semantics |
| Subagent/task linkage | Preserve | Needed for orchestration model | Explicit parent/child runtime linkage |
| Runtime observability and replay concepts | Preserve | Required for operator control | Operators can inspect and replay flows |
| Product-owned memory/retrieval semantics | Preserve and strengthen | Central product capability | Owned retrieval replaces Bedrock KB |
| Existing personal overlay as architecture | Intentionally break | Conflicts with team product boundary | Overlay becomes tenant/runtime data |
| Single-user assumptions in APIs and state | Intentionally break | Product is team-facing | APIs require scoped ownership context |
| Global singleton identity/profile files as runtime assumptions | Intentionally break | Wrong ownership model | Identity/profile modeled as scoped assets |
| Frontend-facing route shapes that are purely accidental | Intentionally break | Avoid preserving design accidents | Cleaner Rust APIs or compatibility wrappers |
| Selected HTTP/SSE compatibility surfaces needed for migration | Transitional | Reduce migration cost | Retained only with replacement path |
| glide-mq execution transport usage | Transitional | Useful bridge, not core truth | Runtime truth moves into Rust store |
| Legacy sidecar-owned mailbox/checkpoint/task truth | Intentionally break | Violates runtime ownership model | Rust runtime is canonical owner |

## Notes

- “Preserve” means preserve semantics, not implementation.
- “Intentionally break” means document and replace, not drift accidentally.
- “Transitional” means there must be an explicit removal or replacement plan.
