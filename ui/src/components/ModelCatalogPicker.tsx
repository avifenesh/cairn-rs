/**
 * ModelCatalogPicker — operator-facing multi-select over the bundled
 * LiteLLM model catalog (GET /v1/models/catalog).
 *
 * Used by:
 * - ProvidersPage add-provider wizard (step 2: pick supported models)
 * - ProvidersPage inline edit row
 * - CostCalculatorPage model list
 *
 * The caller owns the selected-ids set and is the single source of truth;
 * this component is "controlled". Selecting / deselecting fires
 * `onChange(nextIds)` immediately.
 *
 * Design notes:
 * - Filters + pagination happen server-side (the registry has ≈500 chat
 *   entries, so a single 500-limit fetch per filter combo is fine — no
 *   client-side re-filter once the result lands).
 * - Search is debounced 200 ms to avoid a refetch per keystroke.
 * - Keyboard nav: ↑/↓ moves the focused row, Space toggles select,
 *   Cmd/Ctrl+A selects every *currently visible* row (the user can still
 *   walk pages and re-Cmd+A to select them all — explicit > magic).
 */

import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, Loader2, Search } from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import type {
  ModelCatalogEntry,
  ModelCatalogQuery,
  ModelTier,
} from "../lib/types";

// ── Helpers ──────────────────────────────────────────────────────────────────

function formatCost(cost: number): string {
  if (cost === 0) return "—";
  if (cost < 0.01) return `$${cost.toFixed(4)}`;
  return `$${cost.toFixed(2)}`;
}

function formatContext(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}K`;
  return String(n);
}

function useDebounced<T>(value: T, delayMs: number): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const h = setTimeout(() => setV(value), delayMs);
    return () => clearTimeout(h);
  }, [value, delayMs]);
  return v;
}

// ── Capability chip ─────────────────────────────────────────────────────────

interface CapChipProps {
  label:    string;
  active:   boolean;
  onToggle: () => void;
}

function CapChip({ label, active, onToggle }: CapChipProps) {
  return (
    <button
      type="button"
      onClick={onToggle}
      className={clsx(
        "px-2 h-6 rounded-full text-[11px] font-medium transition-colors border",
        active
          ? "bg-indigo-600/20 text-indigo-300 border-indigo-600/60"
          : "text-gray-500 dark:text-zinc-500 border-gray-200 dark:border-zinc-700 hover:border-gray-400 dark:hover:border-zinc-500",
      )}
    >
      {label}
    </button>
  );
}

// ── Props ────────────────────────────────────────────────────────────────────

export interface ModelCatalogPickerProps {
  /** Currently-selected model IDs (controlled). */
  selected: string[];
  /** Fires with the full next set whenever selection changes. */
  onChange: (nextIds: string[]) => void;
  /** Hard cap on fetched page size (default 200). */
  pageSize?: number;
  /** When set, initial provider filter is locked to this value and the
   *  provider dropdown is hidden. Useful when the picker is embedded in a
   *  provider-specific wizard row. */
  lockProvider?: string;
}

// ── Main component ──────────────────────────────────────────────────────────

export function ModelCatalogPicker({
  selected,
  onChange,
  pageSize = 200,
  lockProvider,
}: ModelCatalogPickerProps) {
  // Filter state
  const [searchRaw,      setSearchRaw]      = useState("");
  const [providerFilter, setProviderFilter] = useState<string>(lockProvider ?? "");
  const [tierFilter,     setTierFilter]     = useState<ModelTier | "">("");

  // Keep providerFilter in sync with the lockProvider prop so the parent
  // can switch kinds (e.g. flipping the add-provider wizard between
  // OpenAI and Anthropic) and see the picker re-target immediately.
  // Without this, the first render wins forever. Unlocking (lockProvider
  // going back to undefined) leaves whatever the user last picked —
  // that's the right behavior for the inline-edit flow where we don't
  // want to erase the operator's filter on every re-render.
  useEffect(() => {
    if (lockProvider !== undefined) setProviderFilter(lockProvider);
  }, [lockProvider]);
  const [needsTools,     setNeedsTools]     = useState(false);
  const [needsReasoning, setNeedsReasoning] = useState(false);
  const [freeOnly,       setFreeOnly]       = useState(false);
  const [maxCostEnabled, setMaxCostEnabled] = useState(false);
  const [maxCost,        setMaxCost]        = useState(5);
  const search = useDebounced(searchRaw.trim(), 200);

  // Fetch providers-with-counts (for the dropdown). Cached by TanStack; the
  // endpoint itself is server-side cached, so this is near-free.
  const providersQ = useQuery({
    queryKey: ["model-catalog-providers"],
    queryFn:  () => defaultApi.listCatalogProviders(),
    staleTime: 300_000,
  });
  const providers = providersQ.data?.providers ?? [];

  // Build the query, send it.
  const params: ModelCatalogQuery = useMemo(() => {
    const p: ModelCatalogQuery = { limit: pageSize };
    if (providerFilter) p.provider = providerFilter;
    if (tierFilter) p.tier = tierFilter;
    if (search) p.search = search;
    if (needsTools) p.supports_tools = true;
    if (needsReasoning) p.reasoning = true;
    if (freeOnly) p.free_only = true;
    if (maxCostEnabled) p.max_cost_per_1m = maxCost;
    return p;
  }, [
    pageSize,
    providerFilter,
    tierFilter,
    search,
    needsTools,
    needsReasoning,
    freeOnly,
    maxCostEnabled,
    maxCost,
  ]);

  const catalogQ = useQuery({
    queryKey: ["model-catalog", params],
    queryFn:  () => defaultApi.listModelCatalog(params),
    staleTime: 60_000,
  });
  const items: ModelCatalogEntry[] = catalogQ.data?.items ?? [];
  const total = catalogQ.data?.total ?? 0;

  // Keyboard navigation state
  const [focusIdx, setFocusIdx] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  useEffect(() => { setFocusIdx(0); }, [items]);

  const selectedSet = useMemo(() => new Set(selected), [selected]);

  const toggle = useCallback(
    (id: string) => {
      const next = new Set(selectedSet);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      onChange(Array.from(next));
    },
    [selectedSet, onChange],
  );

  const selectAllVisible = useCallback(() => {
    const next = new Set(selectedSet);
    for (const it of items) next.add(it.id);
    onChange(Array.from(next));
  }, [items, selectedSet, onChange]);

  const clearVisible = useCallback(() => {
    const next = new Set(selectedSet);
    for (const it of items) next.delete(it.id);
    onChange(Array.from(next));
  }, [items, selectedSet, onChange]);

  const handleKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setFocusIdx(i => Math.min(items.length - 1, i + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setFocusIdx(i => Math.max(0, i - 1));
    } else if (e.key === " ") {
      e.preventDefault();
      const it = items[focusIdx];
      if (it) toggle(it.id);
    } else if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "a") {
      e.preventDefault();
      selectAllVisible();
    }
  };

  return (
    <div className="space-y-3" data-testid="model-catalog-picker">
      {/* ── Filter row ─────────────────────────────────────────────────── */}
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative flex-1 min-w-[220px]">
          <Search size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
          <input
            value={searchRaw}
            onChange={e => setSearchRaw(e.target.value)}
            placeholder="Search models by id, name, or provider…"
            className="w-full rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 pl-7 pr-3 h-8 text-xs text-gray-800 dark:text-zinc-200 placeholder-zinc-500 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
            aria-label="Search models"
          />
        </div>

        {!lockProvider && (
          <select
            value={providerFilter}
            onChange={e => setProviderFilter(e.target.value)}
            className="h-8 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-2 text-xs text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500"
            aria-label="Filter by provider"
          >
            <option value="">All providers ({providers.reduce((n, p) => n + p.count, 0)})</option>
            {providers.map(p => (
              <option key={p.name} value={p.name}>
                {p.name} ({p.count})
              </option>
            ))}
          </select>
        )}

        <select
          value={tierFilter}
          onChange={e => setTierFilter(e.target.value as ModelTier | "")}
          className="h-8 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-2 text-xs text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500"
          aria-label="Filter by tier"
        >
          <option value="">Any tier</option>
          <option value="brain">Brain</option>
          <option value="mid">Mid</option>
          <option value="light">Light</option>
        </select>
      </div>

      {/* ── Capability chips + cost slider ─────────────────────────────── */}
      <div className="flex flex-wrap items-center gap-2">
        <CapChip label="Tools"     active={needsTools}     onToggle={() => setNeedsTools(v => !v)} />
        <CapChip label="Reasoning" active={needsReasoning} onToggle={() => setNeedsReasoning(v => !v)} />
        <CapChip label="Free"      active={freeOnly}       onToggle={() => setFreeOnly(v => !v)} />

        <label className="flex items-center gap-1.5 text-[11px] text-gray-500 dark:text-zinc-500 ml-auto">
          <input
            type="checkbox"
            checked={maxCostEnabled}
            onChange={e => setMaxCostEnabled(e.target.checked)}
            className="accent-indigo-500"
          />
          Max $/1M in
          <input
            type="number"
            disabled={!maxCostEnabled}
            min={0}
            step={0.1}
            value={maxCost}
            onChange={e => setMaxCost(Math.max(0, parseFloat(e.target.value) || 0))}
            className="w-16 h-6 rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-1 text-[11px] font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 disabled:opacity-40"
          />
        </label>
      </div>

      {/* ── Selection summary + bulk actions ───────────────────────────── */}
      <div className="flex items-center justify-between text-[11px] text-gray-500 dark:text-zinc-500">
        <span>
          {catalogQ.isFetching ? (
            <Loader2 size={10} className="inline animate-spin mr-1" />
          ) : null}
          {total.toLocaleString()} model{total === 1 ? "" : "s"} match
          {" · "}
          <span className="text-indigo-400">{selected.length} selected</span>
        </span>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={selectAllVisible}
            disabled={items.length === 0}
            className="px-2 h-6 rounded text-[10px] font-medium text-gray-500 dark:text-zinc-400 border border-gray-200 dark:border-zinc-700 hover:border-indigo-500 disabled:opacity-40"
          >
            Select all visible
          </button>
          <button
            type="button"
            onClick={clearVisible}
            disabled={items.length === 0}
            className="px-2 h-6 rounded text-[10px] font-medium text-gray-500 dark:text-zinc-400 border border-gray-200 dark:border-zinc-700 hover:border-red-500 disabled:opacity-40"
          >
            Clear visible
          </button>
        </div>
      </div>

      {/* ── Result table ───────────────────────────────────────────────── */}
      <div
        ref={listRef}
        tabIndex={0}
        onKeyDown={handleKey}
        className="max-h-[320px] overflow-y-auto rounded-md border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 focus:outline-none focus:ring-1 focus:ring-indigo-500"
        role="listbox"
        aria-multiselectable="true"
      >
        {catalogQ.isError ? (
          <div className="p-4 text-[12px] text-red-400">
            Failed to load model catalog: {String(catalogQ.error)}
          </div>
        ) : items.length === 0 && !catalogQ.isLoading ? (
          <div className="p-4 text-[12px] text-gray-400 dark:text-zinc-500 text-center">
            No models match these filters.
          </div>
        ) : (
          <table className="w-full text-[12px]">
            <thead className="sticky top-0 bg-gray-50 dark:bg-zinc-900 border-b border-gray-200 dark:border-zinc-800">
              <tr>
                <th className="w-8 px-2 py-2" />
                <th className="px-2 py-2 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Model</th>
                <th className="px-2 py-2 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Provider</th>
                <th className="px-2 py-2 text-right text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Context</th>
                <th className="px-2 py-2 text-right text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wide">In $/1M</th>
                <th className="px-2 py-2 text-right text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Out $/1M</th>
                <th className="px-2 py-2 text-center text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wide">Tools</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-gray-100 dark:divide-zinc-800/50">
              {items.map((m, i) => {
                const isSelected = selectedSet.has(m.id);
                const isFree     = m.cost_per_1m_input === 0 && m.cost_per_1m_output === 0;
                const isFocused  = i === focusIdx;
                return (
                  <tr
                    key={m.id}
                    role="option"
                    aria-selected={isSelected}
                    onClick={() => { setFocusIdx(i); toggle(m.id); }}
                    className={clsx(
                      "cursor-pointer transition-colors",
                      isSelected  ? "bg-indigo-950/30"          : "",
                      isFocused   ? "ring-1 ring-inset ring-indigo-500/40" : "",
                      !isSelected && !isFocused ? "hover:bg-gray-50 dark:hover:bg-zinc-900" : "",
                    )}
                  >
                    <td className="px-2 py-1.5 w-8">
                      <div className={clsx(
                        "w-4 h-4 rounded border flex items-center justify-center",
                        isSelected
                          ? "bg-indigo-500 border-indigo-500"
                          : "border-gray-300 dark:border-zinc-600",
                      )}>
                        {isSelected && <Check size={10} className="text-white" />}
                      </div>
                    </td>
                    <td className="px-2 py-1.5">
                      <div className="flex items-center gap-1.5">
                        <span className="font-medium text-gray-800 dark:text-zinc-200">{m.display_name}</span>
                        {isFree && (
                          <span className="text-[9px] bg-emerald-600/20 text-emerald-400 rounded px-1 py-px">Free</span>
                        )}
                        {m.reasoning && (
                          <span className="text-[9px] bg-violet-600/20 text-violet-300 rounded px-1 py-px">Reasoning</span>
                        )}
                      </div>
                      <div className="text-[10px] font-mono text-gray-400 dark:text-zinc-500 mt-0.5">{m.id}</div>
                    </td>
                    <td className="px-2 py-1.5 text-gray-500 dark:text-zinc-400">{m.provider}</td>
                    <td className="px-2 py-1.5 text-right font-mono text-gray-500 dark:text-zinc-400">{formatContext(m.context_len)}</td>
                    <td className="px-2 py-1.5 text-right font-mono text-gray-500 dark:text-zinc-400">
                      {isFree ? <span className="text-emerald-400">Free</span> : formatCost(m.cost_per_1m_input)}
                    </td>
                    <td className="px-2 py-1.5 text-right font-mono text-gray-500 dark:text-zinc-400">
                      {isFree ? <span className="text-emerald-400">Free</span> : formatCost(m.cost_per_1m_output)}
                    </td>
                    <td className="px-2 py-1.5 text-center">
                      {m.supports_tools ? <Check size={12} className="inline text-emerald-400" /> : <span className="text-gray-300 dark:text-zinc-700">—</span>}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

export default ModelCatalogPicker;
