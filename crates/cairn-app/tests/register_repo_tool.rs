#![cfg(feature = "in-memory-runtime")]

use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_app::tool_impls::{build_tool_registry, ConcreteRegisterRepoTool};
use cairn_domain::{policy::ExecutionClass, ProjectKey, RepoAccessContext};
use cairn_memory::{
    in_memory::{InMemoryDocumentStore, InMemoryRetrieval},
    ingest::IngestService,
    pipeline::{IngestPipeline, ParagraphChunker},
    retrieval::RetrievalService,
};
use cairn_tools::builtins::{RetrySafety, ToolEffect, ToolHandler, ToolTier};
use cairn_workspace::{
    ProjectRepoAccessService, RepoCloneCache, RepoId, RepoStore, RepoStoreError,
};

struct TestDir {
    path: PathBuf,
}

impl TestDir {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "cairn-app-register-repo-{label}-{}-{unique}",
            process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn project(project_id: &str) -> ProjectKey {
    ProjectKey::new("tenant-a", "workspace-a", project_id)
}

fn repo_ctx(project: &ProjectKey) -> RepoAccessContext {
    RepoAccessContext {
        project: project.clone(),
    }
}

fn make_ingest() -> (
    Arc<InMemoryDocumentStore>,
    Arc<IngestPipeline<Arc<InMemoryDocumentStore>, ParagraphChunker>>,
) {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = Arc::new(IngestPipeline::new(
        store.clone(),
        ParagraphChunker::default(),
    ));
    (store, pipeline)
}

#[tokio::test]
async fn register_repo_is_project_scoped_and_hides_host_path() {
    let temp_dir = TestDir::new("scope");
    let access = Arc::new(ProjectRepoAccessService::new());
    let cache = Arc::new(RepoCloneCache::new(&temp_dir.path));
    let tool = ConcreteRegisterRepoTool::new(access.clone(), cache.clone());
    let project_a = project("project-a");
    let project_b = project("project-b");
    let repo_id = RepoId::new("octocat/hello-world");

    let result = tool
        .execute(
            &project_a,
            serde_json::json!({ "repo_id": repo_id.as_str() }),
        )
        .await
        .unwrap();

    assert_eq!(result.output["repo_id"], repo_id.as_str());
    assert_eq!(result.output["authorization_status"], "granted");
    assert_eq!(result.output["clone_status"], "present");
    assert_eq!(result.output["clone_created"], true);
    assert!(result.output.get("path").is_none());
    assert!(result.output.get("host_path").is_none());

    assert!(access.is_allowed(&repo_ctx(&project_a), &repo_id).await);
    assert!(!access.is_allowed(&repo_ctx(&project_b), &repo_id).await);
    assert!(cache.is_cloned(&project_a.tenant_id, &repo_id).await);

    let store = RepoStore::new(cache, access);
    let err = store
        .resolve(&repo_ctx(&project_b), &repo_id)
        .await
        .unwrap_err();
    assert_eq!(
        err,
        RepoStoreError::NotAllowedForProject {
            project: project_b,
            repo_id,
        }
    );
}

#[tokio::test]
async fn register_repo_rejects_invalid_repo_shape() {
    let tool = ConcreteRegisterRepoTool::new(
        Arc::new(ProjectRepoAccessService::new()),
        Arc::new(RepoCloneCache::default()),
    );

    let err = tool
        .execute(
            &project("project-a"),
            serde_json::json!({ "repo_id": "not-a-valid-repo" }),
        )
        .await
        .unwrap_err();

    assert!(matches!(
        err,
        cairn_tools::builtins::ToolError::InvalidArgs { .. }
    ));
}

#[tokio::test]
async fn registry_exposes_register_repo_with_sensitive_metadata() {
    let temp_dir = TestDir::new("registry");
    let (_, pipeline) = make_ingest();
    let registry = build_tool_registry(
        Arc::new(InMemoryRetrieval::new(Arc::new(
            InMemoryDocumentStore::new(),
        ))) as Arc<dyn RetrievalService>,
        pipeline as Arc<dyn IngestService>,
        Arc::new(ProjectRepoAccessService::new()),
        Arc::new(RepoCloneCache::new(&temp_dir.path)),
    );

    let descriptor = registry
        .prompt_tools()
        .into_iter()
        .find(|tool| tool.name == "cairn.registerRepo")
        .expect("registry should expose cairn.registerRepo");

    assert_eq!(descriptor.tier, ToolTier::Registered);
    assert_eq!(descriptor.tool_effect, ToolEffect::External);
    assert_eq!(descriptor.retry_safety, RetrySafety::DangerousPause);
    assert_eq!(descriptor.execution_class, ExecutionClass::Sensitive);
}
