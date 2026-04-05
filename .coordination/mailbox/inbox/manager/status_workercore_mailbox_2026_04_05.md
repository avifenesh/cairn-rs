# Status Update — Worker Core

## Task: mailbox_messaging
- **Tests**: 10/10 pass
- **Files created**: crates/cairn-store/tests/mailbox_messaging.rs
- **Files changed**: none
- **Issues**: none
- **Notable**: list_by_run sorts lexicographically by message_id (not created_at); deferred messages (deliver_at_ms=0) excluded from list_pending; both verified explicitly
