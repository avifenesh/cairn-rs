use cairn_domain::{EvalRunId, PromptAssetId, PromptReleaseId, PromptVersionId};

use crate::projections::{
    EdgeKind, GraphEdge, GraphNode, GraphProjection, GraphProjectionError, NodeKind,
};

/// Projects prompt registry and eval lifecycle events into graph structure.
///
/// This projector handles the prompt-asset -> version -> release -> eval-run
/// linkage that the RuntimeEvent-based EventProjector does not cover.
/// It is called by prompt/eval services when they create or transition entities.
pub struct EvalGraphProjector<P: GraphProjection> {
    projection: P,
}

impl<P: GraphProjection> EvalGraphProjector<P> {
    pub fn new(projection: P) -> Self {
        Self { projection }
    }

    /// Record a prompt asset in the graph.
    pub async fn on_asset_created(
        &self,
        asset_id: &PromptAssetId,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_node(GraphNode {
                node_id: asset_id.as_str().to_owned(),
                kind: NodeKind::PromptAsset,
                project: None,
                created_at: ts,
            })
            .await
    }

    /// Record a prompt version, linked to its parent asset.
    pub async fn on_version_created(
        &self,
        version_id: &PromptVersionId,
        asset_id: &PromptAssetId,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_node(GraphNode {
                node_id: version_id.as_str().to_owned(),
                kind: NodeKind::PromptVersion,
                project: None,
                created_at: ts,
            })
            .await?;

        // Version derives from asset.
        self.projection
            .add_edge(GraphEdge {
                source_node_id: asset_id.as_str().to_owned(),
                target_node_id: version_id.as_str().to_owned(),
                kind: EdgeKind::DerivedFrom,
                created_at: ts,
                confidence: None,
            })
            .await
    }

    /// Record a prompt release, linked to its version.
    pub async fn on_release_created(
        &self,
        release_id: &PromptReleaseId,
        version_id: &PromptVersionId,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_node(GraphNode {
                node_id: release_id.as_str().to_owned(),
                kind: NodeKind::PromptRelease,
                project: None,
                created_at: ts,
            })
            .await?;

        // Release is released_as from version.
        self.projection
            .add_edge(GraphEdge {
                source_node_id: version_id.as_str().to_owned(),
                target_node_id: release_id.as_str().to_owned(),
                kind: EdgeKind::ReleasedAs,
                created_at: ts,
                confidence: None,
            })
            .await
    }

    /// Record a rollback: new release rolled_back_to a previous release.
    pub async fn on_release_rollback(
        &self,
        new_release_id: &PromptReleaseId,
        rolled_back_to_id: &PromptReleaseId,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_edge(GraphEdge {
                source_node_id: new_release_id.as_str().to_owned(),
                target_node_id: rolled_back_to_id.as_str().to_owned(),
                kind: EdgeKind::RolledBackTo,
                created_at: ts,
                confidence: None,
            })
            .await
    }

    /// Record an eval run, linked to the release it evaluated.
    pub async fn on_eval_run_created(
        &self,
        eval_run_id: &EvalRunId,
        release_id: Option<&PromptReleaseId>,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_node(GraphNode {
                node_id: eval_run_id.as_str().to_owned(),
                kind: NodeKind::EvalRun,
                project: None,
                created_at: ts,
            })
            .await?;

        if let Some(release_id) = release_id {
            // Eval run evaluated_by the release.
            self.projection
                .add_edge(GraphEdge {
                    source_node_id: release_id.as_str().to_owned(),
                    target_node_id: eval_run_id.as_str().to_owned(),
                    kind: EdgeKind::EvaluatedBy,
                    created_at: ts,
                    confidence: None,
                })
                .await?;
        }

        Ok(())
    }

    /// Link a runtime outcome to the prompt release that was used.
    pub async fn on_prompt_used(
        &self,
        run_or_task_node_id: &str,
        release_id: &PromptReleaseId,
        ts: u64,
    ) -> Result<(), GraphProjectionError> {
        self.projection
            .add_edge(GraphEdge {
                source_node_id: run_or_task_node_id.to_owned(),
                target_node_id: release_id.as_str().to_owned(),
                kind: EdgeKind::UsedPrompt,
                created_at: ts,
                confidence: None,
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct MemGraph {
        nodes: Mutex<HashMap<String, GraphNode>>,
        edges: Mutex<Vec<GraphEdge>>,
    }

    impl MemGraph {
        fn new() -> Self {
            Self {
                nodes: Mutex::new(HashMap::new()),
                edges: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl GraphProjection for Arc<MemGraph> {
        async fn add_node(&self, node: GraphNode) -> Result<(), GraphProjectionError> {
            self.nodes
                .lock()
                .unwrap()
                .insert(node.node_id.clone(), node);
            Ok(())
        }
        async fn add_edge(&self, edge: GraphEdge) -> Result<(), GraphProjectionError> {
            self.edges.lock().unwrap().push(edge);
            Ok(())
        }
        async fn node_exists(&self, node_id: &str) -> Result<bool, GraphProjectionError> {
            Ok(self.nodes.lock().unwrap().contains_key(node_id))
        }
    }

    #[tokio::test]
    async fn projects_prompt_asset_version_release_chain() {
        let graph = Arc::new(MemGraph::new());
        let projector = EvalGraphProjector::new(graph.clone());

        let asset_id = PromptAssetId::new("asset_1");
        let version_id = PromptVersionId::new("ver_1");
        let release_id = PromptReleaseId::new("rel_1");

        projector.on_asset_created(&asset_id, 100).await.unwrap();
        projector
            .on_version_created(&version_id, &asset_id, 200)
            .await
            .unwrap();
        projector
            .on_release_created(&release_id, &version_id, 300)
            .await
            .unwrap();

        let nodes = graph.nodes.lock().unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes["asset_1"].kind, NodeKind::PromptAsset);
        assert_eq!(nodes["ver_1"].kind, NodeKind::PromptVersion);
        assert_eq!(nodes["rel_1"].kind, NodeKind::PromptRelease);

        let edges = graph.edges.lock().unwrap();
        assert_eq!(edges.len(), 2);
        assert!(edges.iter().any(|e| e.kind == EdgeKind::DerivedFrom
            && e.source_node_id == "asset_1"
            && e.target_node_id == "ver_1"));
        assert!(edges.iter().any(|e| e.kind == EdgeKind::ReleasedAs
            && e.source_node_id == "ver_1"
            && e.target_node_id == "rel_1"));
    }

    #[tokio::test]
    async fn projects_eval_run_linked_to_release() {
        let graph = Arc::new(MemGraph::new());
        let projector = EvalGraphProjector::new(graph.clone());

        let release_id = PromptReleaseId::new("rel_1");
        let eval_id = EvalRunId::new("eval_1");

        // Pre-create the release node.
        projector
            .on_release_created(&release_id, &PromptVersionId::new("ver_1"), 100)
            .await
            .unwrap();

        projector
            .on_eval_run_created(&eval_id, Some(&release_id), 200)
            .await
            .unwrap();

        let nodes = graph.nodes.lock().unwrap();
        assert_eq!(nodes["eval_1"].kind, NodeKind::EvalRun);

        let edges = graph.edges.lock().unwrap();
        assert!(edges.iter().any(|e| e.kind == EdgeKind::EvaluatedBy
            && e.source_node_id == "rel_1"
            && e.target_node_id == "eval_1"));
    }

    #[tokio::test]
    async fn projects_rollback_edge() {
        let graph = Arc::new(MemGraph::new());
        let projector = EvalGraphProjector::new(graph.clone());

        let old_release = PromptReleaseId::new("rel_old");
        let new_release = PromptReleaseId::new("rel_new");

        projector
            .on_release_rollback(&new_release, &old_release, 300)
            .await
            .unwrap();

        let edges = graph.edges.lock().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, EdgeKind::RolledBackTo);
        assert_eq!(edges[0].source_node_id, "rel_new");
        assert_eq!(edges[0].target_node_id, "rel_old");
    }

    #[tokio::test]
    async fn projects_prompt_used_by_run() {
        let graph = Arc::new(MemGraph::new());
        let projector = EvalGraphProjector::new(graph.clone());

        let release_id = PromptReleaseId::new("rel_1");

        projector
            .on_prompt_used("run_42", &release_id, 400)
            .await
            .unwrap();

        let edges = graph.edges.lock().unwrap();
        assert_eq!(edges[0].kind, EdgeKind::UsedPrompt);
        assert_eq!(edges[0].source_node_id, "run_42");
        assert_eq!(edges[0].target_node_id, "rel_1");
    }
}
