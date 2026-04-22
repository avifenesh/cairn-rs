//! One-shot backfill for `cairn.instance_id` tags on pre-existing
//! executions.
//!
//! # Why this exists
//!
//! [`LeaseHistorySubscriber`](crate::lease_history_subscriber::LeaseHistorySubscriber)
//! filters every lease-history frame by the
//! `cairn.instance_id` tag. Executions created before the filter
//! landed don't carry the tag, so after a binary swap their lease
//! expiries would be silently dropped as "foreign". Operators doing
//! an in-place upgrade (same `CAIRN_FABRIC_INSTANCE_ID`, pre-existing
//! `Running` / `WaitingApproval` executions) need a one-shot pass
//! that stamps the tag onto outstanding executions.
//!
//! # Shape
//!
//! - Gated on the `CAIRN_BACKFILL_INSTANCE_TAG=1` env var. Default
//!   off — a fresh deploy has nothing to backfill.
//! - `SCAN` over `ff:exec:*:tags` hashes, one MATCH pattern per call,
//!   `COUNT` hint of 200 per iteration.
//! - For every hash that has `cairn.project` but lacks
//!   `cairn.instance_id`: `HSET cairn.instance_id <own_instance_id>`.
//! - Idempotent — running twice is a no-op (the second pass skips
//!   hashes that now have the tag). Non-transactional — a crash
//!   mid-scan is safe because HSET is idempotent and the next boot
//!   picks up where this one left off.
//! - Portability — uses standard SCAN + HGET + HSET. Works on Valkey,
//!   Redis OSS, and any RFC-012 future Postgres EngineBackend that
//!   exposes an equivalent tag read/write primitive (tags are
//!   first-class in both storage shapes per the design doc §4).

use ferriskey::{Client, Value};

/// Result of a backfill pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct BackfillOutcome {
    /// Number of exec-tag hashes inspected.
    pub scanned: u64,
    /// Number of hashes on which the tag was newly stamped.
    pub tagged: u64,
    /// Number of hashes skipped because they already carried a tag
    /// (either this instance's id or a different one — we never
    /// overwrite another instance's claim).
    pub skipped_tagged: u64,
    /// Number of hashes skipped because they lacked `cairn.project`
    /// (foreign executions from a non-cairn FF consumer).
    pub skipped_foreign: u64,
}

/// Non-blocking SCAN cursor iteration. Returns the new cursor and the
/// matched keys from this step. Iterate until the cursor returns to
/// `"0"`.
async fn scan_step(
    client: &Client,
    cursor: &str,
    pattern: &str,
    count: u64,
) -> Result<(String, Vec<String>), String> {
    let raw: Value = client
        .cmd("SCAN")
        .arg(cursor)
        .arg("MATCH")
        .arg(pattern)
        .arg("COUNT")
        .arg(count.to_string().as_str())
        .execute()
        .await
        .map_err(|e| format!("SCAN {cursor}: {e}"))?;

    let arr = match raw {
        Value::Array(a) => a,
        other => return Err(format!("SCAN: unexpected reply shape {other:?}")),
    };
    if arr.len() < 2 {
        return Err(format!("SCAN: expected 2-element reply, got {}", arr.len()));
    }
    let next_cursor = match arr[0].as_ref() {
        Ok(Value::BulkString(b)) => String::from_utf8_lossy(b.as_ref()).into_owned(),
        Ok(Value::SimpleString(s)) => s.clone(),
        other => return Err(format!("SCAN cursor: unexpected shape {other:?}")),
    };
    let keys: Vec<String> = match arr[1].as_ref() {
        Ok(Value::Array(items)) => items
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .filter_map(|v| match v {
                Value::BulkString(b) => Some(String::from_utf8_lossy(b.as_ref()).into_owned()),
                Value::SimpleString(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Ok(other) => return Err(format!("SCAN keys: unexpected shape {other:?}")),
        Err(e) => return Err(format!("SCAN keys: {e}")),
    };
    Ok((next_cursor, keys))
}

/// Run the one-shot backfill. Returns the aggregate outcome. Callers
/// should log the outcome once at the end — not per-key, to avoid
/// swamping operator logs on a large keyspace.
pub async fn backfill_instance_tag(
    client: &Client,
    own_instance_id: &str,
) -> Result<BackfillOutcome, String> {
    const SCAN_COUNT: u64 = 200;
    const PATTERN: &str = "ff:exec:*:tags";

    let mut outcome = BackfillOutcome::default();
    let mut cursor = "0".to_owned();
    loop {
        let (next, keys) = scan_step(client, &cursor, PATTERN, SCAN_COUNT).await?;
        for key in keys {
            outcome.scanned += 1;
            // One HMGET reads both fields in a single round-trip
            // instead of two sequential HGETs — saves ~N network
            // trips on a scan of N execs. We only need these two
            // fields (not a full HGETALL) because the tag hash
            // routinely holds many other cairn.* tags that the
            // backfill does not consult.
            let raw: Value = client
                .cmd("HMGET")
                .arg(key.as_str())
                .arg("cairn.project")
                .arg("cairn.instance_id")
                .execute()
                .await
                .map_err(|e| format!("HMGET {key}: {e}"))?;
            let arr = match raw {
                Value::Array(a) => a,
                other => return Err(format!("HMGET {key}: unexpected reply shape {other:?}")),
            };
            if arr.len() != 2 {
                return Err(format!(
                    "HMGET {key}: expected 2-element reply, got {}",
                    arr.len()
                ));
            }
            let has_project = matches!(
                arr[0].as_ref(),
                Ok(Value::BulkString(_)) | Ok(Value::SimpleString(_))
            );
            let already_tagged = matches!(
                arr[1].as_ref(),
                Ok(Value::BulkString(_)) | Ok(Value::SimpleString(_))
            );
            if !has_project {
                outcome.skipped_foreign += 1;
                continue;
            }
            if already_tagged {
                outcome.skipped_tagged += 1;
                continue;
            }
            let _: Value = client
                .cmd("HSET")
                .arg(key.as_str())
                .arg("cairn.instance_id")
                .arg(own_instance_id)
                .execute()
                .await
                .map_err(|e| format!("HSET cairn.instance_id {key}: {e}"))?;
            outcome.tagged += 1;
        }
        if next == "0" {
            break;
        }
        cursor = next;
    }
    Ok(outcome)
}
