2026-04-03 manager summary

- Worker 1 closed this round’s API surface cut for `task_update` / `approval_required` by adding a current-state SSE enrichment seam in [crates/cairn-api/src/sse_payloads.rs](/mnt/c/Users/avife/cairn-rs/crates/cairn-api/src/sse_payloads.rs) and [crates/cairn-api/src/sse_publisher.rs](/mnt/c/Users/avife/cairn-rs/crates/cairn-api/src/sse_publisher.rs). The exact builders can now consume `TaskRecord` / `ApprovalRecord` when store context is available, tests were updated in [crates/cairn-api/tests/sse_payload_alignment.rs](/mnt/c/Users/avife/cairn-rs/crates/cairn-api/tests/sse_payload_alignment.rs) and [crates/cairn-api/tests/migration_report_consistency.rs](/mnt/c/Users/avife/cairn-rs/crates/cairn-api/tests/migration_report_consistency.rs), and the Phase 0 migration reports were refreshed to keep the remaining thin runtime fallback explicit rather than implied. Proof: `cargo test -p cairn-api --quiet` passed.
- Worker 2 confirmed this seam does not need new shared-domain or runtime helper surface. `RuntimeEvent::project()` and `RuntimeEvent::primary_entity_ref()` already provide the task/approval identity the API path needs, so the remaining work stays in API/store composition and runtime shaping rather than `cairn-domain`. Proof: `cargo test -p cairn-domain --lib task_and_approval_events_already_carry_identity_for_enrichment --quiet` and `cargo test -p cairn-api --test sse_payload_alignment --quiet` passed.
- Worker 3 strengthened the store-side proof in [crates/cairn-store/tests/cross_backend_parity.rs](/mnt/c/Users/avife/cairn-rs/crates/cairn-store/tests/cross_backend_parity.rs) so `TaskReadModel::get(...)` and `ApprovalReadModel::get(...)` now guard the backend-stable fields the enriched SSE path depends on, including metadata beyond the older state/decision-only checks. Proof: `cargo test -p cairn-store --test cross_backend_parity --features sqlite` passed with 16/16 tests green.

Residual seam:

- The generic runtime-event fallback for `task_update` / `approval_required` is still intentionally thinner when no current-state read-model row is supplied. The exact builder path is now real and guarded; the next honest follow-up is wiring that current-state context through the live publish path wherever it exists.

Final manager verification:

- `cargo test -p cairn-api --quiet` passed
- `cargo test -p cairn-store --test cross_backend_parity --features sqlite --quiet` passed
- `cargo test -p cairn-domain --lib task_and_approval_events_already_carry_identity_for_enrichment --quiet` passed
