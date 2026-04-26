//! F47 PR1: pure extractor that scans tool_result frames at run Done and
//! produces a [`CompletionVerification`] sidecar for the SSE `finished` event.
//!
//! # Motivation
//!
//! Dogfood M1 (2026-04-26) shipped a Rust crate that emitted
//! `warning: unused imports: Constraint, Direction, Layout, text::Line` in a
//! stored bash tool_result, while the LLM's `complete_run` summary claimed
//! "cargo check must pass with no warnings ✓". Operators had no independent
//! signal that the summary lied. This extractor is that signal.
//!
//! # Contract
//!
//! * Pure function — no IO, no allocation beyond the returned struct and its
//!   owned strings. Fully unit-testable without a runtime.
//! * Never fabricates. If an exit code is not present in the tool_result
//!   structure, `CommandOutcome::exit_code = None`.
//! * Bounded. Warning / error vectors are capped at [`MAX_ENTRIES_PER_BUCKET`]
//!   entries, each truncated to [`MAX_LINE_LEN`] chars. Memory stays flat
//!   regardless of tool_result size.
//! * Non-authoritative. The extractor reports what tool outputs contain; the
//!   orchestrator's loop signal remains the source of truth for run state.
//!
//! # Scope
//!
//! PR1 (this file) makes the sidecar visible on the SSE `finished` event.
//! PR2 adds persistence (event + projection + REST surface). PR3 adds UI.

use cairn_domain::{CommandOutcome, CompletionVerification};
use serde_json::Value;

use crate::context::{ActionResult, ActionStatus};

/// Maximum number of warning or error entries kept per bucket. Lines past
/// this cap are silently dropped; the count is implicit in the vector
/// length. Chosen so a pathological tool_result (e.g. clippy with thousands
/// of lints) cannot bloat an SSE frame.
pub const MAX_ENTRIES_PER_BUCKET: usize = 50;

/// Maximum length of a single matched line kept in a bucket. Longer lines
/// are truncated with an ellipsis marker appended. 500 chars comfortably
/// holds a full Rust diagnostic header without flooding the SSE payload.
pub const MAX_LINE_LEN: usize = 500;

/// Current extractor version. Bump when the matching or truncation policy
/// changes in a way that downstream consumers need to notice. v1 = F47 PR1.
pub const EXTRACTOR_VERSION: u32 = 1;

/// Bash-class tool names. These are scanned for structured `command` and
/// `exit_code` fields. Other tools still contribute to warning / error
/// scanning via their text output but produce no [`CommandOutcome`] entry.
const BASH_TOOL_NAMES: &[&str] = &["bash", "shell_exec", "run_bash"];

/// Scan the `tool_results` observed across a run and distil a
/// [`CompletionVerification`] sidecar.
///
/// The caller accumulates [`ActionResult`] values across iterations (the
/// orchestrator loop does this in `run_inner`) and passes the full set at
/// Done. An empty slice produces a default-valued verification with
/// `tool_results_scanned = 0` and `extractor_version = EXTRACTOR_VERSION`,
/// which the caller can use to distinguish "no tool calls in this run"
/// from "scanned but found nothing."
pub fn extract_verification(tool_results: &[ActionResult]) -> CompletionVerification {
    // Only InvokeTool results are meaningful tool_result frames. Other
    // action kinds (CompleteRun, SpawnSubagent, SendNotification, …) never
    // produce tool output so they are skipped up front.
    let mut verification = CompletionVerification {
        warnings: Vec::new(),
        errors: Vec::new(),
        commands: Vec::new(),
        tool_results_scanned: 0,
        extractor_version: EXTRACTOR_VERSION,
    };

    for result in tool_results {
        if result.proposal.action_type != cairn_domain::ActionType::InvokeTool {
            continue;
        }
        verification.tool_results_scanned += 1;

        let tool_name = result
            .proposal
            .tool_name
            .clone()
            .unwrap_or_else(|| "<unknown>".to_owned());

        // Command outcome. Bash-class tools expose `{ command: "…" }` in the
        // proposal args; the exit code (when present) lives on the tool_output
        // as either `exit_code` or `returncode` (different adapters). Both are
        // treated identically here because the extractor is not opinionated
        // about which tool adapter produced the frame.
        if BASH_TOOL_NAMES
            .iter()
            .any(|b| b.eq_ignore_ascii_case(&tool_name))
        {
            let cmd = result
                .proposal
                .tool_args
                .as_ref()
                .and_then(|v| v.get("command"))
                .and_then(Value::as_str)
                .map(|s| truncate(s, MAX_LINE_LEN))
                .unwrap_or_default();
            let exit_code = result.tool_output.as_ref().and_then(extract_exit_code);
            verification.commands.push(CommandOutcome {
                tool_name: tool_name.clone(),
                cmd,
                exit_code,
            });
        }

        // Text scan. Combine tool_output text (for success) and failure
        // reason text (for ActionStatus::Failed) into one source so a
        // failed tool that wrote warnings to stderr and returned a
        // non-zero status still surfaces its warnings.
        let text_sources = collect_text_sources(result);
        for src in text_sources {
            scan_lines(&src, &mut verification);
        }
    }

    verification
}

/// Pull every scannable text fragment out of an `ActionResult` into a small
/// vector. For structured JSON tool outputs we stringify key fields
/// (`stdout`, `stderr`, `output`, `message`) rather than the whole blob so
/// the scanner doesn't match on unrelated JSON keys like `"warning_count":
/// 3`.
fn collect_text_sources(result: &ActionResult) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(value) = result.tool_output.as_ref() {
        match value {
            Value::String(s) => out.push(s.clone()),
            Value::Object(_) => {
                for key in ["stdout", "stderr", "output", "message", "text", "content"] {
                    if let Some(Value::String(s)) = value.get(key) {
                        out.push(s.clone());
                    }
                }
                // Also consider the whole object serialised as a fallback:
                // some tool adapters flatten the entire payload into a
                // single object with no canonical key. Stringifying still
                // lets the line-based scanner find `warning:` / `error:`.
                if out.is_empty() {
                    out.push(value.to_string());
                }
            }
            other => out.push(other.to_string()),
        }
    }
    if let ActionStatus::Failed { reason } = &result.status {
        out.push(reason.clone());
    }
    out
}

/// Extract an exit code from a bash-class tool_output if structurally
/// present. Accepts both `exit_code` and `returncode` keys (different
/// harness adapters use different names). Non-integer values return
/// `None` rather than coercing.
fn extract_exit_code(output: &Value) -> Option<i32> {
    for key in [
        "exit_code",
        "exitCode",
        "returncode",
        "return_code",
        "status",
    ] {
        if let Some(v) = output.get(key) {
            if let Some(n) = v.as_i64() {
                return i32::try_from(n).ok();
            }
        }
    }
    None
}

/// Scan `text` line-by-line; route warning-prefixed lines into
/// `verification.warnings` and error-prefixed lines into
/// `verification.errors`. Respects `MAX_ENTRIES_PER_BUCKET` and
/// `MAX_LINE_LEN`.
fn scan_lines(text: &str, verification: &mut CompletionVerification) {
    for raw_line in text.lines() {
        // Trim leading whitespace so indented diagnostics (e.g. from a
        // wrapped bash output) still match. Don't trim trailing — loss of
        // a trailing period or bracket changes the meaning of the line.
        let line = raw_line.trim_start();

        if is_warning_line(line) && verification.warnings.len() < MAX_ENTRIES_PER_BUCKET {
            verification.warnings.push(truncate(line, MAX_LINE_LEN));
        } else if is_error_line(line) && verification.errors.len() < MAX_ENTRIES_PER_BUCKET {
            verification.errors.push(truncate(line, MAX_LINE_LEN));
        }
    }
}

/// Case-insensitive ASCII prefix match. Only the first `prefix.len()` bytes
/// of `line` are compared — identical to `str::starts_with` with a
/// lowercased input. Operating on bytes (not `get(..N)` char slicing)
/// avoids any UTF-8 boundary pitfall when non-ASCII tool output is mixed
/// in: the byte prefix match is well-defined whether or not the next byte
/// starts a multi-byte sequence.
fn ascii_prefix_ci(line: &str, prefix: &str) -> bool {
    let bytes = line.as_bytes();
    let pb = prefix.as_bytes();
    if bytes.len() < pb.len() {
        return false;
    }
    bytes[..pb.len()]
        .iter()
        .zip(pb.iter())
        .all(|(a, b)| a.eq_ignore_ascii_case(b))
}

/// Case-insensitive prefix check for `warning:`. Regex-free to keep the
/// extractor dependency-light. Equivalent to `(?mi)^\s*warning:` applied
/// line-by-line — callers trim leading whitespace before dispatch.
fn is_warning_line(line: &str) -> bool {
    ascii_prefix_ci(line, "warning:")
}

/// Case-insensitive prefix check for `error:` / `error[…]:` / `error <ws>`.
/// Accepts the Rust diagnostic forms (`error[E0308]: mismatched types`) as
/// well as generic `error:` lines from bash output.
fn is_error_line(line: &str) -> bool {
    if !ascii_prefix_ci(line, "error") {
        return false;
    }
    // After the `error` prefix, accept `:` or `[` (rustc style) or
    // whitespace-then-content. Reject `errored`, `errorless`, etc. Byte
    // indexing is safe here — `error` is ASCII so the 5th byte is on a
    // valid char boundary regardless of later multi-byte content.
    matches!(
        line.as_bytes().get("error".len()),
        Some(b':') | Some(b'[') | Some(b' ') | Some(b'\t')
    )
}

/// Clip `s` to `max` chars, appending an ellipsis marker when trimmed.
/// Operates on char boundaries so UTF-8 output (e.g. rustc's arrows) is
/// preserved.
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ActionResult, ActionStatus};
    use cairn_domain::ActionProposal;
    use serde_json::json;

    fn bash_result(command: &str, stdout: &str, exit_code: Option<i32>) -> ActionResult {
        let mut output = serde_json::Map::new();
        output.insert("stdout".into(), Value::String(stdout.to_owned()));
        if let Some(code) = exit_code {
            output.insert("exit_code".into(), Value::from(code));
        }
        ActionResult {
            proposal: ActionProposal::invoke_tool(
                "bash",
                json!({ "command": command }),
                "run a shell command",
                0.95,
                false,
            ),
            status: ActionStatus::Succeeded,
            tool_output: Some(Value::Object(output)),
            invocation_id: None,
            duration_ms: 0,
        }
    }

    fn failed_bash(command: &str, reason: &str) -> ActionResult {
        ActionResult {
            proposal: ActionProposal::invoke_tool(
                "bash",
                json!({ "command": command }),
                "run a shell command",
                0.95,
                false,
            ),
            status: ActionStatus::Failed {
                reason: reason.to_owned(),
            },
            tool_output: None,
            invocation_id: None,
            duration_ms: 0,
        }
    }

    /// (a) Two warnings + one error on a cargo-check-style payload must
    /// land in the right buckets with the matching text preserved.
    #[test]
    fn warnings_and_errors_are_bucketed() {
        let cargo_stdout = "\
warning: unused imports: `Constraint`, `Direction`
  --> src/lib.rs:1:5
warning: function `dead` is never used
  --> src/lib.rs:10:4
error[E0308]: mismatched types
  --> src/lib.rs:20:1
";
        let v = extract_verification(&[bash_result("cargo check", cargo_stdout, Some(1))]);

        assert_eq!(v.extractor_version, EXTRACTOR_VERSION);
        assert_eq!(v.tool_results_scanned, 1);
        assert_eq!(v.warnings.len(), 2, "warnings: {:#?}", v.warnings);
        assert!(v.warnings[0].contains("unused imports"));
        assert!(v.warnings[1].contains("never used"));
        assert_eq!(v.errors.len(), 1, "errors: {:#?}", v.errors);
        assert!(v.errors[0].contains("mismatched types"));
        assert_eq!(v.commands.len(), 1);
        assert_eq!(v.commands[0].tool_name, "bash");
        assert_eq!(v.commands[0].cmd, "cargo check");
        assert_eq!(v.commands[0].exit_code, Some(1));
    }

    /// (b) Clean tool output produces empty warning / error vectors but a
    /// non-zero `tool_results_scanned` — this is the "verified clean"
    /// signal operators rely on.
    #[test]
    fn clean_output_yields_empty_buckets_with_nonzero_scan() {
        let v = extract_verification(&[bash_result("echo hello", "hello\n", Some(0))]);
        assert!(v.warnings.is_empty());
        assert!(v.errors.is_empty());
        assert_eq!(v.tool_results_scanned, 1);
        assert_eq!(v.commands[0].exit_code, Some(0));
    }

    /// Empty input produces defaults — distinguishes "no tool calls" from
    /// "scanned and found nothing."
    #[test]
    fn empty_input_produces_default_with_version_stamp() {
        let v = extract_verification(&[]);
        assert_eq!(v.tool_results_scanned, 0);
        assert_eq!(v.extractor_version, EXTRACTOR_VERSION);
        assert!(v.warnings.is_empty());
        assert!(v.errors.is_empty());
        assert!(v.commands.is_empty());
    }

    /// (c) Truncation. Synthesise 60 warning lines and a single 700-char
    /// warning line; verify the cap at 50 entries and the 500-char per-line
    /// trim (with ellipsis marker).
    #[test]
    fn truncation_caps_entries_and_line_length() {
        let mut stdout = String::new();
        for i in 0..60 {
            stdout.push_str(&format!("warning: lint {i}\n"));
        }
        stdout.push_str(&format!("warning: {}\n", "x".repeat(700)));

        let v = extract_verification(&[bash_result("cargo clippy", &stdout, Some(0))]);

        assert_eq!(
            v.warnings.len(),
            MAX_ENTRIES_PER_BUCKET,
            "must cap at {MAX_ENTRIES_PER_BUCKET}"
        );
        // Every retained line must respect the per-line char cap.
        for w in &v.warnings {
            assert!(
                w.chars().count() <= MAX_LINE_LEN,
                "line of {} chars exceeded {} cap: {w}",
                w.chars().count(),
                MAX_LINE_LEN,
            );
        }
    }

    /// (d) Exit code surfaces when present; `None` when absent. The
    /// extractor never fabricates `0`.
    #[test]
    fn exit_code_surfaces_only_when_structurally_present() {
        let with_code = extract_verification(&[bash_result("true", "", Some(0))]);
        assert_eq!(with_code.commands[0].exit_code, Some(0));

        let without_code = extract_verification(&[bash_result("true", "", None)]);
        assert_eq!(without_code.commands[0].exit_code, None);
    }

    /// Non-bash tools contribute text scanning but produce no CommandOutcome.
    #[test]
    fn non_bash_tool_contributes_scan_but_no_command_entry() {
        let result = ActionResult {
            proposal: ActionProposal::invoke_tool(
                "read",
                json!({ "path": "/tmp/x" }),
                "read a file",
                0.9,
                false,
            ),
            status: ActionStatus::Succeeded,
            tool_output: Some(json!({ "content": "warning: header mismatch\nok" })),
            invocation_id: None,
            duration_ms: 0,
        };
        let v = extract_verification(&[result]);
        assert_eq!(v.warnings.len(), 1);
        assert!(v.commands.is_empty(), "read is not bash-class");
        assert_eq!(v.tool_results_scanned, 1);
    }

    /// Failed tool call: warnings emitted on stderr-turned-reason still
    /// flow into the bucket so operators see them even on error paths.
    #[test]
    fn failed_tool_contributes_reason_text_to_scan() {
        let v = extract_verification(&[failed_bash(
            "cargo build",
            "warning: unused import: Foo\nerror: could not compile `cairn-app`",
        )]);
        assert_eq!(v.warnings.len(), 1);
        assert_eq!(v.errors.len(), 1);
        assert!(v.errors[0].contains("could not compile"));
    }

    /// Rust's bracketed diagnostic form `error[E0308]: …` must match the
    /// error bucket — this is the shape that motivated the F47 triage.
    #[test]
    fn rust_bracketed_error_form_matches() {
        assert!(is_error_line("error[E0308]: mismatched types"));
        assert!(is_error_line("error: linker failed"));
        assert!(!is_error_line("errored out"));
        assert!(!is_error_line("errorless"));
    }
}
