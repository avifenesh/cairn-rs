# STATUS: user_message_lifecycle

**Task:** RFC 002 user message lifecycle hardening  
**Tests passed:** 5/5  
**File:** `crates/cairn-store/tests/user_message_lifecycle.rs`

**Code added:**
- `cairn-domain/src/events.rs`: Added `content: String`, `sequence: u64`, `appended_at_ms: u64` fields to `UserMessageAppended` (all `#[serde(default)]` for backward compat)

Tests:
- `user_message_appended_is_stored_in_session_context`
- `multiple_messages_stored_and_ordered_by_sequence`
- `messages_from_different_sessions_are_isolated`
- `message_content_round_trips_without_loss` (Unicode, special chars, 4KB, empty)
- `user_messages_support_cursor_based_pagination`

Key: UserMessageAppended is a no-op projection — messages live in the event log and are retrieved via read_by_entity(EntityRef::Run).
