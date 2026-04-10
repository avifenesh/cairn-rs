//! update_memory — replace an existing document in the knowledge store.
//!
//! Inject via `UpdateMemoryTool::new(remove_fn, reingest_fn)` — the caller
//! provides closures backed by their InMemoryDocumentStore + IngestPipeline so
//! cairn-tools does not need a direct dep on cairn-memory (which would create
//! a cairn-tools → cairn-memory → cairn-api → cairn-tools cycle).
//!
//! ## Parameters
//! ```json
//! { "document_id": "doc_123", "content": "New text...", "source_id": "src_docs" }
//! ```
//!
//! ## Output
//! ```json
//! { "document_id": "doc_123", "updated": true }
//! ```

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, recovery::RetrySafety, ProjectKey};
use serde_json::Value;
use std::sync::Arc;

/// Async function type for reingest: (project, doc_id, source_id, content) → Result<(), String>
pub type ReingestFn = Arc<
    dyn Fn(
            ProjectKey,
            String,
            String,
            String,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

pub struct UpdateMemoryTool {
    reingest: ReingestFn,
}

impl UpdateMemoryTool {
    /// Construct with a reingest closure.
    ///
    /// The closure should: (1) remove the old document, (2) re-ingest with new content.
    pub fn new(reingest: ReingestFn) -> Self {
        Self { reingest }
    }
}

#[async_trait]
impl ToolHandler for UpdateMemoryTool {
    fn name(&self) -> &str {
        "update_memory"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Internal
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::AuthorResponsible
    }
    fn description(&self) -> &str {
        "Replace the content of an existing document in the knowledge store."
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["document_id", "content"],
            "properties": {
                "document_id": { "type": "string" },
                "content":     { "type": "string", "description": "Replacement content." },
                "source_id":   { "type": "string", "description": "Source identifier (default: 'default')." }
            }
        })
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let doc_id = args
            .get("document_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "document_id".into(),
                message: "required".into(),
            })?
            .to_owned();
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "content".into(),
                message: "required".into(),
            })?
            .to_owned();
        if content.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "content".into(),
                message: "must not be empty".into(),
            });
        }
        let source_id = args
            .get("source_id")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_owned();

        (self.reingest)(project.clone(), doc_id.clone(), source_id, content)
            .await
            .map_err(ToolError::Transient)?;

        Ok(ToolResult::ok(
            serde_json::json!({ "document_id": doc_id, "updated": true }),
        ))
    }
}

/// Async function type for delete: (doc_id) → ()
pub type DeleteFn = Arc<dyn Fn(String) + Send + Sync>;

pub struct DeleteMemoryTool {
    delete: DeleteFn,
}

impl DeleteMemoryTool {
    pub fn new(delete: DeleteFn) -> Self {
        Self { delete }
    }
}

#[async_trait]
impl ToolHandler for DeleteMemoryTool {
    fn name(&self) -> &str {
        "delete_memory"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Internal
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::AuthorResponsible
    }
    fn description(&self) -> &str {
        "Remove a document from the knowledge store. SENSITIVE — irreversible."
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::Sensitive
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["document_id"],
            "properties": {
                "document_id": { "type": "string", "description": "ID of the document to delete." }
            }
        })
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let doc_id = args
            .get("document_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "document_id".into(),
                message: "required".into(),
            })?
            .to_owned();
        if doc_id.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "document_id".into(),
                message: "must not be empty".into(),
            });
        }
        (self.delete)(doc_id.clone());
        Ok(ToolResult::ok(
            serde_json::json!({ "document_id": doc_id, "deleted": true }),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    fn make_reingest() -> ReingestFn {
        Arc::new(|_, _, _, _| Box::pin(async { Ok(()) }))
    }
    fn make_delete() -> DeleteFn {
        Arc::new(|_| {})
    }

    #[test]
    fn names() {
        assert_eq!(
            UpdateMemoryTool::new(make_reingest()).name(),
            "update_memory"
        );
        assert_eq!(DeleteMemoryTool::new(make_delete()).name(), "delete_memory");
    }
    #[test]
    fn delete_is_sensitive() {
        assert_eq!(
            DeleteMemoryTool::new(make_delete()).execution_class(),
            ExecutionClass::Sensitive
        );
    }

    #[tokio::test]
    async fn missing_doc_id_is_invalid() {
        let err = UpdateMemoryTool::new(make_reingest())
            .execute(&project(), serde_json::json!({"content":"x"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn update_calls_reingest() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let called = Arc::new(AtomicBool::new(false));
        let c2 = called.clone();
        let reingest: ReingestFn = Arc::new(move |_, _, _, _| {
            c2.store(true, Ordering::SeqCst);
            Box::pin(async { Ok(()) })
        });
        UpdateMemoryTool::new(reingest)
            .execute(
                &project(),
                serde_json::json!({"document_id":"d1","content":"new text"}),
            )
            .await
            .unwrap();
        assert!(called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn delete_calls_delete_fn() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let called = Arc::new(AtomicBool::new(false));
        let c2 = called.clone();
        let delete: DeleteFn = Arc::new(move |_| {
            c2.store(true, Ordering::SeqCst);
        });
        DeleteMemoryTool::new(delete)
            .execute(&project(), serde_json::json!({"document_id":"doc1"}))
            .await
            .unwrap();
        assert!(called.load(Ordering::SeqCst));
    }
}
