//! Proves GraphIntegration + scorecard/release data stay stable enough
//! for Worker 8 to surface without re-deriving prompt/eval semantics.
//!
//! This is the downstream contract proof: Worker 8's API layer should be
//! able to read release state, scorecard entries, and graph linkage data
//! directly from the types exposed here.

use cairn_domain::*;
use cairn_evals::{
    EvalMetrics, EvalRunService, EvalSubjectKind, GraphIntegration, PromptReleaseService,
    PromptReleaseState, ResolutionContext, RolloutTarget, SelectorResolver,
};
use cairn_graph::projections::{GraphEdge, GraphNode, GraphProjection, GraphProjectionError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Local graph store for testing (newtype to satisfy orphan rule).
struct TestGraph(Arc<TestGraphInner>);
struct TestGraphInner {
    nodes: Mutex<HashMap<String, GraphNode>>,
    edges: Mutex<Vec<GraphEdge>>,
}

impl TestGraph {
    fn new() -> Self {
        Self(Arc::new(TestGraphInner {
            nodes: Mutex::new(HashMap::new()),
            edges: Mutex::new(Vec::new()),
        }))
    }
    fn clone_inner(&self) -> Self {
        Self(self.0.clone())
    }
    fn node_count(&self) -> usize {
        self.0.nodes.lock().unwrap().len()
    }
    fn edge_count(&self) -> usize {
        self.0.edges.lock().unwrap().len()
    }
    fn has_node(&self, id: &str) -> bool {
        self.0.nodes.lock().unwrap().contains_key(id)
    }
    fn edges_from(&self, source: &str) -> Vec<GraphEdge> {
        self.0
            .edges
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.source_node_id == source)
            .cloned()
            .collect()
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

/// Full graph-linked prompt/eval lifecycle as Worker 8 would consume it.
///
/// This test exercises the exact path an API endpoint would take:
/// 1. Release service creates releases
/// 2. Graph integration records nodes/edges
/// 3. Eval service creates and completes runs
/// 4. Graph integration records eval linkage
/// 5. Scorecard aggregates results
/// 6. All data is directly readable without re-derivation
#[tokio::test]
async fn graph_linked_prompt_eval_lifecycle_for_api() {
    let graph = TestGraph::new();
    let graph_integration = GraphIntegration::new(graph.clone_inner());
    let release_svc = PromptReleaseService::new();
    let eval_svc = EvalRunService::new();

    let project_id = ProjectId::new("proj_api");
    let asset_id = PromptAssetId::new("planner_system");

    // -- 1. Create prompt asset + version + two releases --

    graph_integration.on_asset_created(&asset_id).await;

    let v1_id = PromptVersionId::new("pv_v1");
    let v2_id = PromptVersionId::new("pv_v2");
    graph_integration
        .on_version_created(&v1_id, &asset_id)
        .await;
    graph_integration
        .on_version_created(&v2_id, &asset_id)
        .await;

    // Release v1 and activate
    release_svc.create(
        PromptReleaseId::new("rel_v1"),
        project_id.clone(),
        asset_id.clone(),
        v1_id.clone(),
        RolloutTarget::project_default(),
    );
    release_svc
        .transition(
            &PromptReleaseId::new("rel_v1"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();
    release_svc
        .transition(
            &PromptReleaseId::new("rel_v1"),
            PromptReleaseState::Active,
            None,
            None,
        )
        .unwrap();
    graph_integration
        .on_release_created(&PromptReleaseId::new("rel_v1"), &v1_id)
        .await;

    // Release v2 (approved, not yet active)
    release_svc.create(
        PromptReleaseId::new("rel_v2"),
        project_id.clone(),
        asset_id.clone(),
        v2_id.clone(),
        RolloutTarget::project_default(),
    );
    release_svc
        .transition(
            &PromptReleaseId::new("rel_v2"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();
    graph_integration
        .on_release_created(&PromptReleaseId::new("rel_v2"), &v2_id)
        .await;

    // -- 2. Verify graph has expected topology --

    assert_eq!(graph.node_count(), 5); // asset + 2 versions + 2 releases
    assert!(graph.has_node("planner_system"));
    assert!(graph.has_node("pv_v1"));
    assert!(graph.has_node("rel_v1"));

    // -- 3. Run evals and link to graph --

    eval_svc.create_run(
        EvalRunId::new("eval_v1"),
        project_id.clone(),
        EvalSubjectKind::PromptRelease,
        "auto".to_owned(),
        Some(asset_id.clone()),
        Some(v1_id.clone()),
        Some(PromptReleaseId::new("rel_v1")),
        None,
    );
    eval_svc.start_run(&EvalRunId::new("eval_v1")).unwrap();
    eval_svc
        .complete_run(
            &EvalRunId::new("eval_v1"),
            EvalMetrics {
                task_success_rate: Some(0.80),
                latency_p50_ms: Some(200),
                ..Default::default()
            },
            Some(0.10),
        )
        .unwrap();
    graph_integration
        .on_eval_run_created(
            &EvalRunId::new("eval_v1"),
            Some(&PromptReleaseId::new("rel_v1")),
        )
        .await;

    eval_svc.create_run(
        EvalRunId::new("eval_v2"),
        project_id.clone(),
        EvalSubjectKind::PromptRelease,
        "auto".to_owned(),
        Some(asset_id.clone()),
        Some(v2_id.clone()),
        Some(PromptReleaseId::new("rel_v2")),
        None,
    );
    eval_svc.start_run(&EvalRunId::new("eval_v2")).unwrap();
    eval_svc
        .complete_run(
            &EvalRunId::new("eval_v2"),
            EvalMetrics {
                task_success_rate: Some(0.95),
                latency_p50_ms: Some(150),
                ..Default::default()
            },
            Some(0.08),
        )
        .unwrap();
    graph_integration
        .on_eval_run_created(
            &EvalRunId::new("eval_v2"),
            Some(&PromptReleaseId::new("rel_v2")),
        )
        .await;

    // -- 4. Verify graph has eval linkage --

    assert_eq!(graph.node_count(), 7); // + 2 eval runs
    assert!(graph.has_node("eval_v1"));
    assert!(graph.has_node("eval_v2"));

    let rel_v1_edges = graph.edges_from("rel_v1");
    assert!(rel_v1_edges.iter().any(|e| e.target_node_id == "eval_v1"));

    // -- 5. Scorecard is directly readable by API layer --

    let scorecard = eval_svc.build_scorecard(&project_id, &asset_id);
    assert_eq!(scorecard.entries.len(), 2);

    // Worker 8 reads the best entry directly
    let best = scorecard
        .entries
        .iter()
        .max_by(|a, b| {
            a.metrics
                .task_success_rate
                .partial_cmp(&b.metrics.task_success_rate)
                .unwrap()
        })
        .unwrap();
    assert_eq!(best.prompt_release_id, PromptReleaseId::new("rel_v2"));
    assert_eq!(best.metrics.task_success_rate, Some(0.95));

    // Worker 8 reads release state directly
    let v2_release = release_svc.get(&PromptReleaseId::new("rel_v2")).unwrap();
    assert_eq!(v2_release.state, PromptReleaseState::Approved);

    // -- 6. Promote v2, verify graph records rollback capability --

    release_svc
        .transition(
            &PromptReleaseId::new("rel_v2"),
            PromptReleaseState::Active,
            None,
            None,
        )
        .unwrap();

    // Selector resolution gives v2
    let v1_rel = release_svc.get(&PromptReleaseId::new("rel_v1")).unwrap();
    let v2_rel = release_svc.get(&PromptReleaseId::new("rel_v2")).unwrap();
    let releases = vec![v1_rel, v2_rel];
    let resolved = SelectorResolver::resolve(
        &releases,
        &project_id,
        &asset_id,
        &ResolutionContext::default(),
    )
    .unwrap();
    assert_eq!(resolved.prompt_release_id, PromptReleaseId::new("rel_v2"));

    // Simulate runtime using the resolved prompt
    graph_integration
        .on_prompt_used("run_42", &PromptReleaseId::new("rel_v2"))
        .await;

    // Total graph state: all nodes, all edges including UsedPrompt
    assert_eq!(graph.node_count(), 7);
    assert!(graph.edge_count() >= 7); // derived_from x2, released_as x2, evaluated_by x2, used_prompt x1
}
