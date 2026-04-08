/**
 * ProjectDashboardPage — stats, runs, tasks, and events scoped to one project.
 *
 * Route: #project/<projectId>
 * The projectId is used as the project_id query param on every API call.
 * tenant_id and workspace_id default to "default"; they can be overridden via
 * ?tenant=T&workspace=W appended to the hash.
 */

import { useState, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  ArrowLeft, RefreshCw, Play, ListChecks,
  CheckCircle2, AlertTriangle, Coins, Clock, Radio,
  ChevronRight, Users, Layers,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { EventLog } from "../components/EventLog";
import { StatCard } from "../components/StatCard";
import { MiniChart } from "../components/MiniChart";
import type { RunRecord, RunState, TaskRecord } from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtMicros(n: number): string {
  if (n === 0) return "$0.00";
  const u = n / 1_000_000;
  return u < 0.001 ? `$${(u * 1000).toFixed(3)}m` : u < 1 ? `$${u.toFixed(4)}` : `$${u.toFixed(2)}`;
}

function fmtAge(ms: number): string {
  const d = Date.now() - ms;
  if (d < 60_000) return `${Math.floor(d / 1_000)}s ago`;
  if (d < 3_600_000) return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)}h ago`;
  return `${Math.floor(d / 86_400_000)}d ago`;
}

function fmtDuration(startMs: number, endMs?: number): string {
  const ms = (endMs ?? Date.now()) - startMs;
  if (ms < 1_000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1_000).toFixed(1)}s`;
  return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1_000)}s`;
}

const shortId = (id: string) =>
  id.length > 20 ? `${id.slice(0, 9)}…${id.slice(-6)}` : id;

const ACTIVE_STATES = new Set<RunState>([
  "running", "pending", "paused", "waiting_approval", "waiting_dependency",
]);

const STATE_COLORS: Partial<Record<RunState, string>> = {
  running:            "text-blue-400",
  pending:            "text-gray-500 dark:text-zinc-400",
  paused:             "text-amber-400",
  waiting_approval:   "text-purple-400",
  waiting_dependency: "text-sky-400",
  completed:          "text-emerald-400",
  failed:             "text-red-400",
  canceled:           "text-gray-400 dark:text-zinc-500",
};

// ── Panel / section primitives (local — not importing from DashboardPage) ─────

function Panel({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={clsx("bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-xl p-4", className)}>
      {children}
    </div>
  );
}

function SectionLabel({ children }: { children: React.ReactNode }) {
  return (
    <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-3">
      {children}
    </p>
  );
}

// ── Active run row ────────────────────────────────────────────────────────────

function RunRow({ run }: { run: RunRecord }) {
  const isTerminal = ["completed", "failed", "canceled"].includes(run.state);
  return (
    <div
      className="flex items-center gap-3 py-2 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0 cursor-pointer hover:bg-white/[0.02] transition-colors -mx-4 px-4"
      onClick={() => { window.location.hash = `run/${run.run_id}`; }}
    >
      <Play size={10} className={clsx("shrink-0", STATE_COLORS[run.state] ?? "text-gray-400 dark:text-zinc-600")} />
      <span className="flex-1 font-mono text-[12px] text-gray-700 dark:text-zinc-300 truncate" title={run.run_id}>
        {shortId(run.run_id)}
      </span>
      <span className={clsx("text-[11px] font-medium capitalize shrink-0", STATE_COLORS[run.state] ?? "text-gray-400 dark:text-zinc-500")}>
        {run.state.replace(/_/g, " ")}
      </span>
      <span className="text-[11px] text-gray-400 dark:text-zinc-600 tabular-nums shrink-0">
        {fmtDuration(run.created_at, isTerminal ? run.updated_at : undefined)}
      </span>
      <ChevronRight size={11} className="text-gray-300 dark:text-zinc-700 shrink-0" />
    </div>
  );
}

// ── Task breakdown donut (CSS-only ring) ──────────────────────────────────────

function TaskRing({ tasks }: { tasks: TaskRecord[] }) {
  const counts = useMemo(() => {
    const m: Record<string, number> = {};
    for (const t of tasks) m[t.state] = (m[t.state] ?? 0) + 1;
    return m;
  }, [tasks]);

  const total = tasks.length;
  if (total === 0) return <p className="text-[12px] text-gray-400 dark:text-zinc-600 italic text-center py-4">No tasks</p>;

  const segments: { label: string; count: number; color: string }[] = [
    { label: "completed", count: counts["completed"] ?? 0,  color: "#10b981" },
    { label: "running",   count: counts["running"]   ?? 0,  color: "#3b82f6" },
    { label: "leased",    count: counts["leased"]    ?? 0,  color: "#6366f1" },
    { label: "queued",    count: counts["queued"]    ?? 0,  color: "#f59e0b" },
    { label: "failed",    count: counts["failed"]    ?? 0,  color: "#ef4444" },
    { label: "canceled",  count: counts["canceled"]  ?? 0,  color: "#52525b" },
  ].filter(s => s.count > 0);

  return (
    <div className="space-y-1.5">
      {segments.map(s => {
        const pct = (s.count / total) * 100;
        return (
          <div key={s.label} className="flex items-center gap-2">
            <span className="w-2 h-2 rounded-full shrink-0" style={{ backgroundColor: s.color }} />
            <span className="text-[11px] text-gray-500 dark:text-zinc-400 flex-1 capitalize">{s.label}</span>
            <div className="w-24 h-1.5 rounded-full bg-gray-100 dark:bg-zinc-800 overflow-hidden">
              <div className="h-full rounded-full" style={{ width: `${pct}%`, backgroundColor: s.color }} />
            </div>
            <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-500 tabular-nums w-6 text-right">
              {s.count}
            </span>
          </div>
        );
      })}
      <p className="text-[10px] text-gray-300 dark:text-zinc-700 text-right pt-1">{total} total</p>
    </div>
  );
}

// ── Scope selector ────────────────────────────────────────────────────────────

function ScopeSelector({
  tenantId, workspaceId,
  onTenantChange, onWorkspaceChange,
}: {
  tenantId: string; workspaceId: string;
  onTenantChange: (v: string) => void;
  onWorkspaceChange: (v: string) => void;
}) {
  return (
    <div className="flex items-center gap-2 text-[11px]">
      <span className="text-gray-400 dark:text-zinc-600">Tenant:</span>
      <input
        value={tenantId}
        onChange={e => onTenantChange(e.target.value || "default")}
        className="h-6 w-24 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-2 font-mono text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500 transition-colors text-[11px]"
      />
      <span className="text-gray-400 dark:text-zinc-600">Workspace:</span>
      <input
        value={workspaceId}
        onChange={e => onWorkspaceChange(e.target.value || "default")}
        className="h-6 w-24 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-2 font-mono text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500 transition-colors text-[11px]"
      />
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export interface ProjectDashboardPageProps {
  projectId: string;
}

export function ProjectDashboardPage({ projectId }: ProjectDashboardPageProps) {
  const [tenantId,    setTenantId]    = useState("default");
  const [workspaceId, setWorkspaceId] = useState("default");

  const scope = { tenant_id: tenantId, workspace_id: workspaceId, project_id: projectId };

  // ── Queries ─────────────────────────────────────────────────────────────────

  const { data: runs, isLoading: runsLoading, refetch: rRuns, isFetching: runsFetching } = useQuery({
    queryKey: ["proj-runs", projectId, tenantId, workspaceId],
    queryFn:  () => defaultApi.getRuns({ ...scope, limit: 200 }),
    refetchInterval: 15_000,
  });

  const { data: tasks, isLoading: tasksLoading, refetch: rTasks } = useQuery({
    queryKey: ["proj-tasks", projectId, tenantId, workspaceId],
    queryFn:  () => defaultApi.getAllTasks({ limit: 500 }),
    refetchInterval: 15_000,
    select: (rows) => rows.filter(r =>
      r.project.project_id === projectId &&
      r.project.tenant_id  === tenantId  &&
      r.project.workspace_id === workspaceId,
    ),
  });

  const { data: approvals, isLoading: apprLoading, refetch: rApprovals } = useQuery({
    queryKey: ["proj-approvals", projectId, tenantId, workspaceId],
    queryFn:  () => defaultApi.getPendingApprovals(scope),
    refetchInterval: 15_000,
  });

  const { data: costs, isLoading: costsLoading } = useQuery({
    queryKey: ["proj-costs"],
    queryFn:  () => defaultApi.getCosts(),
    refetchInterval: 60_000,
  });

  const { data: recentEvents } = useQuery({
    queryKey: ["proj-events"],
    queryFn:  () => defaultApi.getRecentEvents(50),
    staleTime: 30_000,
    retry: false,
  });

  // ── Derived stats ────────────────────────────────────────────────────────────

  const allRuns    = runs        ?? [];
  const allTasks   = tasks       ?? [];
  const allApprovals = approvals ?? [];

  const activeRuns    = allRuns.filter(r => ACTIVE_STATES.has(r.state));
  const completedRuns = allRuns.filter(r => r.state === "completed");
  const failedRuns    = allRuns.filter(r => r.state === "failed");
  const activeTasks   = allTasks.filter(t => ["queued","leased","running"].includes(t.state));

  // Run trend: last 12 h bucketed hourly (completed + failed)
  const runTrend = useMemo(() => {
    const hours = 12;
    const now   = Date.now();
    return Array.from({ length: hours }, (_, i) => {
      const start = now - (hours - i) * 3_600_000;
      const end   = start + 3_600_000;
      return allRuns.filter(r => r.created_at >= start && r.created_at < end).length;
    });
  }, [allRuns]);

  const isLoading = runsLoading || tasksLoading || apprLoading || costsLoading;

  function handleRefresh() {
    void rRuns(); void rTasks(); void rApprovals();
  }

  // ── Render ───────────────────────────────────────────────────────────────────

  return (
    <div className="h-full overflow-y-auto bg-white dark:bg-zinc-950">
      <div className="max-w-5xl mx-auto px-5 py-5 space-y-5">

        {/* Header */}
        <div className="space-y-3">
          <button
            onClick={() => { window.location.hash = "dashboard"; }}
            className="flex items-center gap-1.5 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:text-zinc-300 transition-colors"
          >
            <ArrowLeft size={13} /> Back to Dashboard
          </button>

          <div className="flex items-start justify-between gap-4 flex-wrap">
            <div className="min-w-0">
              <p className="text-[11px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">Project</p>
              <h2 className="text-[18px] font-semibold font-mono text-gray-900 dark:text-zinc-100 break-all">
                {projectId}
              </h2>
              <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-1 font-mono">
                {tenantId} / {workspaceId} / {projectId}
              </p>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              <button
                onClick={handleRefresh}
                disabled={runsFetching}
                className="flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 px-2.5 py-1.5 text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-800 dark:text-zinc-200 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 disabled:opacity-40 transition-colors"
              >
                <RefreshCw size={11} className={runsFetching ? "animate-spin" : ""} />
                Refresh
              </button>
            </div>
          </div>

          {/* Scope selectors */}
          <div className="flex items-center gap-4 flex-wrap">
            <ScopeSelector
              tenantId={tenantId}
              workspaceId={workspaceId}
              onTenantChange={setTenantId}
              onWorkspaceChange={setWorkspaceId}
            />
          </div>
        </div>

        {/* Stat cards */}
        <div>
          <SectionLabel>Activity</SectionLabel>
          <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <StatCard
              label="Total Runs"
              value={allRuns.length}
              description={`${activeRuns.length} active`}
              variant={activeRuns.length > 0 ? "info" : "default"}
              loading={isLoading}
            />
            <StatCard
              label="Completed"
              value={completedRuns.length}
              description={allRuns.length > 0
                ? `${Math.round((completedRuns.length / allRuns.length) * 100)}% pass rate`
                : undefined}
              variant="success"
              loading={isLoading}
            />
            <StatCard
              label="Failed"
              value={failedRuns.length}
              description={failedRuns.length > 0 ? "needs attention" : "none"}
              variant={failedRuns.length > 0 ? "danger" : "default"}
              loading={isLoading}
            />
            <StatCard
              label="Pending Approvals"
              value={allApprovals.length}
              description={allApprovals.length > 0 ? "waiting for decision" : "none"}
              variant={allApprovals.length > 0 ? "warning" : "default"}
              loading={isLoading}
            />
          </div>
        </div>

        {/* Tasks + cost row */}
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
          <StatCard
            label="Total Tasks"
            value={allTasks.length}
            description={`${activeTasks.length} active`}
            variant={activeTasks.length > 0 ? "info" : "default"}
            loading={isLoading}
          />
          <StatCard
            label="Active Workers"
            value={new Set(allTasks.filter(t => t.lease_owner).map(t => t.lease_owner!)).size}
            description="holding task leases"
            loading={isLoading}
          />
          <StatCard
            label="Total Spend"
            value={fmtMicros(costs?.total_cost_micros ?? 0)}
            description="server-wide (no project filter)"
            loading={costsLoading}
          />
          <StatCard
            label="Provider Calls"
            value={(costs?.total_provider_calls ?? 0).toLocaleString()}
            description="server-wide"
            loading={costsLoading}
          />
        </div>

        {/* Run trend + task breakdown */}
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
          {/* Run trend sparkline */}
          <Panel>
            <div className="flex items-center justify-between mb-3">
              <SectionLabel>Run Volume (12 h)</SectionLabel>
              <span className="flex items-center gap-1 text-[10px] text-gray-300 dark:text-zinc-700">
                <Radio size={9} className="text-indigo-500" />
                hourly
              </span>
            </div>
            {isLoading ? (
              <div className="h-12 rounded bg-gray-100 dark:bg-zinc-800 animate-pulse" />
            ) : (
              <>
                <MiniChart
                  data={runTrend}
                  height={48}
                  color="#6366f1"
                  baseline
                  className="w-full"
                />
                <div className="flex justify-between mt-1">
                  {["12h", "10h", "8h", "6h", "4h", "2h", "now"].map(l => (
                    <span key={l} className="text-[9px] text-gray-300 dark:text-zinc-700 font-mono">{l}</span>
                  ))}
                </div>
              </>
            )}
          </Panel>

          {/* Task state breakdown */}
          <Panel>
            <SectionLabel>Task State Breakdown</SectionLabel>
            {tasksLoading ? (
              <div className="space-y-2 animate-pulse">
                {[1,2,3].map(i => <div key={i} className="h-4 rounded bg-gray-100 dark:bg-zinc-800" />)}
              </div>
            ) : (
              <TaskRing tasks={allTasks} />
            )}
          </Panel>
        </div>

        {/* Active runs */}
        <Panel>
          <div className="flex items-center justify-between mb-3">
            <SectionLabel>
              Active Runs
              {activeRuns.length > 0 && (
                <span className="ml-1.5 text-gray-300 dark:text-zinc-700 font-normal normal-case tracking-normal">
                  ({activeRuns.length})
                </span>
              )}
            </SectionLabel>
            {allRuns.length > activeRuns.length && (
              <span className="text-[10px] text-gray-300 dark:text-zinc-700">
                +{allRuns.length - activeRuns.length} completed/failed
              </span>
            )}
          </div>

          {runsLoading ? (
            <div className="space-y-2 animate-pulse">
              {[1,2,3].map(i => <div key={i} className="h-8 rounded bg-gray-100 dark:bg-zinc-800" />)}
            </div>
          ) : activeRuns.length === 0 ? (
            <div className="flex flex-col items-center justify-center py-8 gap-2 text-center">
              <CheckCircle2 size={18} className="text-emerald-600/60" />
              <p className="text-[12px] text-gray-400 dark:text-zinc-600">No active runs</p>
            </div>
          ) : (
            <div>
              {activeRuns.slice(0, 10).map(run => (
                <RunRow key={run.run_id} run={run} />
              ))}
              {activeRuns.length > 10 && (
                <p className="text-[11px] text-gray-300 dark:text-zinc-700 text-center pt-2">
                  +{activeRuns.length - 10} more — <button
                    onClick={() => { window.location.hash = "runs"; }}
                    className="text-indigo-500 hover:text-indigo-400 transition-colors"
                  >view all runs</button>
                </p>
              )}
            </div>
          )}
        </Panel>

        {/* Pending approvals */}
        {allApprovals.length > 0 && (
          <Panel>
            <SectionLabel>Pending Approvals ({allApprovals.length})</SectionLabel>
            <div className="space-y-0">
              {allApprovals.map((appr, i) => (
                <div
                  key={appr.approval_id}
                  className={clsx(
                    "flex items-center gap-3 py-2 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0",
                    i % 2 === 0 ? "" : "bg-gray-50/30 dark:bg-zinc-900/30 -mx-4 px-4",
                  )}
                >
                  <AlertTriangle size={12} className="text-amber-400 shrink-0" />
                  <span className="flex-1 font-mono text-[12px] text-gray-700 dark:text-zinc-300 truncate" title={appr.approval_id}>
                    {shortId(appr.approval_id)}
                  </span>
                  {appr.run_id && (
                    <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-600 truncate max-w-[120px]" title={appr.run_id}>
                      run: {shortId(appr.run_id)}
                    </span>
                  )}
                  <span className="text-[10px] text-gray-400 dark:text-zinc-600 tabular-nums shrink-0">
                    {fmtAge(appr.created_at)}
                  </span>
                  <button
                    onClick={() => { window.location.hash = "approvals"; }}
                    className="text-[11px] text-indigo-500 hover:text-indigo-400 transition-colors shrink-0"
                  >
                    Review →
                  </button>
                </div>
              ))}
            </div>
          </Panel>
        )}

        {/* Recent session + worker summary */}
        <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
          <Panel>
            <SectionLabel>Workers</SectionLabel>
            {tasksLoading ? (
              <div className="h-12 rounded bg-gray-100 dark:bg-zinc-800 animate-pulse" />
            ) : (() => {
              const workers = new Map<string, { active: number; done: number }>();
              for (const t of allTasks) {
                if (!t.lease_owner) continue;
                const e = workers.get(t.lease_owner) ?? { active: 0, done: 0 };
                if (["leased","running"].includes(t.state)) e.active++;
                else if (["completed","failed"].includes(t.state)) e.done++;
                workers.set(t.lease_owner, e);
              }
              if (workers.size === 0) return (
                <p className="text-[12px] text-gray-400 dark:text-zinc-600 italic text-center py-4">No workers seen</p>
              );
              return (
                <div className="space-y-1.5">
                  {[...workers.entries()].slice(0, 6).map(([wid, { active, done }]) => (
                    <div key={wid} className="flex items-center gap-2.5">
                      <Users size={11} className="text-gray-400 dark:text-zinc-600 shrink-0" />
                      <span className="flex-1 font-mono text-[11px] text-gray-500 dark:text-zinc-400 truncate" title={wid}>
                        {shortId(wid)}
                      </span>
                      {active > 0 && <span className="text-[10px] text-blue-400">{active} active</span>}
                      {done > 0   && <span className="text-[10px] text-gray-400 dark:text-zinc-600">{done} done</span>}
                    </div>
                  ))}
                  {workers.size > 6 && (
                    <p className="text-[10px] text-gray-300 dark:text-zinc-700 text-right">+{workers.size - 6} more</p>
                  )}
                </div>
              );
            })()}
          </Panel>

          {/* Resource summary */}
          <Panel>
            <SectionLabel>Resources</SectionLabel>
            <div className="space-y-2">
              {[
                { icon: Layers,    label: "Runs",      value: allRuns.length,     sub: `${activeRuns.length} active`      },
                { icon: ListChecks, label: "Tasks",    value: allTasks.length,    sub: `${activeTasks.length} active`     },
                { icon: AlertTriangle, label: "Approvals", value: allApprovals.length, sub: "pending" },
                { icon: Coins,     label: "Spend",     value: fmtMicros(costs?.total_cost_micros ?? 0), sub: "server-wide" },
                { icon: Clock,     label: "Oldest run", value: allRuns.length > 0
                    ? fmtAge(Math.min(...allRuns.map(r => r.created_at)))
                    : "—",
                  sub: "" },
              ].map(({ icon: Icon, label, value, sub }) => (
                <div key={label} className="flex items-center gap-2.5 py-1.5 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0">
                  <Icon size={12} className="text-gray-400 dark:text-zinc-600 shrink-0" />
                  <span className="text-[12px] text-gray-500 dark:text-zinc-400 flex-1">{label}</span>
                  <span className="text-[12px] font-semibold text-gray-800 dark:text-zinc-200 tabular-nums">{value}</span>
                  {sub && <span className="text-[10px] text-gray-400 dark:text-zinc-600">{sub}</span>}
                </div>
              ))}
            </div>
          </Panel>
        </div>

        {/* Live event log */}
        <Panel>
          <div className="flex items-center justify-between mb-3">
            <SectionLabel>Live Events</SectionLabel>
            <span className="flex items-center gap-1 text-[10px] text-gray-300 dark:text-zinc-700">
              <Radio size={9} className="text-emerald-500" /> SSE
            </span>
          </div>
          <EventLog
            initialEvents={recentEvents ?? []}
            maxEvents={40}
          />
        </Panel>

      </div>
    </div>
  );
}

export default ProjectDashboardPage;
