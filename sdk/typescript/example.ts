/**
 * example.ts — CairnClient usage examples
 *
 * Run against a local cairn-rs server:
 *   npx tsx sdk/typescript/example.ts
 *
 * Requires:
 *   - cairn-rs running on http://localhost:3000
 *   - CAIRN_TOKEN env var (default: cairn-demo-token)
 */

import { CairnClient, CairnApiError } from "./cairn-client.js";

// Portable env-var access — works in Node ≥18, Deno, and Bun.
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const _g = globalThis as any;
const _env: Record<string, string | undefined> =
  _g.process?.env ?? _g.Deno?.env.toObject() ?? {};

const BASE  = _env["CAIRN_URL"]   ?? "http://localhost:3000";
const TOKEN = _env["CAIRN_TOKEN"] ?? "cairn-demo-token";

const cairn = new CairnClient(BASE, TOKEN);

// ── Helper ───────────────────────────────────────────────────────────────────

function section(title: string) {
  console.log(`\n${"─".repeat(60)}`);
  console.log(`  ${title}`);
  console.log("─".repeat(60));
}

function row(label: string, value: unknown) {
  const v = typeof value === "object" ? JSON.stringify(value) : String(value);
  console.log(`  ${label.padEnd(22)} ${v}`);
}

async function main() {

// ── 1. Health & status ────────────────────────────────────────────────────────

section("1 · Health & Status");

const health = await cairn.health();
row("health.ok",         health.ok);

const status = await cairn.status();
row("status.runtime_ok", status.runtime_ok);
row("status.store_ok",   status.store_ok);
row("status.uptime_secs", status.uptime_secs);

const stats = await cairn.stats();
row("stats.total_runs",       stats.total_runs);
row("stats.total_tasks",      stats.total_tasks);
row("stats.active_runs",      stats.active_runs);
row("stats.pending_approvals", stats.pending_approvals);

const dash = await cairn.dashboard();
row("dashboard.active_runs",       dash.active_runs);
row("dashboard.active_tasks",      dash.active_tasks);
row("dashboard.pending_approvals", dash.pending_approvals);
row("dashboard.system_healthy",    dash.system_healthy);

// ── 2. Session lifecycle ──────────────────────────────────────────────────────

section("2 · Session lifecycle");

const TS = Date.now();
const PROJECT = {
  tenant_id:    "default_tenant",
  workspace_id: "default_workspace",
  project_id:   "demo_project",
};

const session = await cairn.createSession({
  ...PROJECT,
  session_id: `example_sess_${TS}`,
});
row("session.session_id", session.session_id);
row("session.state",      session.state);

const { items: sessions, pagination: sessPag } = await cairn.listSessions(10);
row("listSessions.count",      sessions.length);
row("listSessions.totalCount", sessPag.totalCount);

// ── 3. Run lifecycle ──────────────────────────────────────────────────────────

section("3 · Run lifecycle");

const run = await cairn.createRun({
  ...PROJECT,
  session_id: session.session_id,
  run_id:     `example_run_${TS}`,
});
row("run.run_id", run.run_id);
row("run.state",  run.state);

const fetched = await cairn.getRun(run.run_id);
row("getRun.run_id", fetched.run_id);
row("getRun.state",  fetched.state);

const { items: runs, pagination: runPag } = await cairn.listRuns(10);
row("listRuns.count",      runs.length);
row("listRuns.totalCount", runPag.totalCount);
row("listRuns.page",       runPag.page);

// ── 4. Tasks ──────────────────────────────────────────────────────────────────

section("4 · Tasks");

const { items: tasks, pagination: taskPag } = await cairn.listTasks(25);
row("listTasks.count",      tasks.length);
row("listTasks.totalCount", taskPag.totalCount);

// Show state distribution
const byState = tasks.reduce<Record<string, number>>((acc, t) => {
  acc[t.state] = (acc[t.state] ?? 0) + 1;
  return acc;
}, {});
for (const [state, count] of Object.entries(byState)) {
  row(`  state=${state}`, count);
}

// ── 5. Approvals ──────────────────────────────────────────────────────────────

section("5 · Approvals");

const { items: pending, pagination: apprPag } = await cairn.listPendingApprovals();
row("pending.count",      pending.length);
row("pending.totalCount", apprPag.totalCount);

for (const appr of pending.slice(0, 3)) {
  row(`  ${appr.approval_id}`, `run=${appr.run_id} decision=${appr.decision ?? "pending"}`);
}

// ── 6. LLM generation ─────────────────────────────────────────────────────────

section("6 · LLM generation (Ollama)");

let models: string[] = [];
try {
  models = await cairn.listModels();
  row("models.available", models.join(", ") || "(none)");
} catch {
  row("models", "Ollama not configured");
}

if (models.length > 0) {
  // Pick a non-embedding model.
  const genModel = models.find(m => !/embed|nomic/i.test(m)) ?? models[0];
  row("using model", genModel);

  try {
    const resp = await cairn.generate("In one sentence: what is an AI agent?", genModel);
    row("generate.response",  resp.response.slice(0, 100) + (resp.response.length > 100 ? "…" : ""));
    row("generate.latency_ms", resp.latency_ms);

    // Streaming
    let streamBuf = "  streamGenerate → ";
    let streamed = 0;
    for await (const token of cairn.streamGenerate("Say 'hello world'.", genModel)) {
      streamBuf += token;
      if (++streamed > 80) { streamBuf += "…"; break; }
    }
    console.log(streamBuf);
  } catch (err) {
    if (err instanceof CairnApiError) {
      row("llm error", `${err.status} ${err.message}`);
    } else {
      throw err;
    }
  }
} else {
  console.log("  (skipped — no models loaded)");
}

// ── 7. Events ──────────────────────────────────────────────────────────────────

section("7 · Events");

const recent = await cairn.recentEvents(5);
row("recentEvents.count", recent.length);
for (const e of recent) {
  row(`  pos ${String(e.seq).padStart(4)}`, e.event_type);
}

// ── 8. Traces ──────────────────────────────────────────────────────────────────

section("8 · LLM Traces");

const { items: traces, pagination: tracePag } = await cairn.listTraces(10);
row("traces.count",      traces.length);
row("traces.totalCount", tracePag.totalCount);
for (const t of traces.slice(0, 3)) {
  row(`  ${t.trace_id.slice(0, 20)}…`, `${t.model_id} latency=${t.latency_ms}ms tokens=${t.prompt_tokens}+${t.completion_tokens}`);
}

// ── 9. Admin snapshot ──────────────────────────────────────────────────────────

section("9 · Admin snapshot");

const snap = await cairn.snapshot();
row("snapshot.version",     snap.version);
row("snapshot.event_count", snap.event_count);
row("snapshot.created_at",  new Date(snap.created_at_ms).toISOString());

// ── 10. Webhook test ───────────────────────────────────────────────────────────

section("10 · Webhook test (localhost — expected failure)");

// Intentionally sending to an unreachable URL to demonstrate the error shape.
try {
  const wh = await cairn.testWebhook("http://localhost:9999/hook", "agent.run.completed");
  row("webhook.success",     wh.success);
  row("webhook.status_code", wh.status_code);
  row("webhook.latency_ms",  wh.latency_ms);
} catch (err) {
  if (err instanceof CairnApiError) {
    row("webhook error", `${err.status} ${err.message}`);
  } else {
    // Network failure to the target is returned as success:false, not an error.
    throw err;
  }
}

// ── 11. SSE (fire-and-forget demo) ─────────────────────────────────────────────

section("11 · SSE subscription (3-second sample)");

let eventCount = 0;
const unsub = cairn.subscribeEvents((msg) => {
  eventCount++;
  try {
    const parsed = JSON.parse(msg.data as string) as { event_type?: string };
    if (eventCount <= 3) row(`  event ${eventCount}`, parsed.event_type ?? "?");
  } catch { /* ignore */ }
});

await new Promise<void>(resolve => setTimeout(resolve, 3_000));
unsub();
row("events received", eventCount);

// ── Done ───────────────────────────────────────────────────────────────────────

section("Done");
console.log("  All examples completed.\n");

} // end main()

main().catch((err) => { console.error(err); _g.process?.exit?.(1); });
