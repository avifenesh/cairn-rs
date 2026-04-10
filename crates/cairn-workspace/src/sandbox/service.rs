use std::collections::{hash_map::DefaultHasher, HashMap};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::{
    CheckpointKind, DestroyReason, OnExhaustion, PreservationReason, ProjectKey, ResourceDimension,
    RunId, TaskId,
};

use crate::error::WorkspaceError;
use crate::providers::SandboxProvider;
use crate::sandbox::{
    DestroyResult, ProvisionedSandbox, SandboxCheckpoint, SandboxErrorKind, SandboxEvent,
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
}

pub struct SandboxService {
    providers: HashMap<SandboxStrategy, Box<dyn SandboxProvider>>,
    event_sink: Arc<dyn SandboxEventSink>,
    base_dir: PathBuf,
    clock: Arc<dyn Clock>,
    sessions: RwLock<HashMap<RunId, SandboxSession>>,
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
        }
    }

    pub fn base_dir(&self) -> &PathBuf {
        &self.base_dir
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

        let mut summary = SandboxRecoverySummary::default();
        for handle in handles {
            let mut metadata = handle.metadata;
            if matches!(metadata.state, SandboxState::Destroyed | SandboxState::Failed) {
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
                Err(WorkspaceError::BaseRevisionDrift { expected, actual, .. }) => {
                    let detected_at = self.clock.now_millis();
                    if let Some(repo_id) = metadata.repo_id.clone() {
                        self.event_sink.publish(SandboxEvent::SandboxBaseRevisionDrift {
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
                    self.event_sink.publish(SandboxEvent::SandboxProvisioningFailed {
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
