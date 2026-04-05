import { useState, useRef, type FormEvent } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Search, Loader2, X, RefreshCw } from 'lucide-react';
import { HelpTooltip } from '../components/HelpTooltip';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import type { MemoryChunkResult, SourceRecord } from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function truncate(s: string, n: number) {
  return s.length > n ? `${s.slice(0, n)}…` : s;
}

function fmtScore(n: number) {
  return `${Math.round(Math.min(n, 1) * 100)}%`;
}

// ── Score bar — compact 4px height ───────────────────────────────────────────

function ScoreBar({ score }: { score: number }) {
  const pct  = Math.round(Math.min(score, 1) * 100);
  const color = pct >= 70 ? 'bg-emerald-500' : pct >= 40 ? 'bg-amber-500' : 'bg-red-500';
  return (
    <div className="flex items-center gap-2">
      <div className="w-20 h-1 rounded-full bg-zinc-800">
        <div className={clsx('h-1 rounded-full', color)} style={{ width: `${pct}%` }} />
      </div>
      <span className="text-[11px] tabular-nums text-zinc-500 w-7 text-right">{pct}%</span>
    </div>
  );
}

// ── Result row ────────────────────────────────────────────────────────────────

function ResultRow({ result, rank, even }: { result: MemoryChunkResult; rank: number; even: boolean }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className={clsx("px-4 py-3 border-b border-zinc-800/50 last:border-0", even ? "bg-zinc-900" : "bg-zinc-900/50")}>
      {/* Top row: rank + source + score */}
      <div className="flex items-start justify-between gap-4 mb-1.5">
        <div className="flex items-center gap-2 min-w-0">
          <span className="shrink-0 text-[10px] font-mono text-zinc-600 w-4 tabular-nums">{rank}</span>
          <span className="text-[11px] font-mono text-zinc-500 truncate" title={result.chunk.source_id}>
            {truncate(result.chunk.source_id, 32)}
          </span>
          <span className="text-[10px] text-zinc-700 font-mono shrink-0">
            ·pos {result.chunk.position}
          </span>
        </div>
        <ScoreBar score={result.score} />
      </div>

      {/* Text snippet */}
      <p
        className={clsx("text-xs text-zinc-300 leading-relaxed cursor-pointer", !expanded && "line-clamp-2")}
        onClick={() => setExpanded(v => !v)}
      >
        {result.chunk.text}
      </p>

      {/* Breakdown + expand */}
      <div className="flex items-center justify-between mt-1.5">
        <div className="flex gap-3 text-[10px] text-zinc-600">
          <span>lex <span className="text-zinc-500">{fmtScore(result.breakdown.lexical_relevance)}</span></span>
          <span>fresh <span className="text-zinc-500">{fmtScore(result.breakdown.freshness)}</span></span>
          {result.breakdown.source_credibility > 0 && (
            <span>cred <span className="text-zinc-500">{fmtScore(result.breakdown.source_credibility)}</span></span>
          )}
        </div>
        {result.chunk.text.length > 120 && (
          <button
            onClick={() => setExpanded(v => !v)}
            className="text-[10px] text-indigo-500 hover:text-indigo-400 transition-colors"
          >
            {expanded ? 'collapse' : 'expand'}
          </button>
        )}
      </div>
    </div>
  );
}

// ── Source row ────────────────────────────────────────────────────────────────

function SourceRow({ source, even }: { source: SourceRecord; even: boolean }) {
  return (
    <div className={clsx("flex items-center justify-between px-4 h-9 hover:bg-white/5 transition-colors", even ? "bg-zinc-900" : "bg-zinc-900/50")}>
      <span className="text-xs font-mono text-zinc-300 truncate" title={source.source_id}>
        {source.source_id}
      </span>
      <div className="flex items-center gap-4 shrink-0 text-[11px] text-zinc-500">
        <span><span className="text-zinc-300 tabular-nums">{source.document_count}</span> docs</span>
        {source.avg_quality_score > 0 && (
          <span>score <span className="text-zinc-300 tabular-nums">{(source.avg_quality_score * 100).toFixed(0)}%</span></span>
        )}
        {source.last_ingested_at_ms != null && source.last_ingested_at_ms > 0 && (
          <span className="font-mono text-zinc-600">
            {new Date(source.last_ingested_at_ms).toLocaleDateString()}
          </span>
        )}
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function MemoryPage() {
  const [query, setQuery]       = useState('');
  const [submitted, setSubmitted] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);

  const { data: searchData, isFetching, isError: isSearchError, error: searchError } = useQuery({
    queryKey: ['memory-search', submitted],
    queryFn: () => defaultApi.searchMemory({ query_text: submitted, limit: 20 }),
    enabled: submitted.length > 0,
    staleTime: 30_000,
  });

  const { data: sources, isError: isSourcesError, refetch: refetchSources } = useQuery({
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

  return (
    <div className="p-6 space-y-5">
      {/* ── Search bar ──────────────────────────────────────────────────── */}
      <form onSubmit={handleSearch} className="flex gap-2">
        {/* Hint label */}
        <div className="relative flex-1">
          <Search size={13} className="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-600 pointer-events-none" />
          <input
            ref={inputRef}
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder="Search knowledge store…"
            className="w-full rounded-md bg-zinc-900 border border-zinc-800 pl-8 pr-8 h-9
                       text-sm text-zinc-200 placeholder-zinc-600
                       focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500
                       transition-colors"
          />
          {query && (
            <button type="button" onClick={clearSearch}
              className="absolute right-2.5 top-1/2 -translate-y-1/2 text-zinc-600 hover:text-zinc-400">
              <X size={12} />
            </button>
          )}
        </div>
        <button type="submit" disabled={!query.trim() || isFetching}
          className="h-9 px-4 rounded-md bg-indigo-600 hover:bg-indigo-500
                     disabled:bg-zinc-800 disabled:text-zinc-600 text-white text-xs font-medium
                     flex items-center gap-1.5 transition-colors">
          {isFetching ? <Loader2 size={13} className="animate-spin" /> : <Search size={13} />}
          Search
        </button>
        <HelpTooltip
          text="Lexical search over ingested documents. Shorter, specific phrases work best. Results are ranked by relevance, freshness, and source credibility."
          placement="left"
          className="self-center"
        />
      </form>

      {/* ── Search results ──────────────────────────────────────────────── */}
      {submitted && (
        <div className="bg-zinc-900 border border-zinc-800 rounded-lg overflow-hidden">
          {/* Header */}
          <div className="flex items-center justify-between px-4 h-9 border-b border-zinc-800 bg-zinc-900">
            <div className="flex items-center gap-2">
              <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
                Results
              </p>
              <span className="text-[11px] text-zinc-700">for</span>
              <span className="text-[11px] font-mono text-zinc-400">"{submitted}"</span>
              {!isFetching && (
                <span className="text-[11px] text-zinc-700">({results.length})</span>
              )}
            </div>
            {searchData?.diagnostics && (
              <span className="text-[10px] font-mono text-zinc-700">
                {searchData.diagnostics.latency_ms}ms · {searchData.diagnostics.mode_used}
              </span>
            )}
          </div>

          {/* Content */}
          {isFetching ? (
            <div className="flex items-center gap-2 px-4 h-12 text-zinc-600 text-xs">
              <Loader2 size={12} className="animate-spin" /> Searching…
            </div>
          ) : isSearchError ? (
            <div className="px-4 py-3 text-xs text-red-400">
              {searchError instanceof Error ? searchError.message : 'Search failed'}
            </div>
          ) : results.length === 0 ? (
            <div className="px-4 py-8 text-center text-xs text-zinc-600">
              No results for "{submitted}"
            </div>
          ) : (
            <div>
              {/* Column headers */}
              <div className="flex items-center justify-between px-4 h-8 border-b border-zinc-800 bg-zinc-950">
                <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Source · Snippet</span>
                <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Score</span>
              </div>
              {results.map((r, i) => (
                <ResultRow key={r.chunk.chunk_id} result={r} rank={i + 1} even={i % 2 === 0} />
              ))}
            </div>
          )}
        </div>
      )}

      {/* ── Sources panel ───────────────────────────────────────────────── */}
      <div className="bg-zinc-900 border border-zinc-800 rounded-lg overflow-hidden">
        <div className="flex items-center justify-between px-4 h-9 border-b border-zinc-800">
          <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
            Sources
            {sources && sources.length > 0 && (
              <span className="ml-1.5 text-zinc-700 normal-case tracking-normal font-normal">
                ({sources.length})
              </span>
            )}
          </p>
          <button onClick={() => refetchSources()}
            className="flex items-center gap-1 text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors">
            <RefreshCw size={11} /> Refresh
          </button>
        </div>

        {/* Column headers */}
        <div className="flex items-center justify-between px-4 h-8 border-b border-zinc-800 bg-zinc-950">
          <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Source ID</span>
          <span className="text-[10px] text-zinc-600 uppercase tracking-wider">Docs · Score · Ingested</span>
        </div>

        {isSourcesError ? (
          <div className="px-4 py-3 text-xs text-zinc-600 italic">Could not load sources.</div>
        ) : !sources || sources.length === 0 ? (
          <div className="px-4 py-8 text-center text-xs text-zinc-600">
            No sources registered — POST to /v1/memory/ingest to add documents
          </div>
        ) : (
          <div>
            {sources.map((s, i) => (
              <SourceRow key={s.source_id} source={s} even={i % 2 === 0} />
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export default MemoryPage;
