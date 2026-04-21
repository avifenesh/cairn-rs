use crate::{ProjectKey, TaskId};
use serde::{Deserialize, Serialize};

/// Edge-kind taxonomy for task dependencies, mirroring FF 0.2's
/// `dependency_kind` FCALL argument.
///
/// Modelled as an enum (rather than a free-form string) so unknown
/// values are rejected at the serde boundary with a 422 instead of
/// propagating to FF as a generic 500. FF 0.2 ships a single variant
/// (`success_only`); additional variants land as FF surfaces them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyKind {
    /// Downstream becomes eligible only when upstream terminates in a
    /// success state. Any non-success outcome (failed / cancelled /
    /// expired) cascades as `skipped` to downstream.
    #[default]
    SuccessOnly,
}

impl DependencyKind {
    /// Serialise for the FF FCALL `dependency_kind` argument. FF's Lua
    /// contract expects a bare lowercase string, not the full JSON
    /// serialisation.
    pub fn as_ff_str(&self) -> &'static str {
        match self {
            Self::SuccessOnly => "success_only",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDependency {
    pub dependent_task_id: TaskId,
    pub depends_on_task_id: TaskId,
    pub project: ProjectKey,
    pub created_at_ms: u64,
    /// Edge kind (currently only `success_only`). Defaults when absent
    /// so pre-existing event-log records deserialise.
    #[serde(default)]
    pub dependency_kind: DependencyKind,
    /// Opaque caller-supplied reference stored on the FF edge and
    /// surfaced to the downstream task after upstream resolution.
    /// Cairn never dereferences this value; downstream consumers are
    /// responsible for interpreting and validating it. See
    /// `SECURITY.md` for the contract. Defaults to `None` so pre-
    /// existing event-log records deserialise.
    #[serde(default)]
    pub data_passing_ref: Option<String>,
}

/// Wire shape for HTTP responses on the dependency routes.
///
/// Under the FF-authoritative model (see `docs/design/CAIRN-FABRIC-FINALIZED.md`)
/// cairn does not persist dependency records — FF owns edge state via
/// `ff_stage_dependency_edge` / `ff_apply_dependency_to_child`. This
/// struct is synthesized by the service layer when responding to
/// `declare_dependency` / `check_dependencies` calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDependencyRecord {
    pub dependency: TaskDependency,
    pub resolved_at_ms: Option<u64>,
}
