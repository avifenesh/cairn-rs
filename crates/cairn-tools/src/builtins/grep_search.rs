//! grep_search — pure-Rust regex content search over project files.
//!
//! Uses the `regex` crate for pattern matching and a recursive `std::fs`
//! directory walk (same approach as `glob_find.rs`). No external tools needed.
//!
//! ## Parameters
//! ```json
//! {
//!   "pattern":        "fn\\s+\\w+",   // regex or literal
//!   "path":           "src",          // root to search (default ".")
//!   "glob_filter":    "*.rs",         // optional filename glob
//!   "max_results":    50,             // default 50, max 200
//!   "case_sensitive": false           // default false
//! }
//! ```
//!
//! ## Output
//! ```json
//! {
//!   "matches":      [{ "file": "src/lib.rs", "line": 42, "content": "fn main() {" }],
//!   "total_matches": 1,
//!   "pattern":      "fn\\s+\\w+",
//!   "truncated":    false
//! }
//! ```

use async_trait::async_trait;
use cairn_domain::ProjectKey;
use regex::{Regex, RegexBuilder};
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

pub struct GrepSearchTool;

impl Default for GrepSearchTool {
    fn default() -> Self { Self }
}

// ── Glob filter helper (reused from glob_find logic) ─────────────────────────

/// Match only the filename portion against a simple glob pattern.
/// Supports `*` (any chars) and `?` (single char); no path separators in pattern.
fn filename_matches_glob(name: &str, pattern: &str) -> bool {
    let nc: Vec<char> = name.chars().collect();
    let pc: Vec<char> = pattern.chars().collect();
    fn m(n: &[char], p: &[char]) -> bool {
        match (n.first(), p.first()) {
            (None, None)       => true,
            (_, Some('*'))     => (0..=n.len()).any(|i| m(&n[i..], &p[1..])),
            (Some(_), Some('?')) => m(&n[1..], &p[1..]),
            (Some(a), Some(b)) => a == b && m(&n[1..], &p[1..]),
            _                  => false,
        }
    }
    m(&nc, &pc)
}

// ── Recursive search ──────────────────────────────────────────────────────────

fn search_dir(
    root:          &std::path::Path,
    re:            &Regex,
    glob_filter:   Option<&str>,
    max_results:   usize,
    results:       &mut Vec<Value>,
    total_matches: &mut usize,
) {
    let Ok(entries) = std::fs::read_dir(root) else { return };

    let mut sorted: Vec<_> = entries.flatten().collect();
    sorted.sort_by_key(|e| e.file_name());

    for entry in sorted {
        if *total_matches >= max_results * 2 {
            // Stop early once we have enough to detect truncation
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden dirs and common noise dirs
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || matches!(name, "target" | "node_modules" | ".git") {
                continue;
            }
            search_dir(&path, re, glob_filter, max_results, results, total_matches);
        } else {
            // Apply glob filter to the filename
            if let Some(glob) = glob_filter {
                let fname = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !filename_matches_glob(fname, glob) {
                    continue;
                }
            }

            // Skip binary-looking files: check extension allow-list as a fast path
            // then fall back to reading only if the extension is text-like.
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let is_text = matches!(ext,
                "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "c" | "cpp"
                | "h" | "hpp" | "cs" | "rb" | "swift" | "kt" | "scala" | "sh" | "bash"
                | "zsh" | "fish" | "ps1" | "bat" | "toml" | "yaml" | "yml" | "json"
                | "xml" | "html" | "htm" | "css" | "scss" | "sass" | "less" | "md"
                | "markdown" | "txt" | "rst" | "org" | "adoc" | "sql" | "graphql"
                | "proto" | "dockerfile" | "makefile" | "mk" | "conf" | "cfg" | "ini"
                | "env" | "gitignore" | "lock" | "log"
            ) || ext.is_empty(); // also try files with no extension

            if !is_text { continue; }

            let Ok(content) = std::fs::read_to_string(&path) else { continue };

            for (idx, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    *total_matches += 1;
                    if results.len() < max_results {
                        let rel = path.to_string_lossy().replace('\\', "/");
                        results.push(serde_json::json!({
                            "file":    rel,
                            "line":    idx + 1,
                            "content": line.trim_end(),
                        }));
                    }
                }
            }
        }
    }
}

// ── ToolHandler impl ──────────────────────────────────────────────────────────

#[async_trait]
impl ToolHandler for GrepSearchTool {
    fn name(&self) -> &str { "grep_search" }

    fn tier(&self) -> ToolTier { ToolTier::Registered }

    fn description(&self) -> &str {
        "Search file contents using regex patterns. \
         Returns matching lines with file path and line number. \
         Pure Rust — no external tools required."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["pattern"],
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regex or literal search pattern (Rust regex syntax)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory or file to search (default: '.')"
                },
                "glob_filter": {
                    "type": "string",
                    "description": "Filename glob filter e.g. '*.rs' or '*.ts' (matched against filename only)"
                },
                "max_results": {
                    "type": "integer",
                    "default": 50,
                    "maximum": 200,
                    "description": "Maximum number of matching lines to return"
                },
                "case_sensitive": {
                    "type": "boolean",
                    "default": false,
                    "description": "Case-sensitive matching (default: false)"
                }
            }
        })
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let pattern = args["pattern"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidArgs {
                field:   "pattern".into(),
                message: "required string".into(),
            })?
            .trim();

        if pattern.is_empty() {
            return Err(ToolError::InvalidArgs {
                field:   "pattern".into(),
                message: "must not be empty".into(),
            });
        }

        let path        = args["path"].as_str().unwrap_or(".");
        let max_results = args["max_results"].as_u64().unwrap_or(50).clamp(1, 200) as usize;
        let case_sens   = args["case_sensitive"].as_bool().unwrap_or(false);
        let glob_filter = args["glob_filter"].as_str();

        // Compile the regex (case-insensitive by default)
        let re = RegexBuilder::new(pattern)
            .case_insensitive(!case_sens)
            .build()
            .map_err(|e| ToolError::InvalidArgs {
                field:   "pattern".into(),
                message: format!("invalid regex: {e}"),
            })?;

        let root = std::path::Path::new(path);
        if !root.exists() {
            return Err(ToolError::InvalidArgs {
                field:   "path".into(),
                message: format!("path not found: {path}"),
            });
        }

        let mut results       = Vec::new();
        let mut total_matches = 0usize;

        if root.is_file() {
            // Single-file search
            let content = std::fs::read_to_string(root)
                .map_err(|e| ToolError::Permanent(format!("cannot read file: {e}")))?;
            for (idx, line) in content.lines().enumerate() {
                if re.is_match(line) {
                    total_matches += 1;
                    if results.len() < max_results {
                        results.push(serde_json::json!({
                            "file":    path,
                            "line":    idx + 1,
                            "content": line.trim_end(),
                        }));
                    }
                }
            }
        } else {
            search_dir(root, &re, glob_filter, max_results, &mut results, &mut total_matches);
        }

        let truncated = total_matches > max_results;
        Ok(ToolResult::ok(serde_json::json!({
            "matches":       results,
            "total_matches": total_matches,
            "pattern":       pattern,
            "truncated":     truncated,
        })))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn p() -> ProjectKey { ProjectKey::new("t", "w", "p") }

    fn tool() -> GrepSearchTool { GrepSearchTool }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn tier_is_registered() {
        assert_eq!(GrepSearchTool.tier(), ToolTier::Registered);
    }

    #[test]
    fn schema_requires_pattern() {
        let s = GrepSearchTool.parameters_schema();
        let req = s["required"].as_array().unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("pattern")));
    }

    // ── Validation ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn empty_pattern_err() {
        let err = tool().execute(&p(), serde_json::json!({"pattern": ""})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn missing_pattern_err() {
        let err = tool().execute(&p(), serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn invalid_regex_err() {
        let err = tool().execute(&p(), serde_json::json!({"pattern": "[unclosed"})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn nonexistent_path_err() {
        let err = tool().execute(&p(), serde_json::json!({
            "pattern": "hello", "path": "/tmp/cairn_test_nonexistent_xyz_12345"
        })).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    // ── Single-file search ────────────────────────────────────────────────────

    #[tokio::test]
    async fn finds_matches_in_single_file() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "hello world").unwrap();
        writeln!(tmp, "foo bar").unwrap();
        writeln!(tmp, "hello again").unwrap();

        let res = tool().execute(&p(), serde_json::json!({
            "pattern": "hello",
            "path":    tmp.path().to_str().unwrap(),
        })).await.unwrap();

        assert_eq!(res.output["total_matches"], 2);
        let matches = res.output["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0]["line"], 1);
        assert_eq!(matches[1]["line"], 3);
    }

    #[tokio::test]
    async fn case_insensitive_by_default() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "Hello World").unwrap();
        writeln!(tmp, "HELLO RUST").unwrap();

        let res = tool().execute(&p(), serde_json::json!({
            "pattern": "hello",
            "path":    tmp.path().to_str().unwrap(),
        })).await.unwrap();

        assert_eq!(res.output["total_matches"], 2, "case-insensitive should match both");
    }

    #[tokio::test]
    async fn case_sensitive_flag_respected() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "Hello World").unwrap();
        writeln!(tmp, "hello rust").unwrap();

        let res = tool().execute(&p(), serde_json::json!({
            "pattern":        "hello",
            "path":           tmp.path().to_str().unwrap(),
            "case_sensitive": true,
        })).await.unwrap();

        assert_eq!(res.output["total_matches"], 1, "case-sensitive should match only lowercase");
    }

    #[tokio::test]
    async fn regex_pattern_works() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "fn main() {{").unwrap();
        writeln!(tmp, "let x = 42;").unwrap();
        writeln!(tmp, "fn helper() {{").unwrap();

        let res = tool().execute(&p(), serde_json::json!({
            "pattern": "^fn\\s+\\w+",
            "path":    tmp.path().to_str().unwrap(),
        })).await.unwrap();

        assert_eq!(res.output["total_matches"], 2);
    }

    #[tokio::test]
    async fn no_match_returns_empty() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "no match here").unwrap();

        let res = tool().execute(&p(), serde_json::json!({
            "pattern": "xyz_impossible_pattern",
            "path":    tmp.path().to_str().unwrap(),
        })).await.unwrap();

        assert_eq!(res.output["total_matches"], 0);
        assert!(res.output["matches"].as_array().unwrap().is_empty());
        assert_eq!(res.output["truncated"], false);
    }

    // ── Directory search ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn searches_directory_recursively() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "match here\nnothing else\n").unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(sub.join("b.txt"), "another match\n").unwrap();

        let res = tool().execute(&p(), serde_json::json!({
            "pattern": "match",
            "path":    dir.path().to_str().unwrap(),
        })).await.unwrap();

        assert_eq!(res.output["total_matches"], 2);
    }

    #[tokio::test]
    async fn glob_filter_restricts_file_types() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("code.rs"),  "fn match_me() {}").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "match here too").unwrap();

        let res = tool().execute(&p(), serde_json::json!({
            "pattern":     "match",
            "path":        dir.path().to_str().unwrap(),
            "glob_filter": "*.rs",
        })).await.unwrap();

        let matches = res.output["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 1);
        assert!(matches[0]["file"].as_str().unwrap().ends_with(".rs"));
    }

    #[tokio::test]
    async fn max_results_limits_output() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        for i in 0..20 { writeln!(tmp, "match line {i}").unwrap(); }

        let res = tool().execute(&p(), serde_json::json!({
            "pattern":     "match",
            "path":        tmp.path().to_str().unwrap(),
            "max_results": 5,
        })).await.unwrap();

        let matches = res.output["matches"].as_array().unwrap();
        assert_eq!(matches.len(), 5);
        assert_eq!(res.output["total_matches"], 20);
        assert_eq!(res.output["truncated"], true);
    }

    // ── Glob filename helper ──────────────────────────────────────────────────

    #[test]
    fn glob_star_matches_extension() {
        assert!(filename_matches_glob("main.rs",  "*.rs"));
        assert!(!filename_matches_glob("main.ts", "*.rs"));
        assert!(filename_matches_glob("Cargo.toml", "*.toml"));
    }

    #[test]
    fn glob_question_mark() {
        assert!(filename_matches_glob("file1.rs", "file?.rs"));
        assert!(!filename_matches_glob("file12.rs", "file?.rs"));
    }
}
