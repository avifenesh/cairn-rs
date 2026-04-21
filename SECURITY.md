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
  when the Fabric backend is enabled; boot fails loud if unset. Operators
  rotate it at runtime via `POST /v1/admin/rotate-waitpoint-hmac` without
  restarting — see "Waitpoint HMAC secret rotation" below.
- All worker claim paths route through `FabricSchedulerService::claim_for_worker`
  which runs budget / quota / capability admission. There is no direct-Valkey
  bypass.

## Waitpoint HMAC secret rotation

The waitpoint HMAC secret (`CAIRN_FABRIC_WAITPOINT_HMAC_SECRET`) is seeded
at boot into every execution partition's `ff:sec:{fp:N}:waitpoint_hmac`
hash under a named kid (`CAIRN_FABRIC_WAITPOINT_HMAC_KID`, defaults to
`cairn-v1`). Cairn signs every waitpoint token with the current kid and
verifies tokens against both the current kid and any previously
installed kid still within its grace window.

### When to rotate

- Periodic: quarterly (or per internal key-rotation policy).
- Reactive: immediately if the secret may have leaked (operator
  laptop compromise, Git leak, compliance finding).
- After a cairn-app deployment that changed the env-var source (for
  example, moving from raw env to a Vault-sourced secret).

### How to rotate (zero-downtime)

1. Generate a new 32-byte secret:
   ```bash
   openssl rand -hex 32
   ```
2. Pick a fresh kid name that does NOT contain `:` (FF uses `:` as a
   field separator in the on-disk hash). `cairn-v2`, `q1-2026`, etc.
3. Call the admin endpoint with a current admin bearer:
   ```bash
   curl -X POST https://<cairn-host>/v1/admin/rotate-waitpoint-hmac \
     -H "Authorization: Bearer $CAIRN_ADMIN_TOKEN" \
     -H "Content-Type: application/json" \
     -d '{
       "new_kid": "cairn-v2",
       "new_secret_hex": "<hex from step 1>",
       "grace_ms": 60000
     }'
   ```
4. Expect HTTP 200 with `rotated` equal to your partition count and
   `failed` empty. Partial failure? Rotation is idempotent on the
   same `(new_kid, new_secret_hex)` — re-run the exact same request
   once the underlying transport / Valkey fault clears. The
   previously-installed kid stays accepted for `grace_ms` so
   in-flight waitpoints don't fail verification mid-rotation.
5. Update your persistent config store (Vault / env file / container
   secret) so a future restart seeds the new kid + secret. Restart
   before `grace_ms` elapses if you want a clean boot-seed cycle;
   after elapses is also safe, FF cleans up expired kids on the next
   rotation.

### Operational constraints

- **Kid reuse with a different secret is rejected.** Reusing an
  already-installed kid with a new secret returns
  `code: rotation_conflict`. Pick a fresh kid.
- **Concurrent rotate calls serialize per partition.** The FCALL is
  the atomicity boundary; two admins hitting the endpoint
  simultaneously will each observe a consistent outcome (one goes
  first, the other sees either `noop` or `rotation_conflict`).
- **All admin audit log entries for rotations include the kid.** Do
  not include the raw secret in any log, ticket, or rotation
  announcement. Only the kid is safe to share.

### Threat model covered

- Secret disclosure via env-var leak, process dump, or logs: rotation
  installs a new kid; after `grace_ms` the old secret cannot sign new
  waitpoints. Tokens signed before rotation that the operator wants
  to invalidate immediately can be forced out by setting `grace_ms:
  0`, at the cost of in-flight waitpoints failing verification.
- Admin token compromise: an attacker with the admin bearer CAN
  rotate the HMAC, but the rotation is visible in the admin audit
  log and recoverable by rotating again with a known-good secret.

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

## Task dependency `data_passing_ref`

`POST /v1/tasks/{id}/dependencies` accepts an optional
`data_passing_ref` string that cairn forwards verbatim to FF 0.2's
`ff_stage_dependency_edge` / `ff_apply_dependency_to_child` FCALLs.
The value is stored on the FF edge hash and surfaced to the
downstream task after upstream resolution.

### Contract

**Cairn does not dereference `data_passing_ref`.** It is an opaque
string from cairn's perspective: cairn validates only that the bytes
are safe to round-trip through Valkey's Lua HSET and log without
quoting surprises. The semantic meaning of the value — URL, artifact
ID, signed token, Valkey key, file path — is the producer's and
consumer's concern.

### Validation at ingress

- Maximum length: 256 bytes.
- Allowed charset: `[A-Za-z0-9._:/-]`.
- Whitespace, control characters, null bytes, and non-ASCII are
  rejected with 422 `validation_error`.
- Empty string is normalised to absent (same as omitting the field).

These rules are intentionally narrow. They deliberately forbid:
- URL reserved chars `?`, `&`, `=`, `#` (reject passing full URLs
  with query strings; operators who need URLs should wrap them in
  a producer-side indirection that returns a short artifact ID).
- Whitespace and control chars (log-injection defence).
- Non-ASCII (simplifies audit-log review; prevents homograph tricks).

### Downstream consumer responsibilities

Downstream workers that interpret `data_passing_ref` MUST validate
the value at consumption time. Specifically:
- **URLs or network identifiers**: fetching an attacker-controlled
  URL from inside the runtime network is an SSRF opportunity. Apply
  allowlists at the consumer, not at cairn ingress — cairn has no
  workload context to make that decision.
- **Filesystem paths or Valkey keys**: treat as untrusted input;
  apply path-traversal / scope-prefix checks before use.
- **Signed tokens**: verify signatures before acting on them.

### Logging

Cairn emits `data_passing_ref.len` and `data_passing_ref.prefix`
(first 16 chars) in `tracing::debug!` spans at declare time, never
the full value, so log collectors don't end up hoarding signed URLs
or tokens. The persisted `BridgeEvent::TaskDependencyAdded` event
(visible to operators subscribed to the RuntimeEvent stream) does
carry the full value — operators accessing their own event stream
have the same trust level as the declare caller.

### Durability

`data_passing_ref` shares the durability class of every other
FF-owned piece of Valkey state (execution records, lease history,
waitpoint HMAC secrets, flow edges). Operators running Valkey
without AOF appendonly persistence will lose refs across a Valkey
restart; the downstream task then fires without a
`data_passing_ref`. AOF is the recommended production configuration
— see the "Waitpoint HMAC secret rotation" section above for the
same durability requirement applied to secret storage.
