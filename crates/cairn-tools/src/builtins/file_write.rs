//! `file_write` built-in tool — write file contents within the project workspace.
//!
//! Writes text to a file at a path relative to the injected `workspace_root`.
//! The tool is `Deferred` (only shown to the LLM after `tool_search` discovers it)
//! and `Sensitive` (requires operator approval before execution).
//!
//! # Modes
//!
//! | `mode`      | Behaviour                                              |
//! |-------------|--------------------------------------------------------|
//! | `create`    | Create new file; fails if file already exists         |
//! | `overwrite` | Create or truncate; replaces existing content         |
//! | `append`    | Create or append to existing file                     |
//!
//! # Schema
//!
//! ```json
//! {
//!   "path":    "output/report.md",
//!   "content": "# Report\n…",
//!   "mode":    "create"
//! }
//! ```
//!
//! # Output
//!
//! ```json
//! { "path": "output/report.md", "bytes_written": 2048, "mode": "create" }
//! ```

use std::io::Write as IoWrite;
use std::path::PathBuf;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use serde_json::Value;

use super::{
    file_read::resolve_safe_path, PermissionLevel, ToolCategory, ToolError, ToolHandler,
    ToolResult, ToolTier,
};

/// Write a file within the project workspace.
///
/// `ExecutionClass::Sensitive` means the orchestrator must gate every call
/// through `ApprovalService` before it executes.
pub struct FileWriteTool {
    workspace_root: PathBuf,
}

impl FileWriteTool {
    /// Construct with an absolute path to the workspace root.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }
}

#[async_trait]
impl ToolHandler for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }

    fn description(&self) -> &str {
        "Write content to a file in the project workspace. \
         Requires operator approval before executing. \
         Supports create (fails if exists), overwrite (replaces), and append modes."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["path", "content", "mode"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path relative to workspace root (no ../ allowed)"
                },
                "content": {
                    "type": "string",
                    "description": "Text content to write"
                },
                "mode": {
                    "type": "string",
                    "description": "Write mode",
                    "enum": ["create", "overwrite", "append"]
                }
            }
        })
    }

    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::FileSystem
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let rel_path =
            args.get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "path".into(),
                    message: "required string".into(),
                })?;

        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "content".into(),
                message: "required string".into(),
            })?;

        let mode =
            args.get("mode")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "mode".into(),
                    message: "required: 'create' | 'overwrite' | 'append'".into(),
                })?;

        let abs_path = resolve_safe_path(&self.workspace_root, rel_path)?;

        // Create parent directories as needed.
        if let Some(parent) = abs_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                ToolError::Transient(format!("failed to create parent directories: {e}"))
            })?;
        }

        let bytes_written = match mode {
            "create" => {
                // Fail if file already exists.
                if abs_path.exists() {
                    return Err(ToolError::Permanent(format!(
                        "file already exists: {rel_path} (use mode=overwrite to replace)"
                    )));
                }
                write_all(&abs_path, content, false)?
            }
            "overwrite" => write_all(&abs_path, content, false)?,
            "append" => write_all(&abs_path, content, true)?,
            other => {
                return Err(ToolError::InvalidArgs {
                    field: "mode".into(),
                    message: format!("unknown mode '{other}' — use create, overwrite, or append"),
                });
            }
        };

        Ok(ToolResult::ok(serde_json::json!({
            "path":          rel_path,
            "bytes_written": bytes_written,
            "mode":          mode,
        })))
    }
}

/// Write `content` to `path`, appending when `append` is true.
fn write_all(path: &std::path::Path, content: &str, append: bool) -> Result<usize, ToolError> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => {
                ToolError::Permanent(format!("permission denied: {}", path.display()))
            }
            _ => ToolError::Transient(format!("open error: {e}")),
        })?;

    let mut writer = std::io::BufWriter::new(file);
    writer
        .write_all(content.as_bytes())
        .map_err(|e| ToolError::Transient(format!("write error: {e}")))?;
    Ok(content.len())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ProjectKey;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    fn temp_workspace() -> tempfile::TempDir {
        tempfile::tempdir().expect("failed to create temp dir")
    }

    // ── create mode ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn creates_new_file() {
        let ws = temp_workspace();
        let tool = FileWriteTool::new(ws.path());
        let result = tool
            .execute(
                &project(),
                serde_json::json!({
                    "path":    "hello.txt",
                    "content": "Hello, cairn-rs!",
                    "mode":    "create"
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["bytes_written"], 16);
        assert_eq!(result.output["mode"], "create");
        let on_disk = std::fs::read_to_string(ws.path().join("hello.txt")).unwrap();
        assert_eq!(on_disk, "Hello, cairn-rs!");
    }

    #[tokio::test]
    async fn create_fails_if_file_exists() {
        let ws = temp_workspace();
        std::fs::write(ws.path().join("existing.txt"), "old").unwrap();
        let tool = FileWriteTool::new(ws.path());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "path": "existing.txt", "content": "new", "mode": "create"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Permanent(_)));
        assert!(err.to_string().contains("already exists"));
        // Original content must be intact.
        let on_disk = std::fs::read_to_string(ws.path().join("existing.txt")).unwrap();
        assert_eq!(on_disk, "old");
    }

    // ── overwrite mode ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn overwrites_existing_file() {
        let ws = temp_workspace();
        std::fs::write(ws.path().join("file.txt"), "old content").unwrap();
        let tool = FileWriteTool::new(ws.path());
        let result = tool
            .execute(
                &project(),
                serde_json::json!({
                    "path": "file.txt", "content": "new content", "mode": "overwrite"
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["mode"], "overwrite");
        let on_disk = std::fs::read_to_string(ws.path().join("file.txt")).unwrap();
        assert_eq!(on_disk, "new content");
    }

    #[tokio::test]
    async fn overwrite_creates_if_absent() {
        let ws = temp_workspace();
        let tool = FileWriteTool::new(ws.path());
        tool.execute(
            &project(),
            serde_json::json!({
                "path": "new.txt", "content": "fresh", "mode": "overwrite"
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(ws.path().join("new.txt")).unwrap(),
            "fresh"
        );
    }

    // ── append mode ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn appends_to_existing_file() {
        let ws = temp_workspace();
        std::fs::write(ws.path().join("log.txt"), "line1\n").unwrap();
        let tool = FileWriteTool::new(ws.path());
        tool.execute(
            &project(),
            serde_json::json!({
                "path": "log.txt", "content": "line2\n", "mode": "append"
            }),
        )
        .await
        .unwrap();
        let on_disk = std::fs::read_to_string(ws.path().join("log.txt")).unwrap();
        assert_eq!(on_disk, "line1\nline2\n");
    }

    // ── creates parent directories ─────────────────────────────────────────────

    #[tokio::test]
    async fn creates_parent_directories() {
        let ws = temp_workspace();
        let tool = FileWriteTool::new(ws.path());
        tool.execute(
            &project(),
            serde_json::json!({
                "path": "a/b/c/deep.txt", "content": "deep", "mode": "create"
            }),
        )
        .await
        .unwrap();
        assert!(ws.path().join("a/b/c/deep.txt").exists());
    }

    // ── path safety ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rejects_parent_traversal() {
        let ws = temp_workspace();
        let tool = FileWriteTool::new(ws.path());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "path": "../../etc/cron.d/evil", "content": "hack", "mode": "create"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
        assert!(err.to_string().contains("traversal"));
    }

    #[tokio::test]
    async fn rejects_absolute_path() {
        let ws = temp_workspace();
        let tool = FileWriteTool::new(ws.path());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "path": "/etc/shadow", "content": "hack", "mode": "overwrite"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
        assert!(err.to_string().contains("absolute"));
    }

    // ── missing required args ─────────────────────────────────────────────────

    #[tokio::test]
    async fn missing_path_is_invalid_args() {
        let ws = temp_workspace();
        let tool = FileWriteTool::new(ws.path());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "content": "x", "mode": "create"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn unknown_mode_is_invalid_args() {
        let ws = temp_workspace();
        let tool = FileWriteTool::new(ws.path());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "path": "x.txt", "content": "x", "mode": "upsert"
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    // ── metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn tier_is_registered() {
        assert_eq!(FileWriteTool::new("/tmp").tier(), ToolTier::Registered);
    }

    #[test]
    fn execution_class_is_supervised() {
        assert!(matches!(
            FileWriteTool::new("/tmp").execution_class(),
            ExecutionClass::SupervisedProcess
        ));
    }

    #[test]
    fn schema_requires_path_content_mode() {
        let tool = FileWriteTool::new("/tmp");
        let schema = tool.parameters_schema();
        let required: Vec<String> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        assert!(required.iter().any(|s| s == "path"));
        assert!(required.iter().any(|s| s == "content"));
        assert!(required.iter().any(|s| s == "mode"));
    }
}
