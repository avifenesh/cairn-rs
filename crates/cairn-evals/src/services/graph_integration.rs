//! Graph integration for prompt/eval services.
//!
//! Wraps Worker 6's `EvalGraphProjector` to wire graph linkage into
//! the prompt release and eval run lifecycle.

use cairn_domain::{EvalRunId, PromptAssetId, PromptReleaseId, PromptVersionId};
use cairn_graph::eval_projector::EvalGraphProjector;
use cairn_graph::projections::GraphProjection;

/// Graph-integrated prompt/eval lifecycle coordinator.
///
/// Call methods on this alongside the release/eval services to maintain
/// graph linkage. Graph projection failures are non-fatal — the service
/// flow succeeds even if graph writes fail (logged, not propagated).
pub struct GraphIntegration<P: GraphProjection> {
    projector: EvalGraphProjector<P>,
}

impl<P: GraphProjection> GraphIntegration<P> {
    pub fn new(projection: P) -> Self {
        Self {
            projector: EvalGraphProjector::new(projection),
        }
    }

    /// Call after creating a prompt asset.
    pub async fn on_asset_created(&self, asset_id: &PromptAssetId) {
        let _ = self.projector.on_asset_created(asset_id, now_ms()).await;
    }

    /// Call after creating a prompt version.
    pub async fn on_version_created(&self, version_id: &PromptVersionId, asset_id: &PromptAssetId) {
        let _ = self
            .projector
            .on_version_created(version_id, asset_id, now_ms())
            .await;
    }

    /// Call after creating a prompt release.
    pub async fn on_release_created(
        &self,
        release_id: &PromptReleaseId,
        version_id: &PromptVersionId,
    ) {
        let _ = self
            .projector
            .on_release_created(release_id, version_id, now_ms())
            .await;
    }

    /// Call after a rollback swaps active releases.
    pub async fn on_rollback(
        &self,
        from_release_id: &PromptReleaseId,
        to_release_id: &PromptReleaseId,
    ) {
        let _ = self
            .projector
            .on_release_rollback(from_release_id, to_release_id, now_ms())
            .await;
    }

    /// Call after creating an eval run.
    pub async fn on_eval_run_created(
        &self,
        eval_run_id: &EvalRunId,
        release_id: Option<&PromptReleaseId>,
    ) {
        let _ = self
            .projector
            .on_eval_run_created(eval_run_id, release_id, now_ms())
            .await;
    }

    /// Call when a runtime run/task uses a resolved prompt release.
    pub async fn on_prompt_used(&self, run_or_task_id: &str, release_id: &PromptReleaseId) {
        let _ = self
            .projector
            .on_prompt_used(run_or_task_id, release_id, now_ms())
            .await;
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_graph::projections::{GraphEdge, GraphNode, GraphProjectionError};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct TestGraphInner {
        nodes: Mutex<HashMap<String, GraphNode>>,
        edges: Mutex<Vec<GraphEdge>>,
    }

    /// Newtype wrapper to satisfy orphan rule.
    struct TestGraph(Arc<TestGraphInner>);

    impl TestGraph {
        fn new() -> Self {
            Self(Arc::new(TestGraphInner {
                nodes: Mutex::new(HashMap::new()),
                edges: Mutex::new(Vec::new()),
            }))
        }

        fn inner(&self) -> &TestGraphInner {
            &self.0
        }

        fn clone_inner(&self) -> Self {
            Self(self.0.clone())
        }
    }

    #[async_trait::async_trait]
    impl GraphProjection for TestGraph {
        async fn add_node(&self, node: GraphNode) -> Result<(), GraphProjectionError> {
            self.0
                .nodes
                .lock()
                .unwrap()
                .insert(node.node_id.clone(), node);
            Ok(())
        }
        async fn add_edge(&self, edge: GraphEdge) -> Result<(), GraphProjectionError> {
            self.0.edges.lock().unwrap().push(edge);
            Ok(())
        }
        async fn node_exists(&self, node_id: &str) -> Result<bool, GraphProjectionError> {
            Ok(self.0.nodes.lock().unwrap().contains_key(node_id))
        }
    }

    #[tokio::test]
    async fn full_graph_integration_lifecycle() {
        let graph = TestGraph::new();
        let integration = GraphIntegration::new(graph.clone_inner());

        let asset_id = PromptAssetId::new("prompt_planner");
        let version_id = PromptVersionId::new("pv_1");
        let release_id = PromptReleaseId::new("rel_1");
        let eval_id = EvalRunId::new("eval_1");

        // Wire the full lifecycle
        integration.on_asset_created(&asset_id).await;
        integration.on_version_created(&version_id, &asset_id).await;
        integration
            .on_release_created(&release_id, &version_id)
            .await;
        integration
            .on_eval_run_created(&eval_id, Some(&release_id))
            .await;
        integration.on_prompt_used("run_42", &release_id).await;

        let nodes = graph.inner().nodes.lock().unwrap();
        assert_eq!(nodes.len(), 4); // asset, version, release, eval_run

        let edges = graph.inner().edges.lock().unwrap();
        assert_eq!(edges.len(), 4); // derived_from, released_as, evaluated_by, used_prompt
    }

    #[tokio::test]
    async fn rollback_creates_graph_edge() {
        let graph = TestGraph::new();
        let integration = GraphIntegration::new(graph.clone_inner());

        integration
            .on_rollback(
                &PromptReleaseId::new("rel_new"),
                &PromptReleaseId::new("rel_old"),
            )
            .await;

        let edges = graph.inner().edges.lock().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].source_node_id, "rel_new");
        assert_eq!(edges[0].target_node_id, "rel_old");
    }
}
