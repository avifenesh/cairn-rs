//! Worker-registry service — thin shim over [`Engine`].
//!
//! Register / heartbeat / mark-dead flow through the [`Engine`] trait
//! (Phase D PR 1); the Valkey-specific hash / index / capability
//! writes live in `engine/valkey_impl.rs`.
//!
//! **Lean-bridge silence (intentional).** None of this service's
//! methods emit `BridgeEvent`s — worker lifecycle is FF-owned
//! operational state with no corresponding cairn-store projection.
//! See `docs/design/bridge-event-audit.md` §2.5.
use std::sync::Arc;

use ff_core::types::{WorkerId, WorkerInstanceId};

use crate::engine::control_plane_types::WorkerRegistration as EngineWorkerRegistration;
use crate::engine::Engine;
use crate::error::FabricError;

/// Historical service-level name — kept as a re-export so importers of
/// `crate::services::worker_service::WorkerRegistration` keep working.
pub type WorkerRegistration = EngineWorkerRegistration;

pub struct FabricWorkerService {
    engine: Arc<dyn Engine>,
}

impl FabricWorkerService {
    pub fn new(engine: Arc<dyn Engine>) -> Self {
        Self { engine }
    }

    pub async fn register_worker(
        &self,
        worker_id: &WorkerId,
        instance_id: &WorkerInstanceId,
        capabilities: &[String],
    ) -> Result<WorkerRegistration, FabricError> {
        self.engine
            .register_worker(worker_id, instance_id, capabilities)
            .await
    }

    pub async fn heartbeat_worker(
        &self,
        instance_id: &WorkerInstanceId,
    ) -> Result<(), FabricError> {
        self.engine.heartbeat_worker(instance_id).await
    }

    pub async fn mark_worker_dead(
        &self,
        instance_id: &WorkerInstanceId,
    ) -> Result<(), FabricError> {
        self.engine.mark_worker_dead(instance_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ff_core::types::{WorkerId, WorkerInstanceId};

    #[test]
    fn worker_registration_fields() {
        let reg = WorkerRegistration {
            worker_id: WorkerId::new("w1"),
            instance_id: WorkerInstanceId::new("inst1"),
            capabilities: vec!["gpu=true".into(), "model=large".into()],
            registered_at_ms: 1000,
        };
        assert_eq!(reg.worker_id.as_str(), "w1");
        assert_eq!(reg.instance_id.as_str(), "inst1");
        assert_eq!(reg.capabilities.len(), 2);
        assert_eq!(reg.registered_at_ms, 1000);
    }
}
