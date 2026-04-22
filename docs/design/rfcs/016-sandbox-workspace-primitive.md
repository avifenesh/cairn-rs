# RFC 016: Sandbox Workspace Primitive (RepoStore + Per-Run Overlays)

Status: draft (rev 2 — adopts cairn-go's identifier-based RepoStore + per-run overlay model)
Owner: runtime/workspace lead
Depends on: [RFC 002](./002-runtime-event-model.md), [RFC 005](./005-task-session-checkpoint-lifecycle.md), [RFC 007](./007-plugin-protocol-transport.md), [RFC 008](./008-tenant-workspace-profile.md), [RFC 011](./011-deployment-shape.md)

## Summary

Cairn needs two related but distinct concepts to give agents a safe place to do real work:

1. **`RepoStore`** — a **per-tenant** immutable cache of cloned git repositories at `$base/repos/{tenant_id}/{owner}/{repo}`, paired with a **per-project** access allowlist that determines which projects can reference each repo. Each repo is cloned once per tenant, locked read-only via `chmod -R a-w`, and shared across projects in the same tenant that have allowlisted it. The operator (or, with approval, the agent itself via the `cairn.registerRepo` tool) controls the per-project allowlist; project A granting access to `org/foo` does not leak access to project B. See §"RepoStore" for the two-service split (`RepoCloneCache` + `ProjectRepoAccessService`).

2. **`RunSandbox`** — a per-run, durable, isolated, control-plane-owned execution area built as a copy-on-write layer above the immutable `RepoStore` base. The agent only writes to its own per-run upper layer; the base repo is never mutated.

This is the model cairn-go landed on after iterating away from per-task git worktrees (commit `9164bcd: identifier-based repo store, remove worktrees`). It is significantly cleaner than the worktree approach this RFC originally proposed:

- **Disk usage**: `O(repos + per_run_upper_diffs)` instead of `O(repos × concurrent_runs)`
- **No divergence**: the immutable lower layer makes filesystem-level conflicts between concurrent agents impossible. Two agents working on the "same file" each see their own version in their respective upper layers.
- **No git ops at provision time**: just mount an overlay (Linux) or reflink-clone (macOS/Windows). No `git worktree add` per run.
- **One clone per repo, ever**: the operator pre-populates the store, agents reuse it.

The `cairn-workspace` crate ships two providers in v1, both backed by the same `RepoStore`:

- **`OverlayProvider`** (Linux default): mounts an OverlayFS with the locked repo as the read-only `lowerdir` and a per-run `upperdir`. Multi-agent resume via stacked `upper.prev.N` layers.
- **`ReflinkProvider`** (macOS + Windows): clones the locked repo into the run sandbox via `reflink-copy` (APFS `clonefile` on macOS, ReFS Block Cloning on Windows 11 24H2+). Same operator-facing semantics as overlay; different underlying mechanism.

`RunSandbox` is named distinctly from cairn's tenancy workspace (`workspace_impl.rs` per RFC 008) to avoid namespace collision. The crate is `cairn-workspace`; the public concept is `RunSandbox`.

This RFC defines:

- The `RepoStore` entity, lifecycle, and event model
- The `RunSandbox` concept (provisioned sandbox struct), `SandboxProvider` trait, state machine, events, and policy struct
- Two providers (`OverlayProvider`, `ReflinkProvider`) and their fallback rules
- Recovery and credential injection
- The `cairn.registerRepo` built-in tool that lets agents add repos to the allowlist (subject to RFC 019's decision layer)

This RFC does not specify the GitHub plugin, tool implementations beyond `cairn.registerRepo`, or agent loop changes. Those live in their own RFCs.

**Generality**: this is not coding-specific. A research agent that needs a writable scratch directory uses `SandboxBase::Empty`. A data agent that needs to mutate a CSV uses `SandboxBase::Directory`. A code agent uses `SandboxBase::Repo`. The same `RepoStore` + provider model applies — the only difference is what's at the base of the layered view.

## Decisions Already Resolved (this revision)

The following were resolved during the question-by-question pass and are baked into this RFC:

- **Vendoring scope**: only AI-ecosystem code is vendored. The codex-specific bubblewrap wrapper, codex's Landlock policy builder wrapper, and codex's seccomp default policy are vendored under `crates/cairn-workspace/vendor/codex-linux-sandbox/`. The generic infrastructure crates (`landlock`, `seccompiler`, `libmount`, `reflink-copy`, `nix`) are normal Cargo dependencies. See "Dependency Policy" below.

- **Architectural model**: cairn-go's RepoStore + per-run overlay model is adopted. Git worktrees are dropped from v1.

- **Non-Linux fallback**: `ReflinkProvider` for macOS (APFS `clonefile`) and Windows (ReFS Block Cloning, falls back to full copy on NTFS) via the `reflink-copy` Rust crate. Unified semantics with the Linux `OverlayProvider`.

- **Sandbox base directory**: defaults to `$XDG_DATA_HOME/cairn/sandboxes` (typically `~/.local/share/cairn/sandboxes`). Honors `CAIRN_SANDBOX_ROOT` env var and `[sandbox] root` in config for override. Works without root, works in containers.

- **Concurrent sandbox cap**: 256 default per host, configurable. Runs past the cap enter `waiting_dependency` until capacity frees.

- **Divergence detection**: removed entirely. The immutable lower layer makes filesystem-level conflicts between concurrent agents impossible.

## Why

### The gap this closes

Cairn today has no isolated execution area for runs. When an agent needs to run a shell command, the `bash` tool runs it in the cairn-app process's own environment, with the cairn-app's own current directory, file permissions, and environment variables. This is fine for read-only tools (grep, web fetch) and for self-contained calculations. It is not fine for:

- a research agent that needs to write scratch files without contaminating other runs' scratch files
- a data extraction agent that clones a source repository and munges it
- an ops agent that runs a migration script
- a code agent that builds and tests a branch
- any task that produces large intermediate artifacts that must not leak into the next run

Without a sandbox:

1. **Concurrent runs collide on filesystem state.** Two runs writing to `/tmp/scratch` stomp each other.
2. **Recovery is impossible.** If cairn-app crashes mid-run, the files the agent was editing are in an undefined state, and there is no rescue path.
3. **Credentials leak.** A run with GitHub write access operates in the same process as a run without — the second run's agent can read the first run's environment.
4. **Resource limits don't exist.** An agent that writes 50 GB of scratch files fills the host disk.
5. **The product cannot honestly advertise "agents do real work safely."**

### Why this is not just "run Docker"

The user's constraint: Rust-native, disk-level, faster than containers, based on what cairn-go proved. Docker:

- requires an external daemon and root-equivalent privileges
- has ~1 s cold start per container
- cannot cleanly checkpoint and resume an in-flight process
- does not compose with cairn-rs's event log (container state is opaque to the control plane)
- hides filesystem mutations from cairn's event-sourced model

Cairn-go's git-worktree + OverlayFS approach ran sub-second workspace creation, survived process crashes (via `rescue/` branches and preserved upper layers), required no external runtime, and kept state in files cairn's process could inspect. This RFC brings that model to Rust with control-plane event integration that cairn-go did not have.

### Why not a hard dependency on `codex-linux-sandbox`

The existing Apache-2.0 crate from OpenAI's Codex CLI implements the process isolation primitives we need (bubblewrap wrapper + Landlock + seccomp). Depending on it would save work. The problem is ownership: cairn cannot let its sandbox story drift with OpenAI's release cadence or licensing. We will **strip the pieces we need and vendor them** under `crates/cairn-workspace/vendor/codex-linux-sandbox/` with the upstream license preserved and attribution in the crate README. Cairn owns the vendored code — we can patch it, extend it, or rewrite it without waiting for upstream. Future upstream changes are pulled in manually as version bumps, not automatic.

## Product Goals

The sandbox layer must let a run:

1. **Provision** an isolated environment keyed to the run's identity, in under 500 ms p95
2. **Work** inside that environment: read/write files under a sandbox root, run subprocesses with resource limits, make outbound calls subject to network policy, use credentials scoped to the run
3. **Checkpoint** the environment at meaningful progress points so a crash doesn't lose work
4. **Recover** an in-flight sandbox after a cairn-app restart, reattach to a resumed run
5. **Destroy** cleanly on run completion, with full cleanup and an audit event
6. **Fail safely** when resources are exhausted, network policy is violated, or the host kernel lacks a required feature (fall back to a less-isolated strategy with an explicit event recording the degradation)

The sandbox layer must NOT:

- require Docker, Podman, or any container runtime
- require root to run cairn-app in the default strategy (`OverlayProvider` works on kernel ≥ 5.11 unprivileged user namespaces; `ReflinkProvider` is fully unprivileged)
- hide its state from the event log
- couple to git semantics for non-code use cases (the `RepoStore` handles git, but `SandboxBase::Directory` and `SandboxBase::Empty` skip it entirely)
- expose raw kernel syscall errors to the operator — wrap them in `WorkspaceError` with actionable context

## Scope

### In scope for v1

- **New crate: `cairn-workspace`** containing both `RepoStore` and `RunSandbox` concepts
- **`RepoStore`**: splits into `RepoCloneCache` (tenant-scoped physical clone cache at `$base/repos/{tenant_id}/{owner}/{repo}/`, locked read-only) and `ProjectRepoAccessService` (project-scoped access allowlist per RFC 015 per-project isolation); composed by a thin `RepoStore` facade
- **`RunSandbox`** concept (`ProvisionedSandbox` struct) + **`SandboxProvider`** trait
- **Two providers, both backed by `RepoStore`**:
  - `OverlayProvider` (Linux): mounts the locked repo as overlay `lowerdir`; per-run `upperdir` captures all writes; multi-agent stacked `upper.prev.N` for resume
  - `ReflinkProvider` (macOS + Windows): clones the locked repo into per-run sandbox via `reflink-copy` (APFS clonefile / ReFS Block Cloning); same operator-facing semantics
- **Built-in tool: `cairn.registerRepo`** — agents can request "clone this repo and add it to my project's allowlist"; gated by RFC 019's decision layer; the tool returns authorization/clone status only (no host clone path), and the decision cache key is `(project, repo_id)` so approvals do not cross projects
- **Operator HTTP surface** (`/v1/projects/:project/repos`) for listing, adding, and revoking project repo access; physical clone GC is async via a background sweep, not synchronous on DELETE
- **Sandbox provenance**: sandbox events projected into `cairn-graph` via `event_projector` as `GraphNode(Sandbox)` / `GraphNode(RepoBase)` with opaque typed IDs (no raw host paths in graph node_ids; `SandboxBase::Directory` deferred from graph projection until an alias field is added)
- **Vendored pieces from `codex-linux-sandbox`** (codex-specific code only — see Dependency Policy):
  - the bubblewrap wrapper for process namespace + mount isolation
  - codex's Landlock policy builder wrapper
  - codex's seccomp-bpf default policy
- **Generic infra crates as regular dependencies** (not vendored): `landlock`, `seccompiler`, `libmount`, `reflink-copy`, `nix`
- State machine: 9 states (`Initial`, `Provisioning`, `Ready`, `Active`, `Checkpointed`, `Preserved`, `Destroying`, `Destroyed`, `Failed`) plus event variants in the runtime event log
- Policy struct: `SandboxPolicy` with resource limits, network egress allowlist, credential scope, base reference, strategy preference
- Reconnect-on-restart: scan sandbox base directory, reconcile against task engine, drop orphans, reattach active ones
- Credential injection: `GIT_ASKPASS` pattern for git credentials, scoped environment variables for others; tokens never written to disk in plaintext
- Event log integration: sandbox events flow through the existing `RuntimeEvent` pipeline with no separate audit path
- `EnsureAllCloned` startup pass: pre-populate the `RepoCloneCache` from the deduplicated `(tenant, repo_id)` set derived from all project allowlists before the HTTP server opens (per RFC 020 startup order)
- Integration hooks for runtime service (`cairn-runtime/src/services/`) and orchestrator (`cairn-orchestrator/`)
- Hard cap of 256 concurrent sandboxes per host (configurable); excess runs enter `waiting_dependency`

### Explicitly out of scope for v1

- **Git worktrees**: dropped from v1 entirely (the cairn-go RepoStore + per-run overlay model supersedes the worktree-per-task approach)
- **Divergence detection**: dropped from v1 (the immutable lower layer makes filesystem-level conflicts between concurrent agents impossible at the sandbox layer; conflicts only manifest at git remote-push time, which is git's domain)
- Firecracker / libkrun microVM isolation (overkill for trusted agents in v1)
- btrfs / ZFS subvolume snapshots (operationally complex; OverlayFS + reflink covers v1)
- FUSE-based custom CoW filesystems (200+ hours of implementation; not needed)
- Network egress enforcement (the policy field exists and is recorded in events, but actual enforcement is deferred — default is full network access for v1 trusted agents, matching cairn-go's default)
- GPU passthrough
- Per-sandbox metrics dashboard (the existing observability layer handles this once events flow)

## Naming: `RunSandbox` vs "workspace"

The existing `cairn-runtime/src/services/workspace_impl.rs` manages tenancy workspaces per RFC 008. The crate introduced by this RFC uses the name `RunSandbox` for the execution primitive to avoid collision. "Workspace" in cairn-rs hereafter refers only to the tenancy concept. The crate itself is named `cairn-workspace` to avoid `cairn-runsandbox` which reads poorly, but every public type uses `RunSandbox`.

## Canonical Model

### State Machine

```
                         provision()
         [Initial] ──────────────────────▶ [Provisioning]
                                              │
                                       success│   failure
                                              │         \
                                         [Ready]      [Failed]
                                              │
                                   activate()
                                              │
                                         [Active]
                                              │
                            ┌─────────────────┼─────────────────┐
                            │                 │                 │
                 checkpoint()│     completion()│      crash/preempt
                            ▼                 ▼                 ▼
                      [Checkpointed]     [Destroying]      [Preserved]
                            │                 │                 │
                    resume()│         cleanup()│      recover() │
                            ▼                 ▼                 ▼
                      [Provisioning]     [Destroyed]      [Provisioning]
                                                      (resume path)
```

States:

- **Provisioning**: directories, overlay mounts, or reflink clones being created; credentials being resolved; base revision being checked out
- **Ready**: sandbox exists on disk, no agent is currently attached
- **Active**: an agent process is executing inside the sandbox; heartbeats must be received periodically
- **Checkpointed**: a durable recovery point has been written; agent may or may not still be active
- **Preserved**: agent crashed, was preempted, or cairn-app restarted; the sandbox state is kept on disk so a future run can resume
- **Destroying**: cleanup has started; no new operations accepted
- **Destroyed**: sandbox is fully gone; a tombstone record remains in the event log for audit
- **Failed**: provisioning failed; no sandbox exists to destroy

### Events (RuntimeEvent variants)

```rust
pub enum SandboxEvent {
    SandboxProvisioned {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        task_id:      Option<TaskId>,
        project:      ProjectKey,
        strategy:     SandboxStrategy,     // Overlay | Reflink
        base_revision: Option<String>,     // git SHA or equivalent
        policy:       SandboxPolicySnapshot, // full policy as recorded
        path:         PathBuf,
        duration_ms:  u64,
        provisioned_at: u64,
    },
    SandboxActivated {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        pid:          Option<u32>,
        activated_at: u64,
    },
    SandboxHeartbeat {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        heartbeat_at: u64,
    },
    SandboxCheckpointed {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        checkpoint_kind: SandboxCheckpointKind,
        rescue_ref:   Option<String>,
        upper_snapshot: Option<PathBuf>,
        checkpointed_at: u64,
    },
    SandboxPreserved {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        reason:       PreservationReason,
        preserved_at: u64,
    },
    SandboxDestroyed {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        files_changed: u32,
        bytes_written: u64,
        reason:       DestroyReason,
        destroyed_at: u64,
    },
    SandboxProvisioningFailed {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        error_kind:   SandboxErrorKind,
        error:        String,
        failed_at:    u64,
    },
    SandboxPolicyDegraded {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        requested:    SandboxStrategy,
        actual:       SandboxStrategy,
        reason:       String,  // e.g. "overlayfs requires CAP_SYS_ADMIN; fell back to reflink"
        degraded_at:  u64,
    },
    /// Pure observation event fired the moment a sandbox trips a policy-declared
    /// resource cap, regardless of the configured on_resource_exhaustion mode.
    /// Does NOT itself change sandbox state — the follow-on transition (if any)
    /// is recorded on SandboxDestroyed or SandboxPreserved with a matching
    /// ResourceLimitExceeded / AwaitingResourceRaise reason variant.
    SandboxResourceLimitExceeded {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        dimension:    ResourceDimension,
        limit:        u64,     // the policy cap (bytes or ms)
        observed:     u64,     // the measured value that tripped it
        at:           u64,
    },
    /// Overlay-specific: during recovery of a SandboxBase::Repo sandbox using
    /// OverlayProvider, the recorded base_revision did not match the current
    /// locked clone HEAD. The sandbox is transitioned to Preserved pending
    /// operator review. ReflinkProvider sandboxes are physically independent
    /// of the source clone and never emit this event.
    SandboxBaseRevisionDrift {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        project:      ProjectKey,
        repo_id:      RepoId,
        expected:     String,  // meta.base_rev from disk
        actual:       String,  // current clone HEAD
        detected_at:  u64,
    },
    /// During recovery, the sandbox's SandboxBase::Repo { repo_id } was found
    /// to no longer be in the project's allowlist (revoked by an operator
    /// between crash and restart). The sandbox is transitioned to Preserved
    /// pending operator review; previously-authorized work is not retroactively
    /// invalidated but the run cannot accept new tool calls without fresh
    /// operator action (re-grant or cancel).
    SandboxAllowlistRevoked {
        sandbox_id:   SandboxId,
        run_id:       RunId,
        project:      ProjectKey,
        repo_id:      RepoId,
        revoked_at:   u64,     // when the allowlist change landed
        detected_at:  u64,     // when recovery noticed the sandbox was on a revoked repo
    },
}

/// Dimensions on which SandboxPolicy can declare hard caps in v1.
/// CpuTime, FileDescriptors, ProcessCount, and other dimensions are deferred
/// to a future RFC that adds corresponding policy fields. cpu_weight in
/// SandboxPolicy is a cgroup scheduling share, NOT a cap, and is deliberately
/// not enumerated here.
pub enum ResourceDimension {
    DiskBytes,     // policy.disk_quota_bytes
    MemoryBytes,   // policy.memory_limit_bytes
    WallClockMs,   // policy.wall_clock_limit (milliseconds)
}

pub enum DestroyReason {
    Completed,
    Failed,
    Abandoned,
    Stale,
    /// Sandbox was destroyed because it tripped a policy-declared resource cap
    /// and the policy's on_resource_exhaustion mode was `Destroy`. The
    /// limit/observed fields are duplicated from the preceding
    /// SandboxResourceLimitExceeded event for single-event readability.
    ResourceLimitExceeded {
        dimension: ResourceDimension,
        limit:     u64,
        observed:  u64,
    },
}

pub enum PreservationReason {
    AgentCrashed,
    AgentPreempted,
    ControlPlaneRestart,
    /// Sandbox tripped a policy-declared resource cap and the policy's
    /// on_resource_exhaustion mode was `PauseAwaitOperator`. The sandbox stays
    /// Preserved; the run transitions to WaitingApproval per RFC 005. Operator
    /// raises the cap → sandbox resumes; operator cancels → sandbox transitions
    /// Destroyed with Abandoned.
    AwaitingResourceRaise {
        dimension: ResourceDimension,
        limit:     u64,
        observed:  u64,
    },
    /// OverlayProvider-specific: during recovery, meta.base_rev did not match
    /// the current locked clone HEAD. Sandbox preserved for operator review
    /// because the overlay's lower layer has moved under the upper layer.
    BaseRevisionDrift {
        expected: String,
        actual:   String,
    },
    /// During recovery, the project's allowlist no longer includes the repo
    /// this sandbox was provisioned against. Sandbox preserved pending
    /// operator re-grant or cancellation.
    AllowlistRevoked {
        repo_id: RepoId,
    },
}
```

The `SandboxPolicySnapshot` captures the entire policy at provision time so every historical sandbox can be replayed for audit even after the live policy has changed.

### Resource-exhaustion runtime flow

When a sandbox trips a resource limit the provider telemetry emits `SandboxResourceLimitExceeded` unconditionally (regardless of policy mode). The follow-on transition depends on `SandboxPolicy.on_resource_exhaustion`:

- **`Destroy`** — sandbox transitions `Active` → `Destroying` → `Destroyed` and emits `SandboxDestroyed { reason: DestroyReason::ResourceLimitExceeded { dimension, limit, observed } }`. The run (per RFC 005) transitions to `Failed` with `RunFailureReason::ResourceLimitExceeded { dimension }`.
- **`PauseAwaitOperator`** — sandbox transitions `Active` → `Preserved` (the existing state; not a new `Paused` state) and emits `SandboxPreserved { reason: PreservationReason::AwaitingResourceRaise { dimension, limit, observed } }`. The run (per RFC 005) transitions to `WaitingApproval`. The operator raises the cap → sandbox resumes (new `SandboxActivated`) and run returns to `Running`; or operator cancels → sandbox transitions `Preserved` → `Destroyed { reason: Abandoned }` and run transitions to `Cancelled`.
- **`ReportOnly`** — NO state transition on sandbox OR run. The `SandboxResourceLimitExceeded` event is advisory-only and does not propagate a `RunFailureReason`. This mode is for staging/debug runs where the operator wants to observe cap-breaches without blocking work. Because no state transition occurs, `ReportOnly` is observationally distinct from both `Destroy` and `PauseAwaitOperator`.

This design keeps the sandbox state machine and the run state machine distinct: sandbox state tracks filesystem/resource container lifecycle, run state tracks scheduling/policy lifecycle, and they reconverge at operator-decision points.

### Policy

```rust
pub struct SandboxPolicy {
    pub strategy: SandboxStrategyRequest,
        // Preferred | Force(Overlay) | Force(Reflink)
        // Provider selection — see "Provider Resolution" below

    // What sits at the base of the layered view
    pub base: SandboxBase,

    // Credentials to inject. Each entry is a reference, not a value.
    pub credentials: Vec<CredentialReference>,

    // Network egress: None = unrestricted (v1 default), Some = allowlist (future enforcement)
    pub network_egress: Option<Vec<String>>,

    // Resource limits (hard caps; see ResourceDimension enum)
    pub memory_limit_bytes:  Option<u64>,       // cgroup v2 memory.max (Linux only)
    pub cpu_weight:          Option<u32>,       // cgroup v2 cpu.weight (scheduling share, NOT a cap)
    pub disk_quota_bytes:    Option<u64>,
    pub wall_clock_limit:    Option<Duration>,

    // Behavior when a hard cap is exceeded. Each of the three modes produces a
    // distinct SandboxEvent/run transition shape — see "Resource-exhaustion
    // runtime flow" section above.
    pub on_resource_exhaustion: OnExhaustion,

    // Cleanup behavior on agent failure
    pub preserve_on_failure: bool,              // keep upper layer for future resume

    // Host capability requirements (validated at provision time)
    pub required_host_caps: HostCapabilityRequirements,
}

pub enum SandboxBase {
    /// Provision from a repository in the RepoStore. The repo must already
    /// be in the project's allowlist (or the agent must register it via
    /// cairn.registerRepo). The repo's locked, immutable clone serves as the
    /// lower layer.
    Repo {
        repo_id: RepoId,           // "owner/repo" identifier; resolved via RepoStore
        // Optional: a git ref to start the per-run upper layer from. Defaults
        // to the locked clone's HEAD. The upper layer is where the agent
        // creates branches, commits, etc.
        starting_ref: Option<String>,
    },

    /// Provision by overlaying or cloning an arbitrary directory.
    /// The directory is treated as an immutable lower layer the same way a
    /// RepoStore entry would be. Used for non-git data sources (a CSV
    /// directory, a documentation tree, an artifact bundle).
    Directory { path: PathBuf },

    /// Provision an empty scratch directory. No lower layer; the upper layer
    /// is the only layer. Used by research agents that just need a writable
    /// workspace with no base content.
    Empty,
}

pub enum OnExhaustion {
    /// Sandbox is destroyed on cap-breach; run transitions to Failed with
    /// RunFailureReason::ResourceLimitExceeded. Default for production runs.
    Destroy,
    /// Sandbox is preserved on cap-breach; run transitions to WaitingApproval
    /// so the operator can raise the cap and resume, or cancel.
    PauseAwaitOperator,
    /// Sandbox and run continue running after cap-breach. Only the
    /// SandboxResourceLimitExceeded event fires as an advisory signal. Intended
    /// for staging/debug runs where the operator wants to observe cap-breaches
    /// without blocking work.
    ReportOnly,
}
```

**Generality**: `SandboxBase::Directory` and `SandboxBase::Empty` cover non-code use cases. A research agent that just needs a scratch directory uses `Empty`. A data agent that needs to mutate a CSV in place uses `Directory`. A code agent uses `Repo` keyed by `owner/repo` (via the `RepoStore`). The same state machine, events, and recovery semantics apply to all three.

### SandboxBase::Directory access invariant (v1)

`SandboxBase::Directory` is **operator- or system-authored policy only**. In v1, cairn-runtime exposes no agent-controlled path for constructing or mutating a `SandboxPolicy`, so agents cannot select the base their sandbox runs against — they receive whatever policy the run was created with. A run's `SandboxPolicy` is constructed at run-creation time (by the operator, a `RunTemplate` per RFC 022, or a system-trusted code path such as the orchestrator's plan-mode bootstrap) and is immutable for the run's lifetime.

An operator who authors a policy exposing sensitive host paths (`/etc`, `/home`, `/var`) is explicitly accepting that scope as part of the run's reachable state. This is consistent with the general cairn principle that operator-authored configuration is trusted. The invariant is a v1 contract; it is not a claim that the cairn-runtime implementation currently enforces it at every call site (which is future implementation work), but rather that this RFC and the dependent RFCs do not authorize agent-driven `SandboxBase` selection.

This invariant is reopened explicitly if a future RFC (likely an RFC 022 revision) allows template parameterization of sandbox fields. Verified at v1 lock-in: RFC 022 `RunTemplate` currently carries only `sandbox_hint` (not a mutable full `SandboxPolicy`) and variable substitution is scoped to `system_prompt` and `initial_user_message` — no sandbox field is interpolated from agent-supplied values. Any future work that changes this must re-assess the agent-selectability boundary explicitly.

## RepoStore

The `RepoStore` is split into two cleanly separated services in `cairn-workspace`, composed by an optional thin facade:

- **`RepoCloneCache`** — **tenant-scoped** physical clone state on disk. One clone per `(tenant_id, repo_id)`; multiple projects within the same tenant that authorize the same repo share the same physical clone (disk economy).
- **`ProjectRepoAccessService`** — **project-scoped** authorization. A project's `ProjectRepoAllowlist` determines which `repo_id`s agents and policies in that project are permitted to reference. `SandboxBase::Repo { repo_id }` is rejected at `RepoStore::resolve()` if `repo_id` is not in the current project's allowlist, **even if the physical clone already exists** because another project in the tenant allowlisted it first.
- **`RepoStore`** — optional facade composing both. Exists only for runtime code that needs a single handle; the public contract is the two underlying services.

This split resolves the tension between disk economy (tenant-scoped cache) and RFC 015's per-project isolation promise (project-scoped access). Earlier drafts conflated the two concepts under a single tenant-keyed `HashMap<TenantId, HashSet<String>>`, which would have let project A's `cairn.registerRepo("org/secret")` silently make `org/secret` resolvable from project B in the same tenant — a direct violation of the RFC 015 isolation boundary just sealed.

### Context type

The access layer takes a minimal context type owned by `cairn-domain`:

```rust
// cairn-domain/src/contexts.rs
pub struct RepoAccessContext {
    pub project: ProjectKey,
}

// Thin projection from the full VisibilityContext (also in cairn-domain) for
// callers that already have one. cairn-workspace NEVER imports VisibilityContext
// — it only sees RepoAccessContext, keeping plugin/tool concerns out of the
// workspace crate. Both types living in cairn-domain is required for Rust trait
// coherence: the From impl cannot live in cairn-workspace without pulling in
// VisibilityContext, which is the very dependency we are avoiding.
impl From<&VisibilityContext> for RepoAccessContext {
    fn from(vc: &VisibilityContext) -> Self {
        Self { project: vc.project.clone() }
    }
}
```

**Implementer note**: `VisibilityContext` does not yet exist in `crates/`; it is declared in RFCs 015/017/018/019 and will land with the RFC 015 implementation. Either RFC 015 or RFC 016 implementation may land first; whichever does creates `cairn-domain/src/contexts.rs` with `VisibilityContext`, `RepoAccessContext`, and the `From` impl together in one commit. `ProjectKey` already lives at `crates/cairn-domain/src/tenancy.rs`.

### RepoCloneCache (tenant-scoped physical clone layer)

```rust
pub struct RepoCloneCache {
    base_dir: PathBuf,                                     // e.g. $XDG_DATA_HOME/cairn/repos/
    clone_locks: RwLock<HashMap<(TenantId, RepoId), Arc<Mutex<()>>>>,  // serialize concurrent clones
    event_sink: Arc<dyn EventSink>,
}

impl RepoCloneCache {
    /// Filesystem path of the locked clone for a given (tenant, repo_id).
    /// Internal-only — external callers must go through RepoStore::resolve
    /// which applies the project allowlist check first. No Arc<RepoCloneCache>
    /// escape hatch for external code.
    pub(crate) fn path(&self, tenant: &TenantId, repo_id: &RepoId) -> PathBuf;

    /// Ensure the tenant-scoped clone exists. Idempotent: no-op if already
    /// cloned and locked. First call per (tenant, repo_id) clones via gh and
    /// applies `chmod -R a-w`; emits RepoCloneCreated on success.
    pub async fn ensure_cloned(
        &self,
        tenant: &TenantId,
        repo_id: &RepoId,
    ) -> Result<(), RepoStoreError>;

    /// True if the physical clone exists on disk for the tenant.
    pub async fn is_cloned(&self, tenant: &TenantId, repo_id: &RepoId) -> bool;

    /// Refresh an existing clone to a new HEAD. See "Locked clone immutability
    /// invariant" below. This is the ONLY supported mutation path for a clone
    /// after it has been locked; out-of-band mutations (`chmod +w`, manual git
    /// operations) are invariant violations that the drift-detection path will
    /// catch on the next recovery.
    pub async fn refresh(
        &self,
        tenant: &TenantId,
        repo_id: &RepoId,
    ) -> Result<RefreshOutcome, RepoStoreError>;

    /// The full set of cloned (tenant, repo_id) pairs on disk. Used by the
    /// async clone GC sweep and by the EnsureAllCloned startup pass.
    pub async fn cloned_set(&self) -> HashSet<(TenantId, RepoId)>;
}

pub struct RefreshOutcome {
    pub old_head: String,
    pub new_head: String,
    pub drifted_sandboxes: Vec<SandboxId>,   // sandboxes whose meta.base_rev == old_head
}
```

### ProjectRepoAccessService (project-scoped authorization layer)

```rust
pub struct ProjectRepoAccessService {
    // project-keyed allowlist; ProjectKey = { tenant_id, workspace_id, project_id }
    allowed: RwLock<HashMap<ProjectKey, HashSet<RepoId>>>,
    event_sink: Arc<dyn EventSink>,
}

impl ProjectRepoAccessService {
    /// True if the given repo_id is in the project's allowlist.
    pub async fn is_allowed(&self, ctx: &RepoAccessContext, repo_id: &RepoId) -> bool;

    /// Grant a project access to a repo_id. Emits ProjectRepoAllowlistExpanded.
    /// Does NOT trigger a clone by itself — the clone is ensured lazily on the
    /// next RepoStore::resolve call, or eagerly by the EnsureAllCloned startup
    /// pass for repos already in any project's allowlist at boot.
    pub async fn allow(
        &self,
        ctx: &RepoAccessContext,
        repo_id: &RepoId,
        by: ActorRef,
    ) -> Result<(), RepoStoreError>;

    /// Revoke a project's access to a repo_id. Emits ProjectRepoAllowlistShrunk.
    /// Does NOT delete the physical clone on disk — other projects in the same
    /// tenant may still reference it, and the async clone GC sweep handles
    /// physical deletion when the reference count reaches zero AND no active
    /// sandboxes reference the repo.
    pub async fn revoke(
        &self,
        ctx: &RepoAccessContext,
        repo_id: &RepoId,
        by: ActorRef,
    ) -> Result<(), RepoStoreError>;

    /// List repos currently allowlisted for the project.
    pub async fn list_for_project(&self, ctx: &RepoAccessContext) -> Vec<RepoId>;

    /// Full project-allowlist map across all projects in all tenants. Used by
    /// the EnsureAllCloned startup pass to derive the distinct (tenant, repo_id)
    /// set to pre-clone.
    pub async fn list_all(&self) -> HashMap<ProjectKey, Vec<RepoId>>;
}
```

### RepoStore (optional composition facade)

```rust
pub struct RepoStore {
    cache: Arc<RepoCloneCache>,
    access: Arc<ProjectRepoAccessService>,
}

impl RepoStore {
    /// The ONLY public entry point into physical-clone paths from outside
    /// cairn-workspace. Access check first, then clone ensure, then path lookup.
    pub async fn resolve(
        &self,
        ctx: &RepoAccessContext,
        repo_id: &RepoId,
    ) -> Result<PathBuf, RepoStoreError> {
        if !self.access.is_allowed(ctx, repo_id).await {
            return Err(RepoStoreError::NotAllowedForProject {
                project: ctx.project.clone(),
                repo_id: repo_id.clone(),
            });
        }
        let tenant = &ctx.project.tenant_id;
        self.cache.ensure_cloned(tenant, repo_id).await?;
        Ok(self.cache.path(tenant, repo_id))
    }
}
```

### Storage layout

```
{base_dir}/repos/
  {tenant_id}/                 # <-- tenant segment is REQUIRED; two tenants
    {owner}/                   #     both allowlisting github.com/org/foo get
      {repo}/                  #     separate physical clones. No tenant
        .git/                  #     isolation violation at the disk layer.
        ...repo files
```

The `chmod -R a-w` lock is what enforces the immutable lower layer: even root inside the sandbox can read but not modify the clone (overlay's `lowerdir` is also read-only by mount option, but the chmod adds defense in depth). Defense-in-depth matters here because the tenant cache is shared across all projects within the tenant, and a buggy build script in one project's sandbox must not be able to corrupt the cache for every other project.

### Locked clone immutability invariant

Once `RepoCloneCache` has cloned and locked a repo (emitting `RepoCloneCreated` and `RepoCloneLocked`), the clone's HEAD **must not change** for the lifetime of the cairn-app process except via an explicit `RepoCloneCache::refresh()` call. This is a correctness invariant for `OverlayProvider`: the overlay's `lowerdir` is a fixed path pointing at the locked clone; if the clone's HEAD were to move under the upper layer (via `chmod +w` + manual git operations, a maintenance script running as root, a buggy build step, etc.), an active sandbox's upper-layer mutations would silently be applied over a different base than the one the agent was reasoning about. The delta becomes nonsensical.

`RepoCloneCache::refresh(tenant, repo_id)` is the only supported HEAD-movement path. The refresh sequence is:

1. Acquire the `clone_lock` for `(tenant, repo_id)`.
2. `chmod -R u+w` to unlock.
3. `git fetch && git reset --hard <new_head>`.
4. `chmod -R a-w` to re-lock.
5. Emit `RepoStoreRefreshed { tenant, repo_id, old_head, new_head, at }`.
6. For every existing sandbox on disk whose `meta.base_rev == old_head`, emit `SandboxBaseRevisionDrift` and transition the sandbox to `Preserved { reason: PreservationReason::BaseRevisionDrift { expected: old_head, actual: new_head } }`. These sandboxes require operator action (resume against new HEAD or abandon) before they can accept new tool calls.

Any filesystem-level mutation outside the `refresh()` path is an **invariant violation**. The RFC does not rely on filesystem watchers to catch such violations; it relies on the `chmod -R a-w` enforcement plus the drift-detection check on sandbox recovery. A maintenance script that bypasses the chmod is operator error, and the next recovery pass will catch the drift when an overlay sandbox is reattached.

**Reflink provider exception**: `ReflinkProvider` sandboxes are physically independent of the source clone after provisioning — the reflinked copy has its own filesystem identity, and later mutations to the source clone do not propagate into any existing reflinked sandbox. The drift detection described above therefore applies **only** to `OverlayProvider` sandboxes; `ReflinkProvider` sandboxes are exempt and do not emit `SandboxBaseRevisionDrift`. See "Reflink snapshot-semantics requirement" below for the filesystem-level invariant that makes this exemption sound.

### Reflink snapshot-semantics requirement

`ReflinkProvider` is valid **only** on filesystems that provide snapshot-like semantics on clone: specifically, that a reflinked tree is logically independent of the source tree from the point of the clone onward, so that later mutations to the source do not alter the clone. This is the behavior of APFS `clonefile` on macOS, ReFS Block Cloning on Windows Server 2016+ and Windows 11 Dev Drive, and XFS/btrfs reflink when the source and target inodes are independent.

Implementations **must not** advertise `ReflinkProvider` as valid for filesystems where the reflink semantics are not snapshot-like (for example, any reference-counted shared-page layout that could retroactively propagate source mutations to a clone). Implementations that cannot make this guarantee must either fall back to `OverlayProvider` or fail the sandbox policy at provision time with a clear error.

This is a **correctness invariant**, not a performance optimization. A `ReflinkProvider` that violated it would silently apply the agent's upper layer over a moved base during recovery — the exact hazard the `OverlayProvider` drift-detection path catches explicitly. `ReflinkProvider`'s exemption from drift detection is load-bearing on this invariant holding.

### Operator HTTP contract for project repo access

Operators need a surface for listing, adding, and revoking project repo access outside the agent-driven `cairn.registerRepo` path. The `DELETE` path also matters for the v0 → v1 migration: existing tenant-scoped allowlist entries are migrated to every project under the tenant as a permissive default on first v1 boot (emitting `ProjectRepoAllowlistExpanded { added_by: SystemMigration }` per project), and operators then use the HTTP + UI surface to review and revoke entries that should not have been copied.

```
GET    /v1/projects/:project/repos
           List the project's repo allowlist entries with metadata
           (added_by, added_at, physical_clone_status, last_used_at).

POST   /v1/projects/:project/repos
           Body: { "repo_id": "owner/repo" }
           Operator-initiated allowlist expansion. Goes through the same
           RFC 019 decision-layer path as cairn.registerRepo but without the
           agent context. Emits ProjectRepoAllowlistExpanded on success.

GET    /v1/projects/:project/repos/:owner/:repo
           Drill-in: allowlist metadata, recent sandbox usage, recent
           cairn.registerRepo decisions, current physical clone status.

DELETE /v1/projects/:project/repos/:owner/:repo
           Revoke the project's allowlist entry ONLY. Emits
           ProjectRepoAllowlistShrunk and returns 204. Does NOT delete the
           physical clone synchronously — that is handled by the async clone
           GC sweep when the tenant reference count reaches zero AND no
           active sandboxes reference the repo.
```

**Route note on path segments**: the `:owner/:repo` split (rather than a single `:repo_id` segment) is necessary because GitHub repo IDs are of the form `owner/repo` and the `/` breaks single-segment routing. UI and client code that constructs these URLs must not URL-encode the `/` into `%2F`.

### Async physical clone GC

Physical clones are garbage-collected lazily by a background sweep, not synchronously on `DELETE`. The sweep runs at a configurable interval (default: 1 hour, `[sandbox] clone_sweep_interval` in config) and deletes a `(tenant, repo_id)` clone only when **both** conditions hold:

1. **Zero projects** in the tenant have `repo_id` in their allowlist.
2. **Zero active sandboxes** reference `repo_id`.

The two-condition check avoids the race where a fresh `POST /repos` for the same `repo_id` arrives immediately after a `DELETE`; the operator can re-allowlist before the sweep runs and the physical clone is preserved. It also avoids destroying a clone that has been removed from every project's allowlist but is still in use by an in-flight sandbox whose policy snapshot captured the grant.

The sweep's inputs straddle two crates — `RepoCloneCache::cloned_set()` is native to `cairn-workspace` while the active-sandbox enumeration lives in `cairn-runtime::SandboxService` — so `cairn-workspace` defines a minimal read trait and `cairn-runtime` provides the implementation:

```rust
// cairn-workspace/src/clone_gc.rs
#[async_trait]
pub trait ActiveSandboxRepoSource: Send + Sync {
    /// Return the set of (tenant, repo_id) pairs currently referenced by any
    /// active sandbox. Used by the clone GC sweep to check the second
    /// deletion precondition.
    async fn active_repo_references(&self) -> Result<HashSet<(TenantId, RepoId)>, SweepError>;
}

pub struct RepoCloneSweepTask {
    cache:          Arc<RepoCloneCache>,
    access:         Arc<ProjectRepoAccessService>,
    sandbox_source: Arc<dyn ActiveSandboxRepoSource>,
    event_sink:     Arc<dyn EventSink>,
    interval:       Duration,   // default 1 hour
}

impl RepoCloneSweepTask {
    pub async fn run_loop(self) {
        loop {
            let sweep_id = SweepId::new();
            self.emit(RepoCloneSweepStarted { sweep_id, started_at: now() });

            let cloned     = self.cache.cloned_set().await;
            let allowlists = self.access.list_all().await;
            let active     = self.sandbox_source.active_repo_references().await?;

            let referenced_by_allowlist: HashSet<(TenantId, RepoId)> =
                allowlists.iter()
                    .flat_map(|(proj, repos)|
                        repos.iter().map(move |r| (proj.tenant_id.clone(), r.clone())))
                    .collect();

            let mut deleted = 0;
            let mut skipped_active_sandboxes = 0;
            let mut skipped_active_allowlists = 0;

            for pair in &cloned {
                if active.contains(pair) {
                    skipped_active_sandboxes += 1;
                    continue;
                }
                if referenced_by_allowlist.contains(pair) {
                    skipped_active_allowlists += 1;
                    continue;
                }
                // Acquires clone_lock internally; races with concurrent
                // ensure_cloned are resolved by the lock.
                self.cache.delete(&pair.0, &pair.1).await?;
                deleted += 1;
            }

            self.emit(RepoCloneSweepCompleted {
                sweep_id,
                deleted,
                skipped_active_sandboxes,
                skipped_active_allowlists,
                completed_at: now(),
            });

            tokio::time::sleep(self.interval).await;
        }
    }
}
```

`cairn-runtime::SandboxService` implements `ActiveSandboxRepoSource` by walking its active-sandbox map and extracting `(tenant, repo_id)` from any `SandboxBase::Repo` policy. `SandboxBase::Directory` and `SandboxBase::Empty` contribute nothing to the sweep's active set.

The sweep's delete-vs-concurrent-provision race is resolved by the existing `clone_lock` per-repo mutex: `RepoCloneCache::delete` acquires the lock before deletion, and `RepoCloneCache::ensure_cloned` acquires the lock before creation. A concurrent provision either (a) acquires the lock first, blocks the sweep's delete, and wins; or (b) acquires the lock after the sweep, finds no clone, and triggers re-clone. Both outcomes are correct — no sandbox is ever pointed at a mid-deletion clone.

**Placement**: the sweep task and the `ActiveSandboxRepoSource` trait live in `cairn-workspace`. The trait impl lives in `cairn-runtime`. Wire-up happens in `cairn-app` at startup (`tokio::spawn(sweep.run_loop())`). This keeps the physical-clone state management local to the crate that owns it while using dependency injection for the cross-crate data source.

### `cairn.registerRepo` built-in tool

A new built-in tool in `cairn-tools/src/builtins/register_repo.rs`:

```rust
pub struct RegisterRepoTool {
    access:     Arc<ProjectRepoAccessService>,
    cache:      Arc<RepoCloneCache>,
}

#[async_trait]
impl ToolHandler for RegisterRepoTool {
    fn name(&self) -> &str { "cairn.registerRepo" }
    fn tier(&self) -> ToolTier { ToolTier::Registered }
    fn effect(&self) -> ToolEffect { ToolEffect::External }
    // External effect because expanding the project's allowlist changes
    // per-project configuration. RFC 019 decision layer requires approval
    // by default; the decision cache key is (project, repo_id) so project A's
    // approval does NOT auto-grant project B the same access.

    fn parameters_schema(&self) -> Value { /* { repo_id: "owner/repo" } */ }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> ToolResult {
        let repo_id: RepoId = args["repo_id"].as_str()
            .ok_or_else(|| error(...))?
            .parse()?;
        let access_ctx = RepoAccessContext { project: ctx.project.clone() };

        // 1. Add to the CURRENT project's allowlist. If the physical clone
        //    already exists for another project in the tenant, only the
        //    allowlist entry is added. If not, the clone is also created.
        self.access.allow(
            &access_ctx,
            &repo_id,
            ActorRef::Run(ctx.run_id.clone()),
        ).await?;

        // 2. Ensure the physical clone exists for the tenant. Idempotent —
        //    no-op if another project already triggered the clone.
        self.cache.ensure_cloned(&ctx.project.tenant_id, &repo_id).await?;

        // Return authorization/clone status only. NOT the host clone path —
        // agents should learn that access was granted, not that a specific
        // host path exists outside the run sandbox contract.
        Ok(format!(
            "Registered {} for project {}. The repo will be available in \
             future runs at the base path the sandbox service hands the run.",
            repo_id, ctx.project.display()
        ).into())
    }
}
```

The decision layer (RFC 019) intercepts every `cairn.registerRepo` call and applies the project's policy: in most cases, registering a new repo requires explicit operator approval, because expanding the agent's repository access is a meaningful capability grant. After the first approval, the decision cache (RFC 019) can auto-approve subsequent calls for the same `(project, repo_id)` if the operator opts in. **The decision cache key is `(project, repo_id)` specifically — not `(tenant, repo_id)` — so project A's approval of `org/foo` does not auto-grant project B the same access.** This is consistent with RFC 015's per-project isolation promise.

### RepoStore events

```rust
pub enum RepoStoreEvent {
    // Project allowlist events (project-scoped)
    ProjectRepoAllowlistExpanded {
        project: ProjectKey,
        repo_id: RepoId,
        added_by: ActorRef,   // Run(run_id) | Operator(op_id) | SystemMigration
        at: u64,
    },
    ProjectRepoAllowlistShrunk {
        project: ProjectKey,
        repo_id: RepoId,
        removed_by: ActorRef,
        at: u64,
    },

    // Physical clone events (tenant-scoped)
    RepoCloneCloning {
        tenant: TenantId,
        repo_id: RepoId,
        started_at: u64,
    },
    RepoCloneCreated {
        tenant: TenantId,
        repo_id: RepoId,
        path: PathBuf,
        duration_ms: u64,
        at: u64,
    },
    RepoCloneFailed {
        tenant: TenantId,
        repo_id: RepoId,
        error: String,
        failed_at: u64,
    },
    RepoCloneLocked {
        tenant: TenantId,
        repo_id: RepoId,
        at: u64,
    },
    RepoCloneDeleted {
        tenant: TenantId,
        repo_id: RepoId,
        sweep_id: Option<SweepId>,   // None if deleted via direct operator action
        at: u64,
    },

    // Refresh (locked-clone immutability gate)
    RepoStoreRefreshed {
        tenant: TenantId,
        repo_id: RepoId,
        old_head: String,
        new_head: String,
        drifted_sandbox_count: u32,  // number of sandboxes transitioned to Preserved
        at: u64,
    },

    // Async clone GC sweep observability
    RepoCloneSweepStarted {
        sweep_id: SweepId,
        started_at: u64,
    },
    RepoCloneSweepCompleted {
        sweep_id: SweepId,
        deleted: u32,
        skipped_active_sandboxes: u32,
        skipped_active_allowlists: u32,
        completed_at: u64,
    },
}
```

### Trait Interface

```rust
/// A provisioned sandbox. The agent receives `path` as its working directory.
pub struct ProvisionedSandbox {
    pub sandbox_id:  SandboxId,
    pub run_id:      RunId,
    pub path:        PathBuf,
    pub base:        SandboxBase,
    pub strategy:    SandboxStrategy,   // actual, after degrade resolution
    pub branch:      Option<String>,    // git branch name if Repo
    pub is_resumed:  bool,
    pub env:         HashMap<String, String>, // credentials + base env for the agent process
}

#[async_trait]
pub trait SandboxProvider: Send + Sync + 'static {
    fn strategy(&self) -> SandboxStrategy;

    /// Provision a new sandbox for the given run. Idempotent: if a sandbox
    /// already exists for run_id under this provider, return it with is_resumed=true.
    async fn provision(
        &self,
        run_id: &RunId,
        policy: &SandboxPolicy,
    ) -> Result<ProvisionedSandbox, WorkspaceError>;

    /// Reconnect to an existing sandbox after control-plane restart.
    /// Returns Ok(None) if no sandbox exists on disk for this run_id.
    async fn reconnect(
        &self,
        run_id: &RunId,
    ) -> Result<Option<ProvisionedSandbox>, WorkspaceError>;

    /// Checkpoint the sandbox: preserve mutation layer for potential resume.
    async fn checkpoint(
        &self,
        run_id: &RunId,
        kind: SandboxCheckpointKind,
    ) -> Result<SandboxCheckpoint, WorkspaceError>;

    /// Restore from a prior checkpoint (create a fresh sandbox seeded from it).
    async fn restore(
        &self,
        from_checkpoint: &SandboxCheckpoint,
        new_run_id: &RunId,
        policy: &SandboxPolicy,
    ) -> Result<ProvisionedSandbox, WorkspaceError>;

    /// Destroy the sandbox. If preserve=true, keep checkpoint state for future restore.
    async fn destroy(
        &self,
        run_id: &RunId,
        preserve: bool,
    ) -> Result<DestroyResult, WorkspaceError>;

    /// List all known sandboxes for this provider (crash recovery scan).
    async fn list(&self) -> Result<Vec<SandboxHandle>, WorkspaceError>;

    /// Heartbeat: update last-seen timestamp.
    async fn heartbeat(&self, run_id: &RunId) -> Result<(), WorkspaceError>;
}

/// Top-level service that owns all providers and dispatches by policy.
pub struct SandboxService {
    providers: HashMap<SandboxStrategy, Box<dyn SandboxProvider>>,
    event_sink: Arc<dyn EventSink>,  // emits SandboxEvent into the runtime event log
    base_dir:   PathBuf,
    clock:      Arc<dyn Clock>,
}
```

### Provider Resolution

The policy asks for a preferred or forced strategy. The `SandboxService` resolves it:

- **Linux host**: defaults to `OverlayProvider`. Requires kernel ≥ 5.11 for unprivileged overlay mounts, OR cairn-app has `CAP_SYS_ADMIN`. If neither, falls back to `ReflinkProvider` (which works on Linux too via XFS/btrfs reflink); if reflink is also unavailable, emits `SandboxPolicyDegraded` and refuses to provision.
- **macOS host**: uses `ReflinkProvider` directly (APFS `clonefile` is universally available).
- **Windows host**: uses `ReflinkProvider`. On Windows 11 24H2+ with ReFS / Dev Drive, the underlying clone uses ReFS Block Cloning; on NTFS, falls back to a full directory copy (slow but functional).
- `Preferred(strategy)` allows degradation; `Force(strategy)` requires the named strategy or fails the provision.

Both providers expose the same `SandboxProvider` trait with identical observable semantics. The agent receives a `ProvisionedSandbox` and does not know which provider is in use. Code paths in the orchestrator and agent loop are platform-agnostic.

The selected strategy is recorded in `SandboxProvisioned.strategy` and any fallback is recorded in `SandboxPolicyDegraded` so operators can see why a sandbox ended up with a different strategy than requested.

## Storage Layout

Sandbox state on disk lives under a base directory (configurable; default `~/.local/share/cairn/sandboxes/` in local mode, `/var/lib/cairn/sandboxes/` in team mode):

```
{base_dir}/
  {sandbox_id}/
    meta.json              # SandboxMetadata serialized
    strategy               # "overlay_fs" | "reflink"

    # OverlayFS strategy (Linux):
    upper/                 # agent's mutation layer
    work/                  # overlayfs internal scratch
    merged/                # merged view; this is the path given to the agent
    upper.prev.0/          # preserved upper from previous run on this sandbox
    upper.prev.1/
    cgroup                 # cgroup path for resource limits

    # Reflink strategy (macOS/Windows, also Linux fallback):
    root/                  # reflinked copy of the base; this is the path given to the agent
```

`meta.json` is the source of truth for reconnection:

```json
{
  "sandbox_id":   "sbx_abc123",
  "run_id":       "run_xyz",
  "task_id":      "task_qrs",
  "project":      { "tenant_id": "...", "workspace_id": "...", "project_id": "..." },
  "strategy":     "overlay_fs",
  "state":        "active",
  "base_rev":     "a3f9b21c",
  "repo_id":      "octocat/hello",
  "path":         "/.../sbx_abc123/merged",
  "pid":          12345,
  "created_at":   1775759896876,
  "heartbeat_at": 1775759950000,
  "policy_hash":  "sha256:..."
}
```

## Reconnect on Restart

When cairn-app starts, the recovery sequence (per RFC 020) runs `RepoCloneCache::ensure_all_cloned()` and `SandboxService::recover_all()` **in parallel where independent** — sandbox recovery does not depend on every clone being complete, only on the specific repos referenced by recovering sandboxes. Total recovery time is bounded by the longer of the two, not the sum. (See RFC 020 for the resolved startup ordering.)

```text
1. RepoCloneCache.ensure_all_cloned()
   // Iterate the DISTINCT (tenant, repo_id) set derived from all project
   // allowlists via ProjectRepoAccessService::list_all(), NOT a single
   // tenant-level allowlist. Deduplicates by (tenant, repo_id) since the
   // physical clone is tenant-scoped even though the access grants are
   // project-scoped.
   let all_projects = project_access.list_all().await;
   let distinct_clones: HashSet<(TenantId, RepoId)> =
       all_projects.iter()
           .flat_map(|(proj, repos)| repos.iter().map(move |r| (proj.tenant_id.clone(), r.clone())))
           .collect();
   for (tenant, repo_id) in distinct_clones:
       if not is_git_repo(cache.path(&tenant, &repo_id)):
           clone via gh repo clone into {base}/repos/{tenant}/{owner}/{repo}
           chmod -R a-w
           emit RepoCloneCreated { tenant, repo_id, at }

2. SandboxService.recover_all()
   for each subdir in sandbox_base_dir:
       read meta.json
       look up run_id in the task/run engine:

         Run.state == Running|Paused|WaitingApproval → try to reconnect:

           // STEP 2a: Project allowlist check (F-W2-01 recovery invariant)
           // If the project's allowlist no longer includes this repo (revoked
           // between crash and restart), the sandbox is preserved for operator
           // review; previously-authorized work is not retroactively invalidated
           // but the run stops accepting new tool calls without fresh operator
           // action.
           if meta.base is Repo { repo_id } AND
              not project_access.is_allowed(meta.project, repo_id):
               emit SandboxAllowlistRevoked { sandbox_id, run_id, project, repo_id, ... }
               transition sandbox → Preserved { reason: AllowlistRevoked { repo_id } }
               continue  // next sandbox

           // STEP 2b: OverlayProvider base-revision drift check (F-W3-15 invariant)
           // ONLY for OverlayProvider — ReflinkProvider sandboxes are physically
           // independent of the source clone post-provision and are exempt.
           if meta.strategy == overlay_fs AND meta.base_rev is not None:
               let current_head = git_head(cache.path(meta.tenant, meta.repo_id))
               if current_head != meta.base_rev:
                   emit SandboxBaseRevisionDrift { expected: meta.base_rev, actual: current_head, ... }
                   transition sandbox → Preserved { reason: BaseRevisionDrift { expected, actual } }
                   continue  // next sandbox

           // STEP 2c: Provider-specific reattachment (unchanged from previous draft)
           if provider is OverlayProvider:
             verify mounts still exist
             if yes: reattach; emit SandboxHeartbeat
             if no: attempt remount from preserved upper.prev.N layers
                    on failure: emit SandboxPreserved, mark for operator intervention
           if provider is ReflinkProvider:
             verify the reflinked dir still exists at meta.path
             if yes: reattach; emit SandboxHeartbeat
             if no: emit SandboxProvisioningFailed, mark run as needing recovery

         Run.state == Completed|Failed|Cancelled → orphan cleanup:
           destroy the sandbox, emit SandboxDestroyed { reason: Stale }
         Run not found → grace period (24h by default) then destroy
```

Recovery cooperates with `RecoveryServiceImpl::recover_all()` from `cairn-runtime`. The two recovery flows surface different aspects of the same state transition: the sandbox recovery surfaces filesystem reattachment, the runtime recovery surfaces run/task state. The event log is the shared source of truth.

## Credential Injection

Tokens arrive in the sandbox via environment variables set at process spawn time, never written to disk. The exception is git credentials, which must be discoverable by `git push` / `git clone` — for these, cairn uses the `GIT_ASKPASS` pattern:

1. At provision time, the sandbox service resolves the credential references in the policy to short-lived tokens (e.g. a GitHub App installation token valid for 1 h)
2. A tiny helper binary (compiled into cairn-app via `arg0` routing, same pattern cairn-go uses for `--sandbox-exec`) is made available in the sandbox's `PATH`
3. The helper binary reads a short-lived JSON file scoped to the sandbox ID and prints the token when git asks for credentials
4. `GIT_ASKPASS` points to the helper binary, `GIT_TERMINAL_PROMPT=0` ensures git does not fall through to interactive prompts
5. The JSON file is unlinked as soon as the sandbox transitions to `Destroying`

**Variant for non-git credentials**: standard environment variables (`GITHUB_TOKEN`, `AWS_SESSION_TOKEN`, whatever) are injected into the agent process's environment at spawn time. They never hit disk. They are included in the `env` field of `ProvisionedSandbox` and scrubbed from any log or event that reports the env map.

**Audit trail**: `SandboxProvisioned.policy` records the `CredentialReference` (by ID, not value), giving a durable audit of which credential was used for which sandbox.

## Recovery Semantics

Five scenarios the sandbox layer must handle:

| Scenario | State on disk | Action |
|---|---|---|
| Control plane crash, agent still running | meta.json exists, pid alive | Reconnect in `Active`, resume heartbeat |
| Control plane crash, agent also died | meta.json exists, pid dead | Checkpoint to `Preserved`, let the task engine decide whether to resume or abandon |
| Agent completed normally, control plane crashed before marking complete | meta.json exists, no pid, git shows committed work | Mark complete, destroy sandbox, emit `SandboxDestroyed { reason: Completed }` |
| Network partition, lease expired | meta.json exists, lease expired in task engine | Lease reaper re-queues the task; next claim attempt reattaches sandbox via `reconnect()` |
| Host rebooted (Linux + overlay) | mounts are gone, upper/work directories survive | Attempt remount from preserved upper.prev.N layers; on failure, emit SandboxPreserved |
| Host rebooted (macOS/Windows + reflink) | cloned dir survives reboot | Reattach directly; no remount needed |

Recovery must be idempotent — RFC 005's recovery rule. Calling `recover_all()` twice produces the same final state.

## Dependency Policy: Vendor AI-Ecosystem Code, Depend On Everything Else

Cairn has a specific dependency stance:

- **AI-ecosystem code is vendored, not depended on.** Cairn does not take a Cargo dependency on any crate whose maintenance tracks the release cadence of an AI-ecosystem project (OpenAI, Anthropic, LangChain, AutoGen, etc.). If we need code from one of those, we strip what we need, vendor it under `vendor/`, preserve attribution, and own the maintenance forever.
- **Generic open-source infrastructure crates are regular dependencies.** Crates like `landlock` (maintained by the Linux kernel Landlock team), `seccompiler` (maintained by rust-vmm / Firecracker), `libmount`, `reflink-copy`, `nix`, etc., are normal Cargo dependencies. They are infrastructure, not AI ecosystem. Their upstream cadence is independent of any LLM vendor.

This distinction is intentional: cairn must not let its roadmap drift because an AI-ecosystem upstream changed direction, deprecated a crate, or re-licensed. Generic infra crates have none of those risks.

### What gets vendored from `codex-linux-sandbox` (AI-ecosystem code)

The following **codex-specific code** lives under `crates/cairn-workspace/vendor/codex-linux-sandbox/`:

- **The bubblewrap invocation wrapper** — codex's code that composes the `bwrap` command line with the right `--ro-bind`, `--bind`, `--unshare-*`, `--new-session` flags for a read-only host filesystem with a writable sandbox root. This is codex's opinionated policy shape, not a generic bwrap binding.
- **Codex's Landlock policy builder** — codex's wrapper around the `landlock` crate that builds a `Ruleset` with codex's opinionated default read/write/execute paths (Go toolchain, `/tmp`, workspace root, etc.). We vendor the wrapper; we still depend on the upstream `landlock` crate normally for the actual kernel interaction.
- **Codex's seccomp default policy** — codex's ~80-syscall allowlist tuned for development workloads. We vendor the allowlist as data; we still depend on the upstream `seccompiler` crate to compile it to BPF.

### What does NOT get vendored

- **`landlock` crate** (upstream `landlock-lsm/rust-landlock`) — regular dependency, maintained by the kernel team
- **`seccompiler` crate** (upstream `rust-vmm/seccompiler`) — regular dependency, maintained by the Firecracker team
- **`libmount` crate** — regular dependency, generic mount syscall wrapper
- **`reflink-copy` crate** — regular dependency, generic CoW copy primitive
- **`nix` crate** — regular dependency, Unix syscall bindings
- **anything else in the Rust infrastructure ecosystem** — regular dependencies

### Why this distinction is the right one

The codex bubblewrap wrapper encodes product decisions ("what should a development sandbox allow") that track OpenAI's view of agent safety. If OpenAI changes their mind about which directories should be writable, we do not want cairn's sandbox behavior to change automatically with the next codex release. By vendoring, we lock in a snapshot and change it deliberately.

The `landlock` crate, by contrast, is a thin binding over a kernel API. Its behavior is defined by the Linux kernel, not by any AI vendor. Depending on it normally gives us the maintenance benefits of the crates.io ecosystem without coupling to AI-vendor decisions.

### Vendoring protocol

For each vendored piece:

- the upstream Apache 2.0 LICENSE file is preserved verbatim under the vendored directory
- a `VENDOR.md` in the vendored directory records: upstream commit SHA, vendoring date, list of files imported, list of local modifications
- the top-level cairn `NOTICE` file acknowledges the Apache 2.0 vendored code
- each vendored source file keeps its upstream license header unchanged
- a build-time checksum of the vendored source is recorded; changes to vendored code without updating the checksum emit a warning on `cargo build`

**First-pass import rule**: pull in the minimum surface that makes `OverlayProvider` work on Linux. The `ReflinkProvider` (macOS/Windows) does not need any vendored codex code — it uses the generic `reflink-copy` crate as a regular dependency. Everything else stays out. Future expansions are deliberate, not automatic.

**Upstream sync policy**: we do not auto-sync. A human reviews upstream changes on a deliberate cadence (quarterly is plenty) and pulls in specific fixes or improvements as explicit commits with clear rationale. Upstream deprecating the project does not affect us because we own the vendored code.

## Integration Points

### With RFC 005 (Task/Session/Checkpoint Lifecycle)

A sandbox is bound to a **run**, not a task. A run may survive across multiple task claims (pause, resume, worker-change) — the sandbox survives with it. When a task claims a run, the runtime service calls `SandboxService::provision_or_reconnect(run_id)` to get the working directory. When the run terminates, `SandboxService::destroy(run_id)` is called. Checkpoints defined in RFC 005 are complemented by `SandboxCheckpointed` events for filesystem state.

### With RFC 004 (Graph and Eval Matrix Model)

Sandbox events are projected into `cairn-graph` via `cairn-graph::event_projector`. This extends the graph schema with two new node kinds and several new edge kinds — the current `event_projector.rs` does not handle Sandbox or RepoBase node kinds, so this is new projection coverage that must be implemented alongside the sandbox event model.

New typed graph nodes (with opaque IDs — **no raw host paths as node_id or metadata fields**, to avoid leaking filesystem structure into agent/operator graph surfaces):

- **`GraphNode(Sandbox)`** — node_id: `sandbox:<sandbox_id>` (e.g. `sandbox:sbx_abc123`). Created on `SandboxProvisioned`. The `project` scope comes from the event's `project` field (not a separate graph node — `Project` is an existing RFC 008 scoping concept used as a query filter, not a new node kind). The sandbox's lifecycle is represented entirely through edges to events in the event log; the graph node itself carries only `node_id`, `kind`, `project`, and `created_at` — no additional derived attributes. Lifecycle queries ("is this sandbox destroyed? preserved?") join the graph node to the corresponding `SandboxDestroyed` or `SandboxPreserved` events in the event log, which carry the timestamps and reason fields.
- **`GraphNode(RepoBase)`** — node_id: `repobase:<tenant_id>:<owner>/<repo>@<base_rev>` (e.g. `repobase:acme:octocat/hello@a3f9b21c`). Created or reused when a `SandboxProvisioned` event references `SandboxBase::Repo { repo_id }` with a `base_revision`. Represents a specific version of a repo clone — two sandboxes on the same repo at the same revision share the same `RepoBase` node; a `RepoStoreRefreshed` event creates a new `RepoBase` node for the new HEAD.

**`SandboxBase::Directory` is NOT projected into the graph in v1.** The `SandboxBase::Directory` type carries only a raw `PathBuf` with no operator-authored stable alias. Projecting a raw absolute host path (e.g. `/var/data/csvs`) as a graph node_id would leak filesystem structure into agent/operator graph views. A future RFC may add an optional `alias: Option<String>` field to `SandboxBase::Directory` or to `SandboxPolicy`, at which point directory-based sandboxes could be projected as `GraphNode(DirectoryBase)` with `node_id: dirbase:<alias>`. Until that field exists, directory sandboxes are visible only in the raw `SandboxProvisioned` runtime event and the operator drill-in view.

New edges (between **existing** `Run` / `Checkpoint` node kinds from the RFC 005 graph projection and the **new** `Sandbox` / `RepoBase` node kinds):

- `Sandbox` --`provisioned_for`--> `Run` (from `SandboxProvisioned.run_id`)
- `Sandbox` --`based_on`--> `RepoBase` (from `SandboxProvisioned.policy.base` when base is `Repo`)
- `Sandbox` --`checkpoint_of`--> `Checkpoint` (from `SandboxCheckpointed`, linking to the RFC 005 checkpoint node)
- `RepoBase` --`refreshed_to`--> `RepoBase` (new revision, from `RepoStoreRefreshed { old_head, new_head }`)

Failure-path projection: `SandboxProvisioningFailed` creates a `Sandbox` node and a `provisioned_for` edge to the `Run` node. The failure details (`error_kind`, `error`) live on the `SandboxProvisioningFailed` event in the event log, not as attributes on the graph node (the v1 graph contract defines `node_id`, `kind`, `project`, `created_at` on `GraphNode` — no additional per-node attributes). Operators query "which runs failed to provision in the last 24h and why" by joining `Sandbox` nodes (via `provisioned_for` → `Run`) to `SandboxProvisioningFailed` events in the event log.

Operators query the graph via the existing `cairn-graph` query surface combined with event-log joins:

- "runs on this repo in the last 30 days" — traverse `Sandbox` --`based_on`--> `RepoBase` --`refreshed_to`--> chain
- "sandboxes derived from this base revision" — reverse `based_on` from a `RepoBase` node
- "preserved sandboxes pending intervention" — join `Sandbox` nodes to `SandboxPreserved` events in the event log (the graph provides the topology; the event log provides the lifecycle state)

No additional query APIs needed — this is pure event projection into existing graph infrastructure, following the same async-off-the-event-spine pattern as RFC 015's Signal Knowledge Capture. The graph provides navigable topology (which sandbox → which run → which repo base); the event log provides the full lifecycle detail. Neither system needs a schema extension beyond the declared node kinds and edge kinds above.

### With RFC 015 (Plugin Marketplace)

Plugins that run inside a sandbox (e.g. a shell-executing tool provider) do not get their own sandbox — they operate inside the run's sandbox. This means a plugin's tool calls inherit the run's resource limits, network policy, and credential scope. A plugin cannot escalate its sandbox.

### With RFC 011 (Deployment Shape)

- **Local mode**: `base_dir = $XDG_DATA_HOME/cairn/sandboxes/` (typically `~/.local/share/cairn/sandboxes/`); on Linux uses `OverlayProvider`, on macOS/Windows uses `ReflinkProvider`. Both work without root.
- **Team mode**: `base_dir = $XDG_DATA_HOME/cairn/sandboxes/` by default, overridable via `CAIRN_SANDBOX_ROOT` env var or `[sandbox] root` in config. On Linux uses `OverlayProvider` (kernel ≥ 5.11 unprivileged user-namespace mounts, or `CAP_SYS_ADMIN` if available). On non-Linux, uses `ReflinkProvider`.

### With existing `recovery_impl.rs`

`SandboxService::recover_all()` is called by the runtime startup path after the event log is replayed. It coordinates with `RecoveryServiceImpl::recover_all()` — runtime recovery processes lease expirations and run state transitions, sandbox recovery handles filesystem reconciliation.

## Non-Goals

For v1, explicitly out of scope:

- container runtime integration (Docker, Podman, containerd)
- microVM isolation (Firecracker, libkrun, cloud-hypervisor)
- distributed sandboxes across multiple hosts
- GPU access controls
- sandbox migration (move a running sandbox to a different host)
- persistent long-lived sandboxes that outlive runs (every sandbox belongs to exactly one run)
- a standalone `cairn-sandbox` CLI for inspecting sandboxes (inspection happens via the runtime event log and operator views)
- **Automatic indexing of `RepoStore` clone contents into `cairn-memory`**. Agents requiring retrieval over repo contents must use `file_read` and `grep_search` tools directly against the sandbox path, or call `memory_store` explicitly on relevant files during the run. A future RFC may add a `repo_memory_index` hook to `RepoCloneCache` that triggers `IngestService::submit` on clone; the current RFC does not pre-commit to that shape.
- **Agent-driven `SandboxBase` selection**. Agents can only work within the base the operator or policy author provisioned. `SandboxPolicy` is constructed at run-creation time and is immutable for the run's lifetime. Sandbox-field parameterization via agent-supplied template values is deferred to a future RFC with an explicit security review. See §"SandboxBase::Directory access invariant (v1)" above.

## Open Questions

1. **NEEDS DISCUSSION: Should cairn-app ever refuse to start if vendored sandbox code has drifted?** A checksum of the vendored source is embedded at build time. If the source has been modified locally without updating the checksum, should cairn-app refuse to boot, warn, or silently accept? Proposal: warn on stderr and continue; fail the build only if an explicit `--verify-vendor` flag is passed.

2. **NEEDS DISCUSSION: Base directory for sandboxes in team mode.** `/var/lib/cairn/sandboxes/` assumes root-writable paths; in many container deployments cairn-app runs as non-root with its own data dir. Proposal: configurable via `CAIRN_SANDBOX_ROOT` env var and `[sandbox] root = "..."` in config, with a sensible non-root default derived from `$XDG_DATA_HOME` when running as non-root, and `/var/lib/cairn/sandboxes/` when running as root.

3. **Resolved: Concurrent sandbox limit per host = 256.** Configurable. Runs that arrive while the cap is reached enter `waiting_dependency` with reason `sandbox_capacity`. (No further discussion needed; baked into the RFC.)

4. **NEEDS DISCUSSION: Divergence detection between concurrent sandboxes on the same base.** Cairn-go has an advisory `DivergenceMonitor` that detects when two sandboxes on the same base repo mutate overlapping files. Should cairn-rs port this, or skip and trust the run-level isolation? Proposal: port as advisory for v1 (do not block), surface in the operator view so runs that are likely to conflict are flagged.

5. **NEEDS DISCUSSION: OverlayFs provider on non-Linux hosts.** The provider is registered but `provision` fails immediately on macOS/Windows. Should the provider be compile-time excluded on non-Linux, or present-but-always-fallback? Proposal: compile-time excluded via `#[cfg(target_os = "linux")]` to avoid shipping dead code.

6. **Resolved**: resource exhaustion on sandboxes emits a dedicated `SandboxResourceLimitExceeded` discovery event with the tripped `ResourceDimension` (v1: `DiskBytes | MemoryBytes | WallClockMs`), followed by either `SandboxDestroyed { reason: ResourceLimitExceeded { dimension, limit, observed } }`, `SandboxPreserved { reason: AwaitingResourceRaise { dimension, limit, observed } }`, or no transition (`ReportOnly` mode), depending on the policy's `on_resource_exhaustion` field. See "Resource-exhaustion runtime flow" section above for the full state-machine specification. The run layer transitions independently per RFC 005 (`Failed` on Destroy, `WaitingApproval` on PauseAwaitOperator, no transition on ReportOnly). (No further discussion needed; baked into the RFC.)

7. ~~NEEDS DISCUSSION: Rescue-branch naming~~ — **Deleted**. Rescue branches (`rescue/{shortTaskID}`) were a git-worktree-era concept used to hold uncommitted work on an escape-valve branch. In the overlay/reflink model there is no branch-level rescue primitive — the preserved `upper.prev.N` layer IS the rescue state. This open question was dead; the model switch made it obsolete.

8. **NEEDS DISCUSSION: Should `SandboxProvisioned` include the resolved credentials' expiry time?** This helps the agent know when to request a refresh. Proposal: yes, include `credentials_expire_at: Option<u64>` so the orchestrator can schedule refresh calls before expiry.

## Decision

Proceed assuming:

- `cairn-workspace` is a new crate; its concept is `RunSandbox`, not "workspace" (to avoid collision with tenancy workspace)
- two providers ship in v1, both backed by `RepoStore`: `OverlayProvider` (Linux default) and `ReflinkProvider` (macOS + Windows; also Linux fallback when overlay is unavailable)
- git worktrees are dropped from v1 entirely; the overlay/reflink model supersedes worktree-per-task
- the `RepoStore` splits into two services: **`RepoCloneCache`** (tenant-scoped physical clone layer at `$base/repos/{tenant_id}/{owner}/{repo}/`, locked read-only after clone) and **`ProjectRepoAccessService`** (project-scoped access allowlist keyed by `ProjectKey`, consistent with RFC 015's per-project isolation). An optional thin `RepoStore` facade composes both for runtime code that needs a single handle
- `RepoAccessContext { project: ProjectKey }` lives in `cairn-domain` alongside `VisibilityContext` and a `From<&VisibilityContext>` impl; `cairn-workspace` imports only `RepoAccessContext` and never touches `VisibilityContext` (clean crate boundary)
- the `cairn.registerRepo` built-in tool expands the **current project's allowlist only** (not the tenant's); the RFC 019 decision cache key is `(project, repo_id)` — project A's approval does not auto-grant project B. The tool returns authorization/clone status, not a host path
- operator HTTP surface at `/v1/projects/:project/repos` with `:owner/:repo` path-split enables listing, adding, and revoking project allowlist entries independently of the agent `cairn.registerRepo` path
- physical clone GC is async via a background sweep with a `(zero projects allowlisted AND zero active sandboxes)` dual-precondition, owned by `cairn-workspace` via an `ActiveSandboxRepoSource` trait injected by `cairn-runtime`
- divergence detection is dropped from v1; the immutable lower layer makes filesystem-level conflicts between concurrent agents impossible
- the necessary process isolation pieces are **vendored** from `codex-linux-sandbox` under `crates/cairn-workspace/vendor/`, with Apache 2.0 attribution preserved and local ownership of all changes
- sandbox state transitions are first-class events in the existing runtime event log, projected into `cairn-graph` as `GraphNode(Sandbox)` / `GraphNode(RepoBase)` with opaque typed IDs — no raw host paths in graph node IDs; `SandboxBase::Directory` is deferred from graph projection until an alias field is added to the type
- resource exhaustion emits `SandboxResourceLimitExceeded` (v1 dimensions: `DiskBytes`, `MemoryBytes`, `WallClockMs`) followed by policy-dependent state transition (`Destroy`, `PauseAwaitOperator`, or `ReportOnly`). `DestroyReason` and `PreservationReason` carry the `ResourceDimension`, `limit`, and `observed` values for single-event readability
- `SandboxBase::Directory` is operator/system-authored only in v1; agents cannot select or mutate `SandboxBase`; this invariant is reopened explicitly if a future RFC allows parameterized sandbox fields in `RunTemplate`
- locked-clone immutability is a correctness invariant; `RepoCloneCache::refresh()` is the only supported mutation path; `SandboxBaseRevisionDrift` fires on recovery when an overlay sandbox's `base_rev` does not match the current clone HEAD — the sandbox is preserved for operator review. `ReflinkProvider` sandboxes are exempt because they are physically independent of the source post-provision
- `ReflinkProvider` requires filesystems with snapshot-like clonefile semantics (APFS, ReFS, XFS reflink); implementations that cannot guarantee this must not advertise `ReflinkProvider` and must fall back to `OverlayProvider` or fail provisioning
- sandboxes are bound to runs, not tasks; they survive task re-claims within a run
- recovery on restart reconciles sandbox state with the task engine, checks project-allowlist revocations and base-revision drift before reattaching overlay sandboxes, preserves orphans for 24 h, and uses `SandboxPreserved` as the non-terminal "keep it alive for possible resume" state
- credentials are injected via env vars (non-git) or `GIT_ASKPASS` helper (git); never written to disk
- the policy struct records network egress as an allowlist field but does not enforce it in v1 (enforcement is a future work item)
- automatic indexing of `RepoStore` clone contents into `cairn-memory` is out of scope for v1 (explicit non-goal)
- open questions listed above must be resolved before implementation branches diverge

## Integration Tests (Compliance Proof)

The RFC is considered implemented when the following integration tests pass:

1. **RepoStore clone-and-lock**: register a repo via `ProjectRepoAccessService::allow` + `RepoCloneCache::ensure_cloned`; confirm clone happens at `$base/repos/{tenant_id}/{owner}/{repo}/`, files are `chmod -R a-w`, `RepoCloneCreated` + `RepoCloneLocked` events are in the log; second allow + ensure for the same (tenant, repo_id) is a no-op on the clone
2. **Provision-and-work, OverlayProvider on Linux**: create a run with `SandboxBase::Repo { repo_id }`, confirm `provision()` returns an overlay-mounted path in under 500 ms, agent writes a file to the upper layer, base repo files at the locked path are unchanged
3. **Provision-and-work, ReflinkProvider on macOS**: same as above but on macOS; confirm `provision()` reflink-clones the locked base into the run sandbox via APFS clonefile in under 500 ms; agent writes are local to the sandbox
4. **Provider fallback Linux→Reflink**: on a Linux host where overlay mount fails (no `CAP_SYS_ADMIN`, kernel < 5.11), confirm `SandboxPolicyDegraded` is emitted and provisioning succeeds with `ReflinkProvider`
5. **Concurrent runs on same repo**: two runs in different projects (same tenant) both reference `org/dogfood`; both provision sandboxes from the same physical clone; both write to their own upper/reflinked layers; their files are isolated; the locked clone is unchanged
6. **`cairn.registerRepo` requires project-scoped approval**: an agent in project A calls `cairn.registerRepo("org/new-repo")`; the RFC 019 decision layer creates an approval request scoped to `(project_A, org/new-repo)`; on approval, the project allowlist expands and the clone is created; **project B in the same tenant does NOT have `org/new-repo` in its allowlist** and a `RepoStore::resolve` from project B fails with `RepoStoreError::NotAllowedForProject`
7. **EnsureAllCloned at startup**: cairn-app startup iterates the distinct `(tenant, repo_id)` set derived from all project allowlists via `ProjectRepoAccessService::list_all()`; pre-populates the `RepoCloneCache`; first sandbox provision after startup is fast (no clone overhead)
8. **Credential injection**: `git push` from inside the sandbox uses the GIT_ASKPASS helper; the token value never appears on disk or in process argv
9. **Checkpoint and restore**: checkpoint an active sandbox, destroy it, restore from the checkpoint to a new run ID; confirm the agent sees the prior mutations
10. **Crash recovery**: kill cairn-app mid-run; restart; confirm `SandboxService::recover_all()` reattaches the sandbox and the resumed run can continue writing
11. **Stale orphan cleanup**: a sandbox whose run is `Completed` in the task engine is destroyed on startup with `reason: Stale`
12. **Resource-limit enforcement (all three modes)**: (a) a run with `disk_quota_bytes: 1 MB` and `on_resource_exhaustion: Destroy` — agent write exceeds quota; `SandboxResourceLimitExceeded { dimension: DiskBytes }` + `SandboxDestroyed { reason: ResourceLimitExceeded }` emitted; run transitions to `Failed`. (b) same with `on_resource_exhaustion: PauseAwaitOperator` — sandbox transitions to `Preserved { reason: AwaitingResourceRaise }`, run transitions to `WaitingApproval`; operator raises quota → sandbox resumes. (c) same with `on_resource_exhaustion: ReportOnly` — sandbox and run continue; only `SandboxResourceLimitExceeded` event is emitted advisory-only
13. **Non-code base**: a sandbox created with `SandboxBase::Empty` provides a writable scratch directory with no git semantics
14. **Concurrent sandbox isolation**: two runs in the same project with distinct sandboxes do not see each other's files
15. **Event log completeness**: every state transition produces the corresponding `SandboxEvent` variant in the event log; sandbox provenance is projected into `cairn-graph` as `GraphNode(Sandbox)` with opaque typed node IDs
16. **Vendored code integrity**: the vendored `codex-linux-sandbox` source matches the checksum recorded at crate build time
17. **Operator HTTP contract for repo access**: `GET /v1/projects/:project/repos` lists the project's allowlist entries; `POST` adds a new entry and emits `ProjectRepoAllowlistExpanded`; `DELETE /v1/projects/:project/repos/:owner/:repo` revokes and emits `ProjectRepoAllowlistShrunk`; physical clone is NOT deleted synchronously — a subsequent GC sweep with zero allowlist references and zero active sandboxes deletes the clone and emits `RepoCloneDeleted`
18. **Recovery detects allowlist revocation**: sandbox was provisioned against `org/foo` in project A; between crash and restart the operator revokes `org/foo` from project A's allowlist; on recovery `SandboxAllowlistRevoked` is emitted and the sandbox transitions to `Preserved { reason: AllowlistRevoked }`
19. **Recovery detects base-revision drift (Overlay only)**: an overlay sandbox was provisioned with `base_rev: abc123`; an operator calls `RepoCloneCache::refresh()` moving HEAD to `def456`; on recovery `SandboxBaseRevisionDrift { expected: abc123, actual: def456 }` is emitted and the sandbox transitions to `Preserved { reason: BaseRevisionDrift }`. A reflink sandbox on the same tenant does NOT emit drift (physically independent)
20. **v0 → v1 migration**: first boot on a v0 database with a tenant-scoped allowlist emits `ProjectRepoAllowlistExpanded { added_by: SystemMigration }` for every `(project, repo_id)` combination under the tenant; all projects get the full inherited access; the operator can then selectively `DELETE` entries via the HTTP surface
