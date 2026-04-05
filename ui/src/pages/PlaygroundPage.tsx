import { useState, useRef, useEffect, type FormEvent, type KeyboardEvent } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Terminal, Send, Loader2, AlertTriangle,
  Clock, Zap, ChevronDown, ChevronRight, Trash2, Square, Bot, User, Settings2,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_SYSTEM_PROMPT =
  "You are a helpful AI assistant running on cairn-rs, a self-hostable control plane for AI agents.";

const LS_SYSTEM_PROMPT = "cairn_playground_system_prompt";
const LS_SYSTEM_OPEN   = "cairn_playground_system_open";

// ── Types ─────────────────────────────────────────────────────────────────────

type Role = "user" | "assistant";

interface Message {
  role: Role;
  content: string;
  meta?: { latency_ms: number; model: string };
  error?: string;
  streaming?: boolean;
}

// ── Streaming helper ──────────────────────────────────────────────────────────

const API_BASE = import.meta.env.VITE_API_URL ?? "";
const getToken = () => localStorage.getItem("cairn_token") ?? import.meta.env.VITE_API_TOKEN ?? "";

function streamGenerate(
  model: string,
  messages: { role: string; content: string }[],
  onToken: (text: string) => void,
  onDone:  (meta: { latency_ms: number; model: string }) => void,
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
        body: JSON.stringify({ model, messages }),
        signal: controller.signal,
      });

      if (!resp.ok) {
        onError(`HTTP ${resp.status}: ${await resp.text().catch(() => "")}`);
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

        let nl: number;
        while ((nl = buf.indexOf("\n")) !== -1) {
          const line = buf.slice(0, nl).trim();
          buf = buf.slice(nl + 1);
          if (!line.startsWith("data:")) continue;
          const data = line.slice(5).trim();
          if (!data) continue;
          try {
            const parsed = JSON.parse(data) as Record<string, unknown>;
            if (parsed.text !== undefined)
              onToken(String(parsed.text));
            else if (parsed.latency_ms !== undefined)
              onDone({ latency_ms: Number(parsed.latency_ms), model: String(parsed.model ?? model) });
            else if (parsed.error !== undefined)
              onError(String(parsed.error));
          } catch { /* ignore malformed */ }
        }
      }
    } catch (e: unknown) {
      if ((e as { name?: string })?.name === "AbortError") return;
      onError(e instanceof Error ? e.message : "Stream failed");
    }
  })();

  return controller;
}

// ── Model selector ────────────────────────────────────────────────────────────

function ModelSelector({ value, onChange, models, disabled }: {
  value: string; onChange: (m: string) => void; models: string[]; disabled: boolean;
}) {
  return (
    <div className="relative">
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled || models.length === 0}
        className="appearance-none w-full rounded border border-zinc-800 bg-zinc-900
                   text-[13px] text-zinc-300 px-2.5 py-1.5 pr-7
                   focus:outline-none focus:border-indigo-500
                   disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
      >
        {models.length === 0 && <option value="">No models</option>}
        {models.map((m) => <option key={m} value={m}>{m}</option>)}
      </select>
      <ChevronDown size={12} className="absolute right-2 top-1/2 -translate-y-1/2 text-zinc-600 pointer-events-none" />
    </div>
  );
}

// ── System prompt panel ───────────────────────────────────────────────────────

interface SystemPromptPanelProps {
  value: string;
  onChange: (v: string) => void;
  open: boolean;
  onToggle: () => void;
  disabled: boolean;
}

function SystemPromptPanel({ value, onChange, open, onToggle, disabled }: SystemPromptPanelProps) {
  const isDefault = value === DEFAULT_SYSTEM_PROMPT;

  return (
    <div className="border-b border-zinc-800 shrink-0">
      {/* Header row — always visible */}
      <button
        type="button"
        onClick={onToggle}
        className="w-full flex items-center gap-2 px-4 py-2 text-left hover:bg-zinc-900/40 transition-colors"
      >
        {open
          ? <ChevronDown size={12} className="text-zinc-500 shrink-0" />
          : <ChevronRight size={12} className="text-zinc-500 shrink-0" />
        }
        <Settings2 size={12} className="text-zinc-500 shrink-0" />
        <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
          System Prompt
        </span>
        {!isDefault && (
          <span className="ml-1 inline-block w-1.5 h-1.5 rounded-full bg-indigo-500 shrink-0" title="Custom prompt" />
        )}
        {/* Collapsed preview */}
        {!open && (
          <span className="ml-2 text-[11px] text-zinc-600 truncate max-w-xs">
            {value || <em>empty</em>}
          </span>
        )}
      </button>

      {/* Expanded editor */}
      {open && (
        <div className="px-4 pb-3 space-y-2">
          <textarea
            value={value}
            onChange={(e) => onChange(e.target.value)}
            disabled={disabled}
            rows={3}
            className="w-full rounded border border-zinc-800 bg-zinc-900 text-[13px] text-zinc-300
                       placeholder-zinc-600 px-3 py-2 resize-none
                       focus:outline-none focus:border-indigo-500
                       disabled:opacity-50 transition-colors"
            placeholder="System instructions for the model…"
          />
          <div className="flex items-center gap-3">
            {!isDefault && (
              <button
                type="button"
                onClick={() => onChange(DEFAULT_SYSTEM_PROMPT)}
                className="text-[11px] text-zinc-600 hover:text-zinc-400 transition-colors"
              >
                Reset to default
              </button>
            )}
            <span className="ml-auto text-[11px] text-zinc-700">
              {value.length} chars · prepended as <code className="text-zinc-600">system</code> role
            </span>
          </div>
        </div>
      )}
    </div>
  );
}

// ── Chat bubble ───────────────────────────────────────────────────────────────

function ChatBubble({ msg }: { msg: Message }) {
  const isUser = msg.role === "user";

  return (
    <div className={clsx("flex gap-2.5 max-w-[85%]", isUser ? "ml-auto flex-row-reverse" : "mr-auto")}>
      <div className={clsx(
        "shrink-0 w-6 h-6 rounded-full flex items-center justify-center mt-0.5",
        isUser ? "bg-indigo-900/60 text-indigo-400" : "bg-zinc-800 text-zinc-500",
      )}>
        {isUser ? <User size={12} /> : <Bot size={12} />}
      </div>

      <div className="flex flex-col gap-1 min-w-0">
        <div className={clsx(
          "rounded-xl px-3.5 py-2 text-[13px] leading-relaxed",
          isUser
            ? "rounded-tr-sm bg-indigo-600 text-white"
            : clsx(
              "rounded-tl-sm bg-zinc-900 border border-zinc-800 text-zinc-200",
              msg.error && "bg-red-950/40 border-red-800/40 text-red-300",
            ),
        )}>
          {msg.error ? (
            <span className="flex items-start gap-1.5">
              <AlertTriangle size={12} className="shrink-0 mt-0.5 text-red-400" />
              {msg.error}
            </span>
          ) : (
            <pre className="whitespace-pre-wrap break-words font-sans">
              {msg.content}
              {msg.streaming && (
                <span className="inline-block w-0.5 h-3 bg-zinc-400 ml-0.5 animate-pulse align-text-bottom" />
              )}
            </pre>
          )}
        </div>

        {msg.meta && !msg.streaming && (
          <div className="flex items-center gap-2 px-1 text-[10px] text-zinc-700">
            <Clock size={9} />
            {msg.meta.latency_ms >= 1000
              ? `${(msg.meta.latency_ms / 1000).toFixed(1)}s`
              : `${msg.meta.latency_ms}ms`}
            <span className="font-mono">{msg.meta.model}</span>
          </div>
        )}
        {msg.streaming && (
          <div className="flex items-center gap-1.5 px-1 text-[10px] text-indigo-400">
            <Loader2 size={9} className="animate-spin" /> Generating…
          </div>
        )}
      </div>
    </div>
  );
}

// ── Empty state ────────────────────────────────────────────────────────────────

function EmptyChat({ model }: { model: string }) {
  return (
    <div className="flex flex-col items-center justify-center flex-1 gap-3 text-center py-12">
      <div className="w-10 h-10 rounded-full bg-zinc-900 border border-zinc-800 flex items-center justify-center">
        <Bot size={18} className="text-zinc-600" />
      </div>
      <div>
        <p className="text-[13px] font-medium text-zinc-400">Start a conversation</p>
        <p className="text-[11px] text-zinc-600 mt-1">
          {model
            ? <>Model: <span className="font-mono text-zinc-500">{model}</span></>
            : "Select a model to begin"}
        </p>
      </div>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function PlaygroundPage() {
  const [messages, setMessages]     = useState<Message[]>([]);
  const [input, setInput]           = useState("");
  const [selectedModel, setSelectedModel] = useState("");
  const [streaming, setStreaming]   = useState(false);

  // System prompt — persisted in localStorage
  const [systemPrompt, setSystemPromptRaw] = useState<string>(
    () => localStorage.getItem(LS_SYSTEM_PROMPT) ?? DEFAULT_SYSTEM_PROMPT
  );
  const [systemOpen, setSystemOpen] = useState<boolean>(
    () => localStorage.getItem(LS_SYSTEM_OPEN) === "true"
  );

  function setSystemPrompt(v: string) {
    setSystemPromptRaw(v);
    localStorage.setItem(LS_SYSTEM_PROMPT, v);
  }
  function toggleSystemOpen() {
    const next = !systemOpen;
    setSystemOpen(next);
    localStorage.setItem(LS_SYSTEM_OPEN, String(next));
  }

  const abortRef    = useRef<AbortController | null>(null);
  const bottomRef   = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const { data: modelsData, isLoading: modelsLoading, error: modelsError } = useQuery({
    queryKey: ["ollama-models"],
    queryFn:  () => defaultApi.getOllamaModels(),
    retry: false, staleTime: 60_000, refetchOnWindowFocus: false,
  });

  const models      = modelsData?.models ?? [];
  const ollamaDown  = !!modelsError;
  const activeModel = selectedModel || models[0] || "";

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  function handleSubmit(e?: FormEvent) {
    e?.preventDefault();
    const trimmed = input.trim();
    if (!trimmed || !activeModel || streaming) return;

    const userMsg: Message    = { role: "user",      content: trimmed };
    const placeholder: Message = { role: "assistant", content: "", streaming: true };

    setMessages((prev) => [...prev, userMsg, placeholder]);
    setInput("");
    setStreaming(true);

    // Build history: system prompt first, then conversation turns, then new user message.
    const history: { role: string; content: string }[] = [];
    if (systemPrompt.trim()) {
      history.push({ role: "system", content: systemPrompt.trim() });
    }
    [...messages, userMsg].forEach((m) => history.push({ role: m.role, content: m.content }));

    abortRef.current = streamGenerate(
      activeModel,
      history,
      (token) => {
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant") {
            next[next.length - 1] = { ...last, content: last.content + token };
          }
          return next;
        });
      },
      (meta) => {
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant") {
            next[next.length - 1] = { ...last, streaming: false, meta };
          }
          return next;
        });
        setStreaming(false);
      },
      (msg) => {
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant") {
            next[next.length - 1] = { ...last, content: "", streaming: false, error: msg };
          }
          return next;
        });
        setStreaming(false);
      },
    );
  }

  function handleStop() {
    abortRef.current?.abort();
    setMessages((prev) => {
      const next = [...prev];
      const last = next[next.length - 1];
      if (last?.streaming) next[next.length - 1] = { ...last, streaming: false };
      return next;
    });
    setStreaming(false);
  }

  function handleClear() {
    handleStop();
    setMessages([]);
    setInput("");
    setTimeout(() => textareaRef.current?.focus(), 50);
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
      e.preventDefault();
      handleSubmit();
    }
  }

  const turnCount = messages.filter((m) => m.role === "user").length;

  return (
    <div className="flex flex-col h-full bg-zinc-950">
      {/* ── Toolbar ───────────────────────────────────────────────────── */}
      <div className="flex items-center gap-3 px-4 h-11 border-b border-zinc-800 shrink-0">
        <Terminal size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
          LLM Playground
        </span>

        {turnCount > 0 && (
          <span className="text-[10px] text-zinc-700">
            {turnCount} turn{turnCount !== 1 ? "s" : ""}
          </span>
        )}

        <div className="ml-auto flex items-center gap-3">
          {modelsLoading ? (
            <span className="text-[11px] text-zinc-600 flex items-center gap-1">
              <Loader2 size={10} className="animate-spin" /> Checking…
            </span>
          ) : ollamaDown ? (
            <span className="text-[11px] text-amber-600">Ollama offline</span>
          ) : (
            <span className="text-[11px] text-zinc-600 flex items-center gap-1.5">
              <Zap size={10} className="text-emerald-500" />
              {models.length} model{models.length !== 1 ? "s" : ""}
            </span>
          )}

          <div className="w-44">
            <ModelSelector
              value={activeModel}
              onChange={setSelectedModel}
              models={models}
              disabled={streaming}
            />
          </div>

          {/* Clear conversation — always visible when there are messages */}
          {messages.length > 0 && (
            <button
              onClick={handleClear}
              disabled={streaming}
              title="Clear conversation"
              className="flex items-center gap-1.5 text-[11px] text-zinc-600 hover:text-red-400
                         disabled:opacity-30 transition-colors px-1.5 py-1 rounded hover:bg-zinc-900"
            >
              <Trash2 size={12} />
              Clear
            </button>
          )}
        </div>
      </div>

      {/* ── System prompt (collapsible) ───────────────────────────────── */}
      <SystemPromptPanel
        value={systemPrompt}
        onChange={setSystemPrompt}
        open={systemOpen}
        onToggle={toggleSystemOpen}
        disabled={streaming}
      />

      {/* ── Chat messages ─────────────────────────────────────────────── */}
      <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4 min-h-0">
        {messages.length === 0
          ? <EmptyChat model={activeModel} />
          : messages.map((msg, i) => <ChatBubble key={i} msg={msg} />)
        }
        <div ref={bottomRef} />
      </div>

      {/* ── Input bar ─────────────────────────────────────────────────── */}
      <div className="shrink-0 px-4 py-3 border-t border-zinc-800">
        <form onSubmit={handleSubmit} className="flex gap-2 items-end">
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            disabled={streaming || ollamaDown || !activeModel}
            placeholder={
              ollamaDown      ? "Ollama offline" :
              !activeModel    ? "No model selected" :
                                "Message… (⌘↵ to send)"
            }
            rows={1}
            style={{ maxHeight: "120px" }}
            className="flex-1 rounded-lg border border-zinc-800 bg-zinc-900 text-[13px] text-zinc-200
                       placeholder-zinc-600 px-3 py-2.5 resize-none overflow-y-auto
                       focus:outline-none focus:border-indigo-500
                       disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
            onInput={(e) => {
              const el = e.currentTarget;
              el.style.height = "auto";
              el.style.height = `${Math.min(el.scrollHeight, 120)}px`;
            }}
          />

          {streaming ? (
            <button
              type="button"
              onClick={handleStop}
              className="shrink-0 flex items-center gap-1.5 rounded-lg px-3 h-10 text-[12px] font-medium
                         bg-red-900/40 border border-red-800/50 text-red-400
                         hover:bg-red-900/70 transition-colors"
            >
              <Square size={11} /> Stop
            </button>
          ) : (
            <button
              type="submit"
              disabled={!input.trim() || !activeModel || ollamaDown}
              className="shrink-0 w-10 h-10 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-white
                         disabled:bg-zinc-800 disabled:text-zinc-600 disabled:cursor-not-allowed
                         flex items-center justify-center transition-colors"
            >
              <Send size={14} />
            </button>
          )}
        </form>

        <p className="text-[10px] text-zinc-700 mt-1.5 text-center">
          ⌘↵ to send · system prompt + full history sent with each message
        </p>
      </div>
    </div>
  );
}

export default PlaygroundPage;
