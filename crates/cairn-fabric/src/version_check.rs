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

/// Run `INFO server` across every node the client is connected to and
/// return the **minimum** major version observed.
///
/// Taking the min (not the first) matters during rolling upgrades: if
/// any node still reports a pre-upgrade major, this returns that lower
/// value so the outer retry loop keeps waiting until the whole cluster
/// is past the floor. Picking the first entry (as FF's `ff-server`
/// does) opens a race where a map iteration order that happens to put
/// an upgraded node first lets us pass boot while FCALLs sent to
/// stragglers still fail.
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
    let bodies = collect_info_bodies(&raw).map_err(VersionCheckError::Parse)?;
    if bodies.is_empty() {
        return Err(VersionCheckError::Parse(
            "INFO returned no bodies to parse".to_owned(),
        ));
    }
    bodies
        .iter()
        .map(|b| parse_valkey_major_version(b))
        .collect::<Result<Vec<_>, _>>()
        .map_err(VersionCheckError::Parse)?
        .into_iter()
        .min()
        .ok_or_else(|| VersionCheckError::Parse("no major versions parsed".to_owned()))
}

/// Flatten an `INFO server` response into one body string per node.
///
/// Shapes handled:
/// - **Standalone** — single bulk / simple / verbatim string.
/// - **Cluster over RESP3** — `Map { addr → body }`; every body is
///   included.
/// - **Cluster over RESP2** — `Array` in one of two shapes ferriskey
///   emits depending on version: either flat `[addr, body, addr, body,
///   ...]` pairs, or a nested `[[addr, body], [addr, body], ...]`. The
///   collector recurses so both shapes yield the per-node bodies.
///
/// Every multi-shape parser in the crate (`parse_multi_stream_xread`,
/// `parse_field_pairs`, `parse_string_map`) handles both RESP2 `Array`
/// and RESP3 `Map` because ferriskey selects the protocol at connection
/// time and RESP2 is the default. This parser follows that contract.
fn collect_info_bodies(raw: &Value) -> Result<Vec<String>, String> {
    let mut bodies = Vec::new();
    collect_info_bodies_into(raw, &mut bodies)?;
    Ok(bodies)
}

fn collect_info_bodies_into(raw: &Value, out: &mut Vec<String>) -> Result<(), String> {
    match raw {
        Value::BulkString(bytes) => {
            out.push(String::from_utf8_lossy(bytes).into_owned());
            Ok(())
        }
        Value::VerbatimString { text, .. } => {
            out.push(text.clone());
            Ok(())
        }
        Value::SimpleString(s) => {
            out.push(s.clone());
            Ok(())
        }
        Value::Map(entries) => {
            if entries.is_empty() {
                return Err("cluster INFO returned empty map".to_owned());
            }
            for (_, body) in entries {
                collect_info_bodies_into(body, out)?;
            }
            Ok(())
        }
        Value::Array(items) => {
            if items.is_empty() {
                return Err("cluster INFO returned empty array".to_owned());
            }
            // Array entries are `Result<Value, Error>` — each element
            // can independently carry an error if a particular node
            // failed. Surface the first such failure as a parse error.
            let unwrapped: Vec<&Value> = items
                .iter()
                .map(|r| r.as_ref().map_err(|e| format!("array entry failed: {e}")))
                .collect::<Result<Vec<_>, _>>()?;

            // RESP2 cluster INFO is either flat `[addr, body, ...]`
            // pairs or nested `[[addr, body], ...]`. Probe for the
            // nested shape first; otherwise walk flat pairs.
            let all_nested_pairs = unwrapped.iter().all(|v| match v {
                Value::Array(inner) => inner.len() == 2,
                _ => false,
            });
            if all_nested_pairs {
                for item in &unwrapped {
                    if let Value::Array(pair) = item {
                        // Second element of each pair is the body.
                        match pair[1].as_ref() {
                            Ok(body) => collect_info_bodies_into(body, out)?,
                            Err(e) => return Err(format!("nested pair body failed: {e}")),
                        }
                    }
                }
            } else if unwrapped.len().is_multiple_of(2) {
                // Flat pairs: odd indices are bodies.
                for body in unwrapped.iter().skip(1).step_by(2) {
                    collect_info_bodies_into(body, out)?;
                }
            } else {
                return Err(format!(
                    "cluster INFO array has {} entries — expected pairs or nested 2-arrays",
                    unwrapped.len()
                ));
            }
            Ok(())
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

    // Helper: bulk-string body with a given version line.
    fn body(v: &str) -> Value {
        Value::BulkString(format!("valkey_version:{v}\n").as_bytes().to_vec().into())
    }

    // `Value::Array` holds `Vec<Result<Value, Error>>` — tests only
    // need the `Ok` variant, so thread the wrapping through a helper
    // to keep fixtures readable.
    fn arr(values: Vec<Value>) -> Value {
        Value::Array(values.into_iter().map(Ok).collect())
    }

    fn min_major(raw: &Value) -> Result<u32, String> {
        let bodies = collect_info_bodies(raw)?;
        bodies
            .iter()
            .map(|b| parse_valkey_major_version(b))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min()
            .ok_or_else(|| "empty".to_owned())
    }

    #[test]
    fn collect_bulk_string_yields_one_body() {
        let raw = body("8.0.0");
        let bodies = collect_info_bodies(&raw).unwrap();
        assert_eq!(bodies.len(), 1);
        assert!(bodies[0].contains("valkey_version:8.0.0"));
    }

    #[test]
    fn collect_simple_string_yields_one_body() {
        let raw = Value::SimpleString("valkey_version:8.1.0".to_owned());
        let bodies = collect_info_bodies(&raw).unwrap();
        assert_eq!(bodies.len(), 1);
    }

    #[test]
    fn cluster_map_collects_every_node_and_min_wins() {
        // Rolling-upgrade regression: one node is still on 7.x, another
        // is 8.0. Picking the first entry (FF's ff-server behavior)
        // would sometimes pass boot against a partially-upgraded
        // cluster; taking the min forces the outer retry loop to keep
        // waiting until every node is past the floor.
        let raw = Value::Map(vec![
            (Value::SimpleString("node-a".into()), body("8.0.0")),
            (Value::SimpleString("node-b".into()), body("7.2.4")),
        ]);
        assert_eq!(min_major(&raw).unwrap(), 7);
    }

    #[test]
    fn cluster_map_all_upgraded_passes_at_min() {
        let raw = Value::Map(vec![
            (Value::SimpleString("node-a".into()), body("8.1.0")),
            (Value::SimpleString("node-b".into()), body("8.0.2")),
        ]);
        assert_eq!(min_major(&raw).unwrap(), 8);
    }

    #[test]
    fn empty_cluster_map_errors() {
        let raw = Value::Map(vec![]);
        assert!(collect_info_bodies(&raw).is_err());
    }

    #[test]
    fn cluster_array_nested_pairs_resp2() {
        // RESP2 cluster INFO can arrive as [[addr, body], [addr, body]].
        // Every other multi-shape parser in the crate handles this
        // (parse_multi_stream_xread, parse_field_pairs, parse_string_map);
        // the version check needs to, too, or a RESP2 cluster connection
        // hangs for 60 s and then boots with a misleading "unexpected
        // INFO shape" error.
        let raw = arr(vec![
            arr(vec![Value::SimpleString("node-a".into()), body("8.0.0")]),
            arr(vec![Value::SimpleString("node-b".into()), body("7.4.0")]),
        ]);
        assert_eq!(min_major(&raw).unwrap(), 7);
    }

    #[test]
    fn cluster_array_flat_pairs_resp2() {
        // RESP2 flat shape: [addr, body, addr, body, ...].
        let raw = arr(vec![
            Value::SimpleString("node-a".into()),
            body("8.0.0"),
            Value::SimpleString("node-b".into()),
            body("8.0.1"),
        ]);
        assert_eq!(min_major(&raw).unwrap(), 8);
    }

    #[test]
    fn empty_cluster_array_errors() {
        let raw = arr(vec![]);
        assert!(collect_info_bodies(&raw).is_err());
    }

    #[test]
    fn cluster_array_odd_length_errors() {
        // Malformed — neither nested-pairs nor flat-pairs.
        let raw = arr(vec![
            Value::SimpleString("node-a".into()),
            body("8.0.0"),
            body("8.0.0"),
        ]);
        assert!(collect_info_bodies(&raw).is_err());
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
