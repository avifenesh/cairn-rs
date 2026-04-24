# Z.ai provider

Native adapter for [Z.ai](https://z.ai/) GLM models. Lives in
`crates/cairn-providers/src/wire/zai.rs` as a dedicated wire module — kept
out of `openai_compat` deliberately so Z.ai-specific quirks (thinking mode,
cached-token accounting, the `code 1305` HTTP-200 overload envelope) can
evolve without risking regressions in the OpenAI / DeepSeek / Groq paths.

## Endpoints

Z.ai offers three product tiers that share an OpenAI-shaped wire format but
are billed and rate-limited separately:

| Tier | Base URL | Use case |
|------|----------|----------|
| **Coding Plan** | `https://api.z.ai/api/coding/paas/v4/` | GLM Coding Plan subscribers ($10+/mo). Bundled quotas, faster GLM routing. Default for this adapter. |
| **General (paas)** | `https://api.z.ai/api/paas/v4/` | Pay-as-you-go API. Full model catalog including embeddings/vision. |
| **Anthropic-compat** | `https://api.z.ai/api/anthropic` | Drop-in `ANTHROPIC_BASE_URL` for Claude Code and other Anthropic-Messages clients. **Not** what this adapter uses. |

Cairn uses the **OpenAI-shaped endpoints** (tiers 1 and 2) because the rest
of the cairn-providers machinery (tool calls, SSE streaming, `reasoning_content`
deltas) already speaks that dialect.

## Backends

- `Backend::Zai` → General tier (`api.z.ai/api/paas/v4/`), default model `glm-4.7`.
- `Backend::ZaiCoding` → Coding Plan tier (`api.z.ai/api/coding/paas/v4/`), default model `glm-4.7`.

Both are string-parseable from `"zai" | "z_ai" | "z-ai" | "z.ai"` and
`"zai-coding" | "glm-coding" | ...` respectively.

## Authentication

Bearer token in the `Authorization` header. The same API key works across
both OpenAI-shaped tiers (tested 2026-04-24 against the Coding Plan endpoint).

```
Authorization: Bearer <ZAI_API_KEY>
```

## Wire format

### Request

Standard OpenAI `chat/completions` envelope with one Z.ai-specific extension:

```jsonc
{
  "model": "glm-4.7",
  "messages": [...],
  "max_tokens": 1024,
  "temperature": 0.7,
  "stream": false,
  "tools": [...],       // OpenAI-style function tools
  "tool_choice": "...", // OpenAI-style
  "thinking": { "type": "enabled" }   // Z.ai extension — "enabled" | "disabled"
}
```

**`thinking.type`** (Z.ai-specific):
- `"enabled"` (default in this adapter): GLM runs its reasoning chain and
  returns `reasoning_content` on the assistant message and `reasoning_tokens`
  nested in `completion_tokens_details`.
- `"disabled"`: no chain-of-thought, no `reasoning_tokens`. Cheaper; use for
  deterministic or latency-critical calls.

Toggle per-provider via `ZaiProvider::enable_thinking_override`.

### Response (non-streaming)

Captured verbatim from
`POST https://api.z.ai/api/coding/paas/v4/chat/completions` on 2026-04-24:

```json
{
  "choices": [{
    "finish_reason": "tool_calls",
    "index": 0,
    "message": {
      "content": "I'll check the weather in Paris for you.",
      "reasoning_content": "The user is asking for the weather in Paris...",
      "role": "assistant",
      "tool_calls": [{
        "function": {
          "arguments": "{\"city\":\"Paris\"}",
          "name": "get_weather"
        },
        "id": "call_-7682507267639338722",
        "index": 0,
        "type": "function"
      }]
    }
  }],
  "created": 1777040104,
  "id": "20260424221502ec4cefb07a7c417b",
  "model": "glm-4.7",
  "object": "chat.completion",
  "request_id": "20260424221502ec4cefb07a7c417b",
  "usage": {
    "completion_tokens": 78,
    "completion_tokens_details": { "reasoning_tokens": 56 },
    "prompt_tokens": 160,
    "prompt_tokens_details": { "cached_tokens": 0 },
    "total_tokens": 238
  }
}
```

Cairn-visible surface:

- `response.text()` → `content`
- `response.thinking()` → `reasoning_content`
- `response.tool_calls()` → `tool_calls`
- `response.usage()` → `Usage` with `prompt_tokens`, `completion_tokens`,
  `total_tokens`, and **`cached_tokens`** (parsed from
  `prompt_tokens_details.cached_tokens` when non-zero; `None` otherwise).
- `response.finish_reason()` → `"stop" | "tool_calls" | "length" | ...`

### Response (streaming SSE)

Standard OpenAI SSE with `data: {...}\n\n` frames and a trailing
`data: [DONE]`. `delta.reasoning_content` carries chain-of-thought deltas;
`delta.content` carries the final answer; `delta.tool_calls` carries
OpenAI-style function-call deltas with `index`.

Usage is delivered on the penultimate chunk (alongside `finish_reason`) when
`stream_options.include_usage: true` is set, which this adapter always does.

### Error envelope

Z.ai sometimes returns HTTP 200 with an error body for transient conditions:

```json
{"error":{"code":"1305","message":"The service may be temporarily overloaded, please try again later"}}
```

Observed 2026-04-24 on a streaming request during rate-limited hours. The
adapter detects the envelope and surfaces `ProviderError::RateLimited` so
the fallback chain (`cairn_orchestrator::ModelChain`) can retry on another
model/provider.

Codes mapped so far:
- `1305` → `RateLimited` (service overload, retry later)
- `429` (if ever sent as envelope) → `RateLimited`
- Everything else → `InvalidRequest` with redacted message.

## Caching behaviour (coding plan)

The Coding Plan endpoint does **automatic server-side prefix caching** —
there is no `cache_control` per-message hint in the request format (we
checked the live endpoint 2026-04-24). Cache hits are reported via
`usage.prompt_tokens_details.cached_tokens`. The adapter parses this
field into the generic `Usage::cached_tokens` (Option<u32>) for
observability; zero values are normalised to `None` to avoid metric noise.

No action is needed from callers: prompt prefixes that repeat across calls
are cached automatically server-side.

## Model catalog

Live `/models` endpoint as of 2026-04-24:

```
glm-4.5        glm-4.5-air     glm-4.6
glm-4.7        glm-5           glm-5-turbo   glm-5.1
```

`GET /models` works on both the coding and general tiers with the standard
OpenAI shape (`{"object": "list", "data": [...]}`), so the existing
`discover_openai_compat_models_live` probe already handles Z.ai when
`adapter_type` is `openai_compat`. This adapter keeps the default for
back-compat; operators can still explicitly tag a connection `"zai"` in the
UI for display purposes.

## Live smoke transcript (2026-04-24)

```
$ curl -X POST "https://api.z.ai/api/coding/paas/v4/chat/completions" \
    -H "Authorization: Bearer $ZAI_API_KEY" \
    -d '{"model":"glm-4.7","messages":[{"role":"user","content":"Reply with just: hello"}],"max_tokens":30,"stream":false}'

{"choices":[{"finish_reason":"length","index":0,"message":{
  "content":"",
  "reasoning_content":"1.  **Analyze the user's request:** The user explicitly asked me to reply with *just* the word \"hello\"...",
  "role":"assistant"
}}],
"usage":{
  "completion_tokens":30,
  "completion_tokens_details":{"reasoning_tokens":30},
  "prompt_tokens":10,
  "prompt_tokens_details":{"cached_tokens":0},
  "total_tokens":40
}}
```

```
$ cargo test -p cairn-providers --test wire_zai_test zai_live_smoke \
    -- --ignored --nocapture
live smoke text=Some("") usage=Some(Usage { prompt_tokens: 10, completion_tokens: 32,
  total_tokens: 42, cached_tokens: None })
test zai_live_smoke ... ok
```

## What we deliberately did NOT ship

- **`cache_control` message hints.** Z.ai's coding endpoint uses
  server-side automatic caching only. There is no Anthropic-style
  `cache_control: {"type": "ephemeral"}` API surface. If this changes
  upstream, we add it to `ZaiProvider` only (never to `openai_compat`).
- **`response_format` / structured output.** Not documented on the coding
  endpoint; omitted to avoid silent fallback to freeform text. If a user
  needs JSON mode, they can use `Backend::OpenAiCompatible` pointed at
  the general paas endpoint.
- **Parallel tool calls / reasoning_effort.** These are OpenAI-specific
  extensions not honoured by Z.ai per the docs survey 2026-04-24.

## Files

- Adapter: `crates/cairn-providers/src/wire/zai.rs`
- Builder integration: `crates/cairn-providers/src/builder.rs`
- Registry wiring (embeddings fallthrough): `crates/cairn-runtime/src/provider_registry.rs`
- Integration tests: `crates/cairn-providers/tests/wire_zai_test.rs`
- UI: `ui/src/pages/ProvidersPage.tsx` (Z.ai Coding + Z.ai General cards)
