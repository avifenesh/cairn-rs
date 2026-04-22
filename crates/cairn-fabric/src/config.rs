use std::collections::BTreeSet;

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
    /// Capabilities this worker advertises. Threaded into FF's
    /// `ff_issue_claim_grant` via `ff_scheduler::Scheduler::claim_for_worker`;
    /// FF skips executions whose `required_capabilities` are not a subset.
    /// BTreeSet guarantees the CSV FF builds is deterministically ordered.
    /// Empty set = "no capabilities advertised" (FF accepts, matches only
    /// executions that require nothing).
    pub worker_capabilities: BTreeSet<String>,
    /// Hex-encoded 32-byte HMAC secret used to sign waitpoint tokens (RFC-004
    /// §Waitpoint Security). FF mints a token for every waitpoint via
    /// `ff_suspend_execution` and validates it on every `ff_deliver_signal`;
    /// a missing secret causes every suspend to fail with
    /// `hmac_secret_not_initialized`.
    ///
    /// **Security**
    /// - MUST be 32 random bytes (64 hex characters, case-insensitive).
    /// - This secret controls waitpoint signal authentication. Leaking it
    ///   lets an attacker forge signals into any waitpoint on the server —
    ///   approvals, subagent completions, tool results, operator resumes.
    ///   Treat with the same care as a JWT signing key.
    /// - Keep it out of logs. `read_waitpoint_token` + `WaitpointToken`'s
    ///   Debug/Display both redact; the raw secret only lives in this field
    ///   and in Valkey's `hmac_secrets` hash.
    ///
    /// **Rotation** (not in this round) — FF exposes per-kid expiry via
    /// `rotate_waitpoint_hmac_secret` / validate_waitpoint_token's multi-kid
    /// scan. Cairn will wire a rotate endpoint in a later round.
    ///
    /// `None` = no seeding on boot. ff_suspend_execution will fail fast with
    /// the FF-side error code; operators can seed post-boot via the FF
    /// admin path or a dedicated cairn tool.
    pub waitpoint_hmac_secret: Option<String>,
    /// Key identifier paired with `waitpoint_hmac_secret`. Defaults to `"k1"`
    /// when a secret is configured but no kid is supplied. Must be non-empty
    /// when `waitpoint_hmac_secret` is `Some`. Arbitrary operator-chosen
    /// string; FF uses it only as a lookup key in its secrets hash.
    pub waitpoint_hmac_kid: Option<String>,
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
        // Comma-separated capability tokens. Empty / unset = no capabilities.
        // FF validates tokens server-side (no commas, no whitespace/control,
        // CAPS_MAX_TOKENS cap); fail-loud validation lives in
        // ff_scheduler::Scheduler::claim_for_worker.
        let worker_capabilities: BTreeSet<String> =
            std::env::var("CAIRN_FABRIC_WORKER_CAPABILITIES")
                .ok()
                .map(|v| {
                    v.split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
        // HMAC secret: hex-encoded 32-byte key. No default — operators must
        // opt in explicitly. Validation enforces shape in `validate()`.
        let waitpoint_hmac_secret = std::env::var("CAIRN_FABRIC_WAITPOINT_HMAC_SECRET")
            .ok()
            .filter(|s| !s.is_empty());
        let waitpoint_hmac_kid = std::env::var("CAIRN_FABRIC_WAITPOINT_HMAC_KID")
            .ok()
            .filter(|s| !s.is_empty());

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
            worker_capabilities,
            waitpoint_hmac_secret,
            waitpoint_hmac_kid,
        };
        config.validate()?;
        Ok(config)
    }

    /// Resolve the HMAC kid to seed with, falling back to `"k1"` when the
    /// operator sets a secret without specifying a kid. Returns `None` if
    /// no secret is configured (no seeding will run).
    pub fn resolved_waitpoint_hmac_kid(&self) -> Option<&str> {
        self.waitpoint_hmac_secret.as_ref()?;
        Some(
            self.waitpoint_hmac_kid
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("k1"),
        )
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
        if self.grant_ttl_ms == 0 {
            return Err(FabricError::Config("grant_ttl_ms must be > 0".into()));
        }
        if self.fcall_timeout_ms == 0 {
            return Err(FabricError::Config("fcall_timeout_ms must be > 0".into()));
        }
        if self.signal_dedup_ttl_ms == 0 {
            return Err(FabricError::Config(
                "signal_dedup_ttl_ms must be > 0".into(),
            ));
        }
        // HMAC secret: if supplied, MUST be exactly 64 hex chars (256-bit
        // key). Fail loud — a truncated or mis-encoded secret produces an
        // opaque HMAC failure at runtime that's painful to diagnose.
        if let Some(secret) = &self.waitpoint_hmac_secret {
            if secret.len() != 64 {
                return Err(FabricError::Config(format!(
                    "waitpoint_hmac_secret must be 64 hex chars (32 bytes), got {}",
                    secret.len()
                )));
            }
            if !secret.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(FabricError::Config(
                    "waitpoint_hmac_secret must be hex-encoded (0-9, a-f, A-F only)".into(),
                ));
            }
            // Kid must be present and non-empty iff secret is set. An empty
            // kid would produce HSET `secret:` (no suffix) which the Lua
            // loader treats as a malformed record.
            if let Some(kid) = &self.waitpoint_hmac_kid {
                if kid.is_empty() {
                    return Err(FabricError::Config(
                        "waitpoint_hmac_kid must not be empty when waitpoint_hmac_secret is set"
                            .into(),
                    ));
                }
                // Defensive: FF Lua builds the field name `secret:<kid>` and
                // `expires_at:<kid>`; a kid containing `:` would split the
                // hash-field parser (ff lua/helpers.lua:180-188). Reject here
                // so operator typos fail loud instead of silently corrupting
                // the validation path.
                if kid.contains(':') {
                    return Err(FabricError::Config(format!(
                        "waitpoint_hmac_kid must not contain ':' (FF field-name delimiter): {kid:?}"
                    )));
                }
            }
        } else if self.waitpoint_hmac_kid.is_some() {
            return Err(FabricError::Config(
                "waitpoint_hmac_kid set but waitpoint_hmac_secret is None".into(),
            ));
        }
        Ok(())
    }

    /// Construct a ferriskey [`ClientBuilder`] pre-configured with this
    /// fabric's host/port/TLS/cluster settings. Callers call `.build().await`
    /// to get a connected `Client`.
    ///
    /// This replaces the previous `valkey_url()` URL-string path. The
    /// `redis://` scheme was redundant (we never parse a URL — we build one
    /// only to hand it back to ferriskey, which re-parses it) and would
    /// break on non-Redis-cloud hosts that reject the `redis` scheme
    /// prefix. The builder accepts a bare host + port and applies TLS as
    /// an explicit flag, matching the ferriskey 0.2 public API.
    pub fn valkey_client_builder(&self) -> ferriskey::ClientBuilder {
        let mut builder = ferriskey::ClientBuilder::new().host(&self.valkey_host, self.valkey_port);
        if self.tls {
            builder = builder.tls();
        }
        if self.cluster {
            builder = builder.cluster();
        }
        builder
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
    fn valkey_client_builder_without_tls() {
        // ferriskey's `ClientBuilder` does not expose public accessors on
        // its internal `ConnectionRequest`, so we can only assert that the
        // builder constructs without panicking and that `build_lazy()`
        // (the synchronous validation path) accepts the address list.
        // Full wire assertion requires an integration test against a real
        // Valkey instance; those live under `tests/` and in the downstream
        // `cairn-app` integration suite.
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
            worker_capabilities: BTreeSet::new(),
            waitpoint_hmac_secret: None,
            waitpoint_hmac_kid: None,
        };
        // build_lazy validates the address list synchronously without
        // establishing a TCP connection — any misconfiguration (empty
        // addresses, bad protocol/push_sender combo) surfaces here.
        assert!(config.valkey_client_builder().build_lazy().is_ok());
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
            worker_capabilities: BTreeSet::new(),
            waitpoint_hmac_secret: None,
            waitpoint_hmac_kid: None,
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

    // ── HMAC secret validation ────────────────────────────────────────────

    fn base_config() -> FabricConfig {
        FabricConfig {
            valkey_host: "localhost".into(),
            valkey_port: 6379,
            tls: false,
            cluster: false,
            lane_id: LaneId::new("test"),
            worker_id: WorkerId::new("w"),
            worker_instance_id: WorkerInstanceId::new("i"),
            namespace: Namespace::new("ns"),
            lease_ttl_ms: 30_000,
            grant_ttl_ms: 5_000,
            max_concurrent_tasks: 1,
            signal_dedup_ttl_ms: 86_400_000,
            fcall_timeout_ms: 5_000,
            worker_capabilities: BTreeSet::new(),
            waitpoint_hmac_secret: None,
            waitpoint_hmac_kid: None,
        }
    }

    #[test]
    fn hmac_secret_none_validates() {
        let config = base_config();
        assert!(config.validate().is_ok());
        assert_eq!(config.resolved_waitpoint_hmac_kid(), None);
    }

    #[test]
    fn hmac_secret_valid_64_char_hex_validates() {
        let mut config = base_config();
        config.waitpoint_hmac_secret = Some("a".repeat(64));
        assert!(config.validate().is_ok());
        // No kid specified: defaults to "k1".
        assert_eq!(config.resolved_waitpoint_hmac_kid(), Some("k1"));
    }

    #[test]
    fn hmac_secret_explicit_kid_overrides_default() {
        let mut config = base_config();
        config.waitpoint_hmac_secret = Some("0".repeat(64));
        config.waitpoint_hmac_kid = Some("operator-kid-2026-04".into());
        assert!(config.validate().is_ok());
        assert_eq!(
            config.resolved_waitpoint_hmac_kid(),
            Some("operator-kid-2026-04"),
        );
    }

    #[test]
    fn hmac_secret_too_short_errors() {
        let mut config = base_config();
        config.waitpoint_hmac_secret = Some("a".repeat(63));
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("64 hex chars"),
            "expected length error, got {err}"
        );
    }

    #[test]
    fn hmac_secret_too_long_errors() {
        let mut config = base_config();
        config.waitpoint_hmac_secret = Some("a".repeat(65));
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("64 hex chars"),
            "expected length error, got {err}"
        );
    }

    #[test]
    fn hmac_secret_non_hex_errors() {
        let mut config = base_config();
        // Right length, but 'g' is not a hex digit.
        config.waitpoint_hmac_secret = Some("g".repeat(64));
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("hex-encoded"), "expected hex error, got {err}");
    }

    #[test]
    fn hmac_secret_mixed_case_hex_validates() {
        let mut config = base_config();
        // Operators sometimes paste upper-case hex from /dev/urandom tooling.
        config.waitpoint_hmac_secret = Some("AbCdEf0123456789".repeat(4));
        assert!(config.validate().is_ok());
    }

    #[test]
    fn hmac_kid_empty_with_secret_errors() {
        let mut config = base_config();
        config.waitpoint_hmac_secret = Some("a".repeat(64));
        config.waitpoint_hmac_kid = Some(String::new());
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("waitpoint_hmac_kid must not be empty"),
            "expected empty-kid error, got {err}"
        );
    }

    #[test]
    fn hmac_kid_with_colon_errors() {
        // FF Lua builds `secret:<kid>` / `expires_at:<kid>` as hash-field
        // names; a colon in the kid would split the parser.
        let mut config = base_config();
        config.waitpoint_hmac_secret = Some("a".repeat(64));
        config.waitpoint_hmac_kid = Some("bad:kid".into());
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("':'"), "expected delimiter error, got {err}");
    }

    #[test]
    fn hmac_kid_without_secret_errors() {
        // Operator set a kid but forgot the secret: fail loud instead of
        // silently seeding nothing.
        let mut config = base_config();
        config.waitpoint_hmac_kid = Some("k1".into());
        let err = config.validate().unwrap_err().to_string();
        assert!(
            err.contains("waitpoint_hmac_kid set but waitpoint_hmac_secret is None"),
            "expected missing-secret error, got {err}"
        );
    }

    #[test]
    fn valkey_client_builder_with_tls() {
        // Same limitation as `valkey_client_builder_without_tls`: no
        // public accessors on `ClientBuilder`/`ConnectionRequest`. We
        // assert synchronous validation passes with TLS toggled on.
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
            worker_capabilities: BTreeSet::new(),
            waitpoint_hmac_secret: None,
            waitpoint_hmac_kid: None,
        };
        assert!(config.valkey_client_builder().build_lazy().is_ok());
    }
}
