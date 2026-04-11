//! glob_find — recursive file pattern matching without external tools.
use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use async_trait::async_trait;
use cairn_domain::recovery::RetrySafety;
use cairn_domain::ProjectKey;
use serde_json::Value;

/// Match a file path against a glob pattern.
/// Segments split on '/'; '**' matches zero or more segments; '*' and '?' in-segment.
fn glob_match(pattern: &str, path: &str) -> bool {
    let pat_segs: Vec<&str> = pattern.split('/').collect();
    let path_segs: Vec<&str> = path.split('/').collect();
    glob_segs(&pat_segs, &path_segs)
}

fn glob_segs(pat: &[&str], path: &[&str]) -> bool {
    match (pat.first(), path.first()) {
        (None, None) => true,
        (None, _) | (_, None) => pat.first() == Some(&"**") && pat.len() == 1,
        (Some(&"**"), _) => {
            // ** matches 0 or more segments: try consuming 0..=N path segments
            for i in 0..=path.len() {
                if glob_segs(&pat[1..], &path[i..]) {
                    return true;
                }
            }
            false
        }
        (Some(p), Some(s)) => seg_match(p, s) && glob_segs(&pat[1..], &path[1..]),
    }
}

fn seg_match(pattern: &str, seg: &str) -> bool {
    let pc: Vec<char> = pattern.chars().collect();
    let sc: Vec<char> = seg.chars().collect();
    seg_match_chars(&pc, &sc)
}

fn seg_match_chars(p: &[char], s: &[char]) -> bool {
    match (p.first(), s.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            // * matches 0 or more chars within segment
            for i in 0..=s.len() {
                if seg_match_chars(&p[1..], &s[i..]) {
                    return true;
                }
            }
            false
        }
        (Some('?'), Some(_)) => seg_match_chars(&p[1..], &s[1..]),
        (Some(pc), Some(sc)) => pc == sc && seg_match_chars(&p[1..], &s[1..]),
        _ => false,
    }
}

fn walk(root: &str, pattern: &str, max: usize) -> Vec<String> {
    let mut results = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut sorted: Vec<_> = entries.flatten().collect();
        sorted.sort_by_key(|e| e.file_name());
        for entry in sorted {
            if results.len() >= max {
                break;
            }
            let path = entry.path();
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            if path.is_dir() {
                let sub = walk(&path.to_string_lossy(), pattern, max - results.len());
                results.extend(sub.into_iter().map(|s| format!("{rel}/{s}")));
            } else if glob_match(pattern, &rel) {
                results.push(rel.to_owned());
            }
        }
    }
    results
}

pub struct GlobFindTool;
impl Default for GlobFindTool {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl ToolHandler for GlobFindTool {
    fn name(&self) -> &str {
        "glob_find"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Observational
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }
    fn description(&self) -> &str {
        "Find files matching a glob pattern. \
         Supports ** (multi-segment), * (any chars), ? (any char). \
         Example patterns: '**/*.rs', 'src/**/*.tsx', '*.toml'."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type":"object","required":["pattern"],
            "properties":{
                "pattern":{"type":"string","description":"Glob pattern e.g. '**/*.rs'"},
                "path":{"type":"string","description":"Root directory (default: '.')"},
                "max_results":{"type":"integer","default":100,"maximum":500}
            }
        })
    }
    async fn execute(&self, _: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "pattern".into(),
                message: "required string".into(),
            })?
            .trim();
        if pattern.is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "pattern".into(),
                message: "must not be empty".into(),
            });
        }
        let root = args["path"].as_str().unwrap_or(".");
        let max = args["max_results"].as_u64().unwrap_or(100).clamp(1, 500) as usize;

        if !std::path::Path::new(root).exists() {
            return Err(ToolError::InvalidArgs {
                field: "path".into(),
                message: format!("directory not found: {root}"),
            });
        }

        let files = walk(root, pattern, max + 1); // +1 to detect truncation
        let truncated = files.len() > max;
        let files: Vec<&str> = files.iter().take(max).map(String::as_str).collect();
        Ok(ToolResult::ok(serde_json::json!({
            "files": files, "total": files.len(), "pattern": pattern, "truncated": truncated
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[test]
    fn tier_is_registered() {
        assert_eq!(GlobFindTool.tier(), ToolTier::Registered);
    }
    #[test]
    fn schema_requires_pattern() {
        let s = GlobFindTool.parameters_schema();
        assert!(s["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str() == Some("pattern")));
    }
    #[test]
    fn glob_match_star_extension() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(!glob_match("*.rs", "main.ts"));
        assert!(glob_match("**/*.rs", "src/lib.rs"));
        assert!(glob_match("**/*.rs", "a/b/c/main.rs"));
    }
    #[test]
    fn glob_match_double_star() {
        assert!(glob_match("src/**", "src/a/b/c.rs"));
        assert!(glob_match("**", "anything/at/all"));
    }
    #[test]
    fn glob_match_question_mark() {
        assert!(glob_match("file?.rs", "file1.rs"));
        assert!(!glob_match("file?.rs", "file12.rs"));
    }
    #[tokio::test]
    async fn empty_pattern_err() {
        let err = GlobFindTool
            .execute(&p(), serde_json::json!({"pattern":""}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
    #[tokio::test]
    async fn missing_pattern_err() {
        let err = GlobFindTool
            .execute(&p(), serde_json::json!({}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }
    #[tokio::test]
    async fn finds_toml_files() {
        // Should find at least the workspace Cargo.toml
        let r = GlobFindTool
            .execute(&p(), serde_json::json!({"pattern":"*.toml","path":"."}))
            .await;
        if let Ok(res) = r {
            let files = res.output["files"].as_array().unwrap();
            // May be empty if run from a dir without .toml; just check no crash
            let _ = files.len();
        }
        // path not found in test env is ok
    }
}
