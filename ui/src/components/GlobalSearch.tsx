/**
 * GlobalSearch — cross-entity search modal.
 *
 * Queries all entity types in parallel using React Query (reusing cached data
 * from page-level queries where possible), filters client-side, and renders
 * grouped results with keyboard navigation.
 *
 * Entity types searched: runs, sessions, tasks, approvals, traces, prompts.
 */

import {
  useState, useEffect, useRef, useCallback,
  type KeyboardEvent,
} from 'react';
import { useQueries } from '@tanstack/react-query';
import {
  Search, ArrowLeft, Play, MonitorPlay, ListChecks,
  CheckSquare, Waves, FileText, Loader2,
} from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';

// ── Types ─────────────────────────────────────────────────────────────────────

type EntityType = 'run' | 'session' | 'task' | 'approval' | 'trace' | 'prompt';

interface SearchResult {
  type: EntityType;
  id: string;
  /** Primary display text (usually shortened ID + name). */
  title: string;
  /** Secondary info line. */
  subtitle: string;
  /** Right-aligned meta (state badge text, model name, etc.). */
  meta?: string;
  /** Hash navigation target, e.g. 'run/abc123' or 'tasks'. */
  href: string;
}

// ── Entity type config ────────────────────────────────────────────────────────

const ENTITY_CONFIG: Record<EntityType, {
  label: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  badge: string;
}> = {
  run:      { label: 'Runs',      icon: Play,        badge: 'bg-indigo-950/60 text-indigo-300 border-indigo-800/40' },
  session:  { label: 'Sessions',  icon: MonitorPlay,  badge: 'bg-blue-950/60   text-blue-300   border-blue-800/40'   },
  task:     { label: 'Tasks',     icon: ListChecks,   badge: 'bg-violet-950/60 text-violet-300 border-violet-800/40' },
  approval: { label: 'Approvals', icon: CheckSquare,  badge: 'bg-amber-950/60  text-amber-300  border-amber-800/40'  },
  trace:    { label: 'Traces',    icon: Waves,        badge: 'bg-pink-950/60   text-pink-300   border-pink-800/40'    },
  prompt:   { label: 'Prompts',   icon: FileText,     badge: 'bg-teal-950/60   text-teal-300   border-teal-800/40'   },
};

const ENTITY_ORDER: EntityType[] = ['run', 'session', 'task', 'approval', 'trace', 'prompt'];

// ── Helpers ───────────────────────────────────────────────────────────────────

const shortId = (id: string) =>
  id.length > 22 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;


function useDebounce<T>(value: T, delay: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(t);
  }, [value, delay]);
  return debounced;
}

/** Case-insensitive substring check. */
const has = (s: string | null | undefined, q: string): boolean =>
  !!s && s.toLowerCase().includes(q);

// ── Result builder ─────────────────────────────────────────────────────────────

function buildResults(q: string, data: {
  runs?:      ReturnType<typeof Array.prototype.map>;
  sessions?:  ReturnType<typeof Array.prototype.map>;
  tasks?:     ReturnType<typeof Array.prototype.map>;
  approvals?: ReturnType<typeof Array.prototype.map>;
  traces?:    { traces?: unknown[] };
  prompts?:   { items?: unknown[] };
}): Map<EntityType, SearchResult[]> {
  const lq = q.toLowerCase();
  const map = new Map<EntityType, SearchResult[]>();

  // Runs
  const runs = (data.runs ?? []) as Array<{
    run_id: string; session_id: string; state: string;
    failure_class?: string | null; created_at: number;
  }>;
  const matchedRuns = runs.filter(r =>
    has(r.run_id, lq) || has(r.session_id, lq) || has(r.state, lq) || has(r.failure_class, lq),
  ).slice(0, 8).map((r): SearchResult => ({
    type:     'run',
    id:       r.run_id,
    title:    shortId(r.run_id),
    subtitle: `session: ${shortId(r.session_id)}`,
    meta:     r.state,
    href:     `run/${r.run_id}`,
  }));
  if (matchedRuns.length) map.set('run', matchedRuns);

  // Sessions
  const sessions = (data.sessions ?? []) as Array<{
    session_id: string; state: string;
    project: { tenant_id: string; workspace_id: string; project_id: string };
    created_at: number;
  }>;
  const matchedSessions = sessions.filter(s =>
    has(s.session_id, lq) || has(s.state, lq) ||
    has(s.project.tenant_id, lq) || has(s.project.project_id, lq),
  ).slice(0, 8).map((s): SearchResult => ({
    type:     'session',
    id:       s.session_id,
    title:    shortId(s.session_id),
    subtitle: `${s.project.tenant_id} / ${s.project.project_id}`,
    meta:     s.state,
    href:     `session/${s.session_id}`,
  }));
  if (matchedSessions.length) map.set('session', matchedSessions);

  // Tasks
  const tasks = (data.tasks ?? []) as Array<{
    task_id: string; state: string; parent_run_id?: string | null;
    lease_owner?: string | null; created_at: number;
  }>;
  const matchedTasks = tasks.filter(t =>
    has(t.task_id, lq) || has(t.state, lq) ||
    has(t.parent_run_id, lq) || has(t.lease_owner, lq),
  ).slice(0, 8).map((t): SearchResult => ({
    type:     'task',
    id:       t.task_id,
    title:    shortId(t.task_id),
    subtitle: t.parent_run_id ? `run: ${shortId(t.parent_run_id)}` : 'no parent run',
    meta:     t.state,
    href:     'tasks',
  }));
  if (matchedTasks.length) map.set('task', matchedTasks);

  // Approvals
  const approvals = (data.approvals ?? []) as Array<{
    approval_id: string; requirement: string;
    decision?: string | null;
    run_id?: string | null; task_id?: string | null;
    created_at: number;
  }>;
  const matchedApprovals = approvals.filter(a =>
    has(a.approval_id, lq) || has(a.run_id, lq) || has(a.task_id, lq) || has(a.requirement, lq),
  ).slice(0, 8).map((a): SearchResult => ({
    type:     'approval',
    id:       a.approval_id,
    title:    shortId(a.approval_id),
    subtitle: a.run_id ? `run: ${shortId(a.run_id)}` : `task: ${shortId(a.task_id ?? '—')}`,
    meta:     a.decision ?? 'pending',
    href:     'approvals',
  }));
  if (matchedApprovals.length) map.set('approval', matchedApprovals);

  // Traces
  const traces = ((data.traces as { traces?: unknown[] })?.traces ?? []) as Array<{
    trace_id: string; model_id: string; is_error: boolean;
    latency_ms: number; created_at_ms: number;
  }>;
  const matchedTraces = traces.filter(t =>
    has(t.trace_id, lq) || has(t.model_id, lq),
  ).slice(0, 8).map((t): SearchResult => ({
    type:     'trace',
    id:       t.trace_id,
    title:    shortId(t.trace_id),
    subtitle: `model: ${t.model_id}`,
    meta:     t.is_error ? 'error' : `${t.latency_ms}ms`,
    href:     'traces',
  }));
  if (matchedTraces.length) map.set('trace', matchedTraces);

  // Prompts
  const prompts = ((data.prompts as { items?: unknown[] })?.items ?? []) as Array<{
    prompt_asset_id: string; name: string; kind: string;
    created_at: number;
  }>;
  const matchedPrompts = prompts.filter(p =>
    has(p.prompt_asset_id, lq) || has(p.name, lq) || has(p.kind, lq),
  ).slice(0, 8).map((p): SearchResult => ({
    type:     'prompt',
    id:       p.prompt_asset_id,
    title:    p.name || shortId(p.prompt_asset_id),
    subtitle: `${p.kind} · ${shortId(p.prompt_asset_id)}`,
    href:     'prompts',
  }));
  if (matchedPrompts.length) map.set('prompt', matchedPrompts);

  return map;
}

// ── Result row ────────────────────────────────────────────────────────────────

function ResultRow({
  result, active, onSelect,
}: {
  result: SearchResult;
  active: boolean;
  onSelect: () => void;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  const cfg = ENTITY_CONFIG[result.type];

  useEffect(() => {
    if (active) ref.current?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  return (
    <button
      ref={ref}
      onClick={onSelect}
      className={clsx(
        'w-full flex items-center gap-3 px-3 py-2 text-left rounded-md transition-colors',
        active ? 'bg-gray-100 dark:bg-zinc-800 ring-1 ring-inset ring-gray-300 dark:ring-zinc-700' : 'hover:bg-gray-100/60 dark:hover:bg-zinc-800/60',
      )}
    >
      {/* Type badge */}
      <span className={clsx(
        'shrink-0 inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] font-medium',
        cfg.badge,
      )}>
        <cfg.icon size={10} />
        {cfg.label.slice(0, -1)}
      </span>

      {/* Title + subtitle */}
      <div className="flex-1 min-w-0">
        <p className="text-[13px] font-mono text-gray-800 dark:text-zinc-200 truncate" title={result.id}>
          {result.title}
        </p>
        <p className="text-[11px] text-gray-400 dark:text-zinc-500 truncate">{result.subtitle}</p>
      </div>

      {/* Meta (state/latency) */}
      {result.meta && (
        <span className={clsx(
          'shrink-0 text-[10px] font-medium rounded px-1.5 py-0.5',
          result.meta === 'error'    ? 'bg-red-950/60 text-red-400 border border-red-800/40' :
          result.meta === 'pending'  ? 'bg-amber-950/60 text-amber-400 border border-amber-800/40' :
          result.meta === 'running'  ? 'bg-emerald-950/60 text-emerald-400 border border-emerald-800/40' :
          result.meta === 'approved' ? 'bg-emerald-950/60 text-emerald-400 border border-emerald-800/40' :
          result.meta === 'rejected' ? 'bg-red-950/60 text-red-400 border border-red-800/40' :
                                       'bg-gray-100/60 dark:bg-zinc-800/60 text-gray-400 dark:text-zinc-500 border border-gray-200 dark:border-zinc-700',
        )}>
          {result.meta}
        </span>
      )}
    </button>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

interface GlobalSearchProps {
  /** Pre-populated query string from the command palette. */
  initialQuery?: string;
  /** Called when the user dismisses the modal (Escape or backdrop). */
  onClose: () => void;
  /** Called when "← Commands" is clicked to return to the command palette. */
  onBack?: () => void;
}

export function GlobalSearch({ initialQuery = '', onClose, onBack }: GlobalSearchProps) {
  const [query, setQuery]       = useState(initialQuery);
  const [activeIdx, setActiveIdx] = useState(0);
  const inputRef                  = useRef<HTMLInputElement>(null);
  const debouncedQuery            = useDebounce(query, 300);
  const enabled                   = debouncedQuery.trim().length >= 2;

  useEffect(() => {
    requestAnimationFrame(() => inputRef.current?.focus());
  }, []);

  // Fetch all entity types in parallel; reuses cached data from page queries.
  const results = useQueries({
    queries: [
      {
        queryKey: ['runs'],
        queryFn:  () => defaultApi.getRuns({ limit: 200 }),
        enabled,
        staleTime: 60_000,
        retry: false,
      },
      {
        queryKey: ['sessions'],
        queryFn:  () => defaultApi.getSessions({ limit: 200 }),
        enabled,
        staleTime: 60_000,
        retry: false,
      },
      {
        queryKey: ['tasks'],
        queryFn:  () => defaultApi.getAllTasks({ limit: 500 }),
        enabled,
        staleTime: 60_000,
        retry: false,
      },
      {
        queryKey: ['approvals'],
        queryFn:  () => defaultApi.getPendingApprovals(),
        enabled,
        staleTime: 60_000,
        retry: false,
      },
      {
        queryKey: ['traces'],
        queryFn:  () => defaultApi.getTraces(500),
        enabled,
        staleTime: 60_000,
        retry: false,
      },
      {
        queryKey: ['prompt-assets'],
        queryFn:  () => defaultApi.getPromptAssets({ limit: 100 }),
        enabled,
        staleTime: 60_000,
        retry: false,
      },
    ],
  });

  const [runsQ, sessionsQ, tasksQ, approvalsQ, tracesQ, promptsQ] = results;
  const isLoading = results.some(r => r.isLoading);

  const grouped = enabled && !isLoading
    ? buildResults(debouncedQuery.trim(), {
        runs:      runsQ.data      as unknown[],
        sessions:  sessionsQ.data  as unknown[],
        tasks:     tasksQ.data     as unknown[],
        approvals: approvalsQ.data as unknown[],
        traces:    tracesQ.data    as { traces?: unknown[] },
        prompts:   promptsQ.data   as { items?: unknown[] },
      })
    : new Map<EntityType, SearchResult[]>();

  // Flat list for keyboard navigation.
  const flat: SearchResult[] = [];
  ENTITY_ORDER.forEach(type => { grouped.get(type)?.forEach(r => flat.push(r)); });
  const totalResults = flat.length;

  // Clamp active index when results change.
  const clampedIdx = totalResults > 0 ? Math.min(activeIdx, totalResults - 1) : 0;

  const navigate = useCallback((result: SearchResult) => {
    window.location.hash = result.href;
    onClose();
  }, [onClose]);

  function handleKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIdx(i => (i + 1) % Math.max(1, totalResults));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIdx(i => (i - 1 + Math.max(1, totalResults)) % Math.max(1, totalResults));
    } else if (e.key === 'Enter' && flat[clampedIdx]) {
      e.preventDefault();
      navigate(flat[clampedIdx]);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      onBack ? onBack() : onClose();
    }
  }

  // Track running index for active highlighting across groups.
  let runningIdx = 0;

  return (
    <div
      className="fixed inset-0 z-[60] flex items-start justify-center pt-[12vh] px-4"
      style={{ background: 'rgba(0,0,0,0.70)', backdropFilter: 'blur(3px)' }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div
        className="w-full max-w-2xl rounded-xl bg-gray-50 dark:bg-zinc-900 ring-1 ring-zinc-800 shadow-2xl shadow-black/60 overflow-hidden"
        role="dialog"
        aria-modal="true"
        aria-label="Global search"
      >
        {/* Search input */}
        <div className="flex items-center gap-3 px-3.5 py-3 border-b border-gray-200 dark:border-zinc-800">
          {onBack && (
            <button
              onClick={onBack}
              aria-label="Back to commands"
              className="shrink-0 p-1 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors"
            >
              <ArrowLeft size={14} />
            </button>
          )}
          <Search size={15} className="shrink-0 text-gray-400 dark:text-zinc-500" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => { setQuery(e.target.value); setActiveIdx(0); }}
            onKeyDown={handleKeyDown}
            placeholder="Search runs, sessions, tasks, approvals, traces, prompts…"
            aria-label="Search entities"
            className="flex-1 bg-transparent text-[13px] text-gray-900 dark:text-zinc-100 placeholder-zinc-600 outline-none"
          />
          {isLoading && enabled && (
            <Loader2 size={13} className="shrink-0 text-gray-400 dark:text-zinc-600 animate-spin" />
          )}
          {query && (
            <button
              onClick={() => { setQuery(''); setActiveIdx(0); inputRef.current?.focus(); }}
              aria-label="Clear search"
              className="shrink-0 p-0.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors"
            >
              ×
            </button>
          )}
        </div>

        {/* Results */}
        <div className="max-h-[32rem] overflow-y-auto">
          {!enabled ? (
            /* Prompt when query is too short */
            <div className="py-10 flex flex-col items-center gap-2 text-center">
              <Search size={22} className="text-gray-300 dark:text-zinc-600" />
              <p className="text-[13px] text-gray-400 dark:text-zinc-600">
                Type at least 2 characters to search
              </p>
              <p className="text-[11px] text-gray-300 dark:text-zinc-600">
                Searches across runs, sessions, tasks, approvals, traces and prompts
              </p>
            </div>
          ) : isLoading ? (
            <div className="py-10 flex items-center justify-center gap-2 text-gray-400 dark:text-zinc-600">
              <Loader2 size={14} className="animate-spin" />
              <span className="text-[13px]">Searching…</span>
            </div>
          ) : totalResults === 0 ? (
            <div className="py-10 flex flex-col items-center gap-2 text-center">
              <p className="text-[13px] text-gray-400 dark:text-zinc-600">
                No results for &ldquo;{debouncedQuery}&rdquo;
              </p>
              <p className="text-[11px] text-gray-300 dark:text-zinc-600">
                Try a shorter query or different entity type
              </p>
            </div>
          ) : (
            <div className="p-2 space-y-0.5">
              {ENTITY_ORDER.map(type => {
                const items = grouped.get(type);
                if (!items?.length) return null;
                const startIdx = runningIdx;
                runningIdx += items.length;
                const cfg = ENTITY_CONFIG[type];

                return (
                  <div key={type} className="mb-1">
                    {/* Group header */}
                    <div className="flex items-center gap-2 px-3 py-1">
                      <cfg.icon size={11} className="text-gray-400 dark:text-zinc-600 shrink-0" />
                      <span className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-widest">
                        {cfg.label}
                      </span>
                      <span className="text-[10px] text-gray-300 dark:text-zinc-600">{items.length}</span>
                    </div>
                    {/* Results */}
                    {items.map((result, i) => (
                      <ResultRow
                        key={`${type}-${result.id}`}
                        result={result}
                        active={startIdx + i === clampedIdx}
                        onSelect={() => navigate(result)}
                      />
                    ))}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Footer */}
        {enabled && totalResults > 0 && (
          <div className="flex items-center gap-3 px-4 py-2 border-t border-gray-200 dark:border-zinc-800 text-[10px] text-gray-400 dark:text-zinc-600 font-mono select-none">
            <span>{totalResults} result{totalResults !== 1 ? 's' : ''}</span>
            <span className="flex items-center gap-1">↑↓ navigate</span>
            <span className="flex items-center gap-1">↵ open</span>
            {onBack && <span>esc back</span>}
            <span className="ml-auto">all entities</span>
          </div>
        )}
      </div>
    </div>
  );
}
