//! Graph-linkable event data for tool invocations.
//!
//! Produces structured records that cairn-graph's projection layer can
//! consume to create ToolInvocation nodes and UsedTool edges. This module
//! does NOT depend on cairn-graph — it exports data shapes that the graph
//! projection imports.

use cairn_domain::ids::{RunId, TaskId, ToolInvocationId};
use cairn_domain::policy::ExecutionClass;
use cairn_domain::tool_invocation::{
    ToolInvocationOutcomeKind, ToolInvocationRecord, ToolInvocationState, ToolInvocationTarget,
};
use serde::{Deserialize, Serialize};

/// Data for creating a ToolInvocation graph node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationNodeData {
    pub invocation_id: ToolInvocationId,
    pub tool_name: String,
    pub plugin_id: Option<String>,
    pub execution_class: ExecutionClass,
    pub state: ToolInvocationState,
    pub outcome: Option<ToolInvocationOutcomeKind>,
    pub requested_at_ms: u64,
    pub finished_at_ms: Option<u64>,
}

/// Data for creating a UsedTool edge linking a task/run to a tool invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsedToolEdgeData {
    pub invocation_id: ToolInvocationId,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub tool_name: String,
    pub outcome: Option<ToolInvocationOutcomeKind>,
}

/// Extracts graph-linkable node data from a terminal invocation record.
pub fn to_node_data(record: &ToolInvocationRecord) -> ToolInvocationNodeData {
    let (tool_name, plugin_id) = match &record.target {
        ToolInvocationTarget::Builtin { tool_name } => (tool_name.clone(), None),
        ToolInvocationTarget::Plugin {
            plugin_id,
            tool_name,
        } => (tool_name.clone(), Some(plugin_id.clone())),
    };

    ToolInvocationNodeData {
        invocation_id: record.invocation_id.clone(),
        tool_name,
        plugin_id,
        execution_class: record.execution_class,
        state: record.state,
        outcome: record.outcome,
        requested_at_ms: record.requested_at_ms,
        finished_at_ms: record.finished_at_ms,
    }
}

/// Extracts graph-linkable edge data from a terminal invocation record.
pub fn to_edge_data(record: &ToolInvocationRecord) -> UsedToolEdgeData {
    let tool_name = match &record.target {
        ToolInvocationTarget::Builtin { tool_name } => tool_name.clone(),
        ToolInvocationTarget::Plugin { tool_name, .. } => tool_name.clone(),
    };

    UsedToolEdgeData {
        invocation_id: record.invocation_id.clone(),
        run_id: record.run_id.clone(),
        task_id: record.task_id.clone(),
        tool_name,
        outcome: record.outcome,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::policy::ExecutionClass;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::tool_invocation::{
        ToolInvocationOutcomeKind, ToolInvocationRecord, ToolInvocationState, ToolInvocationTarget,
    };

    fn completed_record() -> ToolInvocationRecord {
        let requested = ToolInvocationRecord::new_requested(
            "inv_1".into(),
            ProjectKey::new("t", "w", "p"),
            Some("sess_1".into()),
            Some("run_1".into()),
            Some("task_1".into()),
            ToolInvocationTarget::Plugin {
                plugin_id: "com.example.git".to_owned(),
                tool_name: "git.status".to_owned(),
            },
            ExecutionClass::SandboxedProcess,
            100,
        );

        let started = requested.mark_started(101).unwrap();
        started
            .mark_finished(ToolInvocationOutcomeKind::Success, None, 105)
            .unwrap()
    }

    #[test]
    fn node_data_from_plugin_record() {
        let record = completed_record();
        let node = to_node_data(&record);

        assert_eq!(node.tool_name, "git.status");
        assert_eq!(node.plugin_id, Some("com.example.git".to_owned()));
        assert_eq!(node.execution_class, ExecutionClass::SandboxedProcess);
        assert_eq!(node.state, ToolInvocationState::Completed);
        assert_eq!(node.outcome, Some(ToolInvocationOutcomeKind::Success));
    }

    #[test]
    fn edge_data_links_task_to_invocation() {
        let record = completed_record();
        let edge = to_edge_data(&record);

        assert_eq!(edge.invocation_id.as_str(), "inv_1");
        assert_eq!(edge.run_id.as_ref().unwrap().as_str(), "run_1");
        assert_eq!(edge.task_id.as_ref().unwrap().as_str(), "task_1");
        assert_eq!(edge.tool_name, "git.status");
    }

    #[test]
    fn node_data_from_builtin_record() {
        let mut record = completed_record();
        record.target = ToolInvocationTarget::Builtin {
            tool_name: "fs.read".to_owned(),
        };
        let node = to_node_data(&record);

        assert_eq!(node.tool_name, "fs.read");
        assert!(node.plugin_id.is_none());
    }
}
