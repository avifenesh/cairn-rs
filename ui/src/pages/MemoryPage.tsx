import { useState, useRef, type FormEvent } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Search, Loader2, X, RefreshCw, Database, Upload } from 'lucide-react';
import { HelpTooltip } from '../components/HelpTooltip';
import { FeatureEmptyState } from '../components/FeatureEmptyState';
import { clsx } from 'clsx';
import { defaultApi, ApiError } from '../lib/api';
import { useScope } from '../hooks/useScope';
import { useToast } from '../components/Toast';
import type { MemoryChunkResult, SourceRecord } from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function truncate(s: string, n: number) {
  return s.length > n ? `${s.slice(0, n)}…` : s;
}

/**
 * Detect the "no embedding provider configured" condition from a structured
 * ApiError. The current `memory_search_handler` returns 400 `bad_request`
 * with no dedicated error code for missing provider config, so in practice
 * this branch only fires if/when the backend is extended to emit
 * `code: 'provider_unavailable'`. We key strictly on that stable code so a
 * generic 503 (proxy outage, backend maintenance) correctly falls into the
 * "Search failed" branch instead of misleading the operator toward the
 * Providers page.
 */
function isProviderUnavailable(err: unknown): boolean {
  return err instanceof ApiError && err.code === 'provider_unavailable';
}

function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  return String(err ?? 'Unknown error');
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
      <div className="w-20 h-1 rounded-full bg-gray-100 dark:bg-zinc-800">
        <div className={clsx('h-1 rounded-full', color)} style={{ width: `${pct}%` }} />
      </div>
      <span className="text-[11px] tabular-nums text-gray-400 dark:text-zinc-500 w-7 text-right">{pct}%</span>
    </div>
  );
}

// ── Result row ────────────────────────────────────────────────────────────────

function ResultRow({ result, rank, even }: { result: MemoryChunkResult; rank: number; even: boolean }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className={clsx("px-4 py-3 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0", even ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50")}>
      {/* Top row: rank + source + score */}
      <div className="flex items-start justify-between gap-4 mb-1.5">
        <div className="flex items-center gap-2 min-w-0">
          <span className="shrink-0 text-[10px] font-mono text-gray-400 dark:text-zinc-600 w-4 tabular-nums">{rank}</span>
          <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-500 truncate" title={result.chunk.source_id}>
            {truncate(result.chunk.source_id, 32)}
          </span>
          <span className="text-[10px] text-gray-300 dark:text-zinc-600 font-mono shrink-0">
            ·pos {result.chunk.position}
          </span>
        </div>
        <ScoreBar score={result.score} />
      </div>

      {/* Text snippet */}
      <p
        className={clsx("text-xs text-gray-700 dark:text-zinc-300 leading-relaxed cursor-pointer", !expanded && "line-clamp-2")}
        onClick={() => setExpanded(v => !v)}
      >
        {result.chunk.text}
      </p>

      {/* Breakdown + expand */}
      <div className="flex items-center justify-between mt-1.5">
        <div className="flex gap-3 text-[10px] text-gray-400 dark:text-zinc-600">
          <span>lex <span className="text-gray-400 dark:text-zinc-500">{fmtScore(result.breakdown.lexical_relevance)}</span></span>
          <span>fresh <span className="text-gray-400 dark:text-zinc-500">{fmtScore(result.breakdown.freshness)}</span></span>
          {result.breakdown.source_credibility > 0 && (
            <span>cred <span className="text-gray-400 dark:text-zinc-500">{fmtScore(result.breakdown.source_credibility)}</span></span>
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
    <div className={clsx("flex items-center justify-between px-4 h-9 hover:bg-white/5 transition-colors", even ? "bg-gray-50 dark:bg-zinc-900" : "bg-gray-50/50 dark:bg-zinc-900/50")}>
      <span className="text-xs font-mono text-gray-700 dark:text-zinc-300 truncate" title={source.source_id}>
        {source.source_id}
      </span>
      <div className="flex items-center gap-4 shrink-0 text-[11px] text-gray-400 dark:text-zinc-500">
        <span><span className="text-gray-700 dark:text-zinc-300 tabular-nums">{source.document_count}</span> docs</span>
        {source.avg_quality_score > 0 && (
          <span>score <span className="text-gray-700 dark:text-zinc-300 tabular-nums">{(source.avg_quality_score * 100).toFixed(0)}%</span></span>
        )}
        {source.last_ingested_at_ms != null && source.last_ingested_at_ms > 0 && (
          <span className="font-mono text-gray-400 dark:text-zinc-600">
            {new Date(source.last_ingested_at_ms).toLocaleDateString()}
          </span>
        )}
      </div>
    </div>
  );
}

// ── Ingest form ───────────────────────────────────────────────────────────────

function IngestForm() {
  const toast = useToast();
  const qc    = useQueryClient();
  const [scope] = useScope();

  const [sourceId,   setSourceId]   = useState('');
  const [documentId, setDocumentId] = useState('');
  const [content,    setContent]    = useState('');
  const [sourceType, setSourceType] = useState('');

  const { mutate, isPending } = useMutation({
    mutationFn: () => defaultApi.ingestMemory({
      source_id:    sourceId.trim(),
      document_id:  documentId.trim(),
      content,
      ...(sourceType.trim() ? { source_type: sourceType.trim() } : {}),
      // Pass scope explicitly so the ingest is pinned to the scope the
      // operator saw in the tooltip at submit time — no drift if the
      // active scope in localStorage flips before the mutation resolves.
      tenant_id:    scope.tenant_id,
      workspace_id: scope.workspace_id,
      project_id:   scope.project_id,
    }),
    onSuccess: (res) => {
      toast.success(
        `Ingested document ${res.document_id} into source ${res.source_id} (${res.chunk_count} chunk${res.chunk_count === 1 ? '' : 's'}).`,
      );
      setDocumentId('');
      setContent('');
      // Invalidate both queries. The page-level sources query is keyed
      // ['sources', tenant_id, workspace_id, project_id]; the partial
      // queryKey `['sources']` still matches all scoped variants, so
      // the panel refetches without a parent-level refetch call.
      qc.invalidateQueries({ queryKey: ['memory-search'] });
      qc.invalidateQueries({ queryKey: ['sources'] });
    },
    onError: (e: unknown) => {
      toast.error(e instanceof Error ? e.message : 'Ingest failed.');
    },
  });

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!sourceId.trim() || !documentId.trim() || !content.trim()) return;
    mutate();
  }

  return (
    <form
      onSubmit={submit}
      className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg p-4 space-y-3"
    >
      <div className="flex items-center gap-2">
        <Upload size={13} className="text-indigo-500" />
        <p className="text-[11px] font-medium text-gray-500 dark:text-zinc-400 uppercase tracking-wider">
          Ingest Document
        </p>
        <HelpTooltip
          text={`Ingests a single document into the knowledge store under the current scope (${scope.tenant_id}/${scope.workspace_id}/${scope.project_id}). Source is created on first ingest.`}
          placement="right"
        />
      </div>

      <div className="grid grid-cols-2 gap-2">
        <input
          value={sourceId}
          onChange={e => setSourceId(e.target.value)}
          placeholder="source_id (e.g. docs/handbook)"
          className="rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 h-8 text-xs font-mono text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
          required
        />
        <input
          value={documentId}
          onChange={e => setDocumentId(e.target.value)}
          placeholder="document_id (e.g. onboarding.md)"
          className="rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 h-8 text-xs font-mono text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
          required
        />
      </div>

      <input
        value={sourceType}
        onChange={e => setSourceType(e.target.value)}
        placeholder="source_type (optional: web, file, api, …)"
        className="w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 h-8 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
      />

      <textarea
        value={content}
        onChange={e => setContent(e.target.value)}
        placeholder="Document content to ingest…"
        rows={4}
        className="w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 py-2 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 resize-y"
        required
      />

      <div className="flex justify-end">
        <button
          type="submit"
          disabled={isPending || !sourceId.trim() || !documentId.trim() || !content.trim()}
          className="h-8 px-3 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:bg-gray-200 dark:disabled:bg-zinc-800 disabled:text-gray-400 dark:disabled:text-zinc-600 text-white text-xs font-medium flex items-center gap-1.5 transition-colors"
        >
          {isPending ? <Loader2 size={12} className="animate-spin" /> : <Upload size={12} />}
          Ingest
        </button>
      </div>
    </form>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function MemoryPage() {
  const [query, setQuery]       = useState('');
  const [submitted, setSubmitted] = useState('');
  const inputRef = useRef<HTMLInputElement>(null);
  const [scope] = useScope();

  const { data: searchData, isFetching, isError: isSearchError, error: searchError } = useQuery({
    queryKey: ['memory-search', scope.tenant_id, scope.workspace_id, scope.project_id, submitted],
    queryFn: () => defaultApi.searchMemory({
      tenant_id:    scope.tenant_id,
      workspace_id: scope.workspace_id,
      project_id:   scope.project_id,
      query_text:   submitted,
      limit:        20,
    }),
    enabled: submitted.length > 0,
    staleTime: 30_000,
  });

  // Scope-keyed — same pattern as SourcesPage — so changing scope does not
  // bleed sources from a previous tenant/workspace/project into the panel.
  const { data: sources, isError: isSourcesError, error: sourcesError, refetch: refetchSources } = useQuery({
    queryKey: ['sources', scope.tenant_id, scope.workspace_id, scope.project_id],
    // Pass scope explicitly so the outgoing query params are guaranteed to
    // match the queryKey even if the API client's implicit scope
    // (localStorage) has drifted from the hook state. getSources is GET, so
    // scope travels as `?tenant_id=…&workspace_id=…&project_id=…` on the
    // wire, not in a request body.
    queryFn: () => defaultApi.getSources({
      tenant_id:    scope.tenant_id,
      workspace_id: scope.workspace_id,
      project_id:   scope.project_id,
    }),
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
          <Search size={13} className="absolute left-3 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
          <input
            ref={inputRef}
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder="Search knowledge store…"
            className="w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 pl-8 pr-8 h-9
                       text-sm text-gray-800 dark:text-zinc-200 placeholder-zinc-600
                       focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500
                       transition-colors"
          />
          {query && (
            <button type="button" onClick={clearSearch}
              className="absolute right-2.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400">
              <X size={12} />
            </button>
          )}
        </div>
        <button type="submit" disabled={!query.trim() || isFetching}
          className="h-9 px-4 rounded-md bg-indigo-600 hover:bg-indigo-500
                     disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 text-white text-xs font-medium
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

      {/* ── Ingest form ─────────────────────────────────────────────────── */}
      <IngestForm />

      {/* ── Search results ──────────────────────────────────────────────── */}
      {submitted && (
        <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden">
          {/* Header */}
          <div className="flex items-center justify-between px-4 h-9 border-b border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900">
            <div className="flex items-center gap-2">
              <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
                Results
              </p>
              <span className="text-[11px] text-gray-300 dark:text-zinc-600">for</span>
              <span className="text-[11px] font-mono text-gray-500 dark:text-zinc-400">"{submitted}"</span>
              {!isFetching && (
                <span className="text-[11px] text-gray-300 dark:text-zinc-600">({results.length})</span>
              )}
            </div>
            {searchData?.diagnostics && (
              <span className="text-[10px] font-mono text-gray-300 dark:text-zinc-600">
                {searchData.diagnostics.latency_ms}ms · {searchData.diagnostics.mode_used}
              </span>
            )}
          </div>

          {/* Content */}
          {isFetching ? (
            <div className="flex items-center gap-2 px-4 h-12 text-gray-400 dark:text-zinc-600 text-xs">
              <Loader2 size={12} className="animate-spin" /> Searching…
            </div>
          ) : isSearchError ? (
            // Distinguish "no embedding provider configured" (backend 503 /
            // code=provider_unavailable) from generic errors (network, 4xx,
            // other 5xx). Detection is based on the structured ApiError
            // (status/code), not regex-matching human-readable text.
            isProviderUnavailable(searchError) ? (
              <FeatureEmptyState
                icon={<Database size={20} className="text-gray-400 dark:text-zinc-500" />}
                title="No embedding provider configured"
                description="Memory search requires an embedding provider to generate vector representations. Add one on the Providers page, then ingest documents with the form above."
                actionLabel="Go to Providers"
                actionHref="#providers"
              />
            ) : (
              <div className="px-4 py-6 text-center">
                <p className="text-[12px] font-medium text-red-400">Search failed</p>
                <p className="mt-1 text-[11px] text-gray-400 dark:text-zinc-500 font-mono break-words">
                  {errorMessage(searchError)}
                </p>
              </div>
            )
          ) : results.length === 0 ? (
            <div className="px-4 py-8 text-center text-xs text-gray-400 dark:text-zinc-600">
              No results for "{submitted}"
            </div>
          ) : (
            <div>
              {/* Column headers */}
              <div className="flex items-center justify-between px-4 h-8 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Source · Snippet</span>
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Score</span>
              </div>
              {results.map((r, i) => (
                <ResultRow key={r.chunk.chunk_id} result={r} rank={i + 1} even={i % 2 === 0} />
              ))}
            </div>
          )}
        </div>
      )}

      {/* ── Sources panel ───────────────────────────────────────────────── */}
      <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden">
        <div className="flex items-center justify-between px-4 h-9 border-b border-gray-200 dark:border-zinc-800">
          <p className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
            Sources
            {sources && sources.length > 0 && (
              <span className="ml-1.5 text-gray-300 dark:text-zinc-600 normal-case tracking-normal font-normal">
                ({sources.length})
              </span>
            )}
          </p>
          <button onClick={() => refetchSources()}
            className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors">
            <RefreshCw size={11} /> Refresh
          </button>
        </div>

        {/* Column headers */}
        <div className="flex items-center justify-between px-4 h-8 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950">
          <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Source ID</span>
          <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Docs · Score · Ingested</span>
        </div>

        {isSourcesError ? (
          // /v1/sources errors are unrelated to embedding providers —
          // show the real error message so operators can debug auth /
          // network / validation failures instead of being sent to the
          // Providers page on unrelated issues.
          <div className="px-4 py-8 text-center">
            <p className="text-[12px] font-medium text-red-400">Failed to load sources</p>
            <p className="mt-1 text-[11px] text-gray-400 dark:text-zinc-500 font-mono break-words">
              {errorMessage(sourcesError)}
            </p>
          </div>
        ) : !sources || sources.length === 0 ? (
          <div className="px-4 py-8 text-center text-xs text-gray-400 dark:text-zinc-600">
            No sources registered — use the ingest form above to add documents
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
