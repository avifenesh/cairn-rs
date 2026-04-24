/**
 * TestHarnessPage — interactive API test harness for developers.
 *
 * Pre-built scenarios exercise the full cairn API surface.  Each scenario
 * is a sequence of typed steps with request/response capture and pass/fail
 * timing.  Steps share a context bag so later steps can reference IDs
 * created by earlier ones (e.g. session_id → create run → claim task).
 */

import { useState, useRef, useCallback, useMemo, useEffect } from "react";
import {
  FlaskConical, Play, CheckCircle2, XCircle, Loader2, ChevronDown,
  ChevronRight, Clock, Zap, AlertTriangle,
  RotateCcw,
} from "lucide-react";
import { clsx } from "clsx";
import { card as cardPreset } from "../lib/design-system";
import { defaultApi } from "../lib/api";
import { PAUSABLE_RUN_STATES, TERMINAL_RUN_STATES } from "../lib/runStateErrors";
import { useScope, type ProjectScope } from "../hooks/useScope";

// ── Types ─────────────────────────────────────────────────────────────────────

type StepStatus = "idle" | "running" | "pass" | "fail" | "skipped";

interface StepResult {
  status:     StepStatus;
  durationMs: number;
  request:    unknown;
  response:   unknown;
  error:      string | null;
}

interface StepDef {
  id:          string;
  label:       string;
  description: string;
  /**
   * Receives the shared context bag plus a `shouldAbort()` callback the
   * step can poll if it does any looping/waiting. Returns the response
   * payload to log in the step card.
   */
  run: (ctx: Record<string, unknown>, shouldAbort: () => boolean) => Promise<unknown>;
}

interface ScenarioDef {
  id:          string;
  label:       string;
  description: string;
  group:       string;
  steps:       StepDef[];
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeId(prefix: string): string {
  return `${prefix}_${Math.random().toString(36).slice(2, 9)}`;
}

function fmtMs(ms: number): string {
  if (ms < 1_000) return `${ms}ms`;
  return `${(ms / 1_000).toFixed(2)}s`;
}

/**
 * Poll `getRun` until `run.state` is in `PAUSABLE_RUN_STATES` or the
 * timeout elapses. Returns `{ state, waited_ms }` so the harness step
 * log shows how long it took to reach a pausable state. On timeout or
 * terminal state, throws an `Error` with an operator-readable message
 * rather than letting the downstream pause call surface a raw
 * state-machine error (issue #257).
 *
 * `shouldAbort` is polled before each network call and before each sleep
 * so clicking Stop/Reset mid-poll tears the helper down within one
 * `pollIntervalMs` — no late state updates leak after abort.
 */
async function waitForPausableState(
  runId: string,
  shouldAbort: () => boolean = () => false,
  timeoutMs = 10_000,
  pollIntervalMs = 250,
): Promise<{ state: string; waited_ms: number }> {
  const started = Date.now();
  let last = "unknown";
  while (Date.now() - started < timeoutMs) {
    if (shouldAbort()) {
      throw new Error("aborted");
    }
    const r = await defaultApi.getRun(runId);
    last = r.state;
    if (PAUSABLE_RUN_STATES.has(r.state)) {
      return { state: r.state, waited_ms: Date.now() - started };
    }
    // Terminal states will never reach pausable — fail fast.
    if (TERMINAL_RUN_STATES.has(r.state)) {
      throw new Error(`Run reached terminal state '${r.state}' before becoming pausable`);
    }
    // Interruptible sleep: resolve early on abort so Stop tears down
    // within one poll interval instead of sitting on a dead setTimeout.
    await new Promise<void>(res => {
      const t = setInterval(() => {
        if (shouldAbort()) {
          clearInterval(t);
          res();
        }
      }, 25);
      setTimeout(() => { clearInterval(t); res(); }, pollIntervalMs);
    });
  }
  throw new Error(`Run did not reach pausable state in ${Math.round(timeoutMs / 1000)}s (last state: ${last})`);
}

// ── Scenario definitions ──────────────────────────────────────────────────────

function buildScenarios(scope: ProjectScope): ScenarioDef[] {
  return [

  // ── 1. Full session/run/task lifecycle ──────────────────────────────────────
  {
    id:          "lifecycle",
    label:       "Session → Run → Task Lifecycle",
    description: "Creates a session, starts a run, creates a task by claiming it, then releases the lease and verifies state.",
    group:       "Core",
    steps: [
      {
        id: "health",
        label: "Health probe",
        description: "GET /health — server must respond 200 ok:true",
        run: async () => {
          const r = await defaultApi.getHealth();
          const healthy = r.ok === true || r.status === 'healthy';
          if (!healthy) throw new Error(`ok=false status=${r.status}`);
          return r;
        },
      },
      {
        id: "create_session",
        label: "Create session",
        description: "POST /v1/sessions",
        run: async (ctx) => {
          const sessionId = makeId("sess");
          ctx["session_id"] = sessionId;
          const r = await defaultApi.createSession({
            tenant_id:    scope.tenant_id,
            workspace_id: scope.workspace_id,
            project_id:   scope.project_id,
            session_id:   sessionId,
          });
          if (r.session_id !== sessionId) throw new Error("session_id mismatch");
          if (r.state !== "open") throw new Error(`unexpected state: ${r.state}`);
          return r;
        },
      },
      {
        id: "create_run",
        label: "Create run",
        description: "POST /v1/runs",
        run: async (ctx) => {
          const runId = makeId("run");
          ctx["run_id"] = runId;
          const r = await defaultApi.createRun({
            tenant_id:    scope.tenant_id,
            workspace_id: scope.workspace_id,
            project_id:   scope.project_id,
            session_id:   String(ctx["session_id"]),
            run_id:       runId,
          });
          if (r.run_id !== runId) throw new Error("run_id mismatch");
          return r;
        },
      },
      {
        id: "verify_run_list",
        label: "Verify run in list",
        description: "GET /v1/runs — new run must appear",
        run: async (ctx) => {
          const runs = await defaultApi.getRuns({ limit: 100 });
          const found = runs.find(r => r.run_id === ctx["run_id"]);
          if (!found) throw new Error(`run ${ctx["run_id"]} not found in list`);
          return { found: true, state: found.state };
        },
      },
      {
        id: "wait_pausable",
        label: "Wait for pausable state",
        description: "Poll GET /v1/runs/:id until state is pausable (10s timeout)",
        run: async (ctx, shouldAbort) => {
          // Bug #257 — the harness used to call pause immediately after
          // create_run. Depending on scheduler timing the run could be
          // in an intermediate state (e.g. `partial_fence_triple`) that
          // rejects the pause transition with a raw state-machine error.
          // Poll until the run is in a pausable state or fail with a
          // human-readable message. Thread `shouldAbort` so Stop/Reset
          // from the harness UI tears the poll down mid-flight.
          return waitForPausableState(String(ctx["run_id"]), shouldAbort, 10_000);
        },
      },
      {
        id: "pause_run",
        label: "Pause run",
        description: "POST /v1/runs/:id/pause",
        run: async (ctx) => {
          const r = await defaultApi.pauseRun(String(ctx["run_id"]), { detail: "harness test pause" });
          if (r.state !== "paused") throw new Error(`expected paused, got ${r.state}`);
          ctx["run_version_paused"] = r.version;
          return r;
        },
      },
      {
        id: "resume_run",
        label: "Resume run",
        description: "POST /v1/runs/:id/resume",
        run: async (ctx) => {
          const r = await defaultApi.resumeRun(String(ctx["run_id"]));
          if (r.state !== "running") throw new Error(`expected running, got ${r.state}`);
          if (Number(r.version) <= Number(ctx["run_version_paused"])) {
            throw new Error("version should increment after resume");
          }
          return r;
        },
      },
      {
        id: "check_stats",
        label: "Check stats",
        description: "GET /v1/stats — active_runs must be ≥ 1",
        run: async () => {
          const s = await defaultApi.getStats();
          if (s.total_runs < 1) throw new Error("total_runs should be ≥ 1");
          return s;
        },
      },
    ],
  },

  // ── 2. Server health suite ──────────────────────────────────────────────────
  {
    id:          "health_suite",
    label:       "Server Health Suite",
    description: "Probes every health and status endpoint to verify the server is fully operational.",
    group:       "Diagnostics",
    steps: [
      {
        id: "health",
        label: "Liveness probe",
        description: "GET /health",
        run: async () => {
          const r = await defaultApi.getHealth();
          if (!(r.ok === true || r.status === 'healthy')) throw new Error("ok=false");
          return r;
        },
      },
      {
        id: "status",
        label: "Runtime status",
        description: "GET /v1/status",
        run: async () => {
          const r = await defaultApi.getStatus();
          if (r.status !== 'ok') throw new Error(`status=${r.status}`);
          return r;
        },
      },
      {
        id: "detailed_health",
        label: "Detailed health",
        description: "GET /v1/health/detailed",
        run: async () => {
          const r = await defaultApi.getDetailedHealth();
          if (r.status === "unhealthy") throw new Error(`unhealthy: ${JSON.stringify(r.checks)}`);
          return r;
        },
      },
      {
        id: "dashboard",
        label: "Dashboard data",
        description: "GET /v1/dashboard",
        run: async () => {
          const r = await defaultApi.getDashboard();
          if (!r.system_healthy) throw new Error("system_healthy=false");
          return r;
        },
      },
      {
        id: "stats",
        label: "System stats",
        description: "GET /v1/stats",
        run: async () => defaultApi.getStats(),
      },
      {
        id: "metrics",
        label: "Metrics endpoint",
        description: "GET /v1/metrics",
        run: async () => defaultApi.getMetrics(),
      },
    ],
  },

  // ── 3. Data read suite ──────────────────────────────────────────────────────
  {
    id:          "read_suite",
    label:       "Data Read Suite",
    description: "Exercises all major read endpoints and verifies they return valid JSON arrays.",
    group:       "Diagnostics",
    steps: [
      {
        id: "list_sessions",
        label: "List sessions",
        description: "GET /v1/sessions",
        run: async () => {
          const r = await defaultApi.getSessions({ limit: 10 });
          if (!Array.isArray(r)) throw new Error("expected array");
          return { count: r.length };
        },
      },
      {
        id: "list_runs",
        label: "List runs",
        description: "GET /v1/runs",
        run: async () => {
          const r = await defaultApi.getRuns({ limit: 10 });
          if (!Array.isArray(r)) throw new Error("expected array");
          return { count: r.length };
        },
      },
      {
        id: "list_tasks",
        label: "List tasks",
        description: "GET /v1/tasks",
        run: async () => {
          const r = await defaultApi.getAllTasks({ limit: 10 });
          if (!Array.isArray(r)) throw new Error("expected array");
          return { count: r.length };
        },
      },
      {
        id: "list_approvals",
        label: "Pending approvals",
        description: "GET /v1/approvals/pending",
        run: async () => {
          const r = await defaultApi.getPendingApprovals();
          if (!Array.isArray(r)) throw new Error("expected array");
          return { count: r.length };
        },
      },
      {
        id: "event_log",
        label: "Event log",
        description: "GET /v1/events/recent?limit=5",
        run: async () => {
          const r = await defaultApi.getRecentEvents(5);
          if (!Array.isArray(r)) throw new Error("expected array of recent events");
          // Assert shape when events exist — each must have a non-empty event_type.
          for (const ev of r) {
            if (typeof ev.event_type !== "string" || ev.event_type.length === 0) {
              throw new Error(`event missing event_type: ${JSON.stringify(ev)}`);
            }
          }
          return { count: r.length, sample: r[0] ?? null };
        },
      },
      {
        id: "costs",
        label: "Cost summary",
        description: "GET /v1/costs",
        run: async () => {
          const r = await defaultApi.getCosts();
          if (!Array.isArray(r.items)) throw new Error("missing items array");
          return r;
        },
      },
    ],
  },
  ];
}

// ── Step component ────────────────────────────────────────────────────────────

function StepRow({
  step, result, index,
}: {
  step:   StepDef;
  result: StepResult | null;
  index:  number;
}) {
  const [expanded, setExpanded] = useState(false);
  const s = result?.status ?? "idle";

  const icon = {
    idle:    <span className="w-4 h-4 rounded-full border-2 border-gray-200 dark:border-zinc-700 shrink-0" />,
    running: <Loader2 size={16} className="text-indigo-400 animate-spin shrink-0" />,
    pass:    <CheckCircle2 size={16} className="text-emerald-400 shrink-0" />,
    fail:    <XCircle size={16} className="text-red-400 shrink-0" />,
    skipped: <AlertTriangle size={16} className="text-gray-400 dark:text-zinc-600 shrink-0" />,
  }[s];

  const rowBg = {
    idle:    "",
    running: "bg-indigo-950/20",
    pass:    "bg-emerald-950/10",
    fail:    "bg-red-950/20",
    skipped: "bg-gray-50/30 dark:bg-zinc-900/30",
  }[s];

  return (
    <div className={clsx("rounded-lg border overflow-hidden transition-colors",
      s === "fail"    ? "border-red-900/50"     :
      s === "pass"    ? "border-emerald-900/40"  :
      s === "running" ? "border-indigo-800/40"   :
                        "border-gray-200 dark:border-zinc-800",
      rowBg,
    )}>
      {/* Header */}
      <div
        className="flex items-center gap-3 px-3 py-2.5 cursor-pointer hover:bg-white/[0.02] transition-colors select-none"
        onClick={() => result && setExpanded(v => !v)}
      >
        <span className="text-[10px] font-mono text-gray-300 dark:text-zinc-600 w-5 text-right shrink-0">{index + 1}</span>
        {icon}
        <div className="flex-1 min-w-0">
          <p className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">{step.label}</p>
          <p className="text-[11px] text-gray-400 dark:text-zinc-600 truncate">{step.description}</p>
        </div>
        <div className="flex items-center gap-3 shrink-0">
          {result && result.status !== "idle" && result.status !== "running" && (
            <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-600 tabular-nums">
              {fmtMs(result.durationMs)}
            </span>
          )}
          {result?.status === "fail" && result.error && (
            <span className="text-[11px] text-red-400 font-mono max-w-[200px] truncate" title={result.error}>
              {result.error}
            </span>
          )}
          {result && (
            expanded
              ? <ChevronDown  size={12} className="text-gray-400 dark:text-zinc-600" />
              : <ChevronRight size={12} className="text-gray-400 dark:text-zinc-600" />
          )}
        </div>
      </div>

      {/* Expanded request/response */}
      {expanded && result && (result.request !== undefined || result.response !== undefined) && (
        <div className="border-t border-gray-200 dark:border-zinc-800 grid grid-cols-2 divide-x divide-gray-200 dark:divide-zinc-800">
          {[
            { label: "Request",  data: result.request  },
            { label: "Response", data: result.response },
          ].map(({ label, data }) => (
            <div key={label} className="p-3 bg-white dark:bg-zinc-950/40">
              <p className="text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">{label}</p>
              <pre className="text-[11px] font-mono text-gray-500 dark:text-zinc-400 overflow-x-auto max-h-40 leading-relaxed whitespace-pre-wrap break-words">
                {data === undefined || data === null
                  ? <span className="text-gray-300 dark:text-zinc-600">—</span>
                  : JSON.stringify(data, null, 2)}
              </pre>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Scenario card ─────────────────────────────────────────────────────────────

type ScenarioResults = Map<string, StepResult>;

function ScenarioCard({
  scenario,
  runNonce = 0,
  onComplete,
}: {
  scenario:    ScenarioDef;
  /** When this value changes to a non-zero value, the card runs its scenario. */
  runNonce?:   number;
  /** Called once the triggered run finishes, with per-scenario pass/fail + timing. */
  onComplete?: (result: { pass: boolean; ms: number }) => void;
}) {
  const [results,  setResults]  = useState<ScenarioResults>(new Map());
  const [running,  setRunning]  = useState(false);
  const [expanded, setExpanded] = useState(false);
  const abortRef = useRef(false);

  const totalSteps   = scenario.steps.length;
  const passCount    = [...results.values()].filter(r => r.status === "pass").length;
  const failCount    = [...results.values()].filter(r => r.status === "fail").length;
  const runningCount = [...results.values()].filter(r => r.status === "running").length;
  const totalMs      = [...results.values()].reduce((s, r) => s + r.durationMs, 0);

  const overallStatus: StepStatus =
    running                                             ? "running"  :
    results.size === 0                                  ? "idle"     :
    failCount > 0                                       ? "fail"     :
    passCount + [...results.values()].filter(r => r.status === "skipped").length === totalSteps
                                                        ? "pass"     :
                                                          "idle";

  const runScenario = useCallback(async (): Promise<{ pass: boolean; ms: number }> => {
    if (running) return { pass: false, ms: 0 };
    abortRef.current = false;
    setRunning(true);
    setExpanded(true);
    setResults(new Map());

    const ctx: Record<string, unknown> = {};
    const newResults = new Map<string, StepResult>();
    const suiteT0 = performance.now();
    let suitePass = true;

    for (const step of scenario.steps) {
      if (abortRef.current) {
        newResults.set(step.id, { status: "skipped", durationMs: 0, request: null, response: null, error: "Aborted" });
        setResults(new Map(newResults));
        continue;
      }

      // Mark as running
      newResults.set(step.id, { status: "running", durationMs: 0, request: null, response: null, error: null });
      setResults(new Map(newResults));

      const t0 = performance.now();
      let response: unknown = null;
      let error:    string | null = null;
      let status:   StepStatus = "pass";

      try {
        response = await step.run(ctx, () => abortRef.current);
      } catch (e: unknown) {
        status = "fail";
        error  = e instanceof Error ? e.message : String(e);
        suitePass = false;
        // Abort remaining steps on first failure
        abortRef.current = true;
      }

      const durationMs = Math.round(performance.now() - t0);
      newResults.set(step.id, {
        status,
        durationMs,
        request:  null, // request captured inside step.run if needed
        response: status === "pass" ? response : null,
        error,
      });
      setResults(new Map(newResults));
    }

    setRunning(false);
    return { pass: suitePass, ms: Math.round(performance.now() - suiteT0) };
  }, [running, scenario.steps]);

  // Drive scenario from parent's "Run All" by watching nonce changes.
  // Nonce 0 = no trigger yet; any increment fires the scenario once.
  useEffect(() => {
    if (runNonce === 0) return;
    let cancelled = false;
    (async () => {
      const result = await runScenario();
      if (!cancelled) onComplete?.(result);
    })();
    return () => { cancelled = true; };
    // We intentionally key only on runNonce — runScenario/onComplete change by identity
    // every render and would re-fire the effect otherwise.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runNonce]);

  function resetScenario() {
    abortRef.current = true;
    setRunning(false);
    setResults(new Map());
  }

  const statusColor = {
    idle:    "text-gray-400 dark:text-zinc-500",
    running: "text-indigo-400",
    pass:    "text-emerald-400",
    fail:    "text-red-400",
    skipped: "text-gray-400 dark:text-zinc-600",
  }[overallStatus];

  const borderColor = {
    idle:    "border-gray-200 dark:border-zinc-800",
    running: "border-indigo-800/60",
    pass:    "border-emerald-800/40",
    fail:    "border-red-800/50",
    skipped: "border-gray-200 dark:border-zinc-800",
  }[overallStatus];

  return (
    <div className={clsx("bg-gray-50 dark:bg-zinc-900 rounded-xl border overflow-hidden", borderColor)}>
      {/* Card header */}
      <div className="flex items-start gap-3 px-4 py-3">
        <div className={clsx(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border mt-0.5",
          overallStatus === "pass"    ? "bg-emerald-950/50 border-emerald-800/40" :
          overallStatus === "fail"    ? "bg-red-950/50 border-red-800/40"         :
          overallStatus === "running" ? "bg-indigo-950/50 border-indigo-800/40"   :
                                        "bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700",
        )}>
          {overallStatus === "running"
            ? <Loader2 size={14} className="text-indigo-400 animate-spin" />
            : overallStatus === "pass"
            ? <CheckCircle2 size={14} className="text-emerald-400" />
            : overallStatus === "fail"
            ? <XCircle size={14} className="text-red-400" />
            : <FlaskConical size={14} className="text-gray-400 dark:text-zinc-500" />
          }
        </div>

        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="text-[11px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider font-medium">
              {scenario.group}
            </span>
            <span className="text-[10px] text-gray-300 dark:text-zinc-600">·</span>
            <p className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">{scenario.label}</p>
          </div>
          <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-0.5">{scenario.description}</p>

          {/* Progress summary */}
          {results.size > 0 && (
            <div className="flex items-center gap-3 mt-2">
              <span className={clsx("text-[11px] font-medium", statusColor)}>
                {overallStatus === "pass" ? "All passed" :
                 overallStatus === "fail" ? `${failCount} failed` :
                 overallStatus === "running" ? "Running…" : ""}
              </span>
              <span className="text-[11px] text-gray-400 dark:text-zinc-600">
                {passCount}/{totalSteps} steps
              </span>
              {totalMs > 0 && (
                <span className="flex items-center gap-1 text-[11px] text-gray-300 dark:text-zinc-600">
                  <Clock size={10} />
                  {fmtMs(totalMs)}
                </span>
              )}
              {/* Mini progress bar */}
              <div className="flex-1 h-1 rounded-full bg-gray-100 dark:bg-zinc-800 overflow-hidden max-w-32">
                {totalSteps > 0 && (
                  <div
                    className={clsx(
                      "h-full rounded-full transition-all",
                      overallStatus === "fail" ? "bg-red-500" :
                      overallStatus === "pass" ? "bg-emerald-500" : "bg-indigo-500",
                    )}
                    style={{ width: `${((passCount + failCount) / totalSteps) * 100}%` }}
                  />
                )}
              </div>
            </div>
          )}
        </div>

        {/* Actions */}
        <div className="flex items-center gap-2 shrink-0">
          {results.size > 0 && (
            <button
              onClick={resetScenario}
              title="Reset"
              className="flex items-center gap-1 px-2 py-1 rounded border border-gray-200 dark:border-zinc-700 bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-500 text-[11px] hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600 transition-colors"
            >
              <RotateCcw size={11} />
            </button>
          )}
          <button
            onClick={runningCount > 0 ? () => { abortRef.current = true; } : runScenario}
            disabled={running && runningCount === 0}
            className={clsx(
              "flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-[12px] font-medium transition-colors",
              running
                ? "bg-red-900/30 border border-red-800/40 text-red-400 hover:bg-red-900/50"
                : "bg-indigo-600 hover:bg-indigo-500 text-white disabled:opacity-40",
            )}
          >
            {running
              ? <><Loader2 size={12} className="animate-spin" /> Stop</>
              : <><Play size={11} /> Run</>
            }
          </button>
          <button
            onClick={() => setExpanded(v => !v)}
            className="p-1.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
          >
            {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
          </button>
        </div>
      </div>

      {/* Step list */}
      {expanded && (
        <div className="border-t border-gray-200 dark:border-zinc-800 px-4 py-3 space-y-2 bg-white dark:bg-zinc-950/30">
          {scenario.steps.map((step, i) => (
            <StepRow
              key={step.id}
              step={step}
              result={results.get(step.id) ?? null}
              index={i}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ── Run-all summary banner ────────────────────────────────────────────────────

interface SuiteResult {
  scenario: string;
  pass:     boolean;
  ms:       number;
}

function SuiteSummary({ results, onClear }: {
  results: SuiteResult[];
  onClear: () => void;
}) {
  if (results.length === 0) return null;
  const passed = results.filter(r => r.pass).length;
  const failed = results.length - passed;
  const totalMs = results.reduce((s, r) => s + r.ms, 0);
  const allPass = failed === 0;

  return (
    <div className={clsx(
      "flex items-center gap-4 rounded-xl border px-5 py-3",
      allPass ? "border-emerald-800/40 bg-emerald-950/20" : "border-red-800/50 bg-red-950/20",
    )}>
      {allPass
        ? <CheckCircle2 size={18} className="text-emerald-400 shrink-0" />
        : <XCircle      size={18} className="text-red-400 shrink-0" />
      }
      <div className="flex-1">
        <p className={clsx("text-[13px] font-semibold", allPass ? "text-emerald-300" : "text-red-300")}>
          {allPass ? `All ${passed} scenarios passed` : `${failed} of ${results.length} scenarios failed`}
        </p>
        <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">
          {fmtMs(totalMs)} total · {results.map(r => `${r.scenario}: ${r.pass ? "✓" : "✗"}`).join(" · ")}
        </p>
      </div>
      <button
        onClick={onClear}
        className="text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors"
      >
        Clear
      </button>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function TestHarnessPage() {
  const [scope] = useScope();
  const [suiteResults,  setSuiteResults]  = useState<SuiteResult[]>([]);
  const [runningAll,    setRunningAll]    = useState(false);
  const [groupFilter,   setGroupFilter]   = useState<string>("All");
  /**
   * Per-scenario run nonce. "Run All" bumps the nonce for each visible
   * scenario in sequence; each card watches its own nonce and actually
   * invokes its internal runScenario, so step cards reflect live progress.
   * A nonce of 0 means "never triggered".
   */
  const [runNonces, setRunNonces] = useState<Record<string, number>>({});
  /** Resolver for the currently-awaited scenario run, keyed by scenario id. */
  const pendingResolverRef = useRef<{ id: string; resolve: (r: { pass: boolean; ms: number }) => void } | null>(null);

  const scenarios = useMemo(() => buildScenarios(scope), [scope]);
  const groups = ["All", ...Array.from(new Set(scenarios.map(s => s.group)))];
  const visible = groupFilter === "All" ? scenarios : scenarios.filter(s => s.group === groupFilter);

  const handleScenarioComplete = useCallback((scenarioId: string, result: { pass: boolean; ms: number }) => {
    const pending = pendingResolverRef.current;
    if (pending && pending.id === scenarioId) {
      pendingResolverRef.current = null;
      pending.resolve(result);
    }
  }, []);

  async function handleRunAll() {
    if (runningAll) return;
    setRunningAll(true);
    setSuiteResults([]);

    const results: SuiteResult[] = [];

    for (const scenario of visible) {
      // Wait for this scenario's card to finish before starting the next,
      // so shared-server resources aren't hammered in parallel.
      const done = new Promise<{ pass: boolean; ms: number }>(resolve => {
        pendingResolverRef.current = { id: scenario.id, resolve };
      });
      setRunNonces(prev => ({ ...prev, [scenario.id]: (prev[scenario.id] ?? 0) + 1 }));
      const { pass, ms } = await done;
      results.push({ scenario: scenario.label.slice(0, 20), pass, ms });
      setSuiteResults([...results]);
    }

    setRunningAll(false);
  }

  return (
    <div className="h-full overflow-y-auto bg-white dark:bg-zinc-950">
      <div className="p-6 space-y-5">

        {/* Toolbar — matches established pattern */}
        <div className="flex items-center justify-between">
          <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">Test Harness</p>
          <div className="flex items-center gap-2">
            {suiteResults.length > 0 && (
              <button onClick={() => setSuiteResults([])}
                className="flex items-center gap-1.5 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-2.5 py-1.5 text-[11px] text-gray-400 dark:text-zinc-500 hover:bg-white/5 transition-colors">
                <RotateCcw size={11} /> Reset
              </button>
            )}
            <button onClick={handleRunAll} disabled={runningAll}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 text-white text-[11px] font-medium disabled:opacity-40 transition-colors">
              {runningAll ? <><Loader2 size={11} className="animate-spin" /> Running…</> : <><Zap size={11} /> Run All Scenarios</>}
            </button>
          </div>
        </div>

        {/* How it works — clear instructions */}
        <div className={cardPreset.base}>
          <p className="text-[12px] font-medium text-gray-700 dark:text-zinc-300 mb-1">How the Test Harness works</p>
          <p className="text-[11px] text-gray-400 dark:text-zinc-500 leading-relaxed">
            Each scenario runs a sequence of API calls against the live server. Click <strong>Run</strong> on a scenario to test it,
            or <strong>Run All Scenarios</strong> to exercise the full API surface. Results show pass/fail per step with timing.
            Green = passed, red = failed, gray = not run yet.
          </p>
        </div>

        {/* Suite summary — shown after Run All */}
        <SuiteSummary results={suiteResults} onClear={() => setSuiteResults([])} />

        {/* Stat cards — scenario counts by result */}
        {suiteResults.length > 0 && (
          <div className="grid grid-cols-3 gap-3">
            <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 border-l-2 border-l-emerald-500 rounded-lg p-4">
              <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-2">Passed</p>
              <p className="text-2xl font-semibold tabular-nums text-emerald-400">{suiteResults.filter(r => r.pass).length}</p>
            </div>
            <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 border-l-2 border-l-red-500 rounded-lg p-4">
              <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-2">Failed</p>
              <p className="text-2xl font-semibold tabular-nums text-red-400">{suiteResults.filter(r => !r.pass).length}</p>
            </div>
            <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 border-l-2 border-l-zinc-700 rounded-lg p-4">
              <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-2">Total</p>
              <p className="text-2xl font-semibold tabular-nums text-gray-900 dark:text-zinc-100">{suiteResults.length}</p>
            </div>
          </div>
        )}

        {/* Group filter */}
        <div className="flex items-center gap-1 border-b border-gray-200 dark:border-zinc-800">
          {groups.map(g => (
            <button key={g} onClick={() => setGroupFilter(g)}
              className={clsx(
                "px-3 py-1.5 text-[12px] font-medium border-b-2 -mb-px transition-colors",
                groupFilter === g
                  ? "text-gray-900 dark:text-zinc-100 border-indigo-500"
                  : "text-gray-400 dark:text-zinc-500 border-transparent hover:text-gray-700 dark:hover:text-zinc-300",
              )}>
              {g} {g !== "All" ? "" : `(${visible.length})`}
            </button>
          ))}
        </div>

        {/* Warning */}
        <div className="flex items-start gap-2.5 rounded-lg border border-amber-500/20 bg-amber-500/5 px-4 py-3">
          <AlertTriangle size={13} className="text-amber-400 shrink-0 mt-0.5" />
          <p className="text-[11px] text-gray-500 dark:text-zinc-400">
            Scenarios create real resources (sessions, runs, tasks) in the connected server.
            Clear test data by restarting the server or using the admin API.
          </p>
        </div>

        {/* Scenario cards */}
        {visible.map(scenario => (
          <ScenarioCard
            key={scenario.id}
            scenario={scenario}
            runNonce={runNonces[scenario.id] ?? 0}
            onComplete={r => handleScenarioComplete(scenario.id, r)}
          />
        ))}
      </div>
    </div>
  );
}

export default TestHarnessPage;
