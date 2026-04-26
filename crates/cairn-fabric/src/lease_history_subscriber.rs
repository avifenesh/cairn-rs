//! Tails FF's per-execution `lease_history` streams and emits
//! `BridgeEvent`s for state transitions that never flow through a cairn
//! service call.
//!
//! TODO(ff-upstream: <https://github.com/avifenesh/FlowFabric/issues/324>):
//! This module still uses raw ferriskey XREAD against per-partition
//! lease-history streams. FF 0.9 exposed Stage A `subscribe_lease_history`
//! (FF#282), but the returned `StreamEvent.payload: Bytes` is explicitly
//! unstabilised — consumers would bind to undocumented XADD field shapes.
//! FF#324 requests Stage B helpers (typed `decode_lease_history` +
//! optional `ScannerFilter`-style tag filter arg). CG-c migrates this
//! module onto those helpers and drops the ferriskey direct dep.
//!
//! Cairn's normal bridge is call-then-emit: every `BridgeEvent` is
//! emitted by a cairn wrapper that just called an FCALL. That covers
//! every cairn-initiated transition but misses FF-initiated ones —
//! specifically, the lease-expiry scanner moving a task to
//! `lease_expired_reclaimable` when its worker dies mid-execution. The
//! cairn projection stays stuck at `Running` forever without a
//! subscriber that watches FF state directly.
//!
//! ## Shape
//!
//! - **Single tokio task.** Walks all `num_flow_partitions`
//!   partitions sequentially on each cycle. One task (not per-partition)
//!   because the ferriskey client uses a multiplexed connection —
//!   256 parallel XREAD polls would thrash command pipelining.
//!   Sequential polling at 1s cadence gives us O(num_partitions)
//!   Valkey ops per second, which is fine: each op is a single
//!   ZRANGEBYSCORE + optionally a small XREAD.
//! - **Discovery via `lease_expiry` ZSET.** Every leased execution
//!   appears in `ff:idx:{fp:N}:lease_expiry`. Reading this ZSET gives
//!   us the exact set of streams that can currently emit an `expired`
//!   event; non-leased executions are excluded from the XREAD fan-out.
//! - **Persistent cursor per stream.** After consuming a frame we
//!   upsert `(partition_id, execution_id) → last_stream_id` in
//!   cairn-store so a restart resumes from the right place. On boot,
//!   cursors are loaded from the store; newly-discovered streams
//!   start at `0-0` (full replay). Pre-subscriber `acquired` frames
//!   are safe to re-observe because our emission path only reacts
//!   to `expired` / `reclaimed`, and the no-matching-task projection
//!   guard (cairn-store row absent → event is a no-op) absorbs any
//!   frames from executions belonging to a different tenant on a
//!   shared Valkey.
//! - **Cluster-safe.** Every XREAD stays within one partition's hash
//!   slot via the `{fp:N}` tag.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::Duration;

use cairn_domain::{FailureClass, ProjectKey, RunId, TaskId, TaskState};
use cairn_store::projections::{FfLeaseHistoryCursor, FfLeaseHistoryCursorStore};
use ferriskey::{Client, Value};
use flowfabric::core::keys::{ExecKeyContext, IndexKeys};
use flowfabric::core::partition::{Partition, PartitionFamily};
use flowfabric::core::types::ExecutionId;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::try_parse_project_key;

/// How often each per-partition task refreshes the `lease_expiry`
/// membership set. Trades off discovery latency for Valkey command
/// rate on idle partitions.
const DISCOVERY_POLL_MS: u64 = 1_000;

/// Upper bound on frames pulled in one XREAD. `expired` / `reclaimed`
/// arrive serially per execution; most bursts come from scanner
/// sweeps touching many executions at once.
///
/// XREAD runs in **non-blocking mode** (no `BLOCK` argument): the
/// subscriber pulls any pending frames, then sleeps
/// `DISCOVERY_POLL_MS` and loops. Blocking XREAD would monopolise
/// the shared multiplexed ferriskey connection, starving every
/// other cairn-fabric call on that connection.
const XREAD_COUNT: u64 = 512;

/// Runtime handle for the single subscriber task.
pub struct LeaseHistorySubscriber {
    handle: JoinHandle<()>,
    cancel: CancellationToken,
}

impl LeaseHistorySubscriber {
    /// Spawn the subscriber task. Sequentially walks all
    /// `num_flow_partitions` on each cycle; each partition's poll is
    /// cheap (one ZRANGEBYSCORE + optionally one XREAD).
    pub fn start(
        client: Client,
        num_flow_partitions: u16,
        bridge: Arc<EventBridge>,
        cursor_store: Arc<dyn FfLeaseHistoryCursorStore>,
        own_instance_id: String,
    ) -> Self {
        let cancel = CancellationToken::new();
        let worker = Worker {
            client,
            num_flow_partitions,
            bridge,
            cursor_store,
            cancel: cancel.clone(),
            cursors: HashMap::new(),
            own_instance_id,
        };
        let handle = tokio::spawn(worker.run());
        tracing::info!(
            partitions = num_flow_partitions,
            "lease-history subscriber started"
        );
        Self { handle, cancel }
    }

    /// Signal the worker to stop and await termination.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        if let Err(e) = self.handle.await {
            tracing::warn!(error = %e, "lease-history subscriber worker panicked");
        }
    }
}

struct Worker {
    client: Client,
    num_flow_partitions: u16,
    bridge: Arc<EventBridge>,
    cursor_store: Arc<dyn FfLeaseHistoryCursorStore>,
    cancel: CancellationToken,
    /// In-memory mirror of the persisted cursor table, keyed by
    /// `(partition_tag, execution_id)`.
    cursors: HashMap<(String, String), String>,
    /// This cairn-app instance's id (from `FabricConfig::worker_instance_id`,
    /// threaded through `LeaseHistorySubscriber::start`). Used in
    /// `fetch_entity_context` to reject frames whose exec was created by
    /// another cairn-app sharing the same Valkey. Without this filter,
    /// a two-instance-one-Valkey deploy sees foreign runs' state changes
    /// in its own `/v1/events` log (RFC 020 test #1 flake + production
    /// cross-tenant leak).
    own_instance_id: String,
}

impl Worker {
    async fn run(mut self) {
        // Restart recovery: prime the in-memory cursor map from the
        // persisted table for every partition so we don't replay
        // frames we've already consumed before a restart.
        for index in 0..self.num_flow_partitions {
            let partition = Partition {
                family: PartitionFamily::Execution,
                index,
            };
            let tag = partition.hash_tag();
            match self.cursor_store.list_by_partition(&tag).await {
                Ok(rows) => {
                    for row in rows {
                        self.cursors
                            .insert((tag.clone(), row.execution_id), row.last_stream_id);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        partition = %tag,
                        error = %e,
                        "lease-history subscriber failed to restore cursors"
                    );
                }
            }
        }
        if !self.cursors.is_empty() {
            tracing::debug!(
                restored = self.cursors.len(),
                "lease-history subscriber restored cursors"
            );
        }

        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            for index in 0..self.num_flow_partitions {
                if self.cancel.is_cancelled() {
                    break;
                }
                let partition = Partition {
                    family: PartitionFamily::Execution,
                    index,
                };
                let partition_tag = partition.hash_tag();
                if let Err(e) = self.poll_partition(&partition, &partition_tag).await {
                    tracing::warn!(
                        partition = %partition_tag,
                        error = %e,
                        "lease-history subscriber: partition poll failed"
                    );
                }
            }
            self.sleep_or_cancel(Duration::from_millis(DISCOVERY_POLL_MS))
                .await;
        }
        tracing::info!("lease-history subscriber stopped");
    }

    async fn poll_partition(
        &mut self,
        partition: &Partition,
        partition_tag: &str,
    ) -> Result<(), String> {
        let lease_expiry_key = IndexKeys::new(partition).lease_expiry();
        let active = self.discover_active(&lease_expiry_key).await?;

        // Prune cursors for executions no longer tracked in
        // lease_expiry — their lease is gone (complete / fail /
        // reclaim consumed it), nothing more to tail.
        self.prune_gone(partition_tag, &active).await;

        if active.is_empty() {
            return Ok(());
        }
        tracing::debug!(
            partition = %partition_tag,
            count = active.len(),
            "lease-history subscriber: discovered active streams"
        );

        self.tail_once(partition, partition_tag, &active).await
    }

    async fn sleep_or_cancel(&self, dur: Duration) {
        tokio::select! {
            _ = tokio::time::sleep(dur) => {}
            _ = self.cancel.cancelled() => {}
        }
    }

    /// `ZRANGEBYSCORE lease_expiry:{fp:N} -inf +inf` returns the set
    /// of execution ids with currently tracked leases. Members are
    /// the raw `ExecutionId` strings (already includes the `{fp:N}:`
    /// hash-tag prefix; we keep them as-is for stream-key
    /// composition).
    async fn discover_active(&self, key: &str) -> Result<Vec<String>, String> {
        let raw: Value = self
            .client
            .cmd("ZRANGEBYSCORE")
            .arg(key)
            .arg("-inf")
            .arg("+inf")
            .execute()
            .await
            .map_err(|e| format!("ZRANGEBYSCORE {key}: {e}"))?;

        let arr = match raw {
            Value::Array(a) => a,
            Value::Nil => return Ok(Vec::new()),
            other => return Err(format!("ZRANGEBYSCORE: unexpected reply shape {other:?}")),
        };

        let mut ids = Vec::with_capacity(arr.len());
        for item in arr {
            let item = item.map_err(|e| format!("ZRANGEBYSCORE element: {e}"))?;
            match item {
                Value::BulkString(b) => {
                    ids.push(String::from_utf8_lossy(b.as_ref()).into_owned());
                }
                Value::SimpleString(s) => ids.push(s.clone()),
                other => {
                    tracing::trace!(?other, "ZRANGEBYSCORE: skipping non-string element");
                }
            }
        }
        Ok(ids)
    }

    async fn prune_gone(&mut self, partition_tag: &str, active: &[String]) {
        let active_set: std::collections::HashSet<&str> =
            active.iter().map(|s| s.as_str()).collect();
        let gone: Vec<String> = self
            .cursors
            .iter()
            .filter_map(|((pt, eid), _)| {
                if pt == partition_tag && !active_set.contains(eid.as_str()) {
                    Some(eid.clone())
                } else {
                    None
                }
            })
            .collect();
        for exec_id in gone {
            self.cursors
                .remove(&(partition_tag.to_owned(), exec_id.clone()));
            if let Err(e) = self.cursor_store.delete(partition_tag, &exec_id).await {
                tracing::warn!(
                    partition = %partition_tag,
                    exec_id,
                    error = %e,
                    "lease-history subscriber: failed to delete stale cursor"
                );
            }
        }
    }

    /// Compose the XREAD STREAMS ... IDs ... payload for the active
    /// set, drive it, and dispatch each frame.
    async fn tail_once(
        &mut self,
        partition: &Partition,
        partition_tag: &str,
        active: &[String],
    ) -> Result<(), String> {
        // Build a stable (stream_key → exec_id) index so parse
        // results route back to the right bridge-emission path. We
        // also need sorted ordering for the "STREAMS key1 key2 ...
        // id1 id2 ..." argument pattern XREAD requires.
        let mut pairs: BTreeMap<String, (String, String)> = BTreeMap::new();
        for exec_id in active {
            // Compose the key via ExecKeyContext so the shape stays
            // in sync with FF's canonical key builders (which we
            // can't match by string concatenation: the `<eid>`
            // itself already carries the `{fp:N}:` hash tag, so the
            // full key has the tag twice — and the lease_history
            // suffix is `:lease:history`, not `:lease_history`).
            let Ok(eid) = ExecutionId::parse(exec_id) else {
                tracing::trace!(exec_id, "skipping unparseable execution id");
                continue;
            };
            let stream_key = ExecKeyContext::new(partition, &eid).lease_history();
            // First-sighting cursor: `0-0` replays the full stream.
            // That's safe because `acquired` events (the only kind
            // present pre-expiry) are no-ops for our emission path
            // (we only emit for `expired` / `reclaimed`), and we
            // MUST pick up any `expired` entries that landed before
            // our first XREAD. Setting cursor to the current stream
            // head (via XREVRANGE) would silently miss those — the
            // `expired` entry is usually the latest, and `$` would
            // also miss it since resolution happens at call time.
            let cursor = self
                .cursors
                .get(&(partition_tag.to_owned(), exec_id.clone()))
                .cloned()
                .unwrap_or_else(|| "0-0".to_owned());
            pairs.insert(stream_key, (exec_id.clone(), cursor));
        }

        let keys: Vec<&str> = pairs.keys().map(String::as_str).collect();
        let ids: Vec<&str> = pairs.values().map(|(_, c)| c.as_str()).collect();

        let count_str = XREAD_COUNT.to_string();
        let mut cmd = self
            .client
            .cmd("XREAD")
            .arg("COUNT")
            .arg(count_str.as_str())
            .arg("STREAMS");
        for k in &keys {
            cmd = cmd.arg(*k);
        }
        for i in &ids {
            cmd = cmd.arg(*i);
        }

        let raw: Value = cmd
            .execute()
            .await
            .map_err(|e| format!("XREAD multi-stream: {e}"))?;

        let by_stream = parse_multi_stream_xread(&raw)?;
        for (stream_key, frames) in by_stream {
            let Some((exec_id, _)) = pairs.get(&stream_key) else {
                continue;
            };
            for frame in frames {
                self.handle_frame(partition, partition_tag, exec_id, &frame)
                    .await;
            }
        }
        Ok(())
    }

    async fn handle_frame(
        &mut self,
        partition: &Partition,
        partition_tag: &str,
        exec_id: &str,
        frame: &StreamFrame,
    ) {
        let Some(event_kind) = frame.fields.get("event") else {
            tracing::trace!(exec_id, "lease_history frame missing `event` field");
            return;
        };

        tracing::debug!(
            partition = %partition_tag,
            exec_id,
            event = %event_kind,
            stream_id = %frame.stream_id,
            "lease-history subscriber: received frame"
        );

        // Advance the cursor only when dispatch succeeded OR the
        // frame was permanently skipped (unknown event kind / tags
        // don't identify a cairn execution). A transient Valkey
        // failure during dispatch returns Err — we keep the cursor
        // pinned so the next poll retries, avoiding silent data
        // loss.
        let dispatch = match event_kind.as_str() {
            "expired" => self.dispatch_expired(partition, exec_id, frame).await,
            "reclaimed" => self.dispatch_reclaimed(partition, exec_id, frame).await,
            other => {
                tracing::trace!(exec_id, event = %other, "lease_history: unknown event kind");
                Ok(())
            }
        };
        if let Err(e) = dispatch {
            tracing::warn!(
                partition = %partition_tag,
                exec_id,
                error = %e,
                "lease-history subscriber: dispatch failed, retrying on next poll"
            );
            return;
        }

        self.cursors.insert(
            (partition_tag.to_owned(), exec_id.to_owned()),
            frame.stream_id.clone(),
        );
        let cursor = FfLeaseHistoryCursor {
            partition_id: partition_tag.to_owned(),
            execution_id: exec_id.to_owned(),
            last_stream_id: frame.stream_id.clone(),
            updated_at_ms: now_ms(),
        };
        if let Err(e) = self.cursor_store.upsert(&cursor).await {
            tracing::warn!(
                partition = %partition_tag,
                exec_id,
                error = %e,
                "lease-history subscriber: cursor upsert failed"
            );
        }
    }

    /// `Ok(())` = emitted or permanently skipped (cursor advance ok).
    /// `Err` = transient Valkey failure; cursor should stay pinned.
    async fn dispatch_expired(
        &self,
        partition: &Partition,
        exec_id: &str,
        _frame: &StreamFrame,
    ) -> Result<(), String> {
        let Some(ctx) = self.fetch_entity_context(partition, exec_id).await? else {
            return Ok(());
        };
        match ctx {
            EntityContext::Task { project, task_id } => {
                self.bridge
                    .emit(BridgeEvent::TaskStateChanged {
                        task_id,
                        project,
                        to: TaskState::RetryableFailed,
                        failure_class: Some(FailureClass::LeaseExpired),
                    })
                    .await;
            }
            EntityContext::Run { project, run_id } => {
                self.bridge
                    .emit(BridgeEvent::ExecutionFailed {
                        run_id,
                        project,
                        failure_class: FailureClass::LeaseExpired,
                        prev_state: None,
                    })
                    .await;
            }
        }
        Ok(())
    }

    async fn dispatch_reclaimed(
        &self,
        partition: &Partition,
        exec_id: &str,
        frame: &StreamFrame,
    ) -> Result<(), String> {
        // On reclaim FF writes a new lease_id + lease_epoch and a new
        // worker_id. For tasks we emit TaskLeaseClaimed so the
        // projection can mark the task leased under the new worker.
        // For runs, there is no "RunLeaseClaimed" BridgeEvent variant
        // today — the next cairn-side transition (start / complete /
        // fail) will re-sync the projection, so silence is acceptable.
        let Some(ctx) = self.fetch_entity_context(partition, exec_id).await? else {
            return Ok(());
        };
        let EntityContext::Task { project, task_id } = ctx else {
            return Ok(());
        };
        let lease_owner = frame.fields.get("worker_id").cloned().unwrap_or_default();
        let lease_epoch: u64 = frame
            .fields
            .get("new_lease_epoch")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        // FF does not write a concrete lease_expires_at on the
        // lease_history XADD — it's computed from now_ms + lease_ttl
        // on the claim path. Re-read exec_core to pick up the fresh
        // value. If the HGETALL fails, fall back to 0; the worker's
        // next heartbeat will correct it.
        let lease_expires_at_ms = self
            .fetch_lease_expires_at(partition, exec_id)
            .await
            .unwrap_or(0);
        self.bridge
            .emit(BridgeEvent::TaskLeaseClaimed {
                task_id,
                project,
                lease_owner,
                lease_epoch,
                lease_expires_at_ms,
            })
            .await;
        Ok(())
    }

    /// HGETALL on the exec's `:tags` hash, return a classified
    /// (Task vs Run) + project + id tuple. Returns:
    /// - `Ok(Some(ctx))` — successfully classified our execution.
    /// - `Ok(None)` — the frame belongs to another cairn-app instance
    ///   sharing the Valkey, OR the tags lack `cairn.task_id` /
    ///   `cairn.run_id`, OR any parse failure. Permanent outcome —
    ///   cursor can advance past this frame safely.
    /// - `Err(_)` — transient Valkey failure. `handle_frame` treats
    ///   this as "don't advance the cursor" so we retry on the next
    ///   poll and don't lose the event.
    ///
    /// **Cross-instance isolation (subscriber layer).** Before
    /// classifying, we require the exec's `cairn.instance_id` tag to
    /// match `self.own_instance_id`. Two cairn-app instances sharing a
    /// Valkey otherwise see each other's state-change frames in their
    /// global event log: the `lease_expiry` ZSET is partition-global
    /// (FF-owned, not cairn-scoped), so a poll on partition `N`
    /// enumerates every cairn instance's leased executions on that
    /// partition. The tag filter turns this into a subscriber-side
    /// partition of the frame stream by instance ownership — foreign
    /// frames are dropped with cursor advance, so we don't replay them
    /// on every poll.
    ///
    /// **Why this layer is still needed alongside FF's upstream
    /// `ScannerFilter`** (FF PR #127 / issue #122). The upstream
    /// filter narrows FF's own engine-side scanners and the
    /// `subscribe_completions_filtered` DAG dispatch stream: it
    /// prevents *this* cairn instance's FF scanners from writing
    /// lease_expiry transitions for foreign executions and prevents
    /// this instance's completion dispatch loop from firing on foreign
    /// completions. It does NOT cover this path. `LeaseHistorySubscriber`
    /// XREADs the per-execution `:lease:history` stream discovered via
    /// the partition-global `lease_expiry` ZSET — when *instance B*'s
    /// FF scanner writes an `expired` entry into *B*'s exec stream,
    /// instance A's subscriber still sees that stream key in the shared
    /// ZSET and would XREAD it. The upstream filter cannot drop those
    /// frames because they were written by a separate FF scanner
    /// process; they're legitimate entries on a stream cairn is
    /// polling by-partition. This subscriber-side tag gate is the
    /// mechanism that keeps A's event log blind to B's lease
    /// transitions on that shared stream. Do NOT remove it when the
    /// upstream filter lands — it covers a different boundary.
    ///
    /// Frames without the tag are treated as foreign too. Pre-upgrade
    /// executions that predate the filter lack the tag; operators doing
    /// an in-place binary swap must run `CAIRN_BACKFILL_INSTANCE_TAG=1`
    /// on the new boot to re-tag outstanding `Running` / `WaitingApproval`
    /// executions — otherwise their lease expiries are silently foreign.
    async fn fetch_entity_context(
        &self,
        partition: &Partition,
        exec_id: &str,
    ) -> Result<Option<EntityContext>, String> {
        let Ok(eid) = ExecutionId::parse(exec_id) else {
            // Unparseable ExecutionIds are permanent; advance past.
            return Ok(None);
        };
        let ctx = ExecKeyContext::new(partition, &eid);
        let raw: Value = self
            .client
            .cmd("HGETALL")
            .arg(ctx.tags())
            .execute()
            .await
            .map_err(|e| format!("HGETALL tags: {e}"))?;
        let Some(tags) = parse_string_map(&raw) else {
            return Ok(None);
        };
        let Some(project_str) = tags.get("cairn.project") else {
            return Ok(None);
        };
        let Some(project) = try_parse_project_key(project_str) else {
            return Ok(None);
        };
        // Cross-instance isolation gate. Must come after the
        // `cairn.project` check so we don't pay the tag read cost on
        // obviously-foreign frames, and before the task/run
        // classification so we don't accidentally emit a bridge event
        // for another instance's execution.
        match tags.get("cairn.instance_id") {
            Some(tag) if tag == &self.own_instance_id => {}
            Some(_) | None => {
                tracing::trace!(
                    exec_id,
                    "lease-history frame belongs to a different cairn instance (or is untagged); skipping"
                );
                return Ok(None);
            }
        }
        if let Some(task_id) = tags.get("cairn.task_id") {
            Ok(Some(EntityContext::Task {
                project,
                task_id: TaskId::new(task_id.clone()),
            }))
        } else {
            Ok(tags.get("cairn.run_id").map(|run_id| EntityContext::Run {
                project,
                run_id: RunId::new(run_id.clone()),
            }))
        }
    }

    async fn fetch_lease_expires_at(&self, partition: &Partition, exec_id: &str) -> Option<u64> {
        let eid = ExecutionId::parse(exec_id).ok()?;
        let ctx = ExecKeyContext::new(partition, &eid);
        let raw: Value = self
            .client
            .cmd("HGET")
            .arg(ctx.core())
            .arg("lease_expires_at")
            .execute()
            .await
            .ok()?;
        match raw {
            Value::BulkString(b) => String::from_utf8_lossy(b.as_ref()).parse().ok(),
            Value::SimpleString(s) => s.parse().ok(),
            _ => None,
        }
    }
}

enum EntityContext {
    Task {
        project: ProjectKey,
        task_id: TaskId,
    },
    Run {
        project: ProjectKey,
        run_id: RunId,
    },
}

#[derive(Debug)]
struct StreamFrame {
    stream_id: String,
    fields: HashMap<String, String>,
}

/// Parse the `Value::Map(stream_key → entries)` or RESP2 array shape
/// returned by cross-stream XREAD into a flat `stream_key → Vec<frame>`
/// map. Nil and empty replies map to an empty BTreeMap.
fn parse_multi_stream_xread(raw: &Value) -> Result<BTreeMap<String, Vec<StreamFrame>>, String> {
    let mut out: BTreeMap<String, Vec<StreamFrame>> = BTreeMap::new();
    match raw {
        Value::Nil => Ok(out),
        Value::Map(m) => {
            for (k, v) in m.iter() {
                let key = match k {
                    Value::BulkString(b) => String::from_utf8_lossy(b.as_ref()).into_owned(),
                    Value::SimpleString(s) => s.clone(),
                    other => {
                        tracing::trace!(?other, "XREAD: non-string stream key, skipping");
                        continue;
                    }
                };
                out.insert(key, parse_entries(v)?);
            }
            Ok(out)
        }
        Value::Array(arr) => {
            for item in arr {
                let item = item.as_ref().map_err(|e| format!("XREAD element: {e}"))?;
                let pair = match item {
                    Value::Array(p) => p,
                    other => {
                        tracing::trace!(?other, "XREAD RESP2: non-array element, skipping");
                        continue;
                    }
                };
                if pair.len() != 2 {
                    continue;
                }
                let key = match pair[0].as_ref() {
                    Ok(Value::BulkString(b)) => String::from_utf8_lossy(b.as_ref()).into_owned(),
                    Ok(Value::SimpleString(s)) => s.clone(),
                    _ => continue,
                };
                let entries = match pair[1].as_ref() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                out.insert(key, parse_entries(entries)?);
            }
            Ok(out)
        }
        other => Err(format!("XREAD: unexpected reply shape {other:?}")),
    }
}

fn parse_entries(raw: &Value) -> Result<Vec<StreamFrame>, String> {
    let mut frames = Vec::new();
    match raw {
        Value::Nil => Ok(frames),
        Value::Map(entries_map) => {
            for (id_val, fields_val) in entries_map.iter() {
                let stream_id = match id_val {
                    Value::BulkString(b) => String::from_utf8_lossy(b.as_ref()).into_owned(),
                    Value::SimpleString(s) => s.clone(),
                    _ => continue,
                };
                let fields = parse_field_pairs(fields_val)?;
                frames.push(StreamFrame { stream_id, fields });
            }
            Ok(frames)
        }
        Value::Array(arr) => {
            for entry in arr {
                let entry = entry.as_ref().map_err(|e| format!("XREAD entry: {e}"))?;
                let pair = match entry {
                    Value::Array(p) => p,
                    _ => continue,
                };
                if pair.len() != 2 {
                    continue;
                }
                let stream_id = match pair[0].as_ref() {
                    Ok(Value::BulkString(b)) => String::from_utf8_lossy(b.as_ref()).into_owned(),
                    Ok(Value::SimpleString(s)) => s.clone(),
                    _ => continue,
                };
                let fields_val = match pair[1].as_ref() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                let fields = parse_field_pairs(fields_val)?;
                frames.push(StreamFrame { stream_id, fields });
            }
            Ok(frames)
        }
        other => Err(format!("XREAD entries: unexpected shape {other:?}")),
    }
}

fn parse_field_pairs(raw: &Value) -> Result<HashMap<String, String>, String> {
    let mut fields = HashMap::new();
    match raw {
        Value::Map(m) => {
            for (k, v) in m.iter() {
                if let (Some(k), Some(v)) = (value_to_string(k), value_to_string(v)) {
                    fields.insert(k, v);
                }
            }
            Ok(fields)
        }
        Value::Array(arr) => {
            // ferriskey's XREAD adapter normalises every entry's
            // fields to an Array of 2-element Arrays (ArrayOfPairs),
            // not a flat Array of alternating k/v scalars. Shape:
            //   [[k1, v1], [k2, v2], ...]
            // Also handle the flat-Array fallback for RESP2
            // compatibility: [k1, v1, k2, v2, ...].
            let mut saw_nested_pair = false;
            for elem in arr {
                let Ok(elem) = elem.as_ref() else { continue };
                if let Value::Array(pair) = elem {
                    if pair.len() == 2 {
                        saw_nested_pair = true;
                        let k = pair[0].as_ref().ok().and_then(value_to_string);
                        let v = pair[1].as_ref().ok().and_then(value_to_string);
                        if let (Some(k), Some(v)) = (k, v) {
                            fields.insert(k, v);
                        }
                    }
                }
            }
            if !saw_nested_pair {
                // Flat Array: walk in pairs.
                let items: Vec<String> = arr
                    .iter()
                    .filter_map(|r| r.as_ref().ok().and_then(value_to_string))
                    .collect();
                let mut it = items.into_iter();
                while let (Some(k), Some(v)) = (it.next(), it.next()) {
                    fields.insert(k, v);
                }
            }
            Ok(fields)
        }
        Value::Nil => Ok(fields),
        other => Err(format!("XREAD fields: unexpected shape {other:?}")),
    }
}

fn value_to_string(v: &Value) -> Option<String> {
    match v {
        Value::BulkString(b) => Some(String::from_utf8_lossy(b.as_ref()).into_owned()),
        Value::SimpleString(s) => Some(s.clone()),
        Value::Int(i) => Some(i.to_string()),
        _ => None,
    }
}

/// Parse an HGETALL reply. RESP3 returns `Value::Map`, RESP2 returns
/// `Value::Array` of alternating k, v scalars. ferriskey selects at
/// connection time, so we handle both so the subscriber works on
/// either protocol.
fn parse_string_map(raw: &Value) -> Option<HashMap<String, String>> {
    match raw {
        Value::Map(m) => {
            let mut out = HashMap::new();
            for (k, v) in m.iter() {
                if let (Some(k), Some(v)) = (value_to_string(k), value_to_string(v)) {
                    out.insert(k, v);
                }
            }
            Some(out)
        }
        Value::Array(arr) => {
            let mut out = HashMap::new();
            let items: Vec<String> = arr
                .iter()
                .filter_map(|r| r.as_ref().ok().and_then(value_to_string))
                .collect();
            let mut it = items.into_iter();
            while let (Some(k), Some(v)) = (it.next(), it.next()) {
                out.insert(k, v);
            }
            Some(out)
        }
        Value::Nil => None,
        _ => None,
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
