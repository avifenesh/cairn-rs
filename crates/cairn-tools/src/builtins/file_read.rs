//! `file_read` built-in tool — read file contents from the project workspace.
//!
//! Reads a file at a path relative to the injected `workspace_root`, with:
//! - Path traversal prevention (no `..` components)
//! - 64 KiB content cap (`truncated: true` when the file is larger)
//! - Optional byte-offset + line-limit for large-file pagination
//!
//! # Schema
//!
//! ```json
//! {
//!   "path":   "src/main.rs",
//!   "offset": 0,
//!   "limit":  200
//! }
//! ```
//!
//! # Output
//!
//! ```json
//! {
//!   "path":       "src/main.rs",
//!   "content":    "fn main() { … }",
//!   "size_bytes": 4096,
//!   "truncated":  false
//! }
//! ```

use std::path::{Component, PathBuf};

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey};
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

/// Maximum bytes returned in a single `file_read` call.
const MAX_BYTES: usize = 64 * 1024; // 64 KiB

/// Read a file from the project workspace.
pub struct FileReadTool {
    workspace_root: PathBuf,
}

impl FileReadTool {
    /// Construct with an absolute path to the workspace root.
    ///
    /// All `path` arguments are resolved relative to this root.
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
        }
    }
}

#[async_trait]
impl ToolHandler for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }

    fn description(&self) -> &str {
        "Read the contents of a file in the project workspace. \
         Returns the file content as a string, capped at 64 KiB. \
         Use offset and limit to page through larger files."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["path"],
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path relative to the workspace root (no ../ allowed)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Byte offset to start reading from (default 0)",
                    "default": 0,
                    "minimum": 0
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum bytes to read (default and cap: 65536)",
                    "default": 65536,
                    "minimum": 1,
                    "maximum": 65536
                }
            }
        })
    }

    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SandboxedProcess
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let rel_path =
            args.get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "path".into(),
                    message: "required string".into(),
                })?;

        let abs_path = resolve_safe_path(&self.workspace_root, rel_path)?;

        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(MAX_BYTES))
            .unwrap_or(MAX_BYTES);

        // Read the file using std::fs (blocking — acceptable for CLI/agent use).
        // tokio::fs would be ideal but adds complexity; files should be small.
        let bytes = std::fs::read(&abs_path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                ToolError::Permanent(format!("file not found: {rel_path}"))
            }
            std::io::ErrorKind::PermissionDenied => {
                ToolError::Permanent(format!("permission denied: {rel_path}"))
            }
            _ => ToolError::Transient(format!("read error: {e}")),
        })?;

        let size_bytes = bytes.len();

        // Apply offset + limit.
        let slice_start = offset.min(size_bytes);
        let slice_end = (slice_start + limit).min(size_bytes);
        let truncated = slice_end < size_bytes || offset > size_bytes;
        let slice = &bytes[slice_start..slice_end];

        // Convert to UTF-8, replacing invalid sequences so binary files
        // don't produce hard errors.
        let content = String::from_utf8_lossy(slice).into_owned();

        Ok(ToolResult::ok(serde_json::json!({
            "path":       rel_path,
            "content":    content,
            "size_bytes": size_bytes,
            "truncated":  truncated,
        })))
    }
}

// ── Path safety ───────────────────────────────────────────────────────────────

/// Resolve `rel_path` under `root`, rejecting any path that escapes the root.
///
/// Rejects:
/// - absolute paths (`/etc/passwd`)
/// - paths with `..` components (`../../../etc/passwd`)
/// - paths with null bytes (defence-in-depth)
pub(super) fn resolve_safe_path(root: &PathBuf, rel_path: &str) -> Result<PathBuf, ToolError> {
    if rel_path.contains('\0') {
        return Err(ToolError::InvalidArgs {
            field: "path".into(),
            message: "path contains null byte".into(),
        });
    }

    let input = PathBuf::from(rel_path);

    // Reject absolute paths.
    if input.is_absolute() {
        return Err(ToolError::InvalidArgs {
            field: "path".into(),
            message: "absolute paths are not allowed — use a path relative to the workspace root"
                .into(),
        });
    }

    // Reject any `..` component.
    for component in input.components() {
        if matches!(component, Component::ParentDir) {
            return Err(ToolError::InvalidArgs {
                field: "path".into(),
                message: "path traversal ('..') is not allowed".into(),
            });
        }
    }

    let abs = root.join(&input);

    // Canonicalize to resolve symlinks and verify the result still starts with root.
    // We skip canonicalize for non-existent paths (read will fail later anyway).
    if abs.exists() {
        let canonical = abs
            .canonicalize()
            .map_err(|e| ToolError::Transient(e.to_string()))?;
        let root_canon = root.canonicalize().unwrap_or_else(|_| root.clone());
        if !canonical.starts_with(&root_canon) {
            return Err(ToolError::InvalidArgs {
                field: "path".into(),
                message: "resolved path escapes workspace root".into(),
            });
        }
    }

    Ok(abs)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ProjectKey;
    use std::io::Write;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    fn temp_workspace() -> tempfile::TempDir {
        tempfile::tempdir().expect("failed to create temp dir")
    }

    fn write_file(dir: &tempfile::TempDir, name: &str, content: &str) {
        std::fs::write(dir.path().join(name), content).unwrap();
    }

    // ── happy path ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn reads_existing_file() {
        let ws = temp_workspace();
        write_file(&ws, "hello.txt", "Hello, cairn-rs!");
        let tool = FileReadTool::new(ws.path());
        let result = tool
            .execute(&project(), serde_json::json!({ "path": "hello.txt" }))
            .await
            .unwrap();
        assert_eq!(result.output["content"], "Hello, cairn-rs!");
        assert_eq!(result.output["size_bytes"], 16);
        assert_eq!(result.output["truncated"], false);
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn truncates_large_file() {
        let ws = temp_workspace();
        // Write slightly more than 64 KiB.
        let big: String = "x".repeat(MAX_BYTES + 100);
        write_file(&ws, "big.txt", &big);
        let tool = FileReadTool::new(ws.path());
        let result = tool
            .execute(&project(), serde_json::json!({ "path": "big.txt" }))
            .await
            .unwrap();
        let content = result.output["content"].as_str().unwrap();
        assert_eq!(content.len(), MAX_BYTES);
        assert_eq!(result.output["truncated"], true);
    }

    #[tokio::test]
    async fn respects_offset_and_limit() {
        let ws = temp_workspace();
        write_file(&ws, "abcdef.txt", "ABCDEFGHIJ");
        let tool = FileReadTool::new(ws.path());
        let result = tool
            .execute(
                &project(),
                serde_json::json!({
                    "path": "abcdef.txt", "offset": 2, "limit": 4
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["content"], "CDEF");
    }

    // ── path safety ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rejects_parent_traversal() {
        let ws = temp_workspace();
        let tool = FileReadTool::new(ws.path());
        let err = tool
            .execute(&project(), serde_json::json!({ "path": "../etc/passwd" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
        assert!(err.to_string().contains("traversal"));
    }

    #[tokio::test]
    async fn rejects_absolute_path() {
        let ws = temp_workspace();
        let tool = FileReadTool::new(ws.path());
        let err = tool
            .execute(&project(), serde_json::json!({ "path": "/etc/passwd" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
        assert!(err.to_string().contains("absolute"));
    }

    #[tokio::test]
    async fn file_not_found_is_permanent_error() {
        let ws = temp_workspace();
        let tool = FileReadTool::new(ws.path());
        let err = tool
            .execute(&project(), serde_json::json!({ "path": "nonexistent.txt" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Permanent(_)));
    }

    #[tokio::test]
    async fn missing_path_arg_is_invalid_args() {
        let ws = temp_workspace();
        let tool = FileReadTool::new(ws.path());
        let err = tool
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    // ── schema ─────────────────────────────────────────────────────────────────

    #[test]
    fn schema_requires_path() {
        let tool = FileReadTool::new("/tmp");
        let required = tool.parameters_schema()["required"]
            .as_array()
            .unwrap()
            .clone();
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }

    #[test]
    fn tier_is_registered() {
        assert_eq!(FileReadTool::new("/tmp").tier(), ToolTier::Registered);
    }

    #[test]
    fn execution_class_is_sandboxed() {
        assert!(matches!(
            FileReadTool::new("/tmp").execution_class(),
            ExecutionClass::SandboxedProcess
        ));
    }
}
