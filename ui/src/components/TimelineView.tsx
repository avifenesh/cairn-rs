/**
 * TimelineView — Gantt-style horizontal timeline for runs and tasks.
 *
 * Pure CSS + SVG, no external chart library.
 *
 * Usage (runs list):
 *   <TimelineView runs={runs} zoom="6h" />
 *
 * Usage (single run with tasks):
 *   <TimelineView runs={[run]} tasks={tasks} zoom="1h" />
 */

import { useState, useRef, useCallback } from "react";
import { clsx } from "clsx";
import type { RunRecord, RunState, TaskRecord, TaskState } from "../lib/types";

// ── Types ─────────────────────────────────────────────────────────────────────

export type ZoomLevel = "15m" | "1h" | "6h" | "24h" | "7d";

export interface TimelineViewProps {
  runs:   RunRecord[];
  tasks?: TaskRecord[];
  zoom?:  ZoomLevel;
  /** If true, show only runs scoped to an already-selected run (single-run mode). */
  singleRun?: boolean;
}

// ── Constants ─────────────────────────────────────────────────────────────────

const ZOOM_MS: Record<ZoomLevel, number> = {
  "15m": 15 * 60_000,
  "1h":  60 * 60_000,
  "6h":  6  * 60 * 60_000,
  "24h": 24 * 60 * 60_000,
  "7d":  7  * 24 * 60 * 60_000,
};

const ZOOM_TICK_MS: Record<ZoomLevel, number> = {
  "15m": 60_000,          // 1-min ticks
  "1h":  10 * 60_000,     // 10-min ticks
  "6h":  60 * 60_000,     // 1-h ticks
  "24h": 4  * 60 * 60_000, // 4-h ticks
  "7d":  24 * 60 * 60_000, // 1-day ticks
};

const ZOOM_FMT: Record<ZoomLevel, (ms: number) => string> = {
  "15m": ms => new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }),
  "1h":  ms => new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }),
  "6h":  ms => new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }),
  "24h": ms => new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }),
  "7d":  ms => new Date(ms).toLocaleDateString(undefined,  { weekday: "short", month: "short", day: "numeric" }),
};

const ROW_H   = 28;    // px — height of each run row
const TASK_H  = 10;    // px — height of nested task bars
const LABEL_W = 140;   // px — left label column
const AXIS_H  = 28;    // px — top time axis
const MIN_BAR = 4;     // px — minimum visible bar width

// ── Color maps ────────────────────────────────────────────────────────────────

const RUN_COLORS: Record<RunState, { fill: string; stroke: string; text: string }> = {
  running:            { fill: "#1d4ed8", stroke: "#3b82f6", text: "#93c5fd" },
  pending:            { fill: "#374151", stroke: "#4b5563", text: "#9ca3af" },
  paused:             { fill: "#92400e", stroke: "#d97706", text: "#fcd34d" },
  waiting_approval:   { fill: "#581c87", stroke: "#9333ea", text: "#d8b4fe" },
  waiting_dependency: { fill: "#1e3a5f", stroke: "#4b82b8", text: "#93c5fd" },
  completed:          { fill: "#065f46", stroke: "#10b981", text: "#6ee7b7" },
  failed:             { fill: "#7f1d1d", stroke: "#ef4444", text: "#fca5a5" },
  canceled:           { fill: "#1c1c1c", stroke: "#52525b", text: "#71717a" },
};

const TASK_COLORS: Partial<Record<TaskState, { fill: string; stroke: string }>> = {
  queued:    { fill: "#78350f", stroke: "#d97706" },
  leased:    { fill: "#1e1b4b", stroke: "#6366f1" },
  running:   { fill: "#1e3a8a", stroke: "#60a5fa" },
  completed: { fill: "#064e3b", stroke: "#34d399" },
  failed:    { fill: "#7f1d1d", stroke: "#f87171" },
  canceled:  { fill: "#18181b", stroke: "#52525b" },
  paused:    { fill: "#451a03", stroke: "#f59e0b" },
};

// ── Helpers ───────────────────────────────────────────────────────────────────

function shortId(id: string) {
  return id.length > 18 ? `${id.slice(0, 8)}…${id.slice(-5)}` : id;
}

function fmtDur(ms: number): string {
  if (ms < 1_000)  return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1_000).toFixed(1)}s`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ${Math.floor((ms % 60_000) / 1_000)}s`;
  return `${Math.floor(ms / 3_600_000)}h ${Math.floor((ms % 3_600_000) / 60_000)}m`;
}

/** Map a timestamp into [0, 1] over the visible window. */
function toX(ms: number, windowStart: number, windowMs: number): number {
  return (ms - windowStart) / windowMs;
}

// ── Tooltip ───────────────────────────────────────────────────────────────────

interface TooltipInfo {
  x: number; y: number;
  id: string;
  state: string;
  start: number;
  end: number | null;
  label: string;
}

function Tooltip({ tip, canvasW }: { tip: TooltipInfo; canvasW: number }) {
  const maxX = canvasW - LABEL_W - 8;
  // Pin to the right half if bar is in the left half, otherwise left of cursor
  const tipX = tip.x > maxX / 2 ? tip.x - 200 : tip.x + 8;
  const dur = tip.end ? tip.end - tip.start : Date.now() - tip.start;

  return (
    <div
      className="absolute z-20 pointer-events-none bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 rounded-lg
                 px-3 py-2 shadow-xl text-[11px] space-y-1"
      style={{ left: LABEL_W + tipX, top: tip.y - 4 }}
    >
      <p className="font-mono text-gray-800 dark:text-zinc-200 font-medium">{shortId(tip.id)}</p>
      <p className="text-gray-400 dark:text-zinc-500 capitalize">{tip.state.replace(/_/g, " ")}</p>
      <p className="text-gray-400 dark:text-zinc-600 tabular-nums">{fmtDur(dur)}</p>
      {!tip.end && <p className="text-blue-400">still running</p>}
    </div>
  );
}

// ── Zoom selector ─────────────────────────────────────────────────────────────

export function ZoomSelector({
  value, onChange,
}: { value: ZoomLevel; onChange: (z: ZoomLevel) => void }) {
  const levels: ZoomLevel[] = ["15m", "1h", "6h", "24h", "7d"];
  return (
    <div className="flex items-center rounded border border-gray-200 dark:border-zinc-700 overflow-hidden">
      {levels.map(z => (
        <button
          key={z}
          onClick={() => onChange(z)}
          className={clsx(
            "px-2 py-1 text-[11px] font-mono transition-colors",
            z !== "15m" && "border-l border-gray-200 dark:border-zinc-700",
            value === z
              ? "bg-gray-200 dark:bg-zinc-700 text-gray-800 dark:text-zinc-200"
              : "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300",
          )}
        >
          {z}
        </button>
      ))}
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export function TimelineView({ runs, tasks, zoom = "6h", singleRun: _singleRun }: TimelineViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [tooltip, setTooltip]     = useState<TooltipInfo | null>(null);
  const [canvasW,  setCanvasW]    = useState(800);

  // Measure container width on mount/resize
  const measureRef = useCallback((node: HTMLDivElement | null) => {
    if (!node) return;
    const ro = new ResizeObserver(([entry]) => {
      setCanvasW(entry.contentRect.width || 800);
    });
    ro.observe(node);
  }, []);

  const now       = Date.now();
  const windowMs  = ZOOM_MS[zoom];

  // Window: show the last `windowMs` up to now
  const windowEnd   = now;
  const windowStart = windowEnd - windowMs;

  const tickMs   = ZOOM_TICK_MS[zoom];
  const fmtTick  = ZOOM_FMT[zoom];
  const innerW   = canvasW - LABEL_W;

  // Build tick positions
  const firstTick = Math.ceil(windowStart / tickMs) * tickMs;
  const ticks: number[] = [];
  for (let t = firstTick; t <= windowEnd; t += tickMs) ticks.push(t);

  // Filter runs to those with any overlap with the window
  const visibleRuns = runs.filter(r => {
    const end = r.updated_at;
    return r.created_at <= windowEnd && end >= windowStart;
  });

  // Group tasks by run_id
  const tasksByRun = new Map<string, TaskRecord[]>();
  if (tasks) {
    for (const t of tasks) {
      const rid = t.parent_run_id ?? "__none__";
      if (!tasksByRun.has(rid)) tasksByRun.set(rid, []);
      tasksByRun.get(rid)!.push(t);
    }
  }

  // Calculate total SVG height
  const totalRows = visibleRuns.reduce((sum, r) => {
    const nTasks = tasksByRun.get(r.run_id)?.length ?? 0;
    return sum + 1 + (nTasks > 0 ? 1 : 0); // 1 row + optional task row
  }, 0);
  const svgH = AXIS_H + totalRows * ROW_H + 8;

  function barRect(startMs: number, endMs: number): { x: number; width: number } | null {
    const clampedStart = Math.max(startMs, windowStart);
    const clampedEnd   = Math.min(endMs,   windowEnd);
    if (clampedStart >= clampedEnd) return null;

    const x = toX(clampedStart, windowStart, windowMs) * innerW;
    const w = Math.max(MIN_BAR, toX(clampedEnd, windowStart, windowMs) * innerW - x);
    return { x, width: w };
  }

  function handleBarHover(
    e: React.MouseEvent<SVGRectElement>,
    id: string, state: string, start: number, end: number | null, label: string,
  ) {
    const rect = containerRef.current?.getBoundingClientRect();
    if (!rect) return;
    const relX = e.clientX - rect.left - LABEL_W;
    const relY = e.clientY - rect.top;
    setTooltip({ x: relX, y: relY, id, state, start, end, label });
  }

  // Render rows
  let rowIdx = 0;
  const rowElements: React.ReactNode[] = [];

  for (const run of visibleRuns) {
    const runColors = RUN_COLORS[run.state] ?? RUN_COLORS.pending;
    const isRunning = run.state === "running";
    const runEnd    = isRunning ? now : run.updated_at;
    const runBar    = barRect(run.created_at, runEnd);
    const runY      = AXIS_H + rowIdx * ROW_H;
    const runTasks  = tasksByRun.get(run.run_id) ?? [];

    rowElements.push(
      // Alternating row background
      <rect
        key={`bg-${run.run_id}`}
        x={0} y={runY}
        width={innerW} height={ROW_H}
        fill={rowIdx % 2 === 0 ? "#18181b" : "#1c1c1f"}
        opacity={0.6}
      />,

      // "Now" indicator
      <line
        key={`now-${run.run_id}`}
        x1={innerW} y1={runY} x2={innerW} y2={runY + ROW_H}
        stroke="#3f3f46" strokeWidth={1} strokeDasharray="2 2"
      />,
    );

    if (runBar) {
      const barTop = runY + (ROW_H - 14) / 2;

      // Run bar
      rowElements.push(
        <rect
          key={`run-${run.run_id}`}
          x={runBar.x}
          y={barTop}
          width={runBar.width}
          height={14}
          rx={3}
          fill={runColors.fill}
          stroke={runColors.stroke}
          strokeWidth={0.8}
          className="cursor-pointer"
          onMouseMove={e => handleBarHover(e, run.run_id, run.state, run.created_at, isRunning ? null : runEnd, "run")}
          onMouseLeave={() => setTooltip(null)}
          onClick={() => { window.location.hash = `run/${run.run_id}`; }}
        />,
      );

      // Animated "still running" pulse overlay
      if (isRunning && runBar.width > 20) {
        rowElements.push(
          <rect
            key={`pulse-${run.run_id}`}
            x={runBar.x + runBar.width - 8}
            y={barTop + 3}
            width={8}
            height={8}
            rx={1}
            fill={runColors.stroke}
            opacity={0.5}
          >
            <animate attributeName="opacity" values="0.5;1;0.5" dur="1.5s" repeatCount="indefinite" />
          </rect>,
        );
      }

      // Run label inside bar (if bar is wide enough)
      if (runBar.width > 70) {
        rowElements.push(
          <text
            key={`lbl-${run.run_id}`}
            x={runBar.x + 6}
            y={barTop + 10}
            fill={runColors.text}
            fontSize={9}
            fontFamily="ui-monospace, monospace"
            clipPath={`url(#clip-${run.run_id})`}
          >
            {shortId(run.run_id)}
          </text>,
          <clipPath key={`clip-${run.run_id}`} id={`clip-${run.run_id}`}>
            <rect x={runBar.x} y={barTop} width={runBar.width} height={14} />
          </clipPath>,
        );
      }

      // Nested task bars (on the same row, slightly smaller)
      if (runTasks.length > 0) {
        const taskBarTop = runY + 20; // below the run bar

        for (const task of runTasks) {
          const taskColors = TASK_COLORS[task.state] ?? { fill: "#27272a", stroke: "#52525b" };
          const taskEnd = ["completed","failed","canceled"].includes(task.state)
            ? task.updated_at
            : now;
          const taskBar = barRect(task.created_at, taskEnd);
          if (!taskBar) continue;

          rowElements.push(
            <rect
              key={`task-${task.task_id}`}
              x={taskBar.x}
              y={taskBarTop}
              width={taskBar.width}
              height={TASK_H - 2}
              rx={2}
              fill={taskColors.fill}
              stroke={taskColors.stroke}
              strokeWidth={0.6}
              className="cursor-pointer"
              onMouseMove={e => handleBarHover(
                e, task.task_id, task.state,
                task.created_at,
                ["completed","failed","canceled"].includes(task.state) ? task.updated_at : null,
                "task",
              )}
              onMouseLeave={() => setTooltip(null)}
            />,
          );
        }
      }
    }

    rowIdx += 1;
    if (runTasks.length > 0) rowIdx += 1;
  }

  if (visibleRuns.length === 0) {
    return (
      <div className="flex items-center justify-center h-32 text-[12px] text-gray-400 dark:text-zinc-600">
        No runs in the selected time window.
      </div>
    );
  }

  return (
    <div ref={containerRef} className="relative select-none overflow-hidden">
      <div ref={measureRef} className="absolute inset-0 pointer-events-none" />

      <div className="flex overflow-x-hidden">
        {/* Left label column */}
        <div
          className="shrink-0 border-r border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950"
          style={{ width: LABEL_W }}
        >
          {/* Axis header cell */}
          <div
            className="flex items-end px-2 pb-1 text-[9px] text-gray-300 dark:text-zinc-700 uppercase tracking-wider border-b border-gray-200 dark:border-zinc-800"
            style={{ height: AXIS_H }}
          >
            Run
          </div>
          {/* Run labels */}
          {visibleRuns.map((run, i) => {
            const runTasks = tasksByRun.get(run.run_id) ?? [];
            const hasTasks = runTasks.length > 0;
            return (
              <div
                key={run.run_id}
                style={{ height: ROW_H * (hasTasks ? 2 : 1) }}
                className={clsx(
                  "flex items-start pt-1.5 gap-1.5 px-2 cursor-pointer hover:bg-white/5 transition-colors",
                  i % 2 === 0 ? "bg-white dark:bg-zinc-950" : "bg-gray-50/20 dark:bg-zinc-900/20",
                )}
                onClick={() => { window.location.hash = `run/${run.run_id}`; }}
              >
                <span
                  className="w-2 h-2 rounded-full shrink-0 mt-0.5"
                  style={{ backgroundColor: RUN_COLORS[run.state]?.stroke ?? "#52525b" }}
                />
                <div className="min-w-0">
                  <p className="text-[10px] font-mono text-gray-500 dark:text-zinc-400 truncate">
                    {shortId(run.run_id)}
                  </p>
                  {hasTasks && (
                    <p className="text-[9px] text-gray-300 dark:text-zinc-700 mt-0.5">
                      {runTasks.length} task{runTasks.length !== 1 ? "s" : ""}
                    </p>
                  )}
                </div>
              </div>
            );
          })}
        </div>

        {/* SVG canvas */}
        <div className="flex-1 overflow-x-auto">
          <svg
            width={innerW}
            height={svgH}
            className="block bg-white dark:bg-zinc-950"
            style={{ minWidth: 300 }}
          >
            {/* ── Time axis ── */}
            <rect x={0} y={0} width={innerW} height={AXIS_H} fill="#09090b" />
            <line x1={0} y1={AXIS_H - 1} x2={innerW} y2={AXIS_H - 1} stroke="#27272a" strokeWidth={1} />

            {ticks.map(t => {
              const x = toX(t, windowStart, windowMs) * innerW;
              return (
                <g key={t}>
                  <line x1={x} y1={AXIS_H - 6} x2={x} y2={AXIS_H} stroke="#3f3f46" strokeWidth={1} />
                  <line x1={x} y1={AXIS_H} x2={x} y2={svgH} stroke="#27272a" strokeWidth={0.5} strokeDasharray="3 4" />
                  <text
                    x={x + 3} y={AXIS_H - 8}
                    fill="#52525b"
                    fontSize={9}
                    fontFamily="ui-monospace, monospace"
                  >
                    {fmtTick(t)}
                  </text>
                </g>
              );
            })}

            {/* "Now" vertical line */}
            <line
              x1={innerW} y1={AXIS_H}
              x2={innerW} y2={svgH}
              stroke="#4f46e5" strokeWidth={1} strokeDasharray="4 3"
              opacity={0.5}
            />
            <text x={innerW - 4} y={AXIS_H - 4} fill="#6366f1" fontSize={8} textAnchor="end" fontFamily="ui-monospace, monospace">
              now
            </text>

            {/* Row elements */}
            {rowElements}
          </svg>
        </div>
      </div>

      {/* Tooltip overlay */}
      {tooltip && (
        <Tooltip tip={tooltip} canvasW={canvasW} />
      )}

      {/* Legend */}
      <div className="flex items-center gap-4 px-3 py-1.5 border-t border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 flex-wrap">
        {(Object.entries(RUN_COLORS) as [RunState, typeof RUN_COLORS[RunState]][])
          .filter(([s]) => ["running", "completed", "failed", "canceled", "pending"].includes(s))
          .map(([state, col]) => (
            <span key={state} className="flex items-center gap-1 text-[10px] text-gray-400 dark:text-zinc-600">
              <span className="w-3 h-2 rounded-sm" style={{ backgroundColor: col.stroke }} />
              {state}
            </span>
          ))}
        {tasks && tasks.length > 0 && (
          <>
            <span className="text-zinc-800 text-[10px]">|</span>
            <span className="text-[10px] text-gray-300 dark:text-zinc-700 italic">smaller bars = tasks</span>
          </>
        )}
      </div>
    </div>
  );
}

// ── Gantt view for a single run's tasks ───────────────────────────────────────

export interface GanttProps {
  runStart: number;
  runEnd?:  number;
  tasks:    TaskRecord[];
}

export function GanttView({ runStart, runEnd, tasks }: GanttProps) {
  const now      = Date.now();
  const end      = runEnd ?? now;
  const windowMs = Math.max(end - runStart, 60_000); // at least 1 min

  function pct(ms: number): number {
    return Math.max(0, Math.min(100, ((ms - runStart) / windowMs) * 100));
  }

  // Ticks: ~5 evenly spaced
  const tickCount = 5;
  const ticks = Array.from({ length: tickCount + 1 }, (_, i) =>
    runStart + (i / tickCount) * windowMs,
  );

  const sorted = [...tasks].sort((a, b) => a.created_at - b.created_at);

  return (
    <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden">
      {/* Axis */}
      <div className="relative h-6 bg-white dark:bg-zinc-950 border-b border-gray-200 dark:border-zinc-800" style={{ paddingLeft: LABEL_W }}>
        {ticks.map((t, i) => {
          const left = ((t - runStart) / windowMs) * 100;
          return (
            <div
              key={i}
              className="absolute top-0 h-full flex items-end pb-0.5"
              style={{ left: `${left}%` }}
            >
              <div className="w-px h-2 bg-zinc-700 mb-0.5" />
              <span className="text-[9px] font-mono text-gray-400 dark:text-zinc-600 ml-0.5 whitespace-nowrap">
                +{fmtDur(t - runStart)}
              </span>
            </div>
          );
        })}
        {/* "now" marker */}
        {!runEnd && (
          <div
            className="absolute top-0 h-full border-l border-indigo-500/50 border-dashed"
            style={{ left: `${pct(now)}%` }}
          />
        )}
      </div>

      {/* Task rows */}
      {sorted.length === 0 ? (
        <div className="px-4 py-6 text-center text-[12px] text-gray-400 dark:text-zinc-600">
          No tasks in this run.
        </div>
      ) : (
        sorted.map((task, i) => {
          const taskColors = TASK_COLORS[task.state] ?? { fill: "#27272a", stroke: "#52525b" };
          const taskEnd  = ["completed","failed","canceled"].includes(task.state)
            ? task.updated_at : now;
          const startPct = pct(task.created_at);
          const endPct   = pct(taskEnd);
          const widthPct = Math.max(0.3, endPct - startPct);
          const dur      = taskEnd - task.created_at;

          return (
            <div
              key={task.task_id}
              className={clsx(
                "flex items-center border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0 h-8",
                i % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50",
              )}
            >
              {/* Label */}
              <div className="shrink-0 flex items-center gap-1.5 px-2" style={{ width: LABEL_W }}>
                <span
                  className="w-1.5 h-1.5 rounded-full shrink-0"
                  style={{ backgroundColor: taskColors.stroke }}
                />
                <span className="text-[10px] font-mono text-gray-500 dark:text-zinc-400 truncate" title={task.task_id}>
                  {shortId(task.task_id)}
                </span>
              </div>

              {/* Bar track */}
              <div className="flex-1 relative h-full flex items-center px-0">
                <div className="absolute inset-x-0 h-4 flex items-center">
                  <div
                    className="absolute h-full rounded-sm"
                    style={{
                      left:  `${startPct}%`,
                      width: `${widthPct}%`,
                      backgroundColor: taskColors.fill,
                      border: `1px solid ${taskColors.stroke}60`,
                    }}
                    title={`${task.state} · ${fmtDur(dur)}`}
                  />
                  {/* Duration label inside bar if wide enough */}
                  {widthPct > 8 && (
                    <div
                      className="absolute text-[9px] font-mono px-1 pointer-events-none"
                      style={{
                        left:  `calc(${startPct}% + 2px)`,
                        color: taskColors.stroke,
                      }}
                    >
                      {fmtDur(dur)}
                    </div>
                  )}
                </div>
              </div>
            </div>
          );
        })
      )}
    </div>
  );
}

export default TimelineView;
