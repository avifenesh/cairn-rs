use std::sync::Arc;

use crate::boot::FabricRuntime;
use crate::error::FabricError;
use crate::helpers::now_ms;
use ff_core::keys;
use ff_core::types::{WorkerId, WorkerInstanceId};

pub struct WorkerRegistration {
    pub worker_id: WorkerId,
    pub instance_id: WorkerInstanceId,
    pub capabilities: Vec<String>,
    pub registered_at_ms: u64,
}

/// Worker-registry service.
///
/// **Lean-bridge silence (intentional).** None of this service's methods emit
/// `BridgeEvent`s — worker lifecycle is FF-owned operational state with no
/// corresponding cairn-store projection. `register_worker`, `heartbeat_worker`,
/// and `mark_worker_dead` all mutate FF's worker hash / TTL directly; cairn
/// has no `WorkerReadModel` that needs to stay in sync, so emission would be
/// a projection write with no reader. Production claim flow lives in
/// `CairnWorker::claim_next` (worker_sdk.rs).
///
/// If a future UI surface renders worker-status (alive / dead / last-heartbeat)
/// from the cairn projection rather than from FF admin-reads, add BridgeEvent
/// variants for register/dead/claim and revisit this note. Until then:
/// additions here must not emit.
///
/// See `docs/design/bridge-event-audit.md` §2.5 for the full rationale.
pub struct FabricWorkerService {
    runtime: Arc<FabricRuntime>,
}

impl FabricWorkerService {
    pub fn new(runtime: Arc<FabricRuntime>) -> Self {
        Self { runtime }
    }

    pub async fn register_worker(
        &self,
        worker_id: &WorkerId,
        instance_id: &WorkerInstanceId,
        capabilities: &[String],
    ) -> Result<WorkerRegistration, FabricError> {
        let worker_key = keys::worker_key(instance_id);
        let now_ms = now_ms();

        let now_str = now_ms.to_string();
        self.runtime
            .client
            .cmd("HSET")
            .arg(&worker_key)
            .arg("worker_id")
            .arg(worker_id.to_string())
            .arg("instance_id")
            .arg(instance_id.to_string())
            .arg("capabilities")
            .arg(capabilities.join(","))
            .arg("last_heartbeat_ms")
            .arg(&now_str)
            .arg("is_alive")
            .arg("true")
            .arg("registered_at_ms")
            .arg(&now_str)
            .execute::<u64>()
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET {worker_key}: {e}")))?;

        // TTL-based expiry: dead workers auto-expire if heartbeat stops.
        // mark_worker_dead is explicit opt-out; this is the implicit safety net.
        let ttl_ms = self.runtime.config.lease_ttl_ms * 3;
        self.runtime
            .client
            .cmd("PEXPIRE")
            .arg(&worker_key)
            .arg(ttl_ms.to_string())
            .execute::<u64>()
            .await
            .map_err(|e| FabricError::Valkey(format!("PEXPIRE {worker_key}: {e}")))?;

        let workers_index = keys::workers_index_key();
        self.runtime
            .client
            .cmd("SADD")
            .arg(workers_index)
            .arg(instance_id.to_string())
            .execute::<u64>()
            .await
            .map_err(|e| FabricError::Valkey(format!("SADD workers index: {e}")))?;

        for cap in capabilities {
            if let Some((k, v)) = cap.split_once('=') {
                let cap_key = keys::workers_capability_key(k, v);
                self.runtime
                    .client
                    .cmd("SADD")
                    .arg(cap_key)
                    .arg(instance_id.to_string())
                    .execute::<u64>()
                    .await
                    .map_err(|e| FabricError::Valkey(format!("SADD cap index: {e}")))?;
            }
        }

        Ok(WorkerRegistration {
            worker_id: worker_id.clone(),
            instance_id: instance_id.clone(),
            capabilities: capabilities.to_vec(),
            registered_at_ms: now_ms,
        })
    }

    pub async fn heartbeat_worker(
        &self,
        instance_id: &WorkerInstanceId,
    ) -> Result<(), FabricError> {
        let worker_key = keys::worker_key(instance_id);
        let now = now_ms().to_string();
        self.runtime
            .client
            .hset(&worker_key, "last_heartbeat_ms", &now)
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET heartbeat: {e}")))?;

        let ttl_ms = self.runtime.config.lease_ttl_ms * 3;
        self.runtime
            .client
            .cmd("PEXPIRE")
            .arg(&worker_key)
            .arg(ttl_ms.to_string())
            .execute::<u64>()
            .await
            .map_err(|e| FabricError::Valkey(format!("PEXPIRE heartbeat: {e}")))?;

        Ok(())
    }

    pub async fn mark_worker_dead(
        &self,
        instance_id: &WorkerInstanceId,
    ) -> Result<(), FabricError> {
        let worker_key = keys::worker_key(instance_id);
        self.runtime
            .client
            .hset(&worker_key, "is_alive", "false")
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET is_alive: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn now_ms_positive() {
        let ts = now_ms();
        assert!(ts > 1_700_000_000_000);
    }

    #[test]
    fn worker_key_format() {
        let wid = WorkerInstanceId::new("inst_abc");
        let key = keys::worker_key(&wid);
        assert!(key.contains("inst_abc"));
        assert!(key.starts_with("ff:worker:"));
    }

    #[test]
    fn workers_index_key_stable() {
        let k1 = keys::workers_index_key();
        let k2 = keys::workers_index_key();
        assert_eq!(k1, k2);
        assert_eq!(k1, "ff:idx:workers");
    }

    #[test]
    fn capability_key_format() {
        let key = keys::workers_capability_key("gpu", "true");
        assert_eq!(key, "ff:idx:workers:cap:gpu:true");
    }

    #[test]
    fn capability_parsing_kv() {
        let cap = "gpu=true";
        let (k, v) = cap.split_once('=').unwrap();
        assert_eq!(k, "gpu");
        assert_eq!(v, "true");
    }

    #[test]
    fn capability_without_equals_skipped() {
        let cap = "bare_cap";
        assert!(cap.split_once('=').is_none());
    }
}
