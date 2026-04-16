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
    pub grant_ttl_ms: u64,
    pub max_concurrent_tasks: usize,
    pub signal_dedup_ttl_ms: u64,
    pub fcall_timeout_ms: u64,
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
                .unwrap_or_else(|_| load_or_generate_instance_id()),
        );
        let namespace = Namespace::new(
            std::env::var("CAIRN_FABRIC_NAMESPACE").unwrap_or_else(|_| "cairn".into()),
        );
        let lease_ttl_ms = std::env::var("CAIRN_FABRIC_LEASE_TTL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30_000);
        let grant_ttl_ms = std::env::var("CAIRN_FABRIC_GRANT_TTL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_000);
        let max_concurrent_tasks = std::env::var("CAIRN_FABRIC_MAX_TASKS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(4);
        let signal_dedup_ttl_ms = std::env::var("CAIRN_FABRIC_SIGNAL_DEDUP_TTL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(86_400_000);
        let fcall_timeout_ms = std::env::var("CAIRN_FABRIC_FCALL_TIMEOUT_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5_000);

        let config = Self {
            valkey_host,
            valkey_port,
            tls,
            cluster,
            lane_id,
            worker_id,
            worker_instance_id,
            namespace,
            lease_ttl_ms,
            grant_ttl_ms,
            max_concurrent_tasks,
            signal_dedup_ttl_ms,
            fcall_timeout_ms,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), FabricError> {
        if self.valkey_port == 0 {
            return Err(FabricError::Config("port must be > 0".into()));
        }
        if self.lease_ttl_ms < 1000 {
            return Err(FabricError::Config("lease_ttl_ms must be >= 1000".into()));
        }
        if self.max_concurrent_tasks < 1 {
            return Err(FabricError::Config(
                "max_concurrent_tasks must be >= 1".into(),
            ));
        }
        Ok(())
    }

    pub fn valkey_url(&self) -> String {
        let scheme = if self.tls { "valkeys" } else { "valkey" };
        format!("{}://{}:{}", scheme, self.valkey_host, self.valkey_port)
    }
}

const INSTANCE_ID_FILE: &str = "/tmp/cairn-fabric-instance-id";

fn load_or_generate_instance_id() -> String {
    if let Ok(id) = std::fs::read_to_string(INSTANCE_ID_FILE) {
        let id = id.trim().to_owned();
        if !id.is_empty() {
            return id;
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::write(INSTANCE_ID_FILE, &id);
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn default_config_from_env() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("CAIRN_FABRIC_HOST");
        std::env::remove_var("CAIRN_FABRIC_PORT");
        std::env::remove_var("CAIRN_FABRIC_TLS");
        std::env::remove_var("CAIRN_FABRIC_CLUSTER");
        std::env::remove_var("CAIRN_FABRIC_LANE");
        std::env::remove_var("CAIRN_FABRIC_LEASE_TTL_MS");
        std::env::remove_var("CAIRN_FABRIC_MAX_TASKS");
        std::env::remove_var("CAIRN_FABRIC_GRANT_TTL_MS");

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
            grant_ttl_ms: 5_000,
            max_concurrent_tasks: 1,
            signal_dedup_ttl_ms: 86_400_000,
            fcall_timeout_ms: 5_000,
        };
        assert_eq!(config.valkey_url(), "valkey://myhost:6380");
    }

    fn test_config(
        port: u16,
        lease_ttl_ms: u64,
        max_tasks: usize,
    ) -> Result<FabricConfig, FabricError> {
        let config = FabricConfig {
            valkey_host: "localhost".into(),
            valkey_port: port,
            tls: false,
            cluster: false,
            lane_id: LaneId::new("test"),
            worker_id: WorkerId::new("w"),
            worker_instance_id: WorkerInstanceId::new("i"),
            namespace: Namespace::new("ns"),
            lease_ttl_ms,
            grant_ttl_ms: 5_000,
            max_concurrent_tasks: max_tasks,
            signal_dedup_ttl_ms: 86_400_000,
            fcall_timeout_ms: 5_000,
        };
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn rejects_zero_port() {
        let result = test_config(0, 30_000, 4);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("port"));
    }

    #[test]
    fn rejects_low_lease_ttl() {
        let result = test_config(6379, 500, 4);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("lease_ttl_ms"));
    }

    #[test]
    fn rejects_zero_concurrent_tasks() {
        let result = test_config(6379, 30_000, 0);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("max_concurrent_tasks"));
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
            grant_ttl_ms: 5_000,
            max_concurrent_tasks: 1,
            signal_dedup_ttl_ms: 86_400_000,
            fcall_timeout_ms: 5_000,
        };
        assert_eq!(config.valkey_url(), "valkeys://secure.host:6379");
    }
}
