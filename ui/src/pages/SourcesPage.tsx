import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { RefreshCw, Loader2, ServerCrash, Database, ChevronDown, ChevronRight } from 'lucide-react';
import { clsx } from 'clsx';
import { StatCard } from '../components/StatCard';
import { defaultApi } from '../lib/api';
import type { SourceRecord, SourceQualityRecord } from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtDate(ms: number | null | undefined): string {
  if (!ms) return '—';
  const d = new Date(ms);
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

function fmtRelative(ms: number | null | undefined): string {
  if (!ms) return 'Never';
  const diff = Date.now() - ms;
  const mins  = Math.floor(diff / 60_000);
  const hours = Math.floor(mins / 60);
  const days  = Math.floor(hours / 24);
  if (days  > 0) return `${days}d ago`;
  if (hours > 0) return `${hours}h ago`;
  if (mins  > 0) return `${mins}m ago`;
  return 'Just now';
}

/** Infer a display type from the source_id prefix/pattern. */
function inferType(sourceId: string): string {
  const lower = sourceId.toLowerCase();
  if (lower.startsWith('web/')  || lower.includes('http'))  return 'Web';
  if (lower.startsWith('file/') || lower.includes('.md') || lower.includes('.txt')) return 'File';
  if (lower.startsWith('git/'))  return 'Git';
  if (lower.startsWith('db/') || lower.includes('sql'))     return 'Database';
  if (lower.startsWith('api/'))  return 'API';
  return 'Document';
}

/** Freshness status based on last_ingested_at. */
function sourceStatus(src: SourceRecord): { label: string; color: string } {
  if (!src.last_ingested_at_ms) {
    return { label: 'Pending', color: 'text-gray-400 dark:text-zinc-500 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700' };
  }
  const age = Date.now() - src.last_ingested_at_ms;
  const days = age / 86_400_000;
  if (days < 1)  return { label: 'Fresh',   color: 'text-emerald-400 bg-emerald-400/10 border-emerald-400/20' };
  if (days < 7)  return { label: 'Recent',  color: 'text-amber-400 bg-amber-400/10 border-amber-400/20' };
  return           { label: 'Stale',   color: 'text-red-400 bg-red-400/10 border-red-400/20' };
}

// ── Quality bar ───────────────────────────────────────────────────────────────

function QualityBar({ score }: { score: number }) {
  const pct   = Math.round(Math.min(score, 1) * 100);
  const color = pct >= 70 ? 'bg-emerald-500' : pct >= 40 ? 'bg-amber-500' : 'bg-red-500';
  return (
    <div className="flex items-center gap-2">
      <div className="w-16 h-1 rounded-full bg-gray-100 dark:bg-zinc-800">
        <div className={clsx('h-1 rounded-full', color)} style={{ width: `${pct}%` }} />
      </div>
      <span className="text-[11px] tabular-nums text-gray-400 dark:text-zinc-500 w-7 text-right">{pct}%</span>
    </div>
  );
}

// ── Quality detail panel ──────────────────────────────────────────────────────

function QualityPanel({ sourceId }: { sourceId: string }) {
  const { data, isLoading, isError } = useQuery<SourceQualityRecord>({
    queryKey: ['source-quality', sourceId],
    queryFn:  () => defaultApi.getSourceQuality(sourceId),
    staleTime: 30_000,
  });

  if (isLoading) return (
    <div className="flex items-center gap-2 px-6 py-3 text-gray-400 dark:text-zinc-600 text-[12px]">
      <Loader2 size={12} className="animate-spin" /> Loading quality metrics…
    </div>
  );
  if (isError || !data) return (
    <div className="px-6 py-3 text-[12px] text-gray-400 dark:text-zinc-600 italic">Quality data unavailable.</div>
  );

  const metrics: { label: string; value: string }[] = [
    { label: 'Chunks',          value: data.chunk_count.toLocaleString() },
    { label: 'Retrievals',      value: data.total_retrievals.toLocaleString() },
    { label: 'Credibility',     value: `${(data.credibility_score * 100).toFixed(0)}%` },
    { label: 'Avg Rating',      value: data.avg_rating != null ? data.avg_rating.toFixed(2) : '—' },
  ];

  return (
    <div className="px-6 py-3 border-t border-gray-200/60 dark:border-zinc-800/60 bg-white dark:bg-zinc-950/40">
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-x-8 gap-y-2">
        {metrics.map(({ label, value }) => (
          <div key={label}>
            <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">{label}</p>
            <p className="text-[13px] font-semibold text-gray-800 dark:text-zinc-200 tabular-nums">{value}</p>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── Source row ────────────────────────────────────────────────────────────────

function SourceRow({ source, even, expanded, onToggle }: {
  source: SourceRecord;
  even: boolean;
  expanded: boolean;
  onToggle: () => void;
}) {
  const status = sourceStatus(source);

  return (
    <div className={clsx('border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0', even ? 'bg-gray-50 dark:bg-zinc-900' : 'bg-gray-50/50 dark:bg-zinc-900/50')}>
      {/* Main row */}
      <div
        className="flex items-center gap-3 px-4 h-10 cursor-pointer hover:bg-white/[0.02] transition-colors"
        onClick={onToggle}
      >
        {/* Expand icon */}
        <span className="shrink-0 text-gray-400 dark:text-zinc-600">
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </span>

        {/* Source ID */}
        <span className="flex-1 min-w-0 text-[12px] font-mono text-gray-700 dark:text-zinc-300 truncate" title={source.source_id}>
          {source.source_id}
        </span>

        {/* Type */}
        <span className="w-20 shrink-0 text-[11px] text-gray-400 dark:text-zinc-500">
          {inferType(source.source_id)}
        </span>

        {/* Status */}
        <span className={clsx('w-16 shrink-0 inline-flex items-center px-1.5 py-0.5 rounded border text-[10px] font-medium', status.color)}>
          {status.label}
        </span>

        {/* Last sync */}
        <span className="w-24 shrink-0 text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums" title={fmtDate(source.last_ingested_at_ms)}>
          {fmtRelative(source.last_ingested_at_ms)}
        </span>

        {/* Records */}
        <span className="w-16 shrink-0 text-right text-[12px] tabular-nums text-gray-700 dark:text-zinc-300">
          {source.document_count.toLocaleString()}
        </span>

        {/* Quality bar */}
        <div className="w-28 shrink-0 flex justify-end">
          <QualityBar score={source.avg_quality_score} />
        </div>
      </div>

      {/* Expanded quality panel */}
      {expanded && <QualityPanel sourceId={source.source_id} />}
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function SourcesPage() {
  const [expanded, setExpanded] = useState<string | null>(null);

  const { data: sources, isLoading, isError, error, refetch, isFetching } = useQuery<SourceRecord[]>({
    queryKey: ['sources'],
    queryFn:  () => defaultApi.getSources(),
    refetchInterval: 60_000,
  });

  const rows = sources ?? [];

  const totalDocs   = rows.reduce((s, r) => s + r.document_count, 0);
  const avgQuality  = rows.length > 0
    ? rows.reduce((s, r) => s + r.avg_quality_score, 0) / rows.length
    : 0;

  if (isError) return (
    <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
      <ServerCrash size={32} className="text-red-500" />
      <p className="text-[13px] text-gray-700 dark:text-zinc-300 font-medium">Failed to load sources</p>
      <p className="text-[12px] text-gray-400 dark:text-zinc-500">
        {error instanceof Error ? error.message : 'Unknown error'}
      </p>
      <button onClick={() => refetch()} className="mt-1 px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-700 dark:text-zinc-300 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors">
        Retry
      </button>
    </div>
  );

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Sources
          {!isLoading && (
            <span className="ml-2 text-[12px] text-gray-400 dark:text-zinc-500 font-normal">{rows.length}</span>
          )}
        </span>
        <button
          onClick={() => refetch()}
          disabled={isFetching}
          className="ml-auto flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={11} className={isFetching ? 'animate-spin' : ''} />
          Refresh
        </button>
      </div>

      {/* Stat strip */}
      {!isLoading && rows.length > 0 && (
        <div className="grid grid-cols-3 gap-x-6 px-5 py-3 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <StatCard compact variant="info" label="Sources"     value={rows.length} />
          <StatCard compact variant="info" label="Total Docs"  value={totalDocs.toLocaleString()} />
          <StatCard compact variant="info" label="Avg Quality" value={`${(avgQuality * 100).toFixed(0)}%`} />
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-x-auto overflow-y-auto">
        {isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading…</span>
          </div>
        ) : rows.length === 0 ? (
          <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
              <Database size={24} className="text-gray-400 dark:text-zinc-500" />
            </div>
            <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No sources registered</p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-xs">
              Sources appear after documents are ingested. Use the Memory page to add documents.
            </p>
          </div>
        ) : (
          <div>
            {/* Column headers */}
            <div className="flex items-center gap-3 px-4 h-8 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 sticky top-0">
              <span className="w-4 shrink-0" />
              <span className="flex-1 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Source ID</span>
              <span className="w-20 shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Type</span>
              <span className="w-16 shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Status</span>
              <span className="w-24 shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Last Sync</span>
              <span className="w-16 shrink-0 text-right text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Records</span>
              <span className="w-28 shrink-0 text-right text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Quality</span>
            </div>

            {rows.map((source, i) => (
              <SourceRow
                key={source.source_id}
                source={source}
                even={i % 2 === 0}
                expanded={expanded === source.source_id}
                onToggle={() => setExpanded(v => v === source.source_id ? null : source.source_id)}
              />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export default SourcesPage;
