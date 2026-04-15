use crate::error::FabricError;
use ff_core::types::{LaneId, Namespace, WorkerId, WorkerInstanceId};

#[derive(Clone, Debug)]
pub struct FabricConfig {
    pub valkey_host: String,
    pub valkey_port: u16,
    pub tls: bool,
    pub cluster: bool,
    pub lane_id: LaneId,
    pub worker_id: WorkerId,
    pub worker_instance_id: WorkerInstanceId,
    pub namespace: Namespace,
    pub lease_ttl_ms: u64,
    pub max_concurrent_tasks: usize,
}

impl FabricConfig {
    pub fn from_env() -> Result<Self, FabricError> {
        let valkey_host = std::env::var("CAIRN_FABRIC_HOST").unwrap_or_else(|_| "localhost".into());
        let valkey_port = std::env::var("CAIRN_FABRIC_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6379);
        let tls = std::env::var("CAIRN_FABRIC_TLS")
            .ok()
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let cluster = std::env::var("CAIRN_FABRIC_CLUSTER")
            .ok()
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let lane_id =
            LaneId::new(std::env::var("CAIRN_FABRIC_LANE").unwrap_or_else(|_| "cairn".into()));
        let worker_id = WorkerId::new(
            std::env::var("CAIRN_FABRIC_WORKER_ID").unwrap_or_else(|_| "cairn-worker".into()),
        );
        let worker_instance_id = WorkerInstanceId::new(
            std::env::var("CAIRN_FABRIC_INSTANCE_ID")
                .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string()),
        );
        let namespace = Namespace::new(
            std::env::var("CAIRN_FABRIC_NAMESPACE").unwrap_or_else(|_| "cairn".into()),
        );
        let lease_ttl_ms = std::env::var("CAIRN_FABRIC_LEASE_TTL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30_000);
        let max_concurrent_tasks = std::env::var("CAIRN_FABRIC_MAX_TASKS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);

        Ok(Self {
            valkey_host,
            valkey_port,
            tls,
            cluster,
            lane_id,
            worker_id,
            worker_instance_id,
            namespace,
            lease_ttl_ms,
            max_concurrent_tasks,
        })
    }

    pub fn valkey_url(&self) -> String {
        let scheme = if self.tls { "valkeys" } else { "valkey" };
        format!("{}://{}:{}", scheme, self.valkey_host, self.valkey_port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_from_env() {
        // Clear any existing env vars to test defaults
        std::env::remove_var("CAIRN_FABRIC_HOST");
        std::env::remove_var("CAIRN_FABRIC_PORT");
        std::env::remove_var("CAIRN_FABRIC_TLS");
        std::env::remove_var("CAIRN_FABRIC_CLUSTER");
        std::env::remove_var("CAIRN_FABRIC_LANE");

        let config = FabricConfig::from_env().unwrap();
        assert_eq!(config.valkey_host, "localhost");
        assert_eq!(config.valkey_port, 6379);
        assert!(!config.tls);
        assert!(!config.cluster);
        assert_eq!(config.lane_id.as_str(), "cairn");
        assert_eq!(config.lease_ttl_ms, 30_000);
        assert_eq!(config.max_concurrent_tasks, 4);
    }

    #[test]
    fn valkey_url_without_tls() {
        let config = FabricConfig {
            valkey_host: "myhost".into(),
            valkey_port: 6380,
            tls: false,
            cluster: false,
            lane_id: LaneId::new("test"),
            worker_id: WorkerId::new("w1"),
            worker_instance_id: WorkerInstanceId::new("inst1"),
            namespace: Namespace::new("ns"),
            lease_ttl_ms: 30_000,
            max_concurrent_tasks: 1,
        };
        assert_eq!(config.valkey_url(), "valkey://myhost:6380");
    }

    #[test]
    fn valkey_url_with_tls() {
        let config = FabricConfig {
            valkey_host: "secure.host".into(),
            valkey_port: 6379,
            tls: true,
            cluster: false,
            lane_id: LaneId::new("test"),
            worker_id: WorkerId::new("w1"),
            worker_instance_id: WorkerInstanceId::new("inst1"),
            namespace: Namespace::new("ns"),
            lease_ttl_ms: 30_000,
            max_concurrent_tasks: 1,
        };
        assert_eq!(config.valkey_url(), "valkeys://secure.host:6379");
    }
}
