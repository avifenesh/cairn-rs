import { useState, useRef, useEffect, type FormEvent, type KeyboardEvent } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Terminal, Send, Loader2, AlertTriangle,
  Clock, Zap, ChevronDown, Trash2, Square,
} from "lucide-react";
import { defaultApi } from "../lib/api";

// ── Types ─────────────────────────────────────────────────────────────────────

interface StreamMeta {
  latency_ms: number;
  model: string;
}

// ── Streaming logic ───────────────────────────────────────────────────────────

const API_BASE = import.meta.env.VITE_API_URL ?? "";
const getToken = () => localStorage.getItem("cairn_token") ?? import.meta.env.VITE_API_TOKEN ?? "";

/**
 * Stream tokens from POST /v1/providers/ollama/stream.
 * Calls onToken for each arriving token, onDone when complete, onError on failure.
 * Returns a controller that can be used to abort mid-stream.
 */
function streamGenerate(
  model: string,
  prompt: string,
  onToken: (text: string) => void,
  onDone: (meta: StreamMeta) => void,
  onError: (msg: string) => void,
): AbortController {
  const controller = new AbortController();

  (async () => {
    try {
      const resp = await fetch(`${API_BASE}/v1/providers/ollama/stream`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          Authorization: `Bearer ${getToken()}`,
        },
        body: JSON.stringify({ model, prompt }),
        signal: controller.signal,
      });

      if (!resp.ok) {
        const text = await resp.text().catch(() => "");
        onError(`HTTP ${resp.status}: ${text}`);
        return;
      }

      const reader = resp.body?.getReader();
      if (!reader) { onError("No response body"); return; }

      const decoder = new TextDecoder();
      let buf = "";

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;

        buf += decoder.decode(value, { stream: true });

        // Process complete SSE lines from buffer
        let nl: number;
        while ((nl = buf.indexOf("\n")) !== -1) {
          const line = buf.slice(0, nl).trim();
          buf = buf.slice(nl + 1);

          if (!line.startsWith("data:")) continue;
          const data = line.slice(5).trim();
          if (!data) continue;

          try {
            const parsed = JSON.parse(data) as Record<string, unknown>;
            if (parsed.text !== undefined) {
              onToken(String(parsed.text));
            } else if (parsed.latency_ms !== undefined) {
              onDone({ latency_ms: Number(parsed.latency_ms), model: String(parsed.model ?? model) });
            } else if (parsed.error !== undefined) {
              onError(String(parsed.error));
            }
          } catch {
            // Ignore malformed lines
          }
        }
      }
    } catch (e: unknown) {
      if ((e as { name?: string })?.name === "AbortError") return; // intentional cancel
      onError(e instanceof Error ? e.message : "Stream failed");
    }
  })();

  return controller;
}

// ── Model selector ────────────────────────────────────────────────────────────

function ModelSelector({
  value, onChange, models, disabled,
}: { value: string; onChange: (m: string) => void; models: string[]; disabled: boolean }) {
  return (
    <div className="relative">
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled || models.length === 0}
        className="appearance-none w-full rounded-md border border-zinc-800 bg-zinc-900
                   text-sm text-zinc-300 px-3 py-2 pr-8
                   focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500
                   disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
      >
        {models.length === 0 && <option value="">No models available</option>}
        {models.map((m) => <option key={m} value={m}>{m}</option>)}
      </select>
      <ChevronDown size={14} className="absolute right-2.5 top-1/2 -translate-y-1/2 text-zinc-500 pointer-events-none" />
    </div>
  );
}

// ── Output area ───────────────────────────────────────────────────────────────

function OutputArea({
  text, streaming, error, meta,
}: { text: string; streaming: boolean; error: string | null; meta: StreamMeta | null }) {
  const ref = useRef<HTMLDivElement>(null);

  // Auto-scroll to bottom as tokens arrive
  useEffect(() => {
    if (streaming && ref.current) {
      ref.current.scrollTop = ref.current.scrollHeight;
    }
  }, [text, streaming]);

  if (error) {
    return (
      <div className="flex-1 rounded-lg border border-red-900/40 bg-red-950/10 p-4 min-h-28">
        <p className="flex items-center gap-1.5 text-[11px] text-red-400 mb-2 uppercase tracking-wider">
          <AlertTriangle size={11} /> Error
        </p>
        <p className="text-sm text-red-300 font-mono break-words">{error}</p>
      </div>
    );
  }

  if (!text && !streaming) {
    return (
      <div className="flex-1 rounded-lg border border-zinc-800 bg-zinc-950 p-4 flex items-center justify-center min-h-28">
        <p className="text-xs text-zinc-600">Response will appear here</p>
      </div>
    );
  }

  return (
    <div className="flex-1 flex flex-col gap-2">
      {/* Stats bar — shown after completion */}
      {meta && !streaming && (
        <div className="flex items-center gap-4 text-[11px] text-zinc-500">
          <span className="flex items-center gap-1">
            <Clock size={11} />
            {meta.latency_ms >= 1000 ? `${(meta.latency_ms / 1000).toFixed(2)}s` : `${meta.latency_ms}ms`}
          </span>
          <span className="font-mono text-zinc-600 ml-auto">{meta.model}</span>
        </div>
      )}

      {/* Generating indicator */}
      {streaming && (
        <div className="flex items-center gap-1.5 text-[11px] text-indigo-400">
          <Loader2 size={11} className="animate-spin" />
          Generating…
        </div>
      )}

      {/* Response text with typewriter cursor */}
      <div
        ref={ref}
        className="flex-1 rounded-lg border border-zinc-800 bg-zinc-950 p-4 overflow-y-auto max-h-[50vh]"
      >
        <pre className="text-sm text-zinc-200 font-mono whitespace-pre-wrap break-words leading-relaxed">
          {text}
          {streaming && (
            <span className="inline-block w-0.5 h-4 bg-indigo-400 ml-0.5 animate-pulse align-text-bottom" />
          )}
        </pre>
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function PlaygroundPage() {
  const [prompt, setPrompt]           = useState("");
  const [selectedModel, setSelectedModel] = useState("");
  const [outputText, setOutputText]   = useState("");
  const [streaming, setStreaming]     = useState(false);
  const [error, setError]             = useState<string | null>(null);
  const [meta, setMeta]               = useState<StreamMeta | null>(null);
  const abortRef                      = useRef<AbortController | null>(null);
  const textareaRef                   = useRef<HTMLTextAreaElement>(null);

  const { data: modelsData, isLoading: modelsLoading, error: modelsError } = useQuery({
    queryKey: ["ollama-models"],
    queryFn:  () => defaultApi.getOllamaModels(),
    retry:    false,
    staleTime: 60_000,
    refetchOnWindowFocus: false,
  });

  const models: string[]    = modelsData?.models ?? [];
  const ollamaDown          = !!modelsError;
  const activeModel         = selectedModel || models[0] || "";

  function handleSubmit(e?: FormEvent) {
    e?.preventDefault();
    const trimmed = prompt.trim();
    if (!trimmed || !activeModel || streaming) return;

    // Reset state
    setOutputText("");
    setError(null);
    setMeta(null);
    setStreaming(true);

    abortRef.current = streamGenerate(
      activeModel,
      trimmed,
      (token) => setOutputText((prev) => prev + token),
      (m) => { setMeta(m); setStreaming(false); },
      (msg) => { setError(msg); setStreaming(false); },
    );
  }

  function handleStop() {
    abortRef.current?.abort();
    setStreaming(false);
  }

  function handleClear() {
    handleStop();
    setPrompt("");
    setOutputText("");
    setError(null);
    setMeta(null);
    textareaRef.current?.focus();
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      handleSubmit();
    }
  }

  return (
    <div className="flex flex-col h-full bg-zinc-950">
      {/* ── Toolbar ─────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-3 px-4 py-2.5 border-b border-zinc-800 shrink-0">
        <Terminal size={14} className="text-indigo-400 shrink-0" />
        <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
          LLM Playground
        </span>

        <div className="ml-auto flex items-center gap-2">
          {modelsLoading ? (
            <span className="text-[11px] text-zinc-600 flex items-center gap-1">
              <Loader2 size={10} className="animate-spin" /> Checking…
            </span>
          ) : ollamaDown ? (
            <span className="text-[11px] text-amber-600">Ollama offline</span>
          ) : (
            <span className="text-[11px] text-zinc-600 flex items-center gap-1">
              <Zap size={10} className="text-emerald-500" />
              {models.length} model{models.length !== 1 ? "s" : ""}
            </span>
          )}
          <div className="w-48">
            <ModelSelector
              value={activeModel}
              onChange={setSelectedModel}
              models={models}
              disabled={streaming}
            />
          </div>
        </div>
      </div>

      {/* ── Main area ───────────────────────────────────────────────────── */}
      <div className="flex-1 flex flex-col gap-3 p-4 overflow-hidden">
        {/* Prompt */}
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
            disabled={streaming}
            placeholder="Enter a prompt…"
            rows={5}
            className="w-full rounded-lg border border-zinc-800 bg-zinc-900 text-sm text-zinc-200
                       placeholder-zinc-600 font-mono px-3 py-2.5 resize-none
                       focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500
                       disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
          />
          <div className="flex items-center gap-2 justify-end">
            {(outputText || error || prompt) && !streaming && (
              <button type="button" onClick={handleClear}
                className="flex items-center gap-1 text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors">
                <Trash2 size={11} /> Clear
              </button>
            )}
            {streaming ? (
              <button type="button" onClick={handleStop}
                className="flex items-center gap-1.5 rounded-md px-3 h-8 text-xs font-medium
                           bg-red-900/50 border border-red-800/50 text-red-400
                           hover:bg-red-900/80 transition-colors">
                <Square size={12} /> Stop
              </button>
            ) : (
              <button type="submit"
                disabled={!prompt.trim() || !activeModel || ollamaDown}
                className="flex items-center gap-1.5 rounded-md px-3 h-8 text-xs font-medium
                           bg-indigo-600 hover:bg-indigo-500 text-white
                           disabled:bg-zinc-800 disabled:text-zinc-600 disabled:cursor-not-allowed
                           transition-colors">
                <Send size={13} /> Send
              </button>
            )}
          </div>
        </form>

        {/* Output */}
        <div className="flex-1 flex flex-col min-h-0">
          <p className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider mb-2">
            Response
          </p>
          <OutputArea text={outputText} streaming={streaming} error={error} meta={meta} />
        </div>
      </div>
    </div>
  );
}

export default PlaygroundPage;
