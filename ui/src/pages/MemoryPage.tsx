import { useState, useRef, type FormEvent } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  Search, Database, FileText, Star, Activity,
  Loader2, ServerCrash, Inbox, BarChart2, X,
} from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import type { MemoryChunkResult, SourceQualityRecord, SourceRecord } from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function scoreBar(score: number) {
  const pct = Math.round(Math.min(score, 1) * 100);
  const color =
    pct >= 70 ? 'bg-emerald-500' :
    pct >= 40 ? 'bg-amber-500'  : 'bg-red-500';
  return (
    <div className="flex items-center gap-2">
      <div className="flex-1 h-1.5 rounded-full bg-zinc-800">
        <div className={clsx('h-1.5 rounded-full transition-all', color)} style={{ width: `${pct}%` }} />
      </div>
      <span className="text-xs tabular-nums text-zinc-400 w-8 text-right">{pct}%</span>
    </div>
  );
}

function truncate(s: string, n: number) {
  return s.length > n ? `${s.slice(0, n)}…` : s;
}

// ── Source quality panel ──────────────────────────────────────────────────────

function SourceQualityPanel({ sourceId }: { sourceId: string }) {
  const { data, isLoading } = useQuery<SourceQualityRecord>({
    queryKey: ['source-quality', sourceId],
    queryFn: () => defaultApi.getSourceQuality(sourceId),
    staleTime: 60_000,
    retry: false,
  });

  if (isLoading) return (
    <div className="flex items-center gap-1.5 text-xs text-zinc-600 py-2">
      <Loader2 size={11} className="animate-spin" /> Loading quality…
    </div>
  );
  if (!data) return null;

  return (
    <div className="mt-2 grid grid-cols-2 gap-2 text-xs">
      <div className="rounded-lg bg-zinc-800/60 px-3 py-2">
        <p className="text-zinc-500 mb-1">Credibility</p>
        {scoreBar(data.credibility_score)}
      </div>
      <div className="rounded-lg bg-zinc-800/60 px-3 py-2">
        <p className="text-zinc-500 mb-1">Avg rating</p>
        <p className="text-zinc-200 font-medium">
          {data.avg_rating !== null ? data.avg_rating.toFixed(1) : '—'}
          <span className="text-zinc-500 font-normal"> / 5</span>
        </p>
      </div>
      <div className="rounded-lg bg-zinc-800/60 px-3 py-2">
        <p className="text-zinc-500 mb-1">Retrievals</p>
        <p className="text-zinc-200 font-medium">{data.total_retrievals.toLocaleString()}</p>
      </div>
      <div className="rounded-lg bg-zinc-800/60 px-3 py-2">
        <p className="text-zinc-500 mb-1">Chunks</p>
        <p className="text-zinc-200 font-medium">{data.chunk_count.toLocaleString()}</p>
      </div>
    </div>
  );
}

// ── Search result card ────────────────────────────────────────────────────────

function ResultCard({ result, rank }: { result: MemoryChunkResult; rank: number }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-4 space-y-3">
      {/* Header row */}
      <div className="flex items-start justify-between gap-3">
        <div className="flex items-center gap-2 min-w-0">
          <span className="shrink-0 w-5 h-5 rounded-full bg-indigo-900 text-indigo-300 text-[10px] font-bold flex items-center justify-center">
            {rank}
          </span>
          <span className="text-xs font-mono text-zinc-400 truncate" title={result.chunk.source_id}>
            {truncate(result.chunk.source_id, 28)}
          </span>
        </div>
        <div className="shrink-0 flex items-center gap-2">
          <span className="text-xs text-zinc-500 font-mono">
            pos {result.chunk.position}
          </span>
          {scoreBar(result.score)}
        </div>
      </div>

      {/* Chunk text */}
      <p
        className={clsx(
          'text-sm text-zinc-300 leading-relaxed cursor-pointer',
          !expanded && 'line-clamp-3',
        )}
        onClick={() => setExpanded((v) => !v)}
      >
        {result.chunk.text}
      </p>
      {result.chunk.text.length > 180 && (
        <button
          onClick={() => setExpanded((v) => !v)}
          className="text-xs text-indigo-400 hover:text-indigo-300"
        >
          {expanded ? 'Show less' : 'Show more'}
        </button>
      )}

      {/* Score breakdown */}
      <div className="flex gap-4 text-[11px] text-zinc-500">
        <span>lexical <span className="text-zinc-300">{(result.breakdown.lexical_relevance * 100).toFixed(0)}%</span></span>
        <span>freshness <span className="text-zinc-300">{(result.breakdown.freshness * 100).toFixed(0)}%</span></span>
        {result.breakdown.source_credibility > 0 && (
          <span>credibility <span className="text-zinc-300">{(result.breakdown.source_credibility * 100).toFixed(0)}%</span></span>
        )}
      </div>
    </div>
  );
}

// ── Source row ────────────────────────────────────────────────────────────────

function SourceRow({ source }: { source: SourceRecord }) {
  const [showQuality, setShowQuality] = useState(false);

  return (
    <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="flex items-center gap-3 min-w-0">
          <Database size={14} className="text-zinc-500 shrink-0" />
          <span className="text-sm font-mono text-zinc-200 truncate" title={source.source_id}>
            {source.source_id}
          </span>
        </div>
        <div className="flex items-center gap-4 shrink-0">
          <span className="text-xs text-zinc-500">
            <span className="text-zinc-300">{source.document_count}</span> docs
          </span>
          {source.avg_quality_score > 0 && (
            <span className="text-xs text-zinc-500">
              score <span className="text-zinc-300">{(source.avg_quality_score * 100).toFixed(0)}%</span>
            </span>
          )}
          <button
            onClick={() => setShowQuality((v) => !v)}
            className={clsx(
              'flex items-center gap-1 text-xs transition-colors',
              showQuality ? 'text-indigo-400' : 'text-zinc-500 hover:text-zinc-300',
            )}
          >
            <BarChart2 size={12} />
            {showQuality ? 'Hide' : 'Quality'}
          </button>
        </div>
      </div>
      {showQuality && <SourceQualityPanel sourceId={source.source_id} />}
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function MemoryPage() {
  const [query, setQuery] = useState('');
  const [submitted, setSubmitted] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const {
    data: searchData,
    isFetching,
    isError: isSearchError,
    error: searchError,
  } = useQuery({
    queryKey: ['memory-search', submitted],
    queryFn: () => defaultApi.searchMemory({ query_text: submitted, limit: 20 }),
    enabled: submitted.length > 0,
    staleTime: 30_000,
  });

  const { data: sources, isError: isSourcesError } = useQuery({
    queryKey: ['sources'],
    queryFn: () => defaultApi.getSources(),
    staleTime: 60_000,
  });

  function handleSearch(e: FormEvent) {
    e.preventDefault();
    const q = query.trim();
    if (q) setSubmitted(q);
  }

  function clearSearch() {
    setQuery('');
    setSubmitted('');
    inputRef.current?.focus();
  }

  const results = searchData?.results ?? [];
  const hasResults = results.length > 0;

  return (
    <div className="space-y-6">
      {/* ── Search bar ──────────────────────────────────────────────────── */}
      <form onSubmit={handleSearch} className="flex gap-2">
        <div className="relative flex-1">
          <Search size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-600 pointer-events-none" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search knowledge store…"
            className="w-full rounded-lg bg-zinc-900 ring-1 ring-zinc-800 pl-8 pr-8 py-2.5
                       text-sm text-zinc-100 placeholder-zinc-600
                       focus:outline-none focus:ring-2 focus:ring-indigo-500 transition"
          />
          {query && (
            <button
              type="button"
              onClick={clearSearch}
              className="absolute right-2.5 top-1/2 -translate-y-1/2 text-zinc-600 hover:text-zinc-400"
            >
              <X size={13} />
            </button>
          )}
        </div>
        <button
          type="submit"
          disabled={!query.trim() || isFetching}
          className="px-4 py-2.5 rounded-lg bg-indigo-600 hover:bg-indigo-500
                     disabled:bg-zinc-800 disabled:text-zinc-600 text-white text-sm font-medium
                     flex items-center gap-2 transition"
        >
          {isFetching ? <Loader2 size={14} className="animate-spin" /> : <Search size={14} />}
          Search
        </button>
      </form>

      {/* ── Search results ──────────────────────────────────────────────── */}
      {submitted && (
        <section>
          <div className="flex items-center justify-between mb-3">
            <h2 className="text-xs font-semibold text-zinc-400 flex items-center gap-2">
              <FileText size={13} className="text-indigo-400" />
              Results for <span className="text-zinc-200">"{submitted}"</span>
              {hasResults && (
                <span className="text-zinc-600 font-normal">({results.length})</span>
              )}
            </h2>
            {searchData?.diagnostics && (
              <span className="text-[11px] text-zinc-600 font-mono">
                {searchData.diagnostics.latency_ms}ms · {searchData.diagnostics.mode_used}
              </span>
            )}
          </div>

          {isSearchError ? (
            <div className="flex items-center gap-3 rounded-xl bg-red-950/40 ring-1 ring-red-800/40 p-4">
              <ServerCrash size={18} className="text-red-400 shrink-0" />
              <p className="text-sm text-red-300">
                {searchError instanceof Error ? searchError.message : 'Search failed'}
              </p>
            </div>
          ) : !isFetching && !hasResults ? (
            <div className="flex flex-col items-center justify-center py-10 gap-2 text-zinc-700">
              <Inbox size={28} />
              <p className="text-sm">No results for "{submitted}"</p>
            </div>
          ) : (
            <div className="space-y-3">
              {results.map((r, i) => (
                <ResultCard key={r.chunk.chunk_id} result={r} rank={i + 1} />
              ))}
            </div>
          )}
        </section>
      )}

      {/* ── Sources section ─────────────────────────────────────────────── */}
      <section>
        <h2 className="text-xs font-semibold text-zinc-400 flex items-center gap-2 mb-3">
          <Database size={13} className="text-zinc-500" />
          Sources
          {sources && sources.length > 0 && (
            <span className="text-zinc-600 font-normal">({sources.length})</span>
          )}
        </h2>

        {isSourcesError ? (
          <p className="text-sm text-zinc-600 italic">Could not load sources.</p>
        ) : !sources || sources.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10 gap-2 text-zinc-700 rounded-xl ring-1 ring-zinc-800">
            <Activity size={24} />
            <p className="text-sm">No sources registered</p>
            <p className="text-xs text-zinc-600">
              POST to /v1/memory/ingest to add documents.
            </p>
          </div>
        ) : (
          <div className="space-y-2">
            {sources.map((s) => (
              <SourceRow key={s.source_id} source={s} />
            ))}
          </div>
        )}
      </section>

      {/* ── Source quality hint ─────────────────────────────────────────── */}
      {sources && sources.length > 0 && (
        <p className="text-[11px] text-zinc-700 flex items-center gap-1">
          <Star size={10} />
          Click "Quality" on any source to see credibility score, avg rating, and retrieval counts.
        </p>
      )}
    </div>
  );
}

export default MemoryPage;
