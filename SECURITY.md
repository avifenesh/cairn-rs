# Security Policy

## Supported versions

cairn-rs is pre-1.0. Security fixes land on `main` and are not backported.
Users should track `main` directly or pin a recent tag.

## Reporting a vulnerability

**Do not open a public GitHub issue for security problems.**

Email security reports to **avifenesh@users.noreply.github.com** or open a
[GitHub Security Advisory](https://github.com/avifenesh/cairn-rs/security/advisories/new)
(private disclosure channel).

Include:
- Affected component (crate + file + line when possible)
- Reproduction steps or proof-of-concept
- Impact assessment: who is affected, under what configuration, what data or
  actions are exposed

We aim to acknowledge reports within 72 hours and triage within 7 days.
Coordinated-disclosure timelines are negotiated per report; 90 days is the
default upper bound absent active exploitation.

## Scope

In scope:
- The cairn-rs binary and its HTTP surface
- Included crates under `crates/`
- The embedded operator dashboard (`ui/`)
- Default configuration, the provided Dockerfile, and `docker-compose.yml`

Out of scope:
- Self-hosted deployments where the operator has configured cairn against
  their own risk model (e.g. disabling auth, exposing admin endpoints to the
  internet). We will document hardening guidance but not treat operator
  misconfiguration as a vulnerability unless defaults are the cause.
- External providers (Anthropic, OpenAI, Bedrock, etc.). Report directly
  to those vendors.
- Plugins distributed outside this repository.

## Known security boundaries

- `CAIRN_ADMIN_TOKEN` is the admin authentication for the HTTP surface. Dev
  defaults (`dev-admin-token`, `cairn-demo-token`) must not be used in
  production. Boot logs warn when defaults are active.
- FlowFabric HMAC secret (`CAIRN_FABRIC_WAITPOINT_HMAC_SECRET`) is required
  when the Fabric backend is enabled; boot fails loud if unset.
- All worker claim paths route through `FabricSchedulerService::claim_for_worker`
  which runs budget / quota / capability admission. There is no direct-Valkey
  bypass.

## Debug endpoints feature (`debug-endpoints`)

### What it does

Opt-in Cargo feature on `cairn-app` that enables the admin-only
`GET /v1/admin/debug/partition` endpoint. The endpoint returns the
FF-internal ExecutionId and Valkey partition index for a given run or
task. Used exclusively by the RFC-011 co-location integration tests.

### Default state

**OFF by default.** Production release builds MUST be compiled without
this feature. CI integration tests build with it explicitly.

Verify a binary:
```bash
# Release binary must have no debug_partition symbols:
nm target/release/cairn-app 2>/dev/null | grep debug_partition_handler
# Expected: empty output.

# Dev/test binary built with the feature will show the symbol:
cargo build -p cairn-app --features debug-endpoints
nm target/debug/cairn-app | grep debug_partition_handler
# Expected: one or more mangled symbols.
```

### What enabling exposes

Information that is not otherwise reachable over HTTP:

1. **FF-internal `ExecutionId`.** Previously never on any HTTP surface.
   The ExecutionId is the UUID portion of the Valkey key
   `exec_core:{fp:N}:<uuid>`. An attacker with both a compromised admin
   token AND direct Valkey access could target specific FF keys,
   bypassing cairn-side tenant scoping that gates the normal HTTP
   surface.
2. **Valkey partition index.** Integer 0..N where N is
   `num_flow_partitions` (default 256). Repeated queries let a caller
   build a tenant → partition histogram, revealing hot-shard patterns
   useful for targeted DoS against a single Valkey shard.
3. **Derivation code path.** `session_flow` vs `solo` reveals structural
   details of the task-session relationship.
4. **Audit log content.** The request log records which run/task IDs
   were inspected — itself sensitive in multi-tenant deployments.

### When enabling is acceptable

- CI integration tests (the fabric-integration CI job enables it).
- Local development against a disposable Valkey.
- A bounded diagnostic window in a pre-production environment, followed
  immediately by a rebuild and redeploy without the feature.

### When enabling is **not** acceptable

- Production deployments (any environment serving real users or
  customer data).
- Any shared or multi-tenant environment where an admin token
  compromise would expose more than one tenant.
- Build pipelines that do not gate `--features debug-endpoints` behind
  an explicit review step.
