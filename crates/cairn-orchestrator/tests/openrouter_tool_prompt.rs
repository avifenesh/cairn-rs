//! Integration test: orchestrator tool-prompt against a real small/mid LLM.
//!
//! Regression test for F12b — Qwen 3.6 (and other small open models) were
//! emitting `tool_calls[].name == "invoke_tool"` because the cairn system
//! prompt told them to "Use invoke_tool with tool_name". After the fix
//! aligns the prompt with the proven format from avifenesh/tools, the
//! model should emit the concrete tool name (e.g. `bash`, `read`)
//! directly.
//!
//! ## How to run
//!
//! This test is `#[ignore]` by default so CI does not spend tokens.
//! Run manually when validating the F12b fix:
//!
//! ```text
//! export OPENROUTER_API_KEY=sk-or-...
//! cargo test -p cairn-orchestrator --test openrouter_tool_prompt \
//!   -- --ignored --nocapture
//! ```
//!
//! Optional overrides:
//! - `CAIRN_F12B_MODEL` (default: `qwen/qwen3-30b-a3b-instruct-2507`) —
//!   any OpenRouter model slug with native tool-call support.
//! - `CAIRN_F12B_BASE_URL` (default: `https://openrouter.ai/api/v1/`).
//!
//! ## What it asserts
//!
//! 1. The provider returns at least one native `tool_calls` entry.
//! 2. No entry's `function.name` equals `"invoke_tool"` or
//!    `"spawn_subagent"` (both are cairn meta-verbs, never real tools).
//! 3. At least one call targets a real tool registered in the prompt.

use cairn_domain::providers::{GenerationProvider, ProviderBindingSettings};
use cairn_providers::builder::Backend;
use cairn_providers::wire::openai_compat::OpenAiCompat;

fn require_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "hits OpenRouter; requires OPENROUTER_API_KEY. Run with --ignored to validate F12b fix."]
async fn qwen_does_not_emit_invoke_tool_envelope_under_native_tool_calling() {
    let Some(api_key) = require_env("OPENROUTER_API_KEY") else {
        panic!(
            "OPENROUTER_API_KEY not set — this test is manual only. \
             Run: OPENROUTER_API_KEY=... cargo test -p cairn-orchestrator \
             --test openrouter_tool_prompt -- --ignored"
        );
    };

    let model = std::env::var("CAIRN_F12B_MODEL")
        .unwrap_or_else(|_| "qwen/qwen3-30b-a3b-instruct-2507".to_owned());
    let base_url = std::env::var("CAIRN_F12B_BASE_URL").ok();

    let config = Backend::OpenRouter.config();
    let provider = OpenAiCompat::new(
        config,
        api_key,
        base_url,
        Some(model.clone()),
        Some(512),
        Some(0.3),
        Some(60),
    )
    .expect("build OpenRouter provider");

    // A minimal system prompt that mirrors the native-tool-mode branch of
    // `build_system_prompt`: tools are listed, and the instruction is to
    // call them directly, never via an envelope. Using `concat!` keeps
    // whitespace predictable — line-continuation (`\`) escapes keep the
    // indentation as literal spaces in the prompt, which muddies the
    // "proven format" signal.
    let system = concat!(
        "You are an autonomous coding agent. Call any of the ",
        "following tools directly, by its exact name, using the ",
        "provider's native tool-call mechanism. Do not wrap calls ",
        "in any envelope — emit one tool call per action with the ",
        "tool's JSON arguments.\n",
        "\n",
        "## Available tools\n",
        "- bash(command: string) — Run a shell command and return stdout+stderr.\n",
        "- read(path: string) — Read a file from the local filesystem.",
    );

    let user = concat!(
        "List the files in /tmp by calling the bash tool with ",
        "`ls /tmp`. Call the tool directly — do not wrap it in ",
        "any envelope.",
    );

    let tools = vec![
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "bash",
                "description":
                    "Run a shell command in the current working directory and return stdout and stderr. \
                     Use this when you need to inspect the filesystem, run tests, or execute any command.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "Shell command to execute." }
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }
            }
        }),
        serde_json::json!({
            "type": "function",
            "function": {
                "name": "read",
                "description":
                    "Read a file from the local filesystem. Use this when the user refers to a file by path \
                     and you need its contents to answer.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Absolute file path." }
                    },
                    "required": ["path"],
                    "additionalProperties": false
                }
            }
        }),
    ];

    let messages = vec![
        serde_json::json!({ "role": "system", "content": system }),
        serde_json::json!({ "role": "user",   "content": user   }),
    ];

    let settings = ProviderBindingSettings {
        max_output_tokens: Some(512),
        ..Default::default()
    };

    let resp = provider
        .generate(&model, messages, &settings, &tools)
        .await
        .expect("generate call succeeds");

    eprintln!(
        "[F12b] finish_reason={:?} tool_calls={} text_len={}",
        resp.finish_reason,
        resp.tool_calls.len(),
        resp.text.len()
    );
    for (i, tc) in resp.tool_calls.iter().enumerate() {
        eprintln!("[F12b] tool_call[{i}] = {tc}");
    }

    assert!(
        !resp.tool_calls.is_empty(),
        "model returned no native tool_calls — expected at least one real tool call"
    );

    for tc in &resp.tool_calls {
        let name = tc
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("");
        assert_ne!(
            name, "invoke_tool",
            "model emitted the legacy `invoke_tool` meta-verb as a tool name — prompt regression"
        );
        assert_ne!(
            name, "spawn_subagent",
            "model emitted the legacy `spawn_subagent` meta-verb as a tool name — prompt regression"
        );
    }

    let names: Vec<&str> = resp
        .tool_calls
        .iter()
        .filter_map(|tc| tc.get("function")?.get("name")?.as_str())
        .collect();
    assert!(
        names.iter().any(|n| *n == "bash" || *n == "read"),
        "expected at least one call to `bash` or `read`; got {names:?}"
    );
}
