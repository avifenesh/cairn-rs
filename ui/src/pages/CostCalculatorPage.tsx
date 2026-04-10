/**
 * CostCalculatorPage — estimate LLM API costs before committing to a model.
 *
 * Enter expected token counts, pick a model, and see the estimated spend.
 * The pricing comparison table shows all models ranked by cost for those
 * exact token volumes, making it easy to find the cheapest option.
 *
 * Models are fetched dynamically from the provider registry API.
 * Available (configured) providers are shown first, followed by reference
 * entries for unconfigured providers. A hardcoded fallback array is used
 * while the registry is loading.
 */

import { useState, useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { Calculator, ChevronDown, ChevronUp, Coins, Info } from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { ErrorFallback } from "../components/ErrorFallback";
import type { ProviderRegistryEntry } from "../lib/types";

// ── Pricing data ──────────────────────────────────────────────────────────────
// Prices in USD per 1 000 000 tokens (input / output).
// Sources: provider pricing pages, approximate as of early 2026.

export interface ModelPrice {
  id:         string;
  name:       string;
  provider:   string;
  inputPer1M: number;   // USD per 1M input tokens
  outputPer1M: number;  // USD per 1M output tokens
  contextK:   number;   // context window in thousands
  note?:      string;
  /** true when the provider has an API key configured (or doesn't need one). */
  configured?: boolean;
}

// Hardcoded fallback — used while registry is loading.
const FALLBACK_MODELS: ModelPrice[] = [
  // OpenAI (2025–2026)
  { id: "gpt-4.1",            name: "GPT-4.1",              provider: "OpenAI",     inputPer1M:  2.00,  outputPer1M:  8.00,  contextK: 1000 },
  { id: "gpt-4.1-mini",       name: "GPT-4.1 mini",         provider: "OpenAI",     inputPer1M:  0.40,  outputPer1M:  1.60,  contextK: 1000 },
  { id: "gpt-4.1-nano",       name: "GPT-4.1 nano",         provider: "OpenAI",     inputPer1M:  0.10,  outputPer1M:  0.40,  contextK: 1000 },
  { id: "gpt-4o",             name: "GPT-4o",               provider: "OpenAI",     inputPer1M:  2.50,  outputPer1M: 10.00,  contextK: 128  },
  { id: "gpt-4o-mini",        name: "GPT-4o mini",          provider: "OpenAI",     inputPer1M:  0.15,  outputPer1M:  0.60,  contextK: 128  },
  { id: "o3",                 name: "o3",                   provider: "OpenAI",     inputPer1M: 10.00,  outputPer1M: 40.00,  contextK: 200, note: "reasoning" },
  { id: "o4-mini",            name: "o4-mini",              provider: "OpenAI",     inputPer1M:  1.10,  outputPer1M:  4.40,  contextK: 200, note: "reasoning" },
  // Anthropic (Claude 4.x family)
  { id: "claude-opus-4.6",    name: "Claude Opus 4.6",      provider: "Anthropic",  inputPer1M: 15.00,  outputPer1M: 75.00,  contextK: 200  },
  { id: "claude-sonnet-4.6",  name: "Claude Sonnet 4.6",    provider: "Anthropic",  inputPer1M:  3.00,  outputPer1M: 15.00,  contextK: 200  },
  { id: "claude-sonnet-4.5",  name: "Claude Sonnet 4.5",    provider: "Anthropic",  inputPer1M:  3.00,  outputPer1M: 15.00,  contextK: 200  },
  { id: "claude-haiku-4.5",   name: "Claude Haiku 4.5",     provider: "Anthropic",  inputPer1M:  0.80,  outputPer1M:  4.00,  contextK: 200  },
  // Google (Gemini 2.5)
  { id: "gemini-2.5-flash",   name: "Gemini 2.5 Flash",     provider: "Google",     inputPer1M:  0.15,  outputPer1M:  0.60,  contextK: 1000 },
  { id: "gemini-2.5-pro",     name: "Gemini 2.5 Pro",       provider: "Google",     inputPer1M:  1.25,  outputPer1M: 10.00,  contextK: 1000 },
  // DeepSeek
  { id: "deepseek-v3",        name: "DeepSeek V3",          provider: "DeepSeek",   inputPer1M:  0.27,  outputPer1M:  1.10,  contextK: 128  },
  { id: "deepseek-r1",        name: "DeepSeek R1",          provider: "DeepSeek",   inputPer1M:  0.55,  outputPer1M:  2.19,  contextK: 128, note: "reasoning" },
  // Meta (Llama 4)
  { id: "llama-4-maverick",   name: "Llama 4 Maverick",     provider: "Meta/Groq",  inputPer1M:  0.20,  outputPer1M:  0.60,  contextK: 1000 },
  { id: "llama-4-scout",      name: "Llama 4 Scout",        provider: "Meta/Groq",  inputPer1M:  0.15,  outputPer1M:  0.40,  contextK: 512  },
  // Mistral
  { id: "mistral-large",      name: "Mistral Large",        provider: "Mistral",    inputPer1M:  2.00,  outputPer1M:  6.00,  contextK: 128  },
  { id: "codestral",          name: "Codestral",            provider: "Mistral",    inputPer1M:  0.30,  outputPer1M:  0.90,  contextK: 256  },
  // xAI
  { id: "grok-3",             name: "Grok 3",               provider: "xAI",        inputPer1M:  3.00,  outputPer1M: 15.00,  contextK: 128  },
  { id: "grok-3-mini",        name: "Grok 3 mini",          provider: "xAI",        inputPer1M:  0.30,  outputPer1M:  0.50,  contextK: 128, note: "reasoning" },
  // Local / self-hosted
  { id: "local",              name: "Local (self-hosted)",   provider: "Self-hosted", inputPer1M:  0.00,  outputPer1M:  0.00,  contextK: 0,   note: "free (hardware cost only)" },
];

// ── Registry to ModelPrice mapper ────────────────────────────────────────────

function registryToModels(registry: ProviderRegistryEntry[]): ModelPrice[] {
  const configured: ModelPrice[] = [];
  const reference: ModelPrice[] = [];

  for (const entry of registry) {
    for (const m of entry.models) {
      const model: ModelPrice = {
        id:         `${entry.id}/${m.id}`,
        name:       m.id,
        provider:   entry.name,
        inputPer1M: m.input_cost_per_1m ?? 0,
        outputPer1M: m.output_cost_per_1m ?? 0,
        contextK:   Math.round(m.context_window / 1000),
        configured: entry.available,
        note:       m.capabilities.thinking ? "thinking" : undefined,
      };
      if (entry.available) {
        configured.push(model);
      } else {
        reference.push(model);
      }
    }
  }

  // Configured providers first, then reference providers
  return [...configured, ...reference];
}

// ── Provider colours ─────────────────────────────────────────────────────────

const PROVIDER_COLOR: Record<string, string> = {
  "OpenAI":         "text-emerald-400",
  "Anthropic":      "text-amber-400",
  "Google":         "text-blue-400",
  "DeepSeek":       "text-pink-400",
  "Meta/Groq":      "text-violet-400",
  "Groq":           "text-violet-400",
  "Mistral":        "text-orange-400",
  "xAI":            "text-cyan-400",
  "Self-hosted":    "text-gray-400 dark:text-zinc-500",
  "OpenRouter":     "text-rose-400",
  "AWS Bedrock":    "text-yellow-400",
  "Together":       "text-teal-400",
  "Fireworks":      "text-red-400",
  "Perplexity":     "text-indigo-400",
  "Cerebras":       "text-lime-400",
};

const PROVIDER_DOT: Record<string, string> = {
  "OpenAI":         "bg-emerald-500",
  "Anthropic":      "bg-amber-500",
  "Google":         "bg-blue-500",
  "DeepSeek":       "bg-pink-500",
  "Meta/Groq":      "bg-violet-500",
  "Groq":           "bg-violet-500",
  "Mistral":        "bg-orange-500",
  "xAI":            "bg-cyan-500",
  "Self-hosted":    "bg-zinc-600",
  "OpenRouter":     "bg-rose-500",
  "AWS Bedrock":    "bg-yellow-500",
  "Together":       "bg-teal-500",
  "Fireworks":      "bg-red-500",
  "Perplexity":     "bg-indigo-500",
  "Cerebras":       "bg-lime-500",
};

// ── Helpers ───────────────────────────────────────────────────────────────────

function calcCost(model: ModelPrice, tokensIn: number, tokensOut: number): number {
  return (tokensIn / 1_000_000) * model.inputPer1M
       + (tokensOut / 1_000_000) * model.outputPer1M;
}

function fmtUSD(n: number): string {
  if (n === 0) return "$0.00";
  if (n < 0.0001) return `$${n.toExponential(2)}`;
  if (n < 0.01)   return `$${n.toFixed(6)}`;
  if (n < 1)      return `$${n.toFixed(4)}`;
  if (n < 100)    return `$${n.toFixed(2)}`;
  return `$${n.toLocaleString(undefined, { maximumFractionDigits: 2 })}`;
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000)     return `${(n / 1_000).toFixed(0)}K`;
  return String(n);
}

const TOKEN_PRESETS = [
  { label: "1K",   value: 1_000      },
  { label: "10K",  value: 10_000     },
  { label: "100K", value: 100_000    },
  { label: "1M",   value: 1_000_000  },
  { label: "10M",  value: 10_000_000 },
];

// ── Token input ───────────────────────────────────────────────────────────────

function TokenInput({ label, value, onChange }: {
  label: string;
  value: number;
  onChange: (n: number) => void;
}) {
  return (
    <div>
      <label className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider block mb-2">{label}</label>
      <div className="space-y-2">
        <input
          type="number"
          min={0}
          value={value}
          onChange={e => onChange(Math.max(0, parseInt(e.target.value, 10) || 0))}
          className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[14px] text-gray-900 dark:text-zinc-100
                     font-mono px-3 py-2 focus:outline-none focus:border-indigo-500 transition-colors
                     [appearance:textfield] [&::-webkit-outer-spin-button]:appearance-none
                     [&::-webkit-inner-spin-button]:appearance-none"
        />
        <div className="flex gap-1.5 flex-wrap">
          {TOKEN_PRESETS.map(p => (
            <button key={p.label} onClick={() => onChange(p.value)}
              className={clsx(
                "rounded px-2 py-0.5 text-[10px] font-mono font-medium transition-colors border",
                value === p.value
                  ? "bg-indigo-600/20 text-indigo-300 border-indigo-700/50"
                  : "text-gray-400 dark:text-zinc-600 border-gray-200 dark:border-zinc-800 hover:text-gray-700 dark:hover:text-zinc-300 hover:border-zinc-600",
              )}>
              {p.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

// ── Cost bar ──────────────────────────────────────────────────────────────────

function CostBar({ cost, maxCost }: { cost: number; maxCost: number }) {
  const pct = maxCost > 0 ? Math.min(100, (cost / maxCost) * 100) : 0;
  const color =
    pct > 66 ? "bg-red-500/60"   :
    pct > 33 ? "bg-amber-500/60" :
               "bg-emerald-500/60";
  return (
    <div className="h-1.5 rounded-full bg-gray-100 dark:bg-zinc-800 overflow-hidden w-24">
      <div className={clsx("h-full rounded-full transition-all", color)} style={{ width: `${pct}%` }} />
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function CostCalculatorPage() {
  const [tokensIn,       setTokensIn]       = useState(10_000);
  const [tokensOut,      setTokensOut]      = useState(2_000);
  const [selectedId,     setSelectedId]     = useState("gpt-4o-mini");
  const [sortBy,         setSortBy]         = useState<"cost" | "provider" | "name">("cost");
  const [sortAsc,        setSortAsc]        = useState(true);
  const [filterProvider, setFilterProvider] = useState<string>("all");

  // ── Fetch provider registry ───────────────────────────────────────────────
  const { data: registry, isLoading: registryLoading, isError: regError, error: regErr, refetch: regRefetch } = useQuery({
    queryKey: ["provider-registry"],
    queryFn:  () => defaultApi.getProviderRegistry(),
    staleTime: 120_000,
    retry: 1,
  });

  // Merge registry models with fallback — configured providers first.
  const MODELS = useMemo(() => {
    if (!registry || registry.length === 0) return FALLBACK_MODELS;
    return registryToModels(registry);
  }, [registry]);

  const PROVIDERS = useMemo(() => [...new Set(MODELS.map(m => m.provider))], [MODELS]);

  // When registry loads, try to select the first available model if the
  // current selection doesn't exist in the new model list.
  const selected = MODELS.find(m => m.id === selectedId) ?? MODELS[0];
  const selectedCost = calcCost(selected, tokensIn, tokensOut);

  // Cost breakdown
  const inputCost  = (tokensIn  / 1_000_000) * selected.inputPer1M;
  const outputCost = (tokensOut / 1_000_000) * selected.outputPer1M;

  // Sorted + filtered table
  const tableRows = useMemo(() => {
    let rows = MODELS.map(m => ({ ...m, cost: calcCost(m, tokensIn, tokensOut) }));
    if (filterProvider !== "all") rows = rows.filter(r => r.provider === filterProvider);
    rows.sort((a, b) => {
      let v = 0;
      if (sortBy === "cost")     v = a.cost - b.cost;
      if (sortBy === "provider") v = a.provider.localeCompare(b.provider);
      if (sortBy === "name")     v = a.name.localeCompare(b.name);
      return sortAsc ? v : -v;
    });
    return rows;
  }, [MODELS, tokensIn, tokensOut, sortBy, sortAsc, filterProvider]);

  const maxCost = Math.max(...tableRows.map(r => r.cost), 0.0001);

  function toggleSort(col: typeof sortBy) {
    if (sortBy === col) setSortAsc(v => !v);
    else { setSortBy(col); setSortAsc(true); }
  }

  const SortIcon = ({ col }: { col: typeof sortBy }) =>
    sortBy !== col ? null :
    sortAsc ? <ChevronUp size={10} className="inline ml-0.5" /> :
              <ChevronDown size={10} className="inline ml-0.5" />;

  if (regError) return <ErrorFallback error={regErr} resource="cost calculator" onRetry={() => void regRefetch()} />;

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-950 overflow-y-auto">
      <div className="max-w-5xl mx-auto px-5 py-5 space-y-6 w-full">

        {/* Header */}
        <div className="flex items-center gap-3">
          <div className="w-8 h-8 rounded-lg bg-indigo-500/10 flex items-center justify-center shrink-0">
            <Calculator size={16} className="text-indigo-400" />
          </div>
          <div>
            <h1 className="text-[15px] font-semibold text-gray-900 dark:text-zinc-100">Cost Calculator</h1>
            <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">
              Estimate LLM API spend before choosing a model. Pricing per 1M tokens, approximate.
              {registryLoading && <span className="ml-1 text-indigo-400">Loading providers...</span>}
              {registry && !registryLoading && (
                <span className="ml-1">
                  {registry.filter(r => r.available).length} configured provider{registry.filter(r => r.available).length !== 1 ? "s" : ""}
                </span>
              )}
            </p>
          </div>
        </div>

        {/* Calculator card */}
        <div className="rounded-xl border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 p-5 space-y-5">
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-5">
            {/* Token inputs */}
            <TokenInput label="Input tokens" value={tokensIn}  onChange={setTokensIn}  />
            <TokenInput label="Output tokens" value={tokensOut} onChange={setTokensOut} />

            {/* Model selector */}
            <div>
              <label className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider block mb-2">Model</label>
              <select value={selectedId} onChange={e => setSelectedId(e.target.value)}
                className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[13px] text-gray-800 dark:text-zinc-200
                           px-3 py-2 focus:outline-none focus:border-indigo-500 transition-colors">
                {PROVIDERS.map(prov => (
                  <optgroup key={prov} label={prov}>
                    {MODELS.filter(m => m.provider === prov).map(m => (
                      <option key={m.id} value={m.id}>
                        {m.name}{m.configured === false ? " (ref)" : ""}
                      </option>
                    ))}
                  </optgroup>
                ))}
              </select>
            </div>
          </div>

          {/* Result */}
          <div className="rounded-lg border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 divide-y divide-gray-200 dark:divide-zinc-800">
            {/* Main estimate */}
            <div className="flex items-center justify-between px-4 py-3">
              <div className="flex items-center gap-3">
                <span className={clsx("text-[11px] font-medium", PROVIDER_COLOR[selected.provider] ?? "text-gray-500 dark:text-zinc-400")}>
                  {selected.provider}
                </span>
                <span className="text-[14px] font-medium text-gray-800 dark:text-zinc-200">{selected.name}</span>
                {selected.note && (
                  <span className="text-[10px] text-gray-400 dark:text-zinc-600 bg-gray-100 dark:bg-zinc-800 rounded px-1.5 py-0.5">{selected.note}</span>
                )}
                {selected.configured === false && (
                  <span className="text-[10px] text-gray-400 dark:text-zinc-600 bg-gray-100 dark:bg-zinc-800 rounded px-1.5 py-0.5">reference only</span>
                )}
              </div>
              <div className="text-right">
                <p className="text-[22px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-none">
                  {fmtUSD(selectedCost)}
                </p>
                <p className="text-[10px] text-gray-400 dark:text-zinc-600 mt-0.5">estimated total</p>
              </div>
            </div>

            {/* Breakdown */}
            <div className="grid grid-cols-2 divide-x divide-gray-200 dark:divide-zinc-800">
              <div className="px-4 py-2.5">
                <p className="text-[10px] text-gray-400 dark:text-zinc-600 mb-1">Input: {fmtTokens(tokensIn)} tokens</p>
                <p className="text-[13px] font-mono text-gray-700 dark:text-zinc-300 tabular-nums">{fmtUSD(inputCost)}</p>
                <p className="text-[10px] text-gray-300 dark:text-zinc-600 mt-0.5">${selected.inputPer1M.toFixed(4)} / 1M</p>
              </div>
              <div className="px-4 py-2.5">
                <p className="text-[10px] text-gray-400 dark:text-zinc-600 mb-1">Output: {fmtTokens(tokensOut)} tokens</p>
                <p className="text-[13px] font-mono text-gray-700 dark:text-zinc-300 tabular-nums">{fmtUSD(outputCost)}</p>
                <p className="text-[10px] text-gray-300 dark:text-zinc-600 mt-0.5">${selected.outputPer1M.toFixed(4)} / 1M</p>
              </div>
            </div>

            {/* Context window note */}
            {selected.contextK > 0 && (
              <div className="px-4 py-2 flex items-center gap-2">
                <Info size={11} className="text-gray-400 dark:text-zinc-600 shrink-0" />
                <p className="text-[11px] text-gray-400 dark:text-zinc-600">
                  Context window: <span className="text-gray-400 dark:text-zinc-500 font-mono">{selected.contextK}K tokens</span>
                  {selected.contextK * 1000 < tokensIn + tokensOut && (
                    <span className="ml-2 text-amber-500">-- exceeds context window</span>
                  )}
                </p>
              </div>
            )}
          </div>
        </div>

        {/* Pricing comparison table */}
        <div>
          <div className="flex items-center justify-between mb-3">
            <div className="flex items-center gap-2">
              <Coins size={13} className="text-gray-400 dark:text-zinc-500" />
              <span className="text-[12px] font-medium text-gray-700 dark:text-zinc-300">All Models — Cost for {fmtTokens(tokensIn)} in / {fmtTokens(tokensOut)} out</span>
            </div>
            <select value={filterProvider} onChange={e => setFilterProvider(e.target.value)}
              className="rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[11px] text-gray-500 dark:text-zinc-400 px-2 py-1
                         focus:outline-none focus:border-indigo-500 transition-colors">
              <option value="all">All providers</option>
              {PROVIDERS.map(p => <option key={p} value={p}>{p}</option>)}
            </select>
          </div>

          <div className="rounded-xl border border-gray-200 dark:border-zinc-800 overflow-hidden">
            <table className="min-w-full text-[12px]">
              <thead>
                <tr className="bg-gray-50 dark:bg-zinc-900 border-b border-gray-200 dark:border-zinc-800">
                  <th onClick={() => toggleSort("name")}
                    className="px-4 py-2.5 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider cursor-pointer hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
                    Model <SortIcon col="name" />
                  </th>
                  <th onClick={() => toggleSort("provider")}
                    className="px-3 py-2.5 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider cursor-pointer hover:text-gray-700 dark:hover:text-zinc-300 transition-colors hidden sm:table-cell">
                    Provider <SortIcon col="provider" />
                  </th>
                  <th className="px-3 py-2.5 text-right text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider hidden md:table-cell">
                    Input / 1M
                  </th>
                  <th className="px-3 py-2.5 text-right text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider hidden md:table-cell">
                    Output / 1M
                  </th>
                  <th onClick={() => toggleSort("cost")}
                    className="px-4 py-2.5 text-right text-[10px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider cursor-pointer hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
                    Est. Cost <SortIcon col="cost" />
                  </th>
                  <th className="px-4 py-2.5 w-32 hidden sm:table-cell" />
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
                {tableRows.map((m, i) => {
                  const isSelected = m.id === selectedId;
                  return (
                    <tr key={m.id}
                      onClick={() => setSelectedId(m.id)}
                      className={clsx(
                        "cursor-pointer transition-colors",
                        isSelected
                          ? "bg-indigo-950/30 ring-1 ring-inset ring-indigo-800/40"
                          : i % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900 hover:bg-gray-100/60 dark:hover:bg-zinc-800/60" : "bg-gray-50/50 dark:bg-zinc-900/50 hover:bg-gray-100/60 dark:hover:bg-zinc-800/60",
                      )}>
                      <td className="px-4 py-2.5">
                        <div className="flex items-center gap-2">
                          <span className={clsx("w-1.5 h-1.5 rounded-full shrink-0",
                            PROVIDER_DOT[m.provider] ?? "bg-zinc-600")} />
                          <span className={clsx("font-medium", isSelected ? "text-indigo-300" : "text-gray-800 dark:text-zinc-200")}>
                            {m.name}
                          </span>
                          {m.note && (
                            <span className="text-[9px] text-gray-400 dark:text-zinc-600 bg-gray-100 dark:bg-zinc-800 rounded px-1 py-0.5 hidden sm:inline">
                              {m.note}
                            </span>
                          )}
                          {m.configured === false && (
                            <span className="text-[9px] text-gray-400 dark:text-zinc-600 bg-gray-100 dark:bg-zinc-800/60 rounded px-1 py-0.5 hidden sm:inline">
                              ref
                            </span>
                          )}
                        </div>
                      </td>
                      <td className="px-3 py-2.5 hidden sm:table-cell">
                        <span className={clsx("text-[11px] font-medium", PROVIDER_COLOR[m.provider] ?? "text-gray-400 dark:text-zinc-500")}>
                          {m.provider}
                        </span>
                      </td>
                      <td className="px-3 py-2.5 text-right font-mono text-gray-400 dark:text-zinc-500 hidden md:table-cell">
                        {m.inputPer1M === 0 ? <span className="text-emerald-600">free</span> : `$${m.inputPer1M.toFixed(4)}`}
                      </td>
                      <td className="px-3 py-2.5 text-right font-mono text-gray-400 dark:text-zinc-500 hidden md:table-cell">
                        {m.outputPer1M === 0 ? <span className="text-emerald-600">free</span> : `$${m.outputPer1M.toFixed(4)}`}
                      </td>
                      <td className="px-4 py-2.5 text-right">
                        <span className={clsx(
                          "font-mono font-semibold tabular-nums",
                          m.cost === 0   ? "text-emerald-400" :
                          m.cost < 0.01  ? "text-emerald-300" :
                          m.cost < 1.00  ? "text-gray-800 dark:text-zinc-200"    :
                          m.cost < 10.00 ? "text-amber-300"   : "text-red-300",
                        )}>
                          {fmtUSD(m.cost)}
                        </span>
                      </td>
                      <td className="px-4 py-2.5 hidden sm:table-cell">
                        <CostBar cost={m.cost} maxCost={maxCost} />
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>

          <p className="mt-2 text-[10px] text-gray-300 dark:text-zinc-600 text-center">
            Prices are approximate and subject to change. Always verify with the provider's official pricing page.
            {registry && " Models sourced from your cairn provider registry."}
          </p>
        </div>

      </div>
    </div>
  );
}

export default CostCalculatorPage;
