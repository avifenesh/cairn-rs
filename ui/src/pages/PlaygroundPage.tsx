import { useState, useRef, type FormEvent, type KeyboardEvent } from "react";
import { useQuery, useMutation } from "@tanstack/react-query";
import {
  Terminal, Send, Loader2, AlertTriangle,
  Clock, Zap, ChevronDown, Trash2,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";

// ── Types ─────────────────────────────────────────────────────────────────────

interface GenerateResult {
  text: string;
  model: string;
  tokens_in: number | null;
  tokens_out: number | null;
  latency_ms: number;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function formatMs(ms: number): string {
  return ms >= 1000 ? `${(ms / 1000).toFixed(2)}s` : `${ms}ms`;
}

// ── Model selector ────────────────────────────────────────────────────────────

interface ModelSelectorProps {
  value: string;
  onChange: (m: string) => void;
  models: string[];
  loading: boolean;
  unavailable: boolean;
}

function ModelSelector({ value, onChange, models, loading, unavailable }: ModelSelectorProps) {
  return (
    <div className="relative">
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={unavailable || loading}
        className={clsx(
          "appearance-none w-full rounded-md border text-sm px-3 py-2 pr-8",
          "bg-zinc-900 border-zinc-800 text-zinc-300",
          "focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500",
          "disabled:opacity-50 disabled:cursor-not-allowed",
          "transition-colors"
        )}
      >
        {loading && <option value="">Loading models…</option>}
        {unavailable && <option value="">Ollama unavailable</option>}
        {!loading && !unavailable && models.length === 0 && (
          <option value="">No models found</option>
        )}
        {models.map((m) => (
          <option key={m} value={m}>{m}</option>
        ))}
      </select>
      <ChevronDown
        size={14}
        className="absolute right-2.5 top-1/2 -translate-y-1/2 text-zinc-500 pointer-events-none"
      />
    </div>
  );
}

// ── Output panel ──────────────────────────────────────────────────────────────

function OutputPanel({
  result,
  generating,
  error,
}: {
  result: GenerateResult | null;
  generating: boolean;
  error: string | null;
}) {
  if (generating) {
    return (
      <div className="flex-1 rounded-lg border border-zinc-800 bg-zinc-950 p-4 flex items-center gap-2 text-zinc-500 text-sm min-h-32">
        <Loader2 size={14} className="animate-spin shrink-0" />
        Generating…
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex-1 rounded-lg border border-red-900/50 bg-red-950/20 p-4 min-h-32">
        <p className="flex items-center gap-1.5 text-xs text-red-400 mb-2">
          <AlertTriangle size={12} /> Error
        </p>
        <p className="text-sm text-red-300 font-mono break-words">{error}</p>
      </div>
    );
  }

  if (!result) {
    return (
      <div className="flex-1 rounded-lg border border-zinc-800 bg-zinc-950 p-4 flex items-center justify-center min-h-32">
        <p className="text-xs text-zinc-600">Response will appear here</p>
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col gap-3">
      {/* Stats bar */}
      <div className="flex items-center gap-4 text-[11px] text-zinc-500">
        <span className="flex items-center gap-1">
          <Clock size={11} />
          {formatMs(result.latency_ms)}
        </span>
        {result.tokens_in != null && (
          <span className="flex items-center gap-1">
            <Zap size={11} />
            {result.tokens_in} in
          </span>
        )}
        {result.tokens_out != null && (
          <span>{result.tokens_out} out</span>
        )}
        <span className="ml-auto font-mono text-zinc-600">{result.model}</span>
      </div>

      {/* Response text */}
      <div className="flex-1 rounded-lg border border-zinc-800 bg-zinc-950 p-4 overflow-y-auto max-h-[50vh]">
        <pre className="text-sm text-zinc-200 font-mono whitespace-pre-wrap break-words leading-relaxed">
          {result.text}
        </pre>
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function PlaygroundPage() {
  const [prompt, setPrompt] = useState("");
  const [selectedModel, setSelectedModel] = useState("");
  const [result, setResult] = useState<GenerateResult | null>(null);
  const [error, setError] = useState<string | null>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Fetch available Ollama models
  const {
    data: modelsData,
    isLoading: modelsLoading,
    error: modelsError,
  } = useQuery({
    queryKey: ["ollama-models"],
    queryFn: () => defaultApi.getOllamaModels(),
    retry: false,
    staleTime: 60_000,
    refetchOnWindowFocus: false,
  });

  const models: string[] = modelsData?.models ?? [];
  const ollamaUnavailable = !!modelsError;
  const activeModel = selectedModel || models[0] || "";

  // Generate mutation
  const { mutate: generate, isPending: generating } = useMutation({
    mutationFn: ({ prompt, model }: { prompt: string; model: string }) =>
      defaultApi.ollamaGenerate({ prompt, model }),
    onSuccess: (data) => {
      setResult(data);
      setError(null);
    },
    onError: (e) => {
      setError(e instanceof Error ? e.message : "Generation failed");
      setResult(null);
    },
  });

  function handleSubmit(e?: FormEvent) {
    e?.preventDefault();
    const trimmed = prompt.trim();
    if (!trimmed || !activeModel || generating) return;
    setResult(null);
    setError(null);
    generate({ prompt: trimmed, model: activeModel });
  }

  // Cmd/Ctrl+Enter to submit
  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      handleSubmit();
    }
  }

  function handleClear() {
    setPrompt("");
    setResult(null);
    setError(null);
    textareaRef.current?.focus();
  }

  return (
    <div className="flex flex-col h-full bg-zinc-950">
      {/* ── Toolbar ─────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-3 px-4 py-2.5 border-b border-zinc-800 shrink-0">
        <Terminal size={14} className="text-indigo-400 shrink-0" />
        <span className="text-xs font-medium text-zinc-400 uppercase tracking-wider">
          LLM Playground
        </span>

        <div className="ml-auto flex items-center gap-2">
          {/* Status indicator */}
          {modelsLoading ? (
            <span className="text-[11px] text-zinc-600 flex items-center gap-1">
              <Loader2 size={10} className="animate-spin" /> Checking Ollama…
            </span>
          ) : ollamaUnavailable ? (
            <span className="text-[11px] text-amber-600">
              Ollama offline — set OLLAMA_HOST
            </span>
          ) : (
            <span className="text-[11px] text-zinc-600">
              {models.length} model{models.length !== 1 ? "s" : ""} available
            </span>
          )}

          {/* Model selector */}
          <div className="w-48">
            <ModelSelector
              value={activeModel}
              onChange={setSelectedModel}
              models={models}
              loading={modelsLoading}
              unavailable={ollamaUnavailable}
            />
          </div>
        </div>
      </div>

      {/* ── Main area ───────────────────────────────────────────────────── */}
      <div className="flex-1 flex flex-col gap-3 p-4 overflow-hidden">
        {/* Prompt input */}
        <form onSubmit={handleSubmit} className="flex flex-col gap-2">
          <div className="flex items-center justify-between">
            <label className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
              Prompt
            </label>
            <span className="text-[10px] text-zinc-700">⌘↵ to send</span>
          </div>
          <textarea
            ref={textareaRef}
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            onKeyDown={handleKeyDown}
            disabled={generating}
            placeholder="Enter a prompt… e.g. Explain Rust ownership in one paragraph."
            rows={6}
            className={clsx(
              "w-full rounded-lg border bg-zinc-900 border-zinc-800",
              "text-sm text-zinc-200 placeholder-zinc-600 font-mono",
              "px-3 py-2.5 resize-none",
              "focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500",
              "disabled:opacity-50 disabled:cursor-not-allowed",
              "transition-colors"
            )}
          />
          <div className="flex items-center gap-2 justify-end">
            {(result || error || prompt) && (
              <button
                type="button"
                onClick={handleClear}
                className="flex items-center gap-1 text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors"
              >
                <Trash2 size={11} /> Clear
              </button>
            )}
            <button
              type="submit"
              disabled={!prompt.trim() || !activeModel || generating || ollamaUnavailable}
              className={clsx(
                "flex items-center gap-1.5 rounded-md px-3 h-8 text-xs font-medium",
                "bg-indigo-600 hover:bg-indigo-500 text-white",
                "disabled:bg-zinc-800 disabled:text-zinc-600 disabled:cursor-not-allowed",
                "transition-colors"
              )}
            >
              {generating
                ? <Loader2 size={13} className="animate-spin" />
                : <Send size={13} />}
              {generating ? "Generating…" : "Send"}
            </button>
          </div>
        </form>

        {/* Output */}
        <div className="flex-1 flex flex-col min-h-0">
          <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-2">
            Response
          </p>
          <OutputPanel result={result} generating={generating} error={error} />
        </div>
      </div>
    </div>
  );
}

export default PlaygroundPage;
