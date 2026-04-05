# Status Update — Worker Core

## Task: provider_call_status (RFC 009)
- **Tests**: 10/10 pass
- **Files created**: crates/cairn-store/tests/provider_call_status.rs
- **Files modified**: crates/cairn-store/src/in_memory.rs
  - Fixed bug: error_class was always stored as None; now populated from event
  - Added list_provider_calls_by_project() non-trait helper for scoping tests

## Bug fixed
ProviderCallRecord.error_class was hardcoded to None in the projection.
The ProviderCallCompleted event carries error_class: Option<ProviderCallErrorClass>
but it was being discarded. Now stored correctly from the event.
