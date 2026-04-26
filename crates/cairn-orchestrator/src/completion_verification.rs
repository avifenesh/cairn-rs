//! F47 PR1: tool_result scanner that produces a [`CompletionVerification`]
//! sidecar for the SSE `finished` event.
//!
//! # Motivation
//!
//! Dogfood M1 (2026-04-26) shipped a Rust crate that emitted
//! `warning: unused imports: Constraint, Direction, Layout, text::Line` in a
//! stored bash tool_result, while the LLM's `complete_run` summary claimed
//! "cargo check must pass with no warnings ✓". Operators had no independent
//! signal that the summary lied. This scanner is that signal.
//!
//! # Contract
//!
//! * Pure scanning — no IO. Fully unit-testable without a runtime.
//! * Never fabricates. If an exit code is not present in the tool_result
//!   structure, `CommandOutcome::exit_code = None`.
//! * Bounded retained output. The returned warning / error vectors are
//!   capped at [`MAX_ENTRIES_PER_BUCKET`] entries, each truncated to
//!   [`MAX_LINE_LEN`] chars. Scanning may use small per-call temporaries
//!   but memory retained across a run is proportional to the cap, not the
//!   tool_output size.
//! * Non-authoritative. The scanner reports what tool outputs contain; the
//!   orchestrator's loop signal remains the source of truth for run state.
//!
//! # Usage
//!
//! Two entry points cover the orchestrator loop's access patterns:
//!
//! * [`VerificationAccumulator`] — incremental. Feed one `ActionResult` at
//!   a time from the loop's execute phase; the full `tool_output` payload
//!   is scanned and discarded in place, so retained memory stays bounded
//!   even when the run reads large files via a `read` tool. Used by the
//!   production loop to avoid holding every iteration's raw `tool_output`
//!   in memory for the whole run.
//! * [`extract_verification`] — batch. Accepts a slice of `ActionResult`s
//!   and internally delegates to the accumulator. Kept for the unit tests
//!   and for callers that already have the full set in hand.
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
pub(crate) const BASH_TOOL_NAMES: &[&str] = &["bash", "shell_exec", "run_bash"];

/// Case-insensitive match on any member of [`BASH_TOOL_NAMES`]. Shared
/// with `loop_runner::render_tool_output_preview` so the preview path
/// and the verification scanner agree on which tool names are
/// "bash-class". Matching is case-insensitive because some provider
/// adapters normalize `"bash"` to `"Bash"` during JSON marshalling.
pub(crate) fn is_bash_tool(tool_name: &str) -> bool {
    BASH_TOOL_NAMES
        .iter()
        .any(|b| b.eq_ignore_ascii_case(tool_name))
}

/// Incremental builder. The orchestrator loop feeds one `ActionResult` per
/// iteration into [`VerificationAccumulator::observe`]; at Done it calls
/// [`VerificationAccumulator::finish`] to produce the sidecar. This avoids
/// retaining full `tool_output` payloads (which can be very large — a
/// `read` tool may return an entire file) across the run's lifetime:
/// each result is scanned and dropped in place, leaving only the bounded
/// bucket output in memory.
#[derive(Clone, Debug, Default)]
pub struct VerificationAccumulator {
    warnings: Vec<String>,
    errors: Vec<String>,
    commands: Vec<CommandOutcome>,
    tool_results_scanned: usize,
}

impl VerificationAccumulator {
    /// Create a fresh accumulator.
    pub fn new() -> Self {
        Self::default()
    }

    /// Scan a single `ActionResult`. Non-`InvokeTool` proposals are
    /// ignored up front — they never produce tool output.
    pub fn observe(&mut self, result: &ActionResult) {
        if result.proposal.action_type != cairn_domain::ActionType::InvokeTool {
            return;
        }
        self.tool_results_scanned += 1;

        let tool_name = result.proposal.tool_name.as_deref().unwrap_or("<unknown>");

        // Command outcome. Only bash-class tools produce a CommandOutcome
        // entry; non-bash tools contribute to warning/error scanning via
        // their text output but do not appear in `commands[]`.
        if is_bash_tool(tool_name) {
            let cmd = result
                .proposal
                .tool_args
                .as_ref()
                .and_then(|v| v.get("command"))
                .and_then(Value::as_str)
                .map(|s| truncate(s, MAX_LINE_LEN))
                .unwrap_or_default();
            let exit_code = result.tool_output.as_ref().and_then(extract_exit_code);
            self.commands.push(CommandOutcome {
                tool_name: tool_name.to_owned(),
                cmd,
                exit_code,
            });
        }

        // Text scan. Iterate string sources lazily, scanning in place to
        // avoid cloning large stdout/stderr payloads. Once both buckets
        // are full we can short-circuit the remaining sources entirely.
        for src in iter_text_sources(result) {
            if self.buckets_full() {
                return;
            }
            self.scan_text(&src);
        }
    }

    /// Consume the accumulator and produce the sidecar.
    pub fn finish(self) -> CompletionVerification {
        CompletionVerification {
            warnings: self.warnings,
            errors: self.errors,
            commands: self.commands,
            tool_results_scanned: self.tool_results_scanned,
            extractor_version: EXTRACTOR_VERSION,
        }
    }

    fn buckets_full(&self) -> bool {
        self.warnings.len() >= MAX_ENTRIES_PER_BUCKET && self.errors.len() >= MAX_ENTRIES_PER_BUCKET
    }

    fn scan_text(&mut self, text: &str) {
        for raw_line in text.lines() {
            if self.buckets_full() {
                return;
            }
            // Trim leading whitespace so indented diagnostics still match.
            // Don't trim trailing — loss of a trailing period or bracket
            // changes the meaning of the line.
            let line = raw_line.trim_start();

            if is_warning_line(line) && self.warnings.len() < MAX_ENTRIES_PER_BUCKET {
                self.warnings.push(truncate(line, MAX_LINE_LEN));
            } else if is_error_line(line) && self.errors.len() < MAX_ENTRIES_PER_BUCKET {
                self.errors.push(truncate(line, MAX_LINE_LEN));
            }
        }
    }
}

/// Batch wrapper over [`VerificationAccumulator`]. Prefer
/// `VerificationAccumulator::observe` from the loop so large `tool_output`
/// payloads aren't retained across the whole run; this helper is kept for
/// unit tests and any caller that already holds the full set.
pub fn extract_verification(tool_results: &[ActionResult]) -> CompletionVerification {
    let mut acc = VerificationAccumulator::new();
    for result in tool_results {
        acc.observe(result);
    }
    acc.finish()
}

/// Yield each scannable `&str` from an `ActionResult` without cloning. For
/// structured JSON tool outputs we look at a small set of canonical key
/// names (`stdout`, `stderr`, `output`, `message`, …) rather than the
/// whole blob so the scanner doesn't match on unrelated JSON keys like
/// `"warning_count": 3`. Falls back to the entire object's `to_string()`
/// only when no canonical key matched — that fallback path does allocate
/// but covers tool adapters that flatten their payload into a
/// non-standard shape.
fn iter_text_sources(result: &ActionResult) -> Vec<std::borrow::Cow<'_, str>> {
    use std::borrow::Cow;
    let mut out: Vec<Cow<'_, str>> = Vec::new();
    if let Some(value) = result.tool_output.as_ref() {
        match value {
            Value::String(s) => out.push(Cow::Borrowed(s.as_str())),
            Value::Object(map) => {
                for key in ["stdout", "stderr", "output", "message", "text", "content"] {
                    if let Some(Value::String(s)) = map.get(key) {
                        out.push(Cow::Borrowed(s.as_str()));
                    }
                }
                if out.is_empty() {
                    // Only rarely reached — serialises the whole object
                    // once, not the full tool_result.
                    out.push(Cow::Owned(value.to_string()));
                }
            }
            other => out.push(Cow::Owned(other.to_string())),
        }
    }
    if let ActionStatus::Failed { reason } = &result.status {
        out.push(Cow::Borrowed(reason.as_str()));
    }
    out
}

/// Extract an exit code from a bash-class tool_output if structurally
/// present. Accepts several canonical key names used by different harness
/// adapters (`exit_code`, `exitCode`, `returncode`, …). Non-integer values
/// return `None` rather than coercing.
pub(crate) fn extract_exit_code(output: &Value) -> Option<i32> {
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

/// Case-insensitive ASCII prefix match on the leading bytes of `line`.
/// Byte-level comparison avoids any UTF-8 boundary pitfall when non-ASCII
/// tool output is mixed in: the prefix we match on (`warning`, `error`) is
/// ASCII, so comparing bytes is well-defined whether or not subsequent
/// bytes start a multi-byte sequence. Cursor / Gemini review flagged the
/// earlier `str::get(..N)` char-slice form for missing lines like
/// `"warning: über-thing"` when the char index landed inside a multi-byte
/// char; this helper closes that gap.
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

/// Match `warning:` at the start of a (left-trimmed) line. Case-insensitive
/// but ASCII-only — `WARN:` and other variants are intentionally NOT
/// matched. The scanner is tuned for `rustc`/`cargo`/`clippy` and the
/// generic `warning:` prefix, which covers the M1 dogfood regression.
fn is_warning_line(line: &str) -> bool {
    ascii_prefix_ci(line, "warning:")
}

/// Match `error:` / `error[…]:` / `error <ws>` at the start of a
/// left-trimmed line. Accepts the Rust diagnostic form
/// `error[E0308]: mismatched types` as well as plain `error:` from bash
/// output. Rejects `errored out`, `errorless` etc. so false positives in
/// ordinary prose stay out of the bucket.
fn is_error_line(line: &str) -> bool {
    if !ascii_prefix_ci(line, "error") {
        return false;
    }
    // Byte indexing is safe here — `error` is ASCII so the 5th byte lies
    // on a valid char boundary regardless of subsequent multi-byte bytes.
    matches!(
        line.as_bytes().get("error".len()),
        Some(b':') | Some(b'[') | Some(b' ') | Some(b'\t')
    )
}

/// Clip `s` to `max` chars, appending an ellipsis marker when trimmed.
/// Walks `char_indices` and stops as soon as the cap is known to be
/// exceeded — work is proportional to `max`, not to the full string
/// length. This matters for pathological tool outputs (MB-scale
/// single-line payloads) where `s.chars().count()` would scan the
/// entire buffer.
fn truncate(s: &str, max: usize) -> String {
    // Fast path: ASCII strings that fit.
    if s.len() <= max && s.is_ascii() {
        return s.to_owned();
    }
    // Walk up to `max` chars. Track the byte offset where the `max-3`rd
    // char ended, so we can truncate there on overflow.
    let mut seen = 0usize;
    let mut split_at = 0usize;
    let cutoff = max.saturating_sub(3);
    for (idx, ch) in s.char_indices() {
        if seen == cutoff {
            split_at = idx;
        }
        seen += 1;
        if seen > max {
            let mut out = String::with_capacity(split_at + 3);
            out.push_str(&s[..split_at]);
            out.push_str("...");
            return out;
        }
        // Keep `split_at` up to date for the terminating case where the
        // string is exactly `max` chars long (no truncation needed).
        if seen == max {
            split_at = idx + ch.len_utf8();
        }
    }
    // Reached end within the cap — no truncation needed.
    s.to_owned()
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
    /// trim.
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

    /// Multi-byte characters after the ASCII prefix must not break
    /// detection. Cursor / Gemini review flagged this as a real hazard —
    /// a `"warning: über-thing"` line would have been silently missed by
    /// the earlier `str::get(..N)` char-slice form when byte 10 landed
    /// inside the 2-byte encoding of `ü`.
    #[test]
    fn multi_byte_characters_do_not_break_detection() {
        assert!(is_warning_line("warning: über-thing went wrong"));
        assert!(is_error_line("error[E0308]: mismatched types — 😤"));
        assert!(!is_warning_line("über-thing: not a warning"));
    }

    /// Incremental accumulator behaves identically to the batch helper
    /// for the same inputs. The loop runner uses the incremental form to
    /// avoid retaining full `tool_output` across the whole run.
    #[test]
    fn accumulator_matches_batch_helper() {
        let inputs = [
            bash_result("cargo check", "warning: a\nerror: b\n", Some(1)),
            bash_result("ls", "hello\n", Some(0)),
        ];
        let batch = extract_verification(&inputs);
        let mut acc = VerificationAccumulator::new();
        for r in &inputs {
            acc.observe(r);
        }
        let incremental = acc.finish();
        assert_eq!(batch, incremental);
    }
}
