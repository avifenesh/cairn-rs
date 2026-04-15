use std::sync::Arc;

use crate::active_tasks::ActiveTaskRegistry;
use crate::boot::FabricRuntime;
use crate::error::FabricError;
use crate::services::scheduler_service::FabricSchedulerService;
use ff_core::keys;
use ff_core::types::{ExecutionId, LaneId, WorkerId, WorkerInstanceId};

pub struct WorkerRegistration {
    pub worker_id: WorkerId,
    pub instance_id: WorkerInstanceId,
    pub capabilities: Vec<String>,
    pub registered_at_ms: u64,
}

pub struct ClaimResult {
    pub execution_id: ExecutionId,
    pub grant_key: String,
}

pub struct FabricWorkerService {
    runtime: Arc<FabricRuntime>,
    scheduler: FabricSchedulerService,
    registry: Arc<ActiveTaskRegistry>,
}

impl FabricWorkerService {
    pub fn new(runtime: Arc<FabricRuntime>, registry: Arc<ActiveTaskRegistry>) -> Self {
        let scheduler = FabricSchedulerService::new(&runtime);
        Self {
            runtime,
            scheduler,
            registry,
        }
    }

    pub async fn register_worker(
        &self,
        worker_id: &WorkerId,
        instance_id: &WorkerInstanceId,
        capabilities: &[String],
    ) -> Result<WorkerRegistration, FabricError> {
        let worker_key = keys::worker_key(instance_id);
        let now_ms = now_ms();

        let fields: Vec<(&str, String)> = vec![
            ("worker_id", worker_id.to_string()),
            ("instance_id", instance_id.to_string()),
            ("capabilities", capabilities.join(",")),
            ("last_heartbeat_ms", now_ms.to_string()),
            ("is_alive", "true".into()),
            ("registered_at_ms", now_ms.to_string()),
        ];

        for (field, value) in &fields {
            self.runtime
                .client
                .hset(&worker_key, field, value)
                .await
                .map_err(|e| FabricError::Valkey(format!("HSET {worker_key} {field}: {e}")))?;
        }

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

    pub async fn claim_next(
        &self,
        lane_id: &LaneId,
        worker_id: &WorkerId,
        instance_id: &WorkerInstanceId,
    ) -> Result<Option<ClaimResult>, FabricError> {
        let grant = self
            .scheduler
            .claim_for_worker(lane_id, worker_id, instance_id, 5000)
            .await?;

        let grant = match grant {
            Some(g) => g,
            None => return Ok(None),
        };

        Ok(Some(ClaimResult {
            execution_id: grant.execution_id,
            grant_key: grant.grant_key,
        }))
    }

    pub fn registry(&self) -> &Arc<ActiveTaskRegistry> {
        &self.registry
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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
    fn claim_result_fields() {
        let eid = ExecutionId::from_uuid(uuid::Uuid::nil());
        let result = ClaimResult {
            execution_id: eid.clone(),
            grant_key: "ff:grant:test".into(),
        };
        assert_eq!(result.execution_id, eid);
        assert_eq!(result.grant_key, "ff:grant:test");
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
