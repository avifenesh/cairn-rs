# Status Update — Worker Core

## Task: eval_pipeline (cairn-evals)
- **Tests**: 13/13 pass
- **Files created**: crates/cairn-evals/tests/eval_pipeline.rs
- **Files changed**: none
- **Issues**: one typo (await_or_unwrap → unwrap) caught on first compile, fixed immediately

## Task: mailbox_messaging (cairn-store)
- **Tests**: 10/10 pass
- **Files created**: crates/cairn-store/tests/mailbox_messaging.rs
- **Files changed**: none
- **Issues**: none — mailbox projection was clean
- **Notable**: list_by_run sorts by message_id lexicographically (not created_at); tests verify this explicitly. list_pending only returns messages with deliver_at_ms > 0 AND deliver_at_ms <= now (immediate messages with deliver_at_ms=0 are excluded).
