# cairn-providers integration & contract tests

This directory holds three classes of tests:

| File                         | Kind             | Network? | Purpose                                                              |
| ---------------------------- | ---------------- | -------- | -------------------------------------------------------------------- |
| `wire_test.rs`               | Unit / mock      | No       | Exercises OpenAI-compatible wire parser against hand-crafted bodies. |
| `builder_test.rs`            | Unit             | No       | Validates `ProviderBuilder` → `ProviderConfig` resolution.           |
| `bridge_test.rs`             | Unit             | No       | Validates the chat/completion bridge adapters.                       |
| `contract_openrouter.rs`     | Real-provider    | Opt-in   | Asserts OpenRouter's response shape still matches what cairn parses. |
| `fixtures/`                  | Test data        | —        | Checked-in response bodies captured from real providers.             |

## Contract tests — what and why

Cairn talks to a dozen LLM backends through a single OpenAI-compatible wire
parser. If a provider drifts — renames a field, changes `finish_reason`
semantics, drops `usage`, etc. — our hand-crafted mocks keep passing while
production breaks. Contract tests close that gap by **replaying real
captured response bodies through the real parser**.

Contract tests are fixture-replay by default:

- **CI / default run:** no network, no keys. The fixture in `fixtures/` is
  served from a local `httpmock` server and piped through `OpenAiCompat`.
  Test fails if field extraction breaks.
- **Opt-in refresh run:** hits the real provider, overwrites the fixture,
  which is then committed to git.

## Refreshing a fixture

```bash
OPENROUTER_API_KEY=<your_key> CAIRN_TEST_REFRESH_FIXTURES=1 \
    cargo test -p cairn-providers --test contract_openrouter -- --nocapture
```

You should see a `[REFRESHED] ...` line. Commit the updated fixture:

```bash
git add crates/cairn-providers/tests/fixtures/openrouter_chat_completion.json
git commit -m "test(providers): refresh OpenRouter contract fixture"
```

### Refresh cadence

- **On test failure** due to shape drift — primary trigger.
- **Quarterly** otherwise. The `openrouter_fixture_freshness_warning`
  test emits a `[WARN]` line (not a failure) once the fixture is older
  than 90 days.
- **On provider deprecation.** If OpenRouter retires the model named in
  `REFRESH_MODEL` (check <https://openrouter.ai/models?q=free>), pick
  another `:free` model, update the constant in `contract_openrouter.rs`,
  and refresh.

## Why OpenRouter first

- **Cheap** — free-tier models cost $0 per request.
- **OpenAI-compatible** — exercises the shared parser used by OpenAI,
  Groq, DeepSeek, xAI, MiniMax, Azure OpenAI, and Bedrock's OpenAI gateway.
  One contract test covers the dominant wire shape across eight-plus
  backends.
- **Broad model selection** — operator can pick any `:free` model.
- **Acceptable to fail noisily** — free routes are the most likely to
  churn, giving us early warning before paid providers break.

Future PRs will add contract tests for the non-OpenAI-shaped backends:

- Anthropic native `/v1/messages`
- Bedrock Converse API
- Vertex AI (Google native)
- Ollama (local, but still worth pinning)

## Adding a new provider contract test

1. Create `tests/contract_<provider>.rs`.
2. Create `tests/fixtures/<provider>_chat_completion.json` (seed from
   provider docs if you don't have a key yet, note the source in the
   PR body).
3. Mirror the structure of `contract_openrouter.rs`:
   - Default mode replays the fixture through a local mock server.
   - Refresh mode (`CAIRN_TEST_REFRESH_FIXTURES=1` + `<PROVIDER>_API_KEY`)
     captures a fresh response body.
4. Pin **every field the orchestrator reads**, nothing more. Optional
   fields providers may add or omit are not part of the contract.
5. Add the new test file to the table at the top of this README.

## What these tests are not

- **Not nightly CI wiring.** Refresh must be run manually (for now).
- **Not soak or chaos tests.** Those follow in separate PRs.
- **Not a replacement for `wire_test.rs`.** The wire tests cover edge
  cases (malformed SSE, empty tool calls, etc.); contract tests cover the
  happy path as captured from the real provider.
