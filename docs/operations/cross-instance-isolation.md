# Cross-instance isolation on a shared Valkey

Cairn uses FlowFabric (FF) as its execution engine. FF stores its state
in Valkey under `ff:*` keys and indexes every leased execution in a
partition-global `ff:idx:{fp:N}:lease_expiry` ZSET. That ZSET is
**not** cairn-scoped — every cairn-app instance (or any other FF
consumer) sharing the same Valkey writes into the same ZSET.

Without a filter, the `LeaseHistorySubscriber` in each cairn-app
instance would consume every foreign cairn instance's lease-expiry and
lease-reclaim frames and emit them into its own durable event log
(`/v1/events`). Operators would see runs on instance A that in fact
only exist on instance B.

## How isolation works

1. **Write**. Every cairn execution (runs and tasks) is created with a
   `cairn.instance_id` exec tag equal to `FabricConfig::worker_instance_id`
   (seeded from the `CAIRN_FABRIC_INSTANCE_ID` env var, or a UUID
   persisted at `/tmp/cairn-fabric-instance-id`).

2. **Read**. `LeaseHistorySubscriber::fetch_entity_context` reads the
   exec tags for every frame it picks off a lease-history stream.
   Frames whose `cairn.instance_id` doesn't match this instance's
   id are dropped with cursor advance (they're permanent-skip, not
   transient failures).

3. **Per-process partition.** Each cairn-app process is its own
   isolation domain. Multi-tenant isolation inside a single cairn-app
   is handled a layer up by `ProjectKey` filtering on cairn-store
   projections — unchanged by this feature.

## Deploying a new instance

Fresh deploys need no special action. The tag is written on every
new execution; the subscriber filter starts with every frame from
day one of the binary rollout.

## In-place binary swap with in-flight runs

If you upgrade a cairn-app instance in place and outstanding
`Running` / `WaitingApproval` executions exist in Valkey, those
executions predate the filter and lack the `cairn.instance_id` tag.
Their lease expiries would then be silently filtered out on the new
boot (the filter treats untagged frames as foreign).

Set `CAIRN_BACKFILL_INSTANCE_TAG=1` on the first boot of the new
binary. The process runs a one-shot `SCAN` over `ff:exec:*:tags`,
identifies hashes that carry `cairn.project` (confirming a cairn
execution) but lack `cairn.instance_id`, and HSETs the tag. Output:

```
backfilled cairn.instance_id on 23 executions (scanned=412 skipped_tagged=389 skipped_foreign=0)
```

Drop the env var on subsequent boots — it is idempotent (second pass
is a no-op because every exec now carries a tag), but unnecessary.

## Troubleshooting

- **"My lease expiries aren't propagating after an upgrade."** Check
  whether `CAIRN_FABRIC_INSTANCE_ID` changed across the swap. If it
  did, even the backfill won't help: the new-boot subscriber filters
  on the new id, and the old executions got backfilled with the new
  id, but operators who retained tooling that asserts old-id
  provenance may be confused. Avoid rotating the instance id across
  an in-place swap.

- **Two cairn-apps with the same `CAIRN_FABRIC_INSTANCE_ID`.** The
  filter can't distinguish them — both instances treat each other's
  frames as their own. Always set distinct instance ids per process.
  The default UUID-persisted-to-`/tmp` behavior naturally gives each
  process a unique id.

## Relation to FF namespace prefix (future work)

This is a read-side filter. FF executions from two cairn instances
still co-exist in the same Valkey keyspace — the filter just hides
foreign frames on subscriber read. A stronger defence-in-depth layer
would add a write-side namespace prefix (`FF_KEY_NAMESPACE`)
upstream in FF, landing in a later minor release. Tracked as #188
follow-up.
