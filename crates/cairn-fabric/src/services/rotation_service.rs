//! Waitpoint HMAC rotation service — thin shim over [`ControlPlaneBackend`].
//!
//! Surfaces `ff_rotate_waitpoint_hmac_secret` over every execution
//! partition. The partition fan-out + per-partition classification
//! lives in the backend impl (see
//! `engine/valkey_control_plane_impl.rs`). This service exists so the
//! cairn-app handler and admin wiring can keep their
//! `state.fabric.rotation` path unchanged while the FF
//! partition/keys imports stay confined to the backend.
//!
//! Idempotency + partial-success semantics:
//!   * Each partition's FCALL converges on the same `(new_kid,
//!     new_secret_hex)` via its `noop` outcome — a failed fan-out is
//!     safe to retry with the same inputs.
//!   * Partial success (some partitions rotated, some failed) is
//!     surfaced as [`RotationOutcome::failed`] — operators see the
//!     exact list of failed partition indices and can re-run.
use std::sync::Arc;

use crate::engine::control_plane::ControlPlaneBackend;
use crate::engine::control_plane_types::{
    RotationFailure as EngineRotationFailure, RotationOutcome as EngineRotationOutcome,
};

/// Historical service-level names — kept as re-exports so cairn-app
/// handlers that imported these directly keep working. The underlying
/// types now live on the engine boundary.
pub type RotateOutcome = EngineRotationOutcome;
pub type RotationFailure = EngineRotationFailure;

/// Cairn-side rotation service.
pub struct FabricRotationService {
    backend: Arc<dyn ControlPlaneBackend>,
}

impl FabricRotationService {
    pub fn new(backend: Arc<dyn ControlPlaneBackend>) -> Self {
        Self { backend }
    }

    /// Rotate the waitpoint HMAC signing kid across every execution
    /// partition.
    ///
    /// Caller-facing validation (empty / `:`-containing `new_kid`,
    /// odd-length `new_secret_hex`, etc.) happens server-side via the
    /// FCALL; the service surfaces the typed error unchanged via
    /// [`RotationFailure::code`]. Callers can wrap this in an HTTP 400
    /// when the outcome shows every partition failed with the same
    /// input-validation code.
    pub async fn rotate_waitpoint_hmac(
        &self,
        new_kid: &str,
        new_secret_hex: &str,
        grace_ms: u64,
    ) -> RotateOutcome {
        self.backend
            .rotate_waitpoint_hmac(new_kid, new_secret_hex, grace_ms)
            .await
    }
}
