use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::{
    CheckpointKind, DestroyReason, OnExhaustion, PreservationReason, ProjectKey, RepoAccessContext,
    ResourceDimension, RunId, TaskId,
};

use crate::error::WorkspaceError;
use crate::providers::SandboxProvider;
use crate::repo_store::access_service::ProjectRepoAccessService;
use crate::sandbox::{
    DestroyResult, ProvisionedSandbox, RepoId, SandboxCheckpoint, SandboxErrorKind, SandboxEvent,
    SandboxMetadata, SandboxPolicy, SandboxState, SandboxStrategy, SandboxStrategyRequest,
};

pub trait Clock: Send + Sync + 'static {
    fn now_millis(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_millis() as u64
    }
}

pub trait SandboxEventSink: Send + Sync + 'static {
    fn publish(&self, event: SandboxEvent);
}

#[derive(Debug, Default)]
pub struct BufferedSandboxEventSink {
    events: Mutex<Vec<SandboxEvent>>,
}

impl BufferedSandboxEventSink {
    pub fn drain(&self) -> Vec<SandboxEvent> {
        let mut guard = self.events.lock().expect("sandbox event buffer poisoned");
        std::mem::take(&mut *guard)
    }
}

impl SandboxEventSink for BufferedSandboxEventSink {
    fn publish(&self, event: SandboxEvent) {
        self.events
            .lock()
            .expect("sandbox event buffer poisoned")
            .push(event);
    }
}

#[derive(Clone, Debug)]
struct SandboxSession {
    sandbox_id: crate::sandbox::SandboxId,
    run_id: RunId,
    task_id: Option<TaskId>,
    project: ProjectKey,
    policy: SandboxPolicy,
    state: SandboxState,
    sandbox: Option<ProvisionedSandbox>,
    metadata: Option<SandboxMetadata>,
}

impl SandboxSession {
    fn new(
        run_id: &RunId,
        task_id: Option<TaskId>,
        project: ProjectKey,
        policy: SandboxPolicy,
    ) -> Self {
        Self {
            sandbox_id: crate::sandbox::SandboxId::new(format!("sbx-{}", run_id.as_str())),
            run_id: run_id.clone(),
            task_id,
            project,
            policy,
            state: SandboxState::Initial,
            sandbox: None,
            metadata: None,
        }
    }
}

struct StrategyResolution {
    actual: SandboxStrategy,
    degraded_from: Option<SandboxStrategy>,
    degrade_reason: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SandboxRecoverySummary {
    pub reconnected: u32,
    pub preserved: u32,
    pub failed: u32,
    /// RFC 020 §"Run recovery matrix": sandboxes listed in the recovery
    /// registry whose on-disk root has gone missing between boots.
    /// `lost_runs` carries the `(run_id, project)` pairs so the run-level
    /// `RecoveryService` can transition each bound run to `failed` with
    /// `reason: sandbox_lost`. `lost` is the count for summary logging /
    /// readiness reporting and is always equal to `lost_runs.len()`.
    pub lost: u32,
    pub lost_runs: Vec<(RunId, ProjectKey)>,
    /// RFC 020 §"Run recovery matrix": registry entries for
    /// `SandboxBase::Repo` sandboxes whose bound repo is no longer on the
    /// project's allowlist at recovery time. `allowlist_revoked_runs`
    /// carries `(run_id, project, repo_id)` so the run-level
    /// `RecoveryService` can transition each bound run to
    /// `WaitingApproval` with a synthesized approval asking the operator
    /// to re-grant or cancel (per the `AllowlistRevoked` matrix row).
    /// `preserved_allowlist_revoked` is always equal to
    /// `allowlist_revoked_runs.len()`.
    pub preserved_allowlist_revoked: u32,
    pub allowlist_revoked_runs: Vec<(RunId, ProjectKey, RepoId)>,
}

/// Directory name (relative to `base_dir`) holding the recovery registry —
/// one sidecar JSON per provisioned sandbox that survives the sandbox
/// directory being deleted. Without this sidecar, a sandbox whose root
/// has been removed is indistinguishable from "never provisioned", and
/// `recover_all` cannot attribute a `sandbox_lost` failure to a run.
///
/// Leading dot keeps the registry out of provider `list()` sweeps — all
/// providers skip entries without a `meta.json` (registry entries have
/// a `registry.json` instead), and the leading dot is an additional
/// belt-and-braces filter against any future provider that enumerates
/// by prefix.
const RECOVERY_REGISTRY_DIRNAME: &str = ".registry";

/// Sidecar filename for a single registry entry.
const REGISTRY_ENTRY_FILENAME: &str = "registry.json";

/// One registry entry — the minimum information needed to attribute a
/// missing sandbox directory back to the run that owned it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct RegistryEntry {
    sandbox_id: crate::sandbox::SandboxId,
    run_id: RunId,
    project: ProjectKey,
    strategy: SandboxStrategy,
    /// Expected on-disk root. At recovery time we check `path.exists()`
    /// — a missing path with a present registry entry means
    /// `SandboxLost`.
    path: PathBuf,
    /// Unix-ms timestamp for operator forensics. Not consumed by the
    /// recovery matrix.
    registered_at: u64,
    /// Bound repo for `SandboxBase::Repo` sandboxes. When `Some`, the
    /// allowlist-revoked recovery sweep looks this repo up in
    /// `ProjectRepoAccessService`; if the repo is no longer allowed
    /// under `project`, the sweep emits `SandboxAllowlistRevoked` and
    /// preserves the sandbox. `None` for `SandboxBase::Empty` and
    /// `SandboxBase::Directory` — those bases have no allowlist semantics.
    /// Backward-compat: older registry sidecars written before the
    /// allowlist sweep existed deserialize to `None`, which disables the
    /// check for them (indistinguishable from a non-repo base).
    #[serde(default)]
    repo_id: Option<RepoId>,
    /// Tombstone flag: `true` once the allowlist-revoked sweep has emitted
    /// `SandboxAllowlistRevoked` for this entry. Prevents re-emission
    /// across subsequent boots — the sandbox is `Preserved` and the
    /// operator-facing approval has been synthesized; firing the event
    /// again would duplicate that approval. Operator must re-grant (which
    /// happens via the HTTP repo-allow path and does not touch the
    /// registry) or delete the sandbox (which removes the registry entry).
    #[serde(default)]
    allowlist_revoked_handled: bool,
}

pub struct SandboxService {
    providers: HashMap<SandboxStrategy, Box<dyn SandboxProvider>>,
    event_sink: Arc<dyn SandboxEventSink>,
    base_dir: PathBuf,
    clock: Arc<dyn Clock>,
    sessions: RwLock<HashMap<RunId, SandboxSession>>,
    /// Allowlist source consulted during `recover_all` to decide whether a
    /// `SandboxBase::Repo` sandbox's repo is still authorised under the
    /// owning project. `None` disables the allowlist-revoked sweep
    /// entirely (treat every recovered sandbox as still allowed) — used by
    /// the workspace-layer unit tests which exercise the sweep directly
    /// with seeded entries and would otherwise need to stand up the full
    /// access service.
    allowlist: Option<Arc<ProjectRepoAccessService>>,
}

impl SandboxService {
    pub fn new(
        providers: HashMap<SandboxStrategy, Box<dyn SandboxProvider>>,
        event_sink: Arc<dyn SandboxEventSink>,
        base_dir: impl Into<PathBuf>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            providers,
            event_sink,
            base_dir: base_dir.into(),
            clock,
            sessions: RwLock::new(HashMap::new()),
            allowlist: None,
        }
    }

    /// Wire in the project-scoped repo allowlist. When set, `recover_all`
    /// will emit `SandboxAllowlistRevoked` for any registry entry whose
    /// bound repo is no longer allowed under the sandbox's project.
    /// Without this, the allowlist-revoked sweep is a no-op.
    pub fn with_allowlist(mut self, allowlist: Arc<ProjectRepoAccessService>) -> Self {
        self.allowlist = Some(allowlist);
        self
    }

    pub fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }

    /// Register a lost-sandbox entry directly without going through
    /// provisioning. Exposed for integration tests that need to simulate
    /// "operator deleted the sandbox root between boots" without wiring a
    /// full HTTP provisioning surface. `path` should be a path that does
    /// NOT exist on disk — the recovery sweep keys `SandboxLost` off
    /// `path.exists() == false`.
    pub fn seed_registry_entry_for_test(
        &self,
        sandbox_id: crate::sandbox::SandboxId,
        run_id: RunId,
        project: ProjectKey,
        strategy: SandboxStrategy,
        path: PathBuf,
    ) -> Result<(), WorkspaceError> {
        self.seed_registry_entry_for_test_with_repo(
            sandbox_id, run_id, project, strategy, path, None,
        )
    }

    /// Like `seed_registry_entry_for_test`, but captures the bound
    /// `repo_id`. Used by the RFC 020 `allowlist_revoked` integration
    /// test: the sandbox directory exists but the repo binding is
    /// what the allowlist-revoked sweep keys off.
    pub fn seed_registry_entry_for_test_with_repo(
        &self,
        sandbox_id: crate::sandbox::SandboxId,
        run_id: RunId,
        project: ProjectKey,
        strategy: SandboxStrategy,
        path: PathBuf,
        repo_id: Option<RepoId>,
    ) -> Result<(), WorkspaceError> {
        let entry = RegistryEntry {
            sandbox_id,
            run_id,
            project,
            strategy,
            path,
            registered_at: self.clock.now_millis(),
            repo_id,
            allowlist_revoked_handled: false,
        };
        self.write_registry_entry(&entry)
    }

    pub fn state_for(&self, run_id: &RunId) -> Option<SandboxState> {
        self.sessions
            .read()
            .expect("sandbox session lock poisoned")
            .get(run_id)
            .map(|session| session.state)
    }

    pub fn metadata_for(&self, run_id: &RunId) -> Option<SandboxMetadata> {
        self.sessions
            .read()
            .expect("sandbox session lock poisoned")
            .get(run_id)
            .and_then(|session| session.metadata.clone())
    }

    pub async fn provision_or_reconnect(
        &self,
        run_id: &RunId,
        task_id: Option<TaskId>,
        project: ProjectKey,
        policy: SandboxPolicy,
    ) -> Result<ProvisionedSandbox, WorkspaceError> {
        if let Some(existing) = self.existing_sandbox(run_id)? {
            return Ok(existing);
        }

        let resolution = self.resolve_strategy(&policy.strategy)?;
        let started_at = self.clock.now_millis();
        let (sandbox_id, run_id_owned) = {
            let mut sessions = self
                .sessions
                .write()
                .expect("sandbox session lock poisoned");
            let session = sessions.entry(run_id.clone()).or_insert_with(|| {
                SandboxSession::new(run_id, task_id.clone(), project.clone(), policy.clone())
            });
            if !session.state.can_transition_to(SandboxState::Provisioning) {
                return Err(WorkspaceError::InvalidSandboxStateTransition {
                    run_id: run_id.clone(),
                    from: session.state,
                    to: SandboxState::Provisioning,
                });
            }
            session.task_id = task_id.clone();
            session.project = project.clone();
            session.policy = policy.clone();
            session.state = SandboxState::Provisioning;
            (session.sandbox_id.clone(), session.run_id.clone())
        };

        if let Some(requested) = resolution.degraded_from {
            self.event_sink
                .publish(SandboxEvent::SandboxPolicyDegraded {
                    sandbox_id: sandbox_id.clone(),
                    run_id: run_id_owned.clone(),
                    requested,
                    actual: resolution.actual,
                    reason: resolution
                        .degrade_reason
                        .clone()
                        .unwrap_or_else(|| "preferred strategy unavailable".to_string()),
                    degraded_at: started_at,
                });
        }

        let provider = self.provider(resolution.actual)?;
        match provider.provision(run_id, &project, &policy).await {
            Ok(sandbox) => {
                let provisioned_at = self.clock.now_millis();
                let metadata = SandboxMetadata {
                    sandbox_id: sandbox.sandbox_id.clone(),
                    run_id: run_id.clone(),
                    task_id,
                    project: project.clone(),
                    strategy: sandbox.strategy,
                    state: SandboxState::Ready,
                    base_rev: sandbox.base_revision.clone(),
                    repo_id: repo_id_for(&policy),
                    path: sandbox.path.clone(),
                    pid: None,
                    created_at: provisioned_at,
                    heartbeat_at: provisioned_at,
                    policy_hash: policy_hash(&policy),
                };

                {
                    let mut sessions = self
                        .sessions
                        .write()
                        .expect("sandbox session lock poisoned");
                    let session = sessions
                        .get_mut(run_id)
                        .expect("sandbox session must exist after provisioning");
                    session.sandbox_id = sandbox.sandbox_id.clone();
                    session.state = SandboxState::Ready;
                    session.sandbox = Some(sandbox.clone());
                    session.metadata = Some(metadata.clone());
                }
                // Write the recovery registry sidecar *before* persisting
                // the full metadata. If persisting fails and we never reach
                // the metadata write, the registry entry still attributes
                // the half-provisioned sandbox to its run — better than
                // silently losing track of it. On the happy path both
                // succeed and recover_all sees the sandbox via the
                // provider's meta.json anyway.
                let registry_entry = RegistryEntry {
                    sandbox_id: sandbox.sandbox_id.clone(),
                    run_id: run_id.clone(),
                    project: project.clone(),
                    strategy: sandbox.strategy,
                    path: sandbox.path.clone(),
                    registered_at: provisioned_at,
                    repo_id: repo_id_for(&policy),
                    allowlist_revoked_handled: false,
                };
                if let Err(error) = self.write_registry_entry(&registry_entry) {
                    let failed_at = self.clock.now_millis();
                    {
                        let mut sessions = self
                            .sessions
                            .write()
                            .expect("sandbox session lock poisoned");
                        if let Some(session) = sessions.get_mut(run_id) {
                            session.state = SandboxState::Failed;
                        }
                    }
                    self.event_sink
                        .publish(SandboxEvent::SandboxProvisioningFailed {
                            sandbox_id: sandbox.sandbox_id.clone(),
                            run_id: run_id.clone(),
                            error_kind: SandboxErrorKind::Filesystem,
                            error: error.to_string(),
                            failed_at,
                        });
                    return Err(error);
                }

                if let Err(error) = self.persist_metadata(&metadata) {
                    let failed_at = self.clock.now_millis();
                    {
                        let mut sessions = self
                            .sessions
                            .write()
                            .expect("sandbox session lock poisoned");
                        if let Some(session) = sessions.get_mut(run_id) {
                            session.state = SandboxState::Failed;
                        }
                    }
                    self.event_sink
                        .publish(SandboxEvent::SandboxProvisioningFailed {
                            sandbox_id: sandbox.sandbox_id.clone(),
                            run_id: run_id.clone(),
                            error_kind: SandboxErrorKind::Filesystem,
                            error: error.to_string(),
                            failed_at,
                        });
                    return Err(error);
                }

                self.event_sink.publish(SandboxEvent::SandboxProvisioned {
                    sandbox_id: sandbox.sandbox_id.clone(),
                    run_id: run_id.clone(),
                    task_id: metadata.task_id.clone(),
                    project,
                    strategy: sandbox.strategy,
                    base_revision: sandbox.base_revision.clone(),
                    policy,
                    path: sandbox.path.clone(),
                    duration_ms: provisioned_at.saturating_sub(started_at),
                    provisioned_at,
                });

                Ok(sandbox)
            }
            Err(error) => {
                let failed_at = self.clock.now_millis();
                {
                    let mut sessions = self
                        .sessions
                        .write()
                        .expect("sandbox session lock poisoned");
                    if let Some(session) = sessions.get_mut(run_id) {
                        session.state = SandboxState::Failed;
                    }
                }

                self.event_sink
                    .publish(SandboxEvent::SandboxProvisioningFailed {
                        sandbox_id,
                        run_id: run_id_owned,
                        error_kind: SandboxErrorKind::Filesystem,
                        error: error.to_string(),
                        failed_at,
                    });

                Err(error)
            }
        }
    }

    pub async fn activate(
        &self,
        run_id: &RunId,
        pid: Option<u32>,
    ) -> Result<ProvisionedSandbox, WorkspaceError> {
        let activated_at = self.clock.now_millis();
        let (sandbox_id, sandbox, metadata) = {
            let mut sessions = self
                .sessions
                .write()
                .expect("sandbox session lock poisoned");
            let session = self.session_mut(&mut sessions, run_id)?;
            let resumed = matches!(
                session.state,
                SandboxState::Checkpointed | SandboxState::Preserved | SandboxState::Active
            );
            self.transition(session, SandboxState::Active)?;
            let sandbox =
                session
                    .sandbox
                    .as_mut()
                    .ok_or_else(|| WorkspaceError::SandboxNotFound {
                        run_id: run_id.clone(),
                    })?;
            sandbox.is_resumed = resumed;
            if let Some(metadata) = session.metadata.as_mut() {
                metadata.state = SandboxState::Active;
                metadata.pid = pid;
                metadata.heartbeat_at = activated_at;
            }
            (
                session.sandbox_id.clone(),
                sandbox.clone(),
                session.metadata.clone(),
            )
        };
        if let Some(metadata) = metadata.as_ref() {
            self.persist_metadata(metadata)?;
        }

        self.event_sink.publish(SandboxEvent::SandboxActivated {
            sandbox_id,
            run_id: run_id.clone(),
            pid,
            activated_at,
        });

        Ok(sandbox)
    }

    pub async fn heartbeat(&self, run_id: &RunId) -> Result<(), WorkspaceError> {
        let strategy = {
            let sessions = self.sessions.read().expect("sandbox session lock poisoned");
            let session = sessions
                .get(run_id)
                .ok_or_else(|| WorkspaceError::SandboxNotFound {
                    run_id: run_id.clone(),
                })?;
            if session.state != SandboxState::Active {
                return Err(WorkspaceError::InvalidSandboxStateTransition {
                    run_id: run_id.clone(),
                    from: session.state,
                    to: SandboxState::Active,
                });
            }
            session
                .sandbox
                .as_ref()
                .ok_or_else(|| WorkspaceError::SandboxNotFound {
                    run_id: run_id.clone(),
                })?
                .strategy
        };

        self.provider(strategy)?.heartbeat(run_id).await?;

        let heartbeat_at = self.clock.now_millis();
        let (sandbox_id, metadata) = {
            let mut sessions = self
                .sessions
                .write()
                .expect("sandbox session lock poisoned");
            let session = self.session_mut(&mut sessions, run_id)?;
            if let Some(metadata) = session.metadata.as_mut() {
                metadata.heartbeat_at = heartbeat_at;
            }
            (session.sandbox_id.clone(), session.metadata.clone())
        };
        if let Some(metadata) = metadata.as_ref() {
            self.persist_metadata(metadata)?;
        }

        self.event_sink.publish(SandboxEvent::SandboxHeartbeat {
            sandbox_id,
            run_id: run_id.clone(),
            heartbeat_at,
        });

        Ok(())
    }

    pub async fn checkpoint(
        &self,
        run_id: &RunId,
        kind: CheckpointKind,
    ) -> Result<SandboxCheckpoint, WorkspaceError> {
        let strategy = self.sandbox_strategy(run_id)?;
        let checkpoint = self.provider(strategy)?.checkpoint(run_id, kind).await?;
        let checkpointed_at = self.clock.now_millis();

        let metadata = {
            let mut sessions = self
                .sessions
                .write()
                .expect("sandbox session lock poisoned");
            let session = self.session_mut(&mut sessions, run_id)?;
            self.transition(session, SandboxState::Checkpointed)?;
            if let Some(metadata) = session.metadata.as_mut() {
                metadata.state = SandboxState::Checkpointed;
            }
            session.metadata.clone()
        };
        if let Some(metadata) = metadata.as_ref() {
            self.persist_metadata(metadata)?;
        }

        self.event_sink.publish(SandboxEvent::SandboxCheckpointed {
            sandbox_id: checkpoint.sandbox_id.clone(),
            run_id: checkpoint.run_id.clone(),
            checkpoint_kind: checkpoint.kind,
            rescue_ref: checkpoint.rescue_ref.clone(),
            upper_snapshot: checkpoint.upper_snapshot.clone(),
            checkpointed_at,
        });

        Ok(checkpoint)
    }

    pub fn preserve(
        &self,
        run_id: &RunId,
        reason: PreservationReason,
    ) -> Result<(), WorkspaceError> {
        let preserved_at = self.clock.now_millis();
        let (sandbox_id, metadata) = {
            let mut sessions = self
                .sessions
                .write()
                .expect("sandbox session lock poisoned");
            let session = self.session_mut(&mut sessions, run_id)?;
            self.transition(session, SandboxState::Preserved)?;
            if let Some(metadata) = session.metadata.as_mut() {
                metadata.state = SandboxState::Preserved;
                metadata.pid = None;
            }
            (session.sandbox_id.clone(), session.metadata.clone())
        };
        if let Some(metadata) = metadata.as_ref() {
            self.persist_metadata(metadata)?;
        }

        self.event_sink.publish(SandboxEvent::SandboxPreserved {
            sandbox_id,
            run_id: run_id.clone(),
            reason,
            preserved_at,
        });

        Ok(())
    }

    pub async fn destroy(
        &self,
        run_id: &RunId,
        preserve: bool,
        reason: DestroyReason,
    ) -> Result<DestroyResult, WorkspaceError> {
        let strategy = self.sandbox_strategy(run_id)?;
        {
            let mut sessions = self
                .sessions
                .write()
                .expect("sandbox session lock poisoned");
            let session = self.session_mut(&mut sessions, run_id)?;
            self.transition(session, SandboxState::Destroying)?;
            if let Some(metadata) = session.metadata.as_mut() {
                metadata.state = SandboxState::Destroying;
                metadata.pid = None;
            }
        }

        let result = self.provider(strategy)?.destroy(run_id, preserve).await?;
        let destroyed_at = self.clock.now_millis();

        // Drop the recovery registry entry on a full (non-preserve) destroy.
        // Preserved sandboxes still exist on disk and must stay attributable
        // at the next boot; destroyed ones intentionally must not look
        // `sandbox_lost` when they are picked up again by recovery.
        if !preserve {
            self.remove_registry_entry(&result.sandbox_id)?;
        }

        {
            let mut sessions = self
                .sessions
                .write()
                .expect("sandbox session lock poisoned");
            let session = self.session_mut(&mut sessions, run_id)?;
            self.transition(session, SandboxState::Destroyed)?;
            if let Some(metadata) = session.metadata.as_mut() {
                metadata.state = SandboxState::Destroyed;
            }
        }

        self.event_sink.publish(SandboxEvent::SandboxDestroyed {
            sandbox_id: result.sandbox_id.clone(),
            run_id: run_id.clone(),
            files_changed: result.files_changed,
            bytes_written: result.bytes_written,
            reason,
            destroyed_at,
        });

        Ok(result)
    }

    pub async fn observe_resource_limit(
        &self,
        run_id: &RunId,
        dimension: ResourceDimension,
        observed: u64,
    ) -> Result<(), WorkspaceError> {
        let (sandbox_id, policy) = {
            let sessions = self.sessions.read().expect("sandbox session lock poisoned");
            let session = sessions
                .get(run_id)
                .ok_or_else(|| WorkspaceError::SandboxNotFound {
                    run_id: run_id.clone(),
                })?;
            (session.sandbox_id.clone(), session.policy.clone())
        };
        let limit =
            limit_for(&policy, dimension).ok_or_else(|| WorkspaceError::ResourceLimitMissing {
                run_id: run_id.clone(),
                dimension,
            })?;
        let at = self.clock.now_millis();

        self.event_sink
            .publish(SandboxEvent::SandboxResourceLimitExceeded {
                sandbox_id,
                run_id: run_id.clone(),
                dimension,
                limit,
                observed,
                at,
            });

        match policy.on_resource_exhaustion {
            OnExhaustion::Destroy => {
                self.destroy(
                    run_id,
                    false,
                    DestroyReason::ResourceLimitExceeded {
                        dimension,
                        limit,
                        observed,
                    },
                )
                .await?;
            }
            OnExhaustion::PauseAwaitOperator => {
                self.preserve(
                    run_id,
                    PreservationReason::AwaitingResourceRaise {
                        dimension,
                        limit,
                        observed,
                    },
                )?;
            }
            OnExhaustion::ReportOnly => {}
        }

        Ok(())
    }

    pub async fn recover_all(&self) -> Result<SandboxRecoverySummary, WorkspaceError> {
        let mut handles = Vec::new();
        for provider in self.providers.values() {
            handles.extend(provider.list().await?);
        }
        handles.sort_by(|left, right| {
            left.metadata
                .sandbox_id
                .as_str()
                .cmp(right.metadata.sandbox_id.as_str())
        });

        // Track every sandbox_id that the providers surfaced so we can
        // diff against the recovery registry below. Anything in the
        // registry that the providers did NOT list is a candidate for
        // `sandbox_lost`.
        let mut observed: std::collections::HashSet<String> = std::collections::HashSet::new();
        for handle in &handles {
            observed.insert(handle.metadata.sandbox_id.as_str().to_owned());
        }

        let mut summary = SandboxRecoverySummary::default();
        for handle in handles {
            let mut metadata = handle.metadata;
            if matches!(
                metadata.state,
                SandboxState::Destroyed | SandboxState::Failed
            ) {
                continue;
            }

            let run_id = metadata.run_id.clone();
            match self.provider(metadata.strategy)?.reconnect(&run_id).await {
                Ok(Some(sandbox)) => {
                    metadata.state = recovered_state(metadata.state);
                    metadata.path = sandbox.path.clone();
                    metadata.base_rev = sandbox.base_revision.clone();
                    metadata.pid = None;
                    metadata.heartbeat_at = self.clock.now_millis();
                    self.persist_metadata(&metadata)?;
                    self.remember_recovered_session(
                        metadata.clone(),
                        Some(sandbox),
                        recovery_policy(&metadata),
                    );
                    summary.reconnected += 1;
                }
                Ok(None) => {}
                Err(WorkspaceError::BaseRevisionDrift {
                    expected, actual, ..
                }) => {
                    let detected_at = self.clock.now_millis();
                    if let Some(repo_id) = metadata.repo_id.clone() {
                        self.event_sink
                            .publish(SandboxEvent::SandboxBaseRevisionDrift {
                                sandbox_id: metadata.sandbox_id.clone(),
                                run_id: run_id.clone(),
                                project: metadata.project.clone(),
                                repo_id,
                                expected: expected.clone(),
                                actual: actual.clone(),
                                detected_at,
                            });
                    }

                    metadata.state = SandboxState::Preserved;
                    metadata.pid = None;
                    metadata.heartbeat_at = detected_at;
                    self.persist_metadata(&metadata)?;
                    self.remember_recovered_session(
                        metadata.clone(),
                        None,
                        recovery_policy(&metadata),
                    );
                    self.event_sink.publish(SandboxEvent::SandboxPreserved {
                        sandbox_id: metadata.sandbox_id.clone(),
                        run_id: run_id.clone(),
                        reason: PreservationReason::BaseRevisionDrift { expected, actual },
                        preserved_at: detected_at,
                    });
                    summary.preserved += 1;
                }
                Err(error) => {
                    let failed_at = self.clock.now_millis();
                    metadata.state = SandboxState::Failed;
                    metadata.pid = None;
                    self.persist_metadata(&metadata)?;
                    self.remember_recovered_session(
                        metadata.clone(),
                        None,
                        recovery_policy(&metadata),
                    );
                    self.event_sink
                        .publish(SandboxEvent::SandboxProvisioningFailed {
                            sandbox_id: metadata.sandbox_id.clone(),
                            run_id: run_id.clone(),
                            error_kind: SandboxErrorKind::Recovery,
                            error: error.to_string(),
                            failed_at,
                        });
                    summary.failed += 1;
                }
            }
        }

        // RFC 020 §"Run recovery matrix": for every registry entry whose
        // provider did not surface a live handle, check the filesystem.
        // Absent root → emit `SandboxLost` and record the (run, project)
        // pair for the run-level recovery service to transition to `failed`
        // with `reason: sandbox_lost`. Present root with no provider handle
        // means the provider's own metadata is corrupt — leave it alone;
        // the provider's list() path is responsible for that case and we
        // would double-report otherwise.
        let mut registry = self.list_registry_entries()?;
        registry.sort_by(|a, b| a.sandbox_id.as_str().cmp(b.sandbox_id.as_str()));
        for entry in registry {
            if observed.contains(entry.sandbox_id.as_str()) {
                continue;
            }
            if entry.path.exists() {
                continue;
            }
            let detected_at = self.clock.now_millis();
            self.event_sink.publish(SandboxEvent::SandboxLost {
                sandbox_id: entry.sandbox_id.clone(),
                run_id: entry.run_id.clone(),
                project: entry.project.clone(),
                sandbox_path: entry.path.clone(),
                detected_at,
            });
            // Drop the registry entry so subsequent boots don't re-emit
            // the same `SandboxLost` event for a run that has already
            // been transitioned to terminal state by the run-recovery
            // service.
            self.remove_registry_entry(&entry.sandbox_id)?;
            summary.lost_runs.push((entry.run_id, entry.project));
        }
        summary.lost = summary.lost_runs.len() as u32;

        // RFC 020 §"Run recovery matrix" — `AllowlistRevoked` row. For
        // every registry entry that survived the lost-sweep above and
        // carries a bound `repo_id`, ask the project allowlist whether
        // the repo is still authorised. If not → emit
        // `SandboxAllowlistRevoked` and record the `(run, project, repo)`
        // triple so the run-level recovery service can transition the
        // bound run to `WaitingApproval` with a synthesized approval.
        // Mark the entry as `allowlist_revoked_handled` so subsequent
        // boots do not re-emit the same event / re-create the approval.
        //
        // No allowlist wired (`None`) → skip entirely. Workspace-layer
        // unit tests exercise the sweep directly with seeded entries and
        // would otherwise need to stand up the full access service.
        if let Some(allowlist) = self.allowlist.clone() {
            let entries = self.list_registry_entries()?;
            for mut entry in entries {
                if entry.allowlist_revoked_handled {
                    continue;
                }
                let Some(repo_id) = entry.repo_id.clone() else {
                    continue;
                };
                let ctx = RepoAccessContext {
                    project: entry.project.clone(),
                };
                if allowlist.is_allowed(&ctx, &repo_id).await {
                    continue;
                }
                let detected_at = self.clock.now_millis();
                self.event_sink
                    .publish(SandboxEvent::SandboxAllowlistRevoked {
                        sandbox_id: entry.sandbox_id.clone(),
                        run_id: entry.run_id.clone(),
                        project: entry.project.clone(),
                        repo_id: repo_id.clone(),
                        revoked_at: detected_at,
                        detected_at,
                    });
                entry.allowlist_revoked_handled = true;
                self.write_registry_entry(&entry)?;
                summary
                    .allowlist_revoked_runs
                    .push((entry.run_id, entry.project, repo_id));
            }
            summary.preserved_allowlist_revoked = summary.allowlist_revoked_runs.len() as u32;
        }

        Ok(summary)
    }

    fn provider(&self, strategy: SandboxStrategy) -> Result<&dyn SandboxProvider, WorkspaceError> {
        self.providers
            .get(&strategy)
            .map(|provider| provider.as_ref())
            .ok_or(WorkspaceError::ProviderUnavailable { strategy })
    }

    fn remember_recovered_session(
        &self,
        metadata: SandboxMetadata,
        sandbox: Option<ProvisionedSandbox>,
        policy: SandboxPolicy,
    ) {
        self.sessions
            .write()
            .expect("sandbox session lock poisoned")
            .insert(
                metadata.run_id.clone(),
                SandboxSession {
                    sandbox_id: metadata.sandbox_id.clone(),
                    run_id: metadata.run_id.clone(),
                    task_id: metadata.task_id.clone(),
                    project: metadata.project.clone(),
                    policy,
                    state: metadata.state,
                    sandbox,
                    metadata: Some(metadata),
                },
            );
    }

    fn resolve_strategy(
        &self,
        request: &SandboxStrategyRequest,
    ) -> Result<StrategyResolution, WorkspaceError> {
        match *request {
            SandboxStrategyRequest::Force(strategy) => {
                self.provider(strategy)?;
                Ok(StrategyResolution {
                    actual: strategy,
                    degraded_from: None,
                    degrade_reason: None,
                })
            }
            SandboxStrategyRequest::Preferred(strategy) => {
                if self.providers.contains_key(&strategy) {
                    return Ok(StrategyResolution {
                        actual: strategy,
                        degraded_from: None,
                        degrade_reason: None,
                    });
                }

                let fallback = fallback_strategy(strategy);
                if self.providers.contains_key(&fallback) {
                    Ok(StrategyResolution {
                        actual: fallback,
                        degraded_from: Some(strategy),
                        degrade_reason: Some(format!(
                            "preferred strategy {strategy:?} unavailable; fell back to {fallback:?}"
                        )),
                    })
                } else {
                    Err(WorkspaceError::ProviderUnavailable { strategy })
                }
            }
        }
    }

    fn existing_sandbox(
        &self,
        run_id: &RunId,
    ) -> Result<Option<ProvisionedSandbox>, WorkspaceError> {
        let sessions = self.sessions.read().expect("sandbox session lock poisoned");
        let Some(session) = sessions.get(run_id) else {
            return Ok(None);
        };

        if session.state.is_terminal() {
            return Err(WorkspaceError::InvalidSandboxStateTransition {
                run_id: run_id.clone(),
                from: session.state,
                to: SandboxState::Provisioning,
            });
        }

        Ok(session.sandbox.as_ref().map(|sandbox| {
            let mut resumed = sandbox.clone();
            resumed.is_resumed = true;
            resumed
        }))
    }

    fn sandbox_strategy(&self, run_id: &RunId) -> Result<SandboxStrategy, WorkspaceError> {
        self.sessions
            .read()
            .expect("sandbox session lock poisoned")
            .get(run_id)
            .and_then(|session| session.sandbox.as_ref().map(|sandbox| sandbox.strategy))
            .ok_or_else(|| WorkspaceError::SandboxNotFound {
                run_id: run_id.clone(),
            })
    }

    fn session_mut<'a>(
        &self,
        sessions: &'a mut HashMap<RunId, SandboxSession>,
        run_id: &RunId,
    ) -> Result<&'a mut SandboxSession, WorkspaceError> {
        sessions
            .get_mut(run_id)
            .ok_or_else(|| WorkspaceError::SandboxNotFound {
                run_id: run_id.clone(),
            })
    }

    fn transition(
        &self,
        session: &mut SandboxSession,
        next: SandboxState,
    ) -> Result<(), WorkspaceError> {
        if session.state == next {
            return Ok(());
        }
        if !session.state.can_transition_to(next) {
            return Err(WorkspaceError::InvalidSandboxStateTransition {
                run_id: session.run_id.clone(),
                from: session.state,
                to: next,
            });
        }
        session.state = next;
        Ok(())
    }

    /// Directory where recovery registry sidecars are kept. Lives under
    /// `base_dir/.registry/<sandbox_id>/registry.json` — deliberately
    /// separate from the sandbox root so that `rm -rf <sandbox_root>`
    /// does not take the registry entry with it.
    fn registry_dir(&self) -> PathBuf {
        self.base_dir.join(RECOVERY_REGISTRY_DIRNAME)
    }

    fn registry_entry_path(&self, sandbox_id: &crate::sandbox::SandboxId) -> PathBuf {
        self.registry_dir()
            .join(sandbox_id.as_str())
            .join(REGISTRY_ENTRY_FILENAME)
    }

    fn write_registry_entry(&self, entry: &RegistryEntry) -> Result<(), WorkspaceError> {
        let path = self.registry_entry_path(&entry.sandbox_id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                WorkspaceError::sandbox_op(&entry.run_id, "create_registry_dir", error)
            })?;
        }
        let encoded = serde_json::to_vec_pretty(entry).map_err(|error| {
            WorkspaceError::sandbox_op(&entry.run_id, "serialize_registry_entry", error)
        })?;
        fs::write(&path, encoded)
            .map_err(|error| WorkspaceError::sandbox_op(&entry.run_id, "write_registry", error))
    }

    fn remove_registry_entry(
        &self,
        sandbox_id: &crate::sandbox::SandboxId,
    ) -> Result<(), WorkspaceError> {
        let entry_dir = self.registry_dir().join(sandbox_id.as_str());
        match fs::remove_dir_all(&entry_dir) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(WorkspaceError::sandbox_op(
                &RunId::new(sandbox_id.as_str()),
                "remove_registry_entry",
                error,
            )),
        }
    }

    fn list_registry_entries(&self) -> Result<Vec<RegistryEntry>, WorkspaceError> {
        let dir = self.registry_dir();
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(WorkspaceError::sandbox_op(
                    &RunId::new("_registry"),
                    "read_registry_dir",
                    error,
                ))
            }
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                WorkspaceError::sandbox_op(&RunId::new("_registry"), "read_registry_entry", error)
            })?;
            let path = entry.path().join(REGISTRY_ENTRY_FILENAME);
            if !path.is_file() {
                continue;
            }
            let bytes = fs::read(&path).map_err(|error| {
                WorkspaceError::sandbox_op(&RunId::new("_registry"), "read_registry_body", error)
            })?;
            match serde_json::from_slice::<RegistryEntry>(&bytes) {
                Ok(parsed) => out.push(parsed),
                // A corrupt sidecar must not break recovery — it just means
                // we cannot attribute a lost sandbox to a run. Skip and
                // continue so the rest of the sweep proceeds; operators see
                // the broken file in the base dir.
                Err(_) => continue,
            }
        }
        Ok(out)
    }

    fn persist_metadata(&self, metadata: &SandboxMetadata) -> Result<(), WorkspaceError> {
        let sandbox_dir = self.base_dir.join(metadata.sandbox_id.as_str());
        fs::create_dir_all(&sandbox_dir).map_err(|error| {
            WorkspaceError::sandbox_op(&metadata.run_id, "create_metadata_dir", error)
        })?;
        let metadata_path = sandbox_dir.join("meta.json");
        let encoded = serde_json::to_vec_pretty(metadata).map_err(|error| {
            WorkspaceError::sandbox_op(&metadata.run_id, "serialize_metadata", error)
        })?;
        fs::write(&metadata_path, encoded)
            .map_err(|error| WorkspaceError::sandbox_op(&metadata.run_id, "write_metadata", error))
    }
}

fn fallback_strategy(strategy: SandboxStrategy) -> SandboxStrategy {
    match strategy {
        SandboxStrategy::Overlay => SandboxStrategy::Reflink,
        SandboxStrategy::Reflink => SandboxStrategy::Overlay,
    }
}

fn limit_for(policy: &SandboxPolicy, dimension: ResourceDimension) -> Option<u64> {
    match dimension {
        ResourceDimension::DiskBytes => policy.disk_quota_bytes,
        ResourceDimension::MemoryBytes => policy.memory_limit_bytes,
        ResourceDimension::WallClockMs => policy
            .wall_clock_limit
            .map(|duration| duration.as_millis() as u64),
    }
}

fn repo_id_for(policy: &SandboxPolicy) -> Option<crate::sandbox::RepoId> {
    match &policy.base {
        crate::sandbox::SandboxBase::Repo { repo_id, .. } => Some(repo_id.clone()),
        _ => None,
    }
}

fn policy_hash(policy: &SandboxPolicy) -> String {
    let mut hasher = DefaultHasher::new();
    format!("{policy:?}").hash(&mut hasher);
    format!("policy:{:016x}", hasher.finish())
}

fn recovered_state(state: SandboxState) -> SandboxState {
    match state {
        SandboxState::Initial | SandboxState::Provisioning | SandboxState::Active => {
            SandboxState::Ready
        }
        other => other,
    }
}

fn recovery_policy(metadata: &SandboxMetadata) -> SandboxPolicy {
    let base = match &metadata.repo_id {
        Some(repo_id) => crate::sandbox::SandboxBase::Repo {
            repo_id: repo_id.clone(),
            starting_ref: metadata.base_rev.clone(),
        },
        None => crate::sandbox::SandboxBase::Empty,
    };

    SandboxPolicy {
        strategy: crate::sandbox::SandboxStrategyRequest::Force(metadata.strategy),
        base,
        credentials: Vec::new(),
        network_egress: None,
        memory_limit_bytes: None,
        cpu_weight: None,
        disk_quota_bytes: None,
        wall_clock_limit: None,
        on_resource_exhaustion: OnExhaustion::Destroy,
        preserve_on_failure: true,
        required_host_caps: crate::sandbox::HostCapabilityRequirements::default(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use cairn_domain::{
        CheckpointKind, DestroyReason, OnExhaustion, PreservationReason, ProjectKey,
        ResourceDimension, RunId,
    };

    use super::{BufferedSandboxEventSink, Clock, SandboxService};
    use crate::error::WorkspaceError;
    use crate::providers::SandboxProvider;
    use crate::sandbox::{
        DestroyResult, HostCapabilityRequirements, ProvisionedSandbox, SandboxBase,
        SandboxCheckpoint, SandboxEvent, SandboxHandle, SandboxMetadata, SandboxPolicy,
        SandboxState, SandboxStrategy, SandboxStrategyRequest,
    };

    #[derive(Debug)]
    struct FixedClock {
        now: Mutex<u64>,
    }

    impl FixedClock {
        fn new(start: u64) -> Self {
            Self {
                now: Mutex::new(start),
            }
        }
    }

    impl Clock for FixedClock {
        fn now_millis(&self) -> u64 {
            let mut guard = self.now.lock().expect("fixed clock poisoned");
            *guard += 10;
            *guard
        }
    }

    #[derive(Debug)]
    struct TestProvider {
        strategy: SandboxStrategy,
        provision_calls: Mutex<u32>,
        heartbeats: Mutex<u32>,
    }

    impl TestProvider {
        fn new(strategy: SandboxStrategy) -> Self {
            Self {
                strategy,
                provision_calls: Mutex::new(0),
                heartbeats: Mutex::new(0),
            }
        }
    }

    #[derive(Debug)]
    struct RecoveryProvider {
        strategy: SandboxStrategy,
        handles: Vec<SandboxHandle>,
        reconnect_results:
            Mutex<HashMap<String, Result<Option<ProvisionedSandbox>, WorkspaceError>>>,
    }

    impl RecoveryProvider {
        fn new(
            strategy: SandboxStrategy,
            handles: Vec<SandboxHandle>,
            reconnect_results: HashMap<String, Result<Option<ProvisionedSandbox>, WorkspaceError>>,
        ) -> Self {
            Self {
                strategy,
                handles,
                reconnect_results: Mutex::new(reconnect_results),
            }
        }
    }

    #[async_trait]
    impl SandboxProvider for TestProvider {
        fn strategy(&self) -> SandboxStrategy {
            self.strategy
        }

        async fn provision(
            &self,
            run_id: &RunId,
            _project: &ProjectKey,
            policy: &SandboxPolicy,
        ) -> Result<ProvisionedSandbox, WorkspaceError> {
            *self
                .provision_calls
                .lock()
                .expect("provision call counter poisoned") += 1;

            Ok(ProvisionedSandbox {
                sandbox_id: crate::sandbox::SandboxId::new(format!("sbx-{}", run_id.as_str())),
                run_id: run_id.clone(),
                path: PathBuf::from(format!("/tmp/{}", run_id.as_str())),
                base: policy.base.clone(),
                strategy: self.strategy,
                base_revision: match &policy.base {
                    SandboxBase::Repo { starting_ref, .. } => {
                        Some(starting_ref.clone().unwrap_or_else(|| "head-1".to_string()))
                    }
                    _ => None,
                },
                branch: Some("main".to_string()),
                is_resumed: false,
                env: HashMap::from([("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())]),
            })
        }

        async fn reconnect(
            &self,
            _run_id: &RunId,
        ) -> Result<Option<ProvisionedSandbox>, WorkspaceError> {
            Ok(None)
        }

        async fn checkpoint(
            &self,
            run_id: &RunId,
            kind: CheckpointKind,
        ) -> Result<SandboxCheckpoint, WorkspaceError> {
            Ok(SandboxCheckpoint {
                sandbox_id: crate::sandbox::SandboxId::new(format!("sbx-{}", run_id.as_str())),
                run_id: run_id.clone(),
                kind,
                rescue_ref: Some("refs/cairn/checkpoint".to_string()),
                upper_snapshot: Some(PathBuf::from(format!(
                    "/tmp/{}/upper.prev.0",
                    run_id.as_str()
                ))),
            })
        }

        async fn restore(
            &self,
            _from_checkpoint: &SandboxCheckpoint,
            _new_run_id: &RunId,
            _project: &ProjectKey,
            _policy: &SandboxPolicy,
        ) -> Result<ProvisionedSandbox, WorkspaceError> {
            Err(WorkspaceError::unimplemented(
                "restore not used in step 3 tests",
            ))
        }

        async fn destroy(
            &self,
            run_id: &RunId,
            _preserve: bool,
        ) -> Result<DestroyResult, WorkspaceError> {
            Ok(DestroyResult {
                sandbox_id: crate::sandbox::SandboxId::new(format!("sbx-{}", run_id.as_str())),
                files_changed: 4,
                bytes_written: 512,
            })
        }

        async fn list(&self) -> Result<Vec<SandboxHandle>, WorkspaceError> {
            Ok(Vec::new())
        }

        async fn heartbeat(&self, _run_id: &RunId) -> Result<(), WorkspaceError> {
            *self.heartbeats.lock().expect("heartbeat counter poisoned") += 1;
            Ok(())
        }
    }

    #[async_trait]
    impl SandboxProvider for RecoveryProvider {
        fn strategy(&self) -> SandboxStrategy {
            self.strategy
        }

        async fn provision(
            &self,
            _run_id: &RunId,
            _project: &ProjectKey,
            _policy: &SandboxPolicy,
        ) -> Result<ProvisionedSandbox, WorkspaceError> {
            Err(WorkspaceError::unimplemented(
                "recovery provider does not provision in tests",
            ))
        }

        async fn reconnect(
            &self,
            run_id: &RunId,
        ) -> Result<Option<ProvisionedSandbox>, WorkspaceError> {
            self.reconnect_results
                .lock()
                .expect("reconnect results lock poisoned")
                .get(run_id.as_str())
                .cloned()
                .unwrap_or(Ok(None))
        }

        async fn checkpoint(
            &self,
            _run_id: &RunId,
            _kind: CheckpointKind,
        ) -> Result<SandboxCheckpoint, WorkspaceError> {
            Err(WorkspaceError::unimplemented(
                "recovery provider does not checkpoint in tests",
            ))
        }

        async fn restore(
            &self,
            _from_checkpoint: &SandboxCheckpoint,
            _new_run_id: &RunId,
            _project: &ProjectKey,
            _policy: &SandboxPolicy,
        ) -> Result<ProvisionedSandbox, WorkspaceError> {
            Err(WorkspaceError::unimplemented(
                "recovery provider does not restore in tests",
            ))
        }

        async fn destroy(
            &self,
            _run_id: &RunId,
            _preserve: bool,
        ) -> Result<DestroyResult, WorkspaceError> {
            Err(WorkspaceError::unimplemented(
                "recovery provider does not destroy in tests",
            ))
        }

        async fn list(&self) -> Result<Vec<SandboxHandle>, WorkspaceError> {
            Ok(self.handles.clone())
        }

        async fn heartbeat(&self, _run_id: &RunId) -> Result<(), WorkspaceError> {
            Ok(())
        }
    }

    fn policy(
        strategy: SandboxStrategyRequest,
        on_resource_exhaustion: OnExhaustion,
    ) -> SandboxPolicy {
        SandboxPolicy {
            strategy,
            base: SandboxBase::Repo {
                repo_id: "octocat/hello".into(),
                starting_ref: Some("abc123".to_string()),
            },
            credentials: vec![crate::sandbox::CredentialReference::Named(
                "github_installation".to_string(),
            )],
            network_egress: None,
            memory_limit_bytes: Some(256),
            cpu_weight: Some(100),
            disk_quota_bytes: Some(128),
            wall_clock_limit: Some(Duration::from_secs(5)),
            on_resource_exhaustion,
            preserve_on_failure: true,
            required_host_caps: HostCapabilityRequirements::default(),
        }
    }

    fn service_with_providers(
        providers: Vec<(SandboxStrategy, Box<dyn SandboxProvider>)>,
    ) -> (SandboxService, Arc<BufferedSandboxEventSink>) {
        let sink = Arc::new(BufferedSandboxEventSink::default());
        let base_dir = unique_test_dir("service");
        let service = SandboxService::new(
            HashMap::from_iter(providers),
            sink.clone(),
            base_dir,
            Arc::new(FixedClock::new(1_000)),
        );
        (service, sink)
    }

    fn unique_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cairn-workspace-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&dir).expect("test temp dir should be creatable");
        dir
    }

    fn run_id() -> RunId {
        RunId::new("run-1")
    }

    fn project() -> ProjectKey {
        ProjectKey::new("tenant-a", "workspace-a", "project-a")
    }

    fn recovery_metadata(run_id: &RunId, state: SandboxState) -> SandboxMetadata {
        SandboxMetadata {
            sandbox_id: crate::sandbox::SandboxId::new(format!("sbx-{}", run_id.as_str())),
            run_id: run_id.clone(),
            task_id: None,
            project: project(),
            strategy: SandboxStrategy::Overlay,
            state,
            base_rev: Some("abc123".to_string()),
            repo_id: Some(crate::sandbox::RepoId::new("octocat/hello")),
            path: PathBuf::from(format!("/tmp/{}", run_id.as_str())),
            pid: Some(42),
            created_at: 1_000,
            heartbeat_at: 1_010,
            policy_hash: "policy:test".to_string(),
        }
    }

    fn reconnected_sandbox(run_id: &RunId) -> ProvisionedSandbox {
        ProvisionedSandbox {
            sandbox_id: crate::sandbox::SandboxId::new(format!("sbx-{}", run_id.as_str())),
            run_id: run_id.clone(),
            path: PathBuf::from(format!("/tmp/{}/merged", run_id.as_str())),
            base: SandboxBase::Repo {
                repo_id: crate::sandbox::RepoId::new("octocat/hello"),
                starting_ref: Some("abc123".to_string()),
            },
            strategy: SandboxStrategy::Overlay,
            base_revision: Some("abc123".to_string()),
            branch: Some("main".to_string()),
            is_resumed: true,
            env: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn preferred_strategy_falls_back_and_emits_degraded_event() {
        let (service, sink) = service_with_providers(vec![(
            SandboxStrategy::Reflink,
            Box::new(TestProvider::new(SandboxStrategy::Reflink)),
        )]);

        let sandbox = service
            .provision_or_reconnect(
                &run_id(),
                None,
                project(),
                policy(
                    SandboxStrategyRequest::Preferred(SandboxStrategy::Overlay),
                    OnExhaustion::Destroy,
                ),
            )
            .await
            .unwrap();

        assert_eq!(sandbox.strategy, SandboxStrategy::Reflink);
        assert_eq!(service.state_for(&run_id()), Some(SandboxState::Ready));

        let events = sink.drain();
        assert!(matches!(
            &events[0],
            SandboxEvent::SandboxPolicyDegraded {
                requested: SandboxStrategy::Overlay,
                actual: SandboxStrategy::Reflink,
                ..
            }
        ));
        assert!(matches!(
            &events[1],
            SandboxEvent::SandboxProvisioned {
                strategy: SandboxStrategy::Reflink,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn provision_reconnect_activate_checkpoint_and_destroy_flow() {
        let (service, sink) = service_with_providers(vec![(
            SandboxStrategy::Overlay,
            Box::new(TestProvider::new(SandboxStrategy::Overlay)),
        )]);
        let run = run_id();

        let provisioned = service
            .provision_or_reconnect(
                &run,
                Some(cairn_domain::TaskId::new("task-1")),
                project(),
                policy(
                    SandboxStrategyRequest::Force(SandboxStrategy::Overlay),
                    OnExhaustion::Destroy,
                ),
            )
            .await
            .unwrap();
        let reconnected = service
            .provision_or_reconnect(
                &run,
                Some(cairn_domain::TaskId::new("task-1")),
                project(),
                policy(
                    SandboxStrategyRequest::Force(SandboxStrategy::Overlay),
                    OnExhaustion::Destroy,
                ),
            )
            .await
            .unwrap();
        assert!(reconnected.is_resumed);
        assert_eq!(reconnected.sandbox_id, provisioned.sandbox_id);

        service.activate(&run, Some(42)).await.unwrap();
        service.heartbeat(&run).await.unwrap();
        service
            .checkpoint(&run, CheckpointKind::Intent)
            .await
            .unwrap();
        service
            .destroy(&run, false, DestroyReason::Completed)
            .await
            .unwrap();

        assert_eq!(service.state_for(&run), Some(SandboxState::Destroyed));
        let metadata = service.metadata_for(&run).unwrap();
        assert_eq!(metadata.task_id, Some(cairn_domain::TaskId::new("task-1")));

        let events = sink.drain();
        assert!(matches!(events[0], SandboxEvent::SandboxProvisioned { .. }));
        assert!(matches!(
            events[1],
            SandboxEvent::SandboxActivated { pid: Some(42), .. }
        ));
        assert!(matches!(events[2], SandboxEvent::SandboxHeartbeat { .. }));
        assert!(matches!(
            events[3],
            SandboxEvent::SandboxCheckpointed {
                checkpoint_kind: CheckpointKind::Intent,
                ..
            }
        ));
        assert!(matches!(
            events[4],
            SandboxEvent::SandboxDestroyed {
                reason: DestroyReason::Completed,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn resource_limit_destroy_mode_emits_limit_and_destroyed() {
        let (service, sink) = service_with_providers(vec![(
            SandboxStrategy::Overlay,
            Box::new(TestProvider::new(SandboxStrategy::Overlay)),
        )]);
        let run = run_id();

        service
            .provision_or_reconnect(
                &run,
                None,
                project(),
                policy(
                    SandboxStrategyRequest::Force(SandboxStrategy::Overlay),
                    OnExhaustion::Destroy,
                ),
            )
            .await
            .unwrap();
        service.activate(&run, Some(7)).await.unwrap();
        service
            .observe_resource_limit(&run, ResourceDimension::DiskBytes, 256)
            .await
            .unwrap();

        assert_eq!(service.state_for(&run), Some(SandboxState::Destroyed));
        let events = sink.drain();
        assert!(matches!(
            events[2],
            SandboxEvent::SandboxResourceLimitExceeded {
                dimension: ResourceDimension::DiskBytes,
                limit: 128,
                observed: 256,
                ..
            }
        ));
        assert!(matches!(
            events[3],
            SandboxEvent::SandboxDestroyed {
                reason: DestroyReason::ResourceLimitExceeded {
                    dimension: ResourceDimension::DiskBytes,
                    limit: 128,
                    observed: 256,
                },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn resource_limit_pause_mode_preserves_sandbox() {
        let (service, sink) = service_with_providers(vec![(
            SandboxStrategy::Overlay,
            Box::new(TestProvider::new(SandboxStrategy::Overlay)),
        )]);
        let run = run_id();

        service
            .provision_or_reconnect(
                &run,
                None,
                project(),
                policy(
                    SandboxStrategyRequest::Force(SandboxStrategy::Overlay),
                    OnExhaustion::PauseAwaitOperator,
                ),
            )
            .await
            .unwrap();
        service.activate(&run, Some(8)).await.unwrap();
        service
            .observe_resource_limit(&run, ResourceDimension::MemoryBytes, 300)
            .await
            .unwrap();

        assert_eq!(service.state_for(&run), Some(SandboxState::Preserved));
        let events = sink.drain();
        assert!(matches!(
            events[3],
            SandboxEvent::SandboxPreserved {
                reason: PreservationReason::AwaitingResourceRaise {
                    dimension: ResourceDimension::MemoryBytes,
                    limit: 256,
                    observed: 300,
                },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn resource_limit_report_only_keeps_sandbox_active() {
        let (service, sink) = service_with_providers(vec![(
            SandboxStrategy::Overlay,
            Box::new(TestProvider::new(SandboxStrategy::Overlay)),
        )]);
        let run = run_id();

        service
            .provision_or_reconnect(
                &run,
                None,
                project(),
                policy(
                    SandboxStrategyRequest::Force(SandboxStrategy::Overlay),
                    OnExhaustion::ReportOnly,
                ),
            )
            .await
            .unwrap();
        service.activate(&run, Some(9)).await.unwrap();
        service
            .observe_resource_limit(&run, ResourceDimension::WallClockMs, 7_500)
            .await
            .unwrap();

        assert_eq!(service.state_for(&run), Some(SandboxState::Active));
        let events = sink.drain();
        assert_eq!(events.len(), 3);
        assert!(matches!(
            events[2],
            SandboxEvent::SandboxResourceLimitExceeded {
                dimension: ResourceDimension::WallClockMs,
                limit: 5_000,
                observed: 7_500,
                ..
            }
        ));
    }

    #[test]
    fn metadata_captures_task_id_and_policy_hash() {
        let metadata = SandboxMetadata {
            sandbox_id: crate::sandbox::SandboxId::new("sbx-run"),
            run_id: run_id(),
            task_id: Some(cairn_domain::TaskId::new("task-1")),
            project: project(),
            strategy: SandboxStrategy::Overlay,
            state: SandboxState::Ready,
            base_rev: Some("abc123".to_string()),
            repo_id: Some("octocat/hello".into()),
            path: PathBuf::from("/tmp/run-1"),
            pid: None,
            created_at: 10,
            heartbeat_at: 20,
            policy_hash: "policy:abc".to_string(),
        };

        assert_eq!(metadata.task_id, Some(cairn_domain::TaskId::new("task-1")));
        assert_eq!(metadata.policy_hash, "policy:abc");
    }

    #[tokio::test]
    async fn service_persists_metadata_for_recovery() {
        let (service, _sink) = service_with_providers(vec![(
            SandboxStrategy::Overlay,
            Box::new(TestProvider::new(SandboxStrategy::Overlay)),
        )]);
        let run = run_id();

        service
            .provision_or_reconnect(
                &run,
                Some(cairn_domain::TaskId::new("task-9")),
                project(),
                policy(
                    SandboxStrategyRequest::Force(SandboxStrategy::Overlay),
                    OnExhaustion::Destroy,
                ),
            )
            .await
            .unwrap();

        let metadata_path = service.base_dir().join("sbx-run-1").join("meta.json");
        let metadata: SandboxMetadata =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        assert_eq!(metadata.state, SandboxState::Ready);
        assert_eq!(metadata.task_id, Some(cairn_domain::TaskId::new("task-9")));

        service.activate(&run, Some(77)).await.unwrap();
        let metadata: SandboxMetadata =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        assert_eq!(metadata.state, SandboxState::Active);
        assert_eq!(metadata.pid, Some(77));

        service
            .checkpoint(&run, CheckpointKind::Result)
            .await
            .unwrap();
        let metadata: SandboxMetadata =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        assert_eq!(metadata.state, SandboxState::Checkpointed);

        service
            .preserve(&run, PreservationReason::ControlPlaneRestart)
            .unwrap();
        let metadata: SandboxMetadata =
            serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        assert_eq!(metadata.state, SandboxState::Preserved);
        assert_eq!(metadata.pid, None);
    }

    #[tokio::test]
    async fn recover_all_reconnects_sandbox_and_normalizes_active_state() {
        let run = RunId::new("run-recover-ok");
        let provider = RecoveryProvider::new(
            SandboxStrategy::Overlay,
            vec![SandboxHandle {
                metadata: recovery_metadata(&run, SandboxState::Active),
            }],
            HashMap::from([(run.to_string(), Ok(Some(reconnected_sandbox(&run))))]),
        );
        let (service, _sink) =
            service_with_providers(vec![(SandboxStrategy::Overlay, Box::new(provider))]);

        let summary = service.recover_all().await.unwrap();

        assert_eq!(summary.reconnected, 1);
        assert_eq!(summary.preserved, 0);
        assert_eq!(summary.failed, 0);
        assert_eq!(service.state_for(&run), Some(SandboxState::Ready));

        let recovered = service.metadata_for(&run).unwrap();
        assert_eq!(recovered.state, SandboxState::Ready);
        assert_eq!(recovered.pid, None);
        assert_eq!(recovered.base_rev.as_deref(), Some("abc123"));
        assert_eq!(recovered.path, PathBuf::from("/tmp/run-recover-ok/merged"));
    }

    #[tokio::test]
    async fn recover_all_emits_sandbox_lost_when_registry_entry_has_no_directory() {
        // RFC 020 §"Run recovery matrix": a registry sidecar whose
        // sandbox directory is missing at recovery must surface as
        // `SandboxLost` + `summary.lost_runs` so the run-level service
        // can transition the bound run to `failed` with
        // `reason: sandbox_lost`.
        let run = RunId::new("run-lost");
        let provider = RecoveryProvider::new(SandboxStrategy::Overlay, Vec::new(), HashMap::new());
        let (service, sink) =
            service_with_providers(vec![(SandboxStrategy::Overlay, Box::new(provider))]);

        // Seed a registry entry pointing at a path that deliberately
        // doesn't exist. `base_dir()` is the live service base dir so
        // the registry sidecar writer picks it up.
        let entry = super::RegistryEntry {
            sandbox_id: crate::sandbox::SandboxId::new(format!("sbx-{}", run.as_str())),
            run_id: run.clone(),
            project: project(),
            strategy: SandboxStrategy::Overlay,
            path: service.base_dir().join("sbx-run-lost-missing-root"),
            registered_at: 1_000,
            repo_id: None,
            allowlist_revoked_handled: false,
        };
        service
            .write_registry_entry(&entry)
            .expect("write registry entry");

        let summary = service.recover_all().await.unwrap();

        assert_eq!(summary.reconnected, 0);
        assert_eq!(summary.preserved, 0);
        assert_eq!(summary.failed, 0);
        assert_eq!(summary.lost, 1);
        assert_eq!(summary.lost_runs.len(), 1);
        assert_eq!(summary.lost_runs[0].0, run);
        assert_eq!(summary.lost_runs[0].1, project());

        let events = sink.drain();
        assert_eq!(events.len(), 1, "expected exactly one SandboxLost event");
        match &events[0] {
            SandboxEvent::SandboxLost {
                run_id, project: p, ..
            } => {
                assert_eq!(run_id, &run);
                assert_eq!(p, &project());
            }
            other => panic!("expected SandboxLost, got {other:?}"),
        }

        // Idempotency: a second recovery sweep must not re-emit the
        // event — the entry was cleared after the first detection.
        let second = service.recover_all().await.unwrap();
        assert_eq!(second.lost, 0);
        assert!(second.lost_runs.is_empty());
    }

    #[tokio::test]
    async fn recover_all_emits_allowlist_revoked_when_repo_not_allowlisted() {
        // RFC 020 §"Run recovery matrix" — AllowlistRevoked row.
        // Seed a registry entry for a `SandboxBase::Repo` sandbox
        // whose path exists on disk (so the lost-sweep skips it) but
        // whose bound repo is NOT in the project allowlist. Recovery
        // must emit `SandboxAllowlistRevoked` and surface the triple
        // on `summary.allowlist_revoked_runs`.
        use crate::repo_store::access_service::ProjectRepoAccessService;

        let run = RunId::new("run-revoked");
        let repo_id = crate::sandbox::RepoId::new("octocat/hello");
        let provider = RecoveryProvider::new(SandboxStrategy::Overlay, Vec::new(), HashMap::new());
        let allowlist = Arc::new(ProjectRepoAccessService::new());
        let sink = Arc::new(BufferedSandboxEventSink::default());
        let base_dir = unique_test_dir("allowlist-revoked");
        let service = SandboxService::new(
            HashMap::from([(
                SandboxStrategy::Overlay,
                Box::new(provider) as Box<dyn crate::providers::SandboxProvider>,
            )]),
            sink.clone(),
            base_dir.clone(),
            Arc::new(FixedClock::new(1_000)),
        )
        .with_allowlist(allowlist);

        // Sandbox path exists → lost-sweep skips. Repo not in
        // allowlist → allowlist-revoked sweep fires.
        let sandbox_path = base_dir.join("sbx-run-revoked");
        fs::create_dir_all(&sandbox_path).expect("create stub sandbox dir");
        service
            .seed_registry_entry_for_test_with_repo(
                crate::sandbox::SandboxId::new("sbx-run-revoked"),
                run.clone(),
                project(),
                SandboxStrategy::Overlay,
                sandbox_path,
                Some(repo_id.clone()),
            )
            .expect("seed registry entry");

        let summary = service.recover_all().await.unwrap();

        assert_eq!(summary.lost, 0);
        assert_eq!(summary.preserved_allowlist_revoked, 1);
        assert_eq!(summary.allowlist_revoked_runs.len(), 1);
        assert_eq!(summary.allowlist_revoked_runs[0].0, run);
        assert_eq!(summary.allowlist_revoked_runs[0].1, project());
        assert_eq!(summary.allowlist_revoked_runs[0].2, repo_id);

        let events = sink.drain();
        assert_eq!(events.len(), 1, "expected exactly one event");
        match &events[0] {
            SandboxEvent::SandboxAllowlistRevoked {
                run_id,
                project: p,
                repo_id: r,
                ..
            } => {
                assert_eq!(run_id, &run);
                assert_eq!(p, &project());
                assert_eq!(r, &repo_id);
            }
            other => panic!("expected SandboxAllowlistRevoked, got {other:?}"),
        }

        // Idempotency: second sweep must not re-emit.
        let second = service.recover_all().await.unwrap();
        assert_eq!(second.preserved_allowlist_revoked, 0);
        assert!(second.allowlist_revoked_runs.is_empty());
        assert!(sink.drain().is_empty());
    }

    #[tokio::test]
    async fn recover_all_skips_allowlist_revoked_when_repo_still_allowed() {
        // Allowlisted repo → no emission.
        use crate::repo_store::access_service::ProjectRepoAccessService;
        use cairn_domain::{ActorRef, OperatorId, RepoAccessContext};

        let run = RunId::new("run-still-allowed");
        let repo_id = crate::sandbox::RepoId::new("octocat/hello");
        let provider = RecoveryProvider::new(SandboxStrategy::Overlay, Vec::new(), HashMap::new());
        let allowlist = Arc::new(ProjectRepoAccessService::new());
        allowlist
            .allow(
                &RepoAccessContext { project: project() },
                &repo_id,
                ActorRef::Operator {
                    operator_id: OperatorId::new("op"),
                },
            )
            .await
            .expect("allow repo");
        let sink = Arc::new(BufferedSandboxEventSink::default());
        let base_dir = unique_test_dir("allowlist-still-allowed");
        let service = SandboxService::new(
            HashMap::from([(
                SandboxStrategy::Overlay,
                Box::new(provider) as Box<dyn crate::providers::SandboxProvider>,
            )]),
            sink.clone(),
            base_dir.clone(),
            Arc::new(FixedClock::new(1_000)),
        )
        .with_allowlist(allowlist);

        let sandbox_path = base_dir.join("sbx-run-still-allowed");
        fs::create_dir_all(&sandbox_path).expect("create stub sandbox dir");
        service
            .seed_registry_entry_for_test_with_repo(
                crate::sandbox::SandboxId::new("sbx-run-still-allowed"),
                run.clone(),
                project(),
                SandboxStrategy::Overlay,
                sandbox_path,
                Some(repo_id),
            )
            .expect("seed registry entry");

        let summary = service.recover_all().await.unwrap();
        assert_eq!(summary.preserved_allowlist_revoked, 0);
        assert!(summary.allowlist_revoked_runs.is_empty());
        assert!(sink.drain().is_empty());
    }

    #[tokio::test]
    async fn recover_all_preserves_sandbox_on_base_revision_drift() {
        let run = RunId::new("run-recover-drift");
        let provider = RecoveryProvider::new(
            SandboxStrategy::Overlay,
            vec![SandboxHandle {
                metadata: recovery_metadata(&run, SandboxState::Ready),
            }],
            HashMap::from([(
                run.to_string(),
                Err(WorkspaceError::BaseRevisionDrift {
                    run_id: run.clone(),
                    expected: "abc123".to_string(),
                    actual: "def456".to_string(),
                }),
            )]),
        );
        let (service, sink) =
            service_with_providers(vec![(SandboxStrategy::Overlay, Box::new(provider))]);

        let summary = service.recover_all().await.unwrap();

        assert_eq!(summary.reconnected, 0);
        assert_eq!(summary.preserved, 1);
        assert_eq!(summary.failed, 0);
        assert_eq!(service.state_for(&run), Some(SandboxState::Preserved));

        let events = sink.drain();
        assert!(matches!(
            &events[0],
            SandboxEvent::SandboxBaseRevisionDrift {
                expected,
                actual,
                ..
            } if expected == "abc123" && actual == "def456"
        ));
        assert!(matches!(
            &events[1],
            SandboxEvent::SandboxPreserved {
                reason: PreservationReason::BaseRevisionDrift { expected, actual },
                ..
            } if expected == "abc123" && actual == "def456"
        ));
    }
}
