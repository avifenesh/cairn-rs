//! Valkey version check — refuses to boot against Valkey < 7.0 and
//! warns on 7.x.
//!
//! Functions (FCALL / FUNCTION LOAD / FCALL_RO) landed in Redis 7.0 and
//! were inherited by Valkey 7.x. The hard gate sits at 7.0 because
//! pre-7.0 Valkey physically lacks the Functions API cairn-fabric and
//! FF depend on.
//!
//! A soft WARN is emitted when the connected Valkey reports a major
//! below 8.0. The 8.x line patched a series of Lua-sandbox CVEs
//! (CVE-2024-46981, CVE-2024-31449, CVE-2025-49844, CVE-2025-46817,
//! CVE-2025-46818, CVE-2025-46819) and is the version FlowFabric's own
//! CI matrix validates against. Operators shipping to production should
//! run 8.0.x+; 7.x is functionally supported but unvalidated.
//!
//! # Rolling-upgrade tolerance
//!
//! During a rolling Valkey upgrade, the node we connect to may
//! temporarily be pre-upgrade. The check retries the whole verification
//! (including low-version responses) with exponential backoff, capped
//! at a 60 s budget.
//!
//! Retries on:
//! - Low-version responses (`detected_major < REQUIRED_VALKEY_MAJOR`) —
//!   may resolve as the rolling upgrade progresses onto the connected
//!   node.
//! - Retryable ferriskey transport errors — connection refused,
//!   `BusyLoadingError`, `ClusterDown`, etc., classified via
//!   `ff_script::retry::is_retryable_kind`.
//! - Missing/unparsable version field — treated as transient (fresh-boot
//!   server may not have the INFO fields populated yet).
//!
//! Does NOT retry on:
//! - Non-retryable transport errors (auth failures, permission denied,
//!   invalid client config) — these are operator misconfiguration, not
//!   transient cluster state; fast-fail preserves a clear signal.

use std::time::Duration;

use ferriskey::{Client, Value};

use crate::error::FabricError;

/// Minimum supported Valkey major version — below this, the Functions
/// API (FCALL / FUNCTION LOAD) does not exist and cairn-fabric cannot
/// function.
pub const REQUIRED_VALKEY_MAJOR: u32 = 7;

/// Recommended Valkey major version — the 8.x line patches a series of
/// Lua sandbox CVEs and is FlowFabric's tested floor. Running below
/// this emits a WARN but does not refuse to boot.
pub const RECOMMENDED_VALKEY_MAJOR: u32 = 8;

/// Total budget for retrying transient / rolling-upgrade version check
/// failures before giving up and exiting boot.
const VERSION_CHECK_RETRY_BUDGET: Duration = Duration::from_secs(60);

/// Initial backoff between retry attempts. Doubles on each attempt up to
/// [`VERSION_CHECK_BACKOFF_MAX`].
const VERSION_CHECK_BACKOFF_INITIAL: Duration = Duration::from_millis(200);

/// Per-attempt backoff cap. Keeps the retry loop responsive inside the
/// 60 s budget rather than ballooning to minute-long sleeps on long
/// rolling upgrades.
const VERSION_CHECK_BACKOFF_MAX: Duration = Duration::from_secs(5);

/// Verify the connected Valkey reports a major version
/// `>= REQUIRED_VALKEY_MAJOR`. Emits a boot-time WARN when the detected
/// major is below [`RECOMMENDED_VALKEY_MAJOR`].
///
/// On success, logs `"valkey version accepted"` at INFO and returns
/// `Ok(())`. On budget exhaustion, returns the last observed error —
/// either [`FabricError::ValkeyVersionTooLow`] or [`FabricError::Valkey`].
pub async fn verify_valkey_version(client: &Client) -> Result<(), FabricError> {
    let deadline = tokio::time::Instant::now() + VERSION_CHECK_RETRY_BUDGET;
    let mut backoff = VERSION_CHECK_BACKOFF_INITIAL;
    loop {
        let (should_retry, err_for_budget_exhaust, log_detail): (bool, FabricError, String) =
            match query_valkey_version(client).await {
                Ok(detected_major) if detected_major >= REQUIRED_VALKEY_MAJOR => {
                    if detected_major < RECOMMENDED_VALKEY_MAJOR {
                        tracing::warn!(
                            detected_major,
                            recommended = RECOMMENDED_VALKEY_MAJOR,
                            "valkey {detected_major}.x is below the recommended {RECOMMENDED_VALKEY_MAJOR}.x floor — \
                             Functions work, but the 8.x line patches several Lua sandbox CVEs \
                             (CVE-2024-46981, CVE-2024-31449, CVE-2025-49844, CVE-2025-46817, \
                             CVE-2025-46818, CVE-2025-46819) and is what FlowFabric's CI validates against; \
                             plan an upgrade"
                        );
                    } else {
                        tracing::info!(
                            detected_major,
                            required = REQUIRED_VALKEY_MAJOR,
                            "valkey version accepted"
                        );
                    }
                    return Ok(());
                }
                Ok(detected_major) => (
                    // Below 7.0 — may be a rolling-upgrade stale node.
                    // Retry within budget; after exhaustion, the cluster
                    // is misconfigured and fast-fail is the correct signal.
                    true,
                    FabricError::ValkeyVersionTooLow {
                        detected: detected_major.to_string(),
                        required: format!("{REQUIRED_VALKEY_MAJOR}.0"),
                    },
                    format!("detected_major={detected_major} < required={REQUIRED_VALKEY_MAJOR}"),
                ),
                Err(VersionCheckError::Transport { retryable, error }) => (
                    // Only retry if the underlying Valkey error is
                    // classified retryable. Auth / permission / invalid-
                    // config should fast-fail so operators see the true
                    // root cause immediately, not a 60s hang.
                    retryable,
                    FabricError::Valkey(format!("version check: {error}")),
                    error,
                ),
                Err(VersionCheckError::Parse(detail)) => (
                    // Missing / unparsable version field — treat as
                    // transient. A fresh-boot Valkey may not have the
                    // INFO fields populated yet.
                    true,
                    FabricError::Valkey(format!("version check: {detail}")),
                    detail,
                ),
            };

        if !should_retry {
            return Err(err_for_budget_exhaust);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(err_for_budget_exhaust);
        }
        tracing::warn!(
            backoff_ms = backoff.as_millis() as u64,
            detail = %log_detail,
            "valkey version check transient failure; retrying"
        );
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(VERSION_CHECK_BACKOFF_MAX);
    }
}

/// Internal result type — separates retryable classification from the
/// boot-error surfaced to the operator.
enum VersionCheckError {
    Transport { retryable: bool, error: String },
    Parse(String),
}

/// Run `INFO server` and extract the major component of the Valkey version.
async fn query_valkey_version(client: &Client) -> Result<u32, VersionCheckError> {
    let raw: Value = client
        .cmd("INFO")
        .arg("server")
        .execute()
        .await
        .map_err(|e| VersionCheckError::Transport {
            retryable: ff_script::retry::is_retryable_kind(e.kind()),
            error: format!("INFO server: {e}"),
        })?;
    let info_body = extract_info_body(&raw).map_err(VersionCheckError::Parse)?;
    parse_valkey_major_version(&info_body).map_err(VersionCheckError::Parse)
}

/// Normalize an `INFO server` response to a single string body.
///
/// Standalone returns the body as a bulk / simple / verbatim string.
/// Cluster returns a map keyed by node address whose values are bodies;
/// pick the first entry since every healthy node reports the same
/// version. Divergent versions during a rolling upgrade are reconciled
/// by the outer retry loop.
fn extract_info_body(raw: &Value) -> Result<String, String> {
    match raw {
        Value::BulkString(bytes) => Ok(String::from_utf8_lossy(bytes).into_owned()),
        Value::VerbatimString { text, .. } => Ok(text.clone()),
        Value::SimpleString(s) => Ok(s.clone()),
        Value::Map(entries) => {
            let (_, body) = entries
                .first()
                .ok_or_else(|| "cluster INFO returned empty map".to_owned())?;
            extract_info_body(body)
        }
        other => Err(format!("unexpected INFO shape: {other:?}")),
    }
}

/// Extract the major component of the Valkey version from an `INFO server`
/// response body.
///
/// Prefers the `valkey_version:` field introduced in Valkey 8.0+.
/// Falls back to `redis_version:` for Valkey 7.x compat.
///
/// **Note:** Valkey 8.x/9.x still emits `redis_version:7.2.4` for
/// Redis-client compatibility; the real server version is in
/// `valkey_version:`, so always check that field first.
fn parse_valkey_major_version(info: &str) -> Result<u32, String> {
    let extract_major = |line: &str| -> Result<u32, String> {
        let trimmed = line.trim();
        let major_str = trimmed.split('.').next().unwrap_or("").trim();
        if major_str.is_empty() {
            return Err(format!("empty version field in '{trimmed}'"));
        }
        major_str
            .parse::<u32>()
            .map_err(|_| format!("non-numeric major in '{trimmed}'"))
    };
    if let Some(valkey_line) = info
        .lines()
        .find_map(|line| line.strip_prefix("valkey_version:"))
    {
        return extract_major(valkey_line);
    }
    if let Some(redis_line) = info
        .lines()
        .find_map(|line| line.strip_prefix("redis_version:"))
    {
        return extract_major(redis_line);
    }
    Err("INFO missing valkey_version and redis_version".to_owned())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valkey_version_line() {
        let info = "# Server\nvalkey_version:8.0.1\nredis_version:7.2.4\n";
        assert_eq!(parse_valkey_major_version(info).unwrap(), 8);
    }

    #[test]
    fn valkey_line_wins_over_redis_line() {
        // Valkey 8.x keeps redis_version pinned to 7.2.4; reading the
        // redis line would under-report the real server version.
        let info = "redis_version:7.2.4\nvalkey_version:9.0.0\n";
        assert_eq!(parse_valkey_major_version(info).unwrap(), 9);
    }

    #[test]
    fn falls_back_to_redis_version_on_valkey_7() {
        // Valkey 7.x does not emit valkey_version; the check falls back
        // to redis_version so pre-8.0 nodes report their real major.
        let info = "# Server\nredis_version:7.2.4\n";
        assert_eq!(parse_valkey_major_version(info).unwrap(), 7);
    }

    #[test]
    fn missing_both_fields_is_parse_error() {
        let info = "# Server\nos:Linux\n";
        assert!(parse_valkey_major_version(info).is_err());
    }

    #[test]
    fn empty_version_is_parse_error() {
        let info = "valkey_version:\n";
        assert!(parse_valkey_major_version(info).is_err());
    }

    #[test]
    fn non_numeric_major_is_parse_error() {
        let info = "valkey_version:abc.0.0\n";
        assert!(parse_valkey_major_version(info).is_err());
    }

    #[test]
    fn extract_info_body_bulk_string() {
        let raw = Value::BulkString(b"valkey_version:8.0.0\n".as_slice().into());
        assert_eq!(extract_info_body(&raw).unwrap(), "valkey_version:8.0.0\n");
    }

    #[test]
    fn extract_info_body_simple_string() {
        let raw = Value::SimpleString("valkey_version:8.1.0".to_owned());
        assert_eq!(extract_info_body(&raw).unwrap(), "valkey_version:8.1.0");
    }

    #[test]
    fn extract_info_body_cluster_map_picks_first() {
        let raw = Value::Map(vec![
            (
                Value::SimpleString("node-a".into()),
                Value::BulkString(b"valkey_version:8.0.0\n".as_slice().into()),
            ),
            (
                Value::SimpleString("node-b".into()),
                Value::BulkString(b"valkey_version:8.0.1\n".as_slice().into()),
            ),
        ]);
        let body = extract_info_body(&raw).unwrap();
        assert!(body.contains("valkey_version:8.0.0"));
    }

    #[test]
    fn extract_info_body_empty_cluster_map_errors() {
        let raw = Value::Map(vec![]);
        assert!(extract_info_body(&raw).is_err());
    }

    // Compile-time invariant: recommended floor must sit above the hard
    // gate, else the WARN branch is unreachable. Bumping either
    // constant is a deliberate choice; a const assert forces a
    // conscious edit when they drift.
    const _: () = assert!(
        RECOMMENDED_VALKEY_MAJOR > REQUIRED_VALKEY_MAJOR,
        "RECOMMENDED_VALKEY_MAJOR must sit above REQUIRED_VALKEY_MAJOR"
    );

    #[test]
    fn version_gate_constants_match_spec() {
        assert_eq!(REQUIRED_VALKEY_MAJOR, 7);
        assert_eq!(RECOMMENDED_VALKEY_MAJOR, 8);
    }
}
