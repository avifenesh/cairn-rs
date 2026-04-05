# STATUS: credential_rotation

**Task:** RFC 002 credential rotation lifecycle hardening  
**Tests passed:** 6/6  
**File:** `crates/cairn-store/tests/credential_rotation.rs`

Tests:
- `credential_stored_is_readable`
- `credential_key_rotation_creates_rotation_record`
- `multiple_rotations_accumulate_in_audit_log`
- `credential_revoked_marks_inactive`
- `cross_tenant_credential_isolation`
- `full_credential_lifecycle_in_event_log`

Notes: CredentialRotationReadModel uses `list_rotations()` not `list_by_tenant()`. Both CredentialReadModel and CredentialRotationReadModel caused E0782 type ambiguity — resolved with typed helper functions using explicit trait dispatch.
