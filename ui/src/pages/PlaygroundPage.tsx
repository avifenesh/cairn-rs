import {
  useState, useRef, useEffect, useCallback,
  type FormEvent, type KeyboardEvent,
} from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Terminal, Send, Loader2, AlertTriangle,
  Clock, Zap, ChevronDown, ChevronRight, Trash2, Square, Bot, User, Settings2,
  Plus, X, Copy, Check, PanelLeftClose, PanelLeft, GitCompare, Download,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { useToast } from "../components/Toast";

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_SYSTEM_PROMPT =
  "You are a helpful AI assistant running on cairn-rs, a self-hostable control plane for AI agents.";

const LS_SYSTEM_PROMPT  = "cairn_playground_system_prompt";
const LS_SYSTEM_OPEN    = "cairn_playground_system_open";
const LS_TEMPERATURE    = "cairn_playground_temperature";
const LS_MAX_TOKENS     = "cairn_playground_max_tokens";
const LS_CONVERSATIONS  = "cairn_playground_conversations";
const LS_ACTIVE_CONV    = "cairn_playground_active_conv";
const LS_SIDEBAR_OPEN   = "cairn_playground_sidebar_open";
const LS_COMPARE_MODEL  = "cairn_playground_compare_model";

const DEFAULT_TEMPERATURE = 0.7;
const DEFAULT_MAX_TOKENS  = 2048;
const MAX_CONVERSATIONS   = 50;

// ── Types ─────────────────────────────────────────────────────────────────────

type Role = "user" | "assistant";

interface MessageMeta {
  latency_ms: number;
  model: string;
  tokens_in?: number;
  tokens_out?: number;
}

interface Message {
  role: Role;
  content: string;
  meta?: MessageMeta;
  error?: string;
  streaming?: boolean;
}

interface Conversation {
  id: string;
  title: string;
  messages: Message[];
  timestamp: number;
  model: string;
}

// ── LocalStorage helpers ──────────────────────────────────────────────────────

function loadConversations(): Conversation[] {
  try { return JSON.parse(localStorage.getItem(LS_CONVERSATIONS) ?? "[]") as Conversation[]; }
  catch { return []; }
}

function persistConversations(convs: Conversation[]) {
  localStorage.setItem(LS_CONVERSATIONS, JSON.stringify(convs.slice(0, MAX_CONVERSATIONS)));
}

function autoTitle(firstMessage: string): string {
  const t = firstMessage.trim().slice(0, 42);
  return firstMessage.trim().length > 42 ? t + "…" : t;
}

function makeConvId(): string {
  return `conv-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
}

function fmtAgo(ts: number): string {
  const d = Date.now() - ts;
  if (d < 60_000)       return "now";
  if (d < 3_600_000)    return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000)   return `${Math.floor(d / 3_600_000)}h ago`;
  return new Date(ts).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

// ── Streaming helper ──────────────────────────────────────────────────────────

const API_BASE = import.meta.env.VITE_API_URL ?? "";
const getToken = () => localStorage.getItem("cairn_token") ?? import.meta.env.VITE_API_TOKEN ?? "";

interface GenerateParams {
  model: string;
  messages: { role: string; content: string }[];
  temperature: number;
  max_tokens: number;
}

function streamGenerate(
  params: GenerateParams,
  onToken: (text: string) => void,
  onDone: (meta: MessageMeta) => void,
  onError: (msg: string) => void,
): AbortController {
  const controller = new AbortController();
  (async () => {
    try {
      const resp = await fetch(`${API_BASE}/v1/providers/ollama/stream`, {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${getToken()}` },
        body: JSON.stringify(params),
        signal: controller.signal,
      });
      if (!resp.ok) { onError(`HTTP ${resp.status}: ${await resp.text().catch(() => "")}`); return; }
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
            const p = JSON.parse(data) as Record<string, unknown>;
            if (p.text !== undefined)          onToken(String(p.text));
            else if (p.latency_ms !== undefined) onDone({
              latency_ms: Number(p.latency_ms),
              model:      String(p.model ?? params.model),
              tokens_in:  p.tokens_in  !== undefined ? Number(p.tokens_in)  : undefined,
              tokens_out: p.tokens_out !== undefined ? Number(p.tokens_out) : undefined,
            });
            else if (p.error !== undefined)     onError(String(p.error));
          } catch { /* ignore */ }
        }
      }
    } catch (e: unknown) {
      if ((e as { name?: string })?.name === "AbortError") return;
      onError(e instanceof Error ? e.message : "Stream failed");
    }
  })();
  return controller;
}

// ── useChatStream hook ────────────────────────────────────────────────────────

function useChatStream() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [streaming, setStreaming] = useState(false);
  const abortRef = useRef<AbortController | null>(null);

  // Stable reference to messages for use inside callbacks
  const messagesRef = useRef(messages);
  messagesRef.current = messages;

  const submit = useCallback(function submit(
    model: string,
    userContent: string,
    systemPrompt: string,
    temperature: number,
    maxTokens: number,
    onComplete?: (finalMessages: Message[]) => void,
  ) {
    const base = messagesRef.current;
    const history: { role: string; content: string }[] = [];
    // Some models (e.g. gemma) don't support system/developer instructions.
    // Only include the system prompt for models that accept it.
    const skipSystem = model.includes('gemma');
    if (systemPrompt.trim() && !skipSystem) history.push({ role: "system", content: systemPrompt.trim() });
    [...base, { role: "user" as Role, content: userContent }].forEach((m) =>
      history.push({ role: m.role, content: m.content })
    );

    setMessages((prev) => [
      ...prev,
      { role: "user", content: userContent },
      { role: "assistant", content: "", streaming: true },
    ]);
    setStreaming(true);

    abortRef.current = streamGenerate(
      { model, messages: history, temperature, max_tokens: maxTokens },
      (token) =>
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant") next[next.length - 1] = { ...last, content: last.content + token };
          return next;
        }),
      (meta) => {
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant") next[next.length - 1] = { ...last, streaming: false, meta };
          onComplete?.(next);
          return next;
        });
        setStreaming(false);
      },
      (msg) => {
        setMessages((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.role === "assistant")
            next[next.length - 1] = { ...last, content: "", streaming: false, error: msg };
          return next;
        });
        setStreaming(false);
      },
    );
  }, []);

  const stop = useCallback(() => {
    abortRef.current?.abort();
    setMessages((prev) => {
      const next = [...prev];
      const last = next[next.length - 1];
      if (last?.streaming) next[next.length - 1] = { ...last, streaming: false };
      return next;
    });
    setStreaming(false);
  }, []);

  const clear = useCallback(() => {
    abortRef.current?.abort();
    setMessages([]);
    setStreaming(false);
  }, []);

  const load = useCallback((msgs: Message[]) => {
    abortRef.current?.abort();
    setMessages(msgs);
    setStreaming(false);
  }, []);

  return { messages, streaming, submit, stop, clear, load };
}


// ── Markdown renderer (regex-based, no external library) ──────────────────────

/** Parse inline markdown to React nodes: bold, italic, code, links. */
function parseInline(text: string, depth = 0): React.ReactNode {
  if (depth > 8 || !text) return text;
  type Rule = { re: RegExp; render: (m: RegExpExecArray) => React.ReactNode };
  const rules: Rule[] = [
    // Inline code: `code`
    { re: /`([^`\n]+)`/,
      render: (m) => (
        <code className="font-mono text-[11.5px] bg-zinc-700/70 text-amber-300 px-1 py-px rounded mx-0.5">
          {m[1]}
        </code>
      ),
    },
    // Bold: **text**
    { re: /\*\*([^*\n]+)\*\*/,
      render: (m) => <strong className="font-semibold text-gray-900 dark:text-zinc-100">{parseInline(m[1], depth + 1)}</strong>,
    },
    // Italic: *text* (not ** or ***)
    { re: /(?<!\*)\*(?!\*)([^*\n]+)(?<!\*)\*(?!\*)/,
      render: (m) => <em className="italic text-gray-700 dark:text-zinc-300">{parseInline(m[1], depth + 1)}</em>,
    },
    // Link: [text](url)
    { re: /\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/,
      render: (m) => (
        <a href={m[2]} target="_blank" rel="noopener noreferrer"
           className="text-indigo-400 underline underline-offset-2 hover:text-indigo-300 transition-colors">
          {parseInline(m[1], depth + 1)}
        </a>
      ),
    },
  ];

  let best: { idx: number; len: number; node: React.ReactNode } | null = null;
  for (const { re, render } of rules) {
    const m = re.exec(text);
    if (m !== null && (best === null || m.index < best.idx)) {
      best = { idx: m.index, len: m[0].length, node: render(m) };
    }
  }

  if (!best) return text;
  return (
    <>
      {best.idx > 0 ? text.slice(0, best.idx) : null}
      {best.node}
      {text.length > best.idx + best.len
        ? parseInline(text.slice(best.idx + best.len), depth + 1)
        : null}
    </>
  );
}

type MarkdownBlock =
  | { type: "h1" | "h2" | "h3"; content: string }
  | { type: "code"; lang: string; content: string }
  | { type: "list"; items: string[]; ordered: boolean }
  | { type: "hr" }
  | { type: "paragraph"; content: string };

function parseBlocks(text: string): MarkdownBlock[] {
  const lines = text.split("\n");
  const blocks: MarkdownBlock[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Fenced code block
    if (line.startsWith("```")) {
      const lang = line.slice(3).trim();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) { codeLines.push(lines[i]); i++; }
      i++; // skip closing ```
      blocks.push({ type: "code", lang, content: codeLines.join("\n") });
      continue;
    }

    // Headers
    if (line.startsWith("### ")) { blocks.push({ type: "h3", content: line.slice(4) }); i++; continue; }
    if (line.startsWith("## "))  { blocks.push({ type: "h2", content: line.slice(3) }); i++; continue; }
    if (line.startsWith("# "))   { blocks.push({ type: "h1", content: line.slice(2) }); i++; continue; }

    // HR
    if (/^[-_*]{3,}$/.test(line.trim())) { blocks.push({ type: "hr" }); i++; continue; }

    // List (unordered or ordered)
    if (/^[-*] /.test(line) || /^\d+\. /.test(line)) {
      const ordered = /^\d+\. /.test(line);
      const items: string[] = [];
      while (i < lines.length && (/^[-*] /.test(lines[i]) || /^\d+\. /.test(lines[i]))) {
        items.push(lines[i].replace(/^[-*] |^\d+\. /, "").replace(/^\d+\.\s/, ""));
        i++;
      }
      // Remove leading numbering properly
      const cleanItems = items.map(it => it.replace(/^\d+\.\s*/, ""));
      blocks.push({ type: "list", items: cleanItems, ordered });
      continue;
    }

    // Empty line
    if (line.trim() === "") { i++; continue; }

    // Paragraph — accumulate consecutive plain lines
    const paraLines: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() !== "" &&
      !lines[i].startsWith("```") &&
      !lines[i].startsWith("# ") &&
      !lines[i].startsWith("## ") &&
      !lines[i].startsWith("### ") &&
      !/^[-*] /.test(lines[i]) &&
      !/^\d+\. /.test(lines[i]) &&
      !/^[-_*]{3,}$/.test(lines[i].trim())
    ) {
      paraLines.push(lines[i]);
      i++;
    }
    if (paraLines.length > 0) {
      blocks.push({ type: "paragraph", content: paraLines.join("\n") });
    }
  }

  return blocks;
}

/** Copy-button that appears on code blocks. */
function CopyCodeButton({ code }: { code: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <button
      onClick={() => {
        void navigator.clipboard.writeText(code).then(() => {
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        });
      }}
      className="text-[10px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:text-zinc-300 transition-colors flex items-center gap-1"
    >
      {copied ? <><Check size={10} /> Copied</> : <><Copy size={10} /> Copy</>}
    </button>
  );
}

/** Render parsed markdown blocks. */
function MarkdownContent({ text }: { text: string }) {
  if (!text.trim()) return null;
  const blocks = parseBlocks(text);
  return (
    <div className="space-y-1.5 min-w-0">
      {blocks.map((block, i) => {
        switch (block.type) {
          case "h1":
            return <h1 key={i} className="text-[17px] font-bold text-gray-900 dark:text-zinc-100 mt-2 mb-1 leading-snug">{parseInline(block.content)}</h1>;
          case "h2":
            return <h2 key={i} className="text-[14px] font-semibold text-gray-800 dark:text-zinc-200 mt-2 mb-0.5 leading-snug">{parseInline(block.content)}</h2>;
          case "h3":
            return <h3 key={i} className="text-[13px] font-semibold text-gray-700 dark:text-zinc-300 mt-1.5 mb-0.5 leading-snug">{parseInline(block.content)}</h3>;
          case "code":
            return (
              <div key={i} className="my-1.5 rounded-lg overflow-hidden border border-gray-200 dark:border-zinc-700/60">
                <div className="flex items-center justify-between px-3 py-1.5 bg-gray-100 dark:bg-zinc-800 border-b border-gray-200 dark:border-zinc-700/60">
                  <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-500">{block.lang || "code"}</span>
                  <CopyCodeButton code={block.content} />
                </div>
                <pre className="p-3 bg-gray-100/50 dark:bg-zinc-800/50 overflow-x-auto text-[12px] leading-relaxed">
                  <code className="font-mono text-gray-800 dark:text-zinc-200 whitespace-pre">{block.content}</code>
                </pre>
              </div>
            );
          case "list": {
            const Tag = block.ordered ? "ol" : "ul";
            return (
              <Tag key={i} className={clsx("pl-5 space-y-0.5 my-1", block.ordered ? "list-decimal" : "list-disc")}>
                {block.items.map((item, j) => (
                  <li key={j} className="text-[13px] text-gray-800 dark:text-zinc-200 leading-relaxed">
                    {parseInline(item)}
                  </li>
                ))}
              </Tag>
            );
          }
          case "hr":
            return <hr key={i} className="border-gray-200 dark:border-zinc-700 my-2" />;
          case "paragraph":
            return (
              <p key={i} className="text-[13px] text-gray-800 dark:text-zinc-200 leading-relaxed whitespace-pre-wrap break-words">
                {parseInline(block.content)}
              </p>
            );
        }
      })}
    </div>
  );
}

// ── Export conversation ───────────────────────────────────────────────────────

function exportConversation(messages: Message[], model: string, systemPrompt: string) {
  const lines: string[] = [
    "# Conversation",
    `> **Model:** ${model}  `,
    `> **Exported:** ${new Date().toLocaleString()}`,
    "",
  ];

  if (systemPrompt.trim()) {
    lines.push("## System Prompt", "", systemPrompt, "");
  }

  for (const msg of messages) {
    lines.push(`## ${msg.role === "user" ? "User" : "Assistant"}`, "");
    if (msg.error) {
      lines.push(`> ⚠️ Error: ${msg.error}`, "");
    } else {
      lines.push(msg.content, "");
    }
    if (msg.meta) {
      const parts = [
        `${msg.meta.latency_ms >= 1000 ? (msg.meta.latency_ms / 1000).toFixed(1) + "s" : msg.meta.latency_ms + "ms"}`,
        msg.meta.model,
        ...(msg.meta.tokens_in  !== undefined ? [`${msg.meta.tokens_in}↑`]  : []),
        ...(msg.meta.tokens_out !== undefined ? [`${msg.meta.tokens_out}↓`] : []),
      ];
      lines.push(`*${parts.join(" · ")}*`, "");
    }
    lines.push("---", "");
  }

  const blob = new Blob([lines.join("\n")], { type: "text/markdown;charset=utf-8" });
  const url  = URL.createObjectURL(blob);
  const a    = document.createElement("a");
  a.href     = url;
  a.download = `conversation-${new Date().toISOString().slice(0, 10)}.md`;
  a.click();
  URL.revokeObjectURL(url);
}

// ── ModelSelector ─────────────────────────────────────────────────────────────

function ModelSelector({ value, onChange, models, disabled }: {
  value: string; onChange: (m: string) => void; models: string[]; disabled: boolean;
}) {
  return (
    <div className="relative">
      <select
        value={value}
        onChange={(e) => onChange(e.target.value)}
        disabled={disabled || models.length === 0}
        className="appearance-none w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900
                   text-[13px] text-gray-700 dark:text-zinc-300 px-2.5 py-1.5 pr-7
                   focus:outline-none focus:border-indigo-500
                   disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
      >
        {models.length === 0 && <option value="">No models</option>}
        {models.map((m) => <option key={m} value={m}>{m}</option>)}
      </select>
      <ChevronDown size={12} className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
    </div>
  );
}

// ── ModelSettings ─────────────────────────────────────────────────────────────

function ModelSettings({ temperature, onTemperature, maxTokens, onMaxTokens, disabled }: {
  temperature: number; onTemperature: (v: number) => void;
  maxTokens: number;   onMaxTokens: (v: number) => void;
  disabled: boolean;
}) {
  return (
    <div className="flex items-center gap-6 px-4 py-2 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 shrink-0">
      <div className="flex items-center gap-2.5 min-w-0">
        <label className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 whitespace-nowrap">Temp</label>
        <input type="range" min={0} max={2} step={0.1} value={temperature}
          onChange={(e) => onTemperature(Number(e.target.value))}
          disabled={disabled} className="w-24 accent-indigo-500 disabled:opacity-40 cursor-pointer" />
        <span className="text-[12px] font-mono text-gray-700 dark:text-zinc-300 w-8 text-right tabular-nums">
          {temperature.toFixed(1)}
        </span>
        {temperature === 0     && <span className="text-[10px] text-sky-500">deterministic</span>}
        {temperature >= 1.5    && <span className="text-[10px] text-amber-500">creative</span>}
      </div>
      <div className="w-px h-4 bg-gray-100 dark:bg-zinc-800 shrink-0" />
      <div className="flex items-center gap-2.5">
        <label className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 whitespace-nowrap">Max tokens</label>
        <input type="number" min={64} max={8192} step={64} value={maxTokens}
          onChange={(e) => onMaxTokens(Math.max(64, Math.min(8192, Number(e.target.value) || 64)))}
          disabled={disabled}
          className="w-20 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[12px] font-mono
                     text-gray-700 dark:text-zinc-300 px-2 py-1 text-right focus:outline-none focus:border-indigo-500
                     disabled:opacity-40 [appearance:textfield]
                     [&::-webkit-outer-spin-button]:appearance-none [&::-webkit-inner-spin-button]:appearance-none" />
      </div>
      <div className="ml-auto text-[10px] text-gray-300 dark:text-zinc-700 hidden lg:block">
        passed to model as <code className="text-gray-400 dark:text-zinc-600">temperature</code> · <code className="text-gray-400 dark:text-zinc-600">max_tokens</code>
      </div>
    </div>
  );
}

// ── SystemPromptPanel ─────────────────────────────────────────────────────────

function SystemPromptPanel({ value, onChange, open, onToggle, disabled }: {
  value: string; onChange: (v: string) => void;
  open: boolean; onToggle: () => void;
  disabled: boolean;
}) {
  const isDefault = value === DEFAULT_SYSTEM_PROMPT;
  return (
    <div className="border-b border-gray-200 dark:border-zinc-800 shrink-0">
      <button type="button" onClick={onToggle}
        aria-expanded={open}
        aria-controls="system-prompt-panel"
        className="w-full flex items-center gap-2 px-4 py-2 text-left hover:bg-gray-50/40 dark:bg-zinc-900/40 transition-colors">
        {open ? <ChevronDown size={12} className="text-gray-400 dark:text-zinc-500 shrink-0" />
               : <ChevronRight size={12} className="text-gray-400 dark:text-zinc-500 shrink-0" />}
        <Settings2 size={12} className="text-gray-400 dark:text-zinc-500 shrink-0" />
        <span className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">System Prompt</span>
        {!isDefault && <span className="ml-1 inline-block w-1.5 h-1.5 rounded-full bg-indigo-500 shrink-0" />}
        {!open && (
          <span className="ml-2 text-[11px] text-gray-400 dark:text-zinc-600 truncate max-w-xs">{value || <em>empty</em>}</span>
        )}
      </button>
      {open && (
        <div id="system-prompt-panel" className="px-4 pb-3 space-y-2">
          <textarea value={value} onChange={(e) => onChange(e.target.value)}
            disabled={disabled} rows={3}
            className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[13px] text-gray-700 dark:text-zinc-300
                       placeholder-zinc-600 px-3 py-2 resize-none
                       focus:outline-none focus:border-indigo-500 disabled:opacity-50 transition-colors"
            placeholder="System instructions for the model…" />
          <div className="flex items-center gap-3">
            {!isDefault && (
              <button type="button" onClick={() => onChange(DEFAULT_SYSTEM_PROMPT)}
                className="text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 transition-colors">
                Reset to default
              </button>
            )}
            <span className="ml-auto text-[11px] text-gray-300 dark:text-zinc-700">
              {value.length} chars · prepended as <code className="text-gray-400 dark:text-zinc-600">system</code> role
            </span>
          </div>
        </div>
      )}
    </div>
  );
}

// ── ChatBubble ────────────────────────────────────────────────────────────────

function ChatBubble({ msg }: { msg: Message }) {
  const isUser = msg.role === "user";
  const toast  = useToast();
  const [copied, setCopied] = useState(false);

  function handleCopy() {
    void navigator.clipboard.writeText(msg.content).then(() => {
      setCopied(true);
      toast.success("Copied!");
      setTimeout(() => setCopied(false), 1500);
    });
  }

  return (
    <div className={clsx("flex gap-2.5 max-w-[85%] group/bubble", isUser ? "ml-auto flex-row-reverse" : "mr-auto")}>
      <div className={clsx(
        "shrink-0 w-6 h-6 rounded-full flex items-center justify-center mt-0.5",
        isUser ? "bg-indigo-900/60 text-indigo-400" : "bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-500",
      )}>
        {isUser ? <User size={12} /> : <Bot size={12} />}
      </div>

      <div className="flex flex-col gap-1 min-w-0">
        <div className={clsx(
          "rounded-xl px-3.5 py-2 text-[13px] leading-relaxed relative",
          isUser
            ? "rounded-tr-sm bg-indigo-600 text-white"
            : clsx(
              "rounded-tl-sm bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 text-gray-800 dark:text-zinc-200",
              msg.error && "bg-red-950/40 border-red-800/40 text-red-300",
            ),
        )}>
          {msg.error ? (
            <span className="flex items-start gap-1.5">
              <AlertTriangle size={12} className="shrink-0 mt-0.5 text-red-400" />
              {msg.error}
            </span>
          ) : isUser ? (
            /* User messages: plain pre-wrap */
            <p className="whitespace-pre-wrap break-words text-[13px] leading-relaxed">
              {msg.content}
            </p>
          ) : (
            /* Assistant messages: markdown rendering */
            <>
              {msg.content
                ? <MarkdownContent text={msg.content} />
                : null}
              {msg.streaming && (
                <span className="inline-block w-0.5 h-3 bg-zinc-400 ml-0.5 mt-1 animate-pulse align-text-bottom" />
              )}
              {/* Copy button */}
              {!msg.streaming && msg.content && (
                <button
                  onClick={handleCopy}
                  title="Copy response"
                  className={clsx(
                    "absolute top-2 right-2 p-1 rounded transition-all",
                    "opacity-0 group-hover/bubble:opacity-100",
                    copied
                      ? "text-emerald-400 bg-emerald-950/40"
                      : "text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800",
                  )}
                >
                  {copied ? <Check size={11} /> : <Copy size={11} />}
                </button>
              )}
            </>
          )}
        </div>

        {msg.meta && !msg.streaming && (
          <div className="flex items-center flex-wrap gap-x-2 gap-y-0.5 px-1 text-[10px] text-gray-300 dark:text-zinc-700">
            <span className="flex items-center gap-1">
              <Clock size={9} />
              {msg.meta.latency_ms >= 1000
                ? `${(msg.meta.latency_ms / 1000).toFixed(1)}s`
                : `${msg.meta.latency_ms}ms`}
            </span>
            <span className="font-mono text-indigo-500/70 bg-indigo-950/40 px-1.5 py-px rounded text-[10px]">
              {msg.meta.model}
            </span>
            {(msg.meta.tokens_in !== undefined || msg.meta.tokens_out !== undefined) && (
              <span className="text-gray-300 dark:text-zinc-700 font-mono">
                {msg.meta.tokens_in ?? "—"}↑ {msg.meta.tokens_out ?? "—"}↓
              </span>
            )}
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

// ── EmptyChat ─────────────────────────────────────────────────────────────────

function EmptyChat({ model, noProviders }: { model: string; noProviders?: boolean }) {
  if (noProviders) {
    return (
      <div className="flex flex-col items-center justify-center flex-1 gap-3 text-center py-12 px-6">
        <div className="w-10 h-10 rounded-full bg-amber-950/40 border border-amber-800/30 flex items-center justify-center">
          <Bot size={18} className="text-amber-600" />
        </div>
        <div className="space-y-1">
          <p className="text-[13px] font-medium text-amber-500">No LLM providers available</p>
          <p className="text-[11px] text-gray-400 dark:text-zinc-600 max-w-xs">
            Configure at least one provider to use the Playground.
          </p>
        </div>
        <div className="rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 px-4 py-3 text-left max-w-xs space-y-1.5">
          <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-500 uppercase tracking-wider">Options</p>
          {[
            '1. Set OPENROUTER_API_KEY for OpenRouter',
            '2. Configure Bedrock via CAIRN_BRAIN_URL',
            '3. Add any OpenAI-compatible provider',
            '4. Optionally set OLLAMA_HOST for local models',
          ].map(step => (
            <p key={step} className="text-[11px] text-gray-500 dark:text-zinc-400 font-mono">{step}</p>
          ))}
        </div>
      </div>
    );
  }
  return (
    <div className="flex flex-col items-center justify-center flex-1 gap-3 text-center py-12">
      <div className="w-10 h-10 rounded-full bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 flex items-center justify-center">
        <Bot size={18} className="text-gray-400 dark:text-zinc-600" />
      </div>
      <div>
        <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">Start a conversation</p>
        <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-1">
          {model ? <>Model: <span className="font-mono text-gray-400 dark:text-zinc-500">{model}</span></> : "Select a model to begin"}
        </p>
      </div>
    </div>
  );
}

// ── ConversationSidebar ───────────────────────────────────────────────────────

function ConversationSidebar({ conversations, activeId, onNew, onSelect, onDelete }: {
  conversations: Conversation[];
  activeId: string | null;
  onNew: () => void;
  onSelect: (id: string) => void;
  onDelete: (id: string) => void;
}) {
  return (
    <div className="flex flex-col w-[200px] shrink-0 border-r border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 h-full overflow-hidden">
      <div className="flex items-center gap-2 px-3 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        <span className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider flex-1 truncate">
          History
        </span>
        <button
          onClick={onNew}
          title="New chat"
          className="p-1 rounded text-gray-400 dark:text-zinc-500 hover:text-gray-800 dark:text-zinc-200 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors shrink-0"
        >
          <Plus size={13} />
        </button>
      </div>

      <div className="flex-1 overflow-y-auto py-1">
        {conversations.length === 0 ? (
          <p className="px-3 py-6 text-[11px] text-gray-300 dark:text-zinc-700 text-center italic">No conversations yet</p>
        ) : (
          conversations.map((conv) => (
            <div
              key={conv.id}
              className={clsx(
                "group flex items-start gap-1 px-3 py-2 cursor-pointer transition-colors select-none",
                conv.id === activeId
                  ? "bg-gray-100/60 dark:bg-zinc-800/60"
                  : "hover:bg-gray-50/60 dark:bg-zinc-900/60",
              )}
              onClick={() => onSelect(conv.id)}
            >
              <div className="flex-1 min-w-0">
                <p className={clsx(
                  "text-[12px] leading-snug truncate",
                  conv.id === activeId ? "text-gray-800 dark:text-zinc-200" : "text-gray-500 dark:text-zinc-400",
                )}>
                  {conv.title || "Untitled"}
                </p>
                <p className="text-[10px] text-gray-300 dark:text-zinc-700 mt-0.5">{fmtAgo(conv.timestamp)}</p>
              </div>
              <button
                onClick={(e) => { e.stopPropagation(); onDelete(conv.id); }}
                title="Delete"
                className="opacity-0 group-hover:opacity-100 p-0.5 rounded text-gray-400 dark:text-zinc-600 hover:text-red-400
                           transition-all shrink-0 mt-0.5"
              >
                <X size={10} />
              </button>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

// ── CompareHeader (per-panel header in compare mode) ─────────────────────────

function CompareHeader({ label, model, onModelChange, models, streaming, onStop, onClear }: {
  label: string;
  model: string;
  onModelChange: (m: string) => void;
  models: string[];
  streaming: boolean;
  onStop: () => void;
  onClear: () => void;
}) {
  return (
    <div className="flex items-center gap-2 px-3 py-2 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-white dark:bg-zinc-950">
      <span className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider w-4 shrink-0">
        {label}
      </span>
      <div className="flex-1">
        <ModelSelector value={model} onChange={onModelChange} models={models} disabled={streaming} />
      </div>
      {streaming ? (
        <button onClick={onStop} title="Stop"
          className="shrink-0 p-1 rounded text-red-400 hover:bg-red-950/40 transition-colors">
          <Square size={11} />
        </button>
      ) : (
        <button onClick={onClear} title="Clear" disabled={streaming}
          className="shrink-0 p-1 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors">
          <Trash2 size={11} />
        </button>
      )}
    </div>
  );
}

// ── InputBar ──────────────────────────────────────────────────────────────────

function InputBar({ onSubmit, disabled, placeholder, streaming, onStop }: {
  onSubmit: (text: string) => void;
  disabled: boolean;
  placeholder: string;
  streaming: boolean;
  onStop: () => void;
}) {
  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  function handleSubmit(e?: FormEvent) {
    e?.preventDefault();
    const t = input.trim();
    if (!t || disabled) return;
    onSubmit(t);
    setInput("");
    // Reset height
    if (textareaRef.current) textareaRef.current.style.height = "auto";
  }

  function handleKeyDown(e: KeyboardEvent<HTMLTextAreaElement>) {
    if ((e.metaKey || e.ctrlKey) && e.key === "Enter") { e.preventDefault(); handleSubmit(); }
  }

  return (
    <div className="shrink-0 px-4 py-3 border-t border-gray-200 dark:border-zinc-800">
      <form onSubmit={handleSubmit} className="flex gap-2 items-end">
        <textarea
          ref={textareaRef}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          disabled={disabled}
          placeholder={placeholder}
          rows={1}
          style={{ maxHeight: "120px" }}
          className="flex-1 rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[13px] text-gray-800 dark:text-zinc-200
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
          <button type="button" onClick={onStop}
            className="shrink-0 flex items-center gap-1.5 rounded-lg px-3 h-10 text-[12px] font-medium
                       bg-red-900/40 border border-red-800/50 text-red-400 hover:bg-red-900/70 transition-colors">
            <Square size={11} /> Stop
          </button>
        ) : (
          <button type="submit" disabled={!input.trim() || disabled}
            className="shrink-0 w-10 h-10 rounded-lg bg-indigo-600 hover:bg-indigo-500 text-white
                       disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 disabled:cursor-not-allowed
                       flex items-center justify-center transition-colors">
            <Send size={14} />
          </button>
        )}
      </form>
      <p className="text-[10px] text-gray-300 dark:text-zinc-700 mt-1.5 text-center">
        ⌘↵ to send · system prompt + full history sent with each message
      </p>
    </div>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function PlaygroundPage() {
  // Sidebar / mode state
  const [sidebarOpen, setSidebarOpen]   = useState(() => localStorage.getItem(LS_SIDEBAR_OPEN) !== "false");
  const [compareMode, setCompareMode]   = useState(false);

  // Conversation history
  const [conversations, setConversations] = useState<Conversation[]>(loadConversations);
  const [activeConvId, setActiveConvId]   = useState<string | null>(() => localStorage.getItem(LS_ACTIVE_CONV));
  const activeConvIdRef = useRef(activeConvId);
  activeConvIdRef.current = activeConvId;

  // Shared generation settings — persisted
  const [systemPrompt, setSystemPromptRaw] = useState(
    () => localStorage.getItem(LS_SYSTEM_PROMPT) ?? DEFAULT_SYSTEM_PROMPT,
  );
  const [systemOpen, setSystemOpen] = useState(() => localStorage.getItem(LS_SYSTEM_OPEN) === "true");
  const [temperature, setTemperatureRaw]  = useState(
    () => parseFloat(localStorage.getItem(LS_TEMPERATURE) ?? String(DEFAULT_TEMPERATURE)),
  );
  const [maxTokens, setMaxTokensRaw] = useState(
    () => parseInt(localStorage.getItem(LS_MAX_TOKENS) ?? String(DEFAULT_MAX_TOKENS), 10),
  );

  function setSystemPrompt(v: string)  { setSystemPromptRaw(v); localStorage.setItem(LS_SYSTEM_PROMPT, v); }
  function toggleSystemOpen()          { const n = !systemOpen; setSystemOpen(n); localStorage.setItem(LS_SYSTEM_OPEN, String(n)); }
  function setTemperature(v: number)   { setTemperatureRaw(v);  localStorage.setItem(LS_TEMPERATURE, String(v)); }
  function setMaxTokens(v: number)     { setMaxTokensRaw(v);    localStorage.setItem(LS_MAX_TOKENS, String(v)); }

  // Chat streams — always instantiate both (hooks must not be conditional)
  const primary   = useChatStream();
  const secondary = useChatStream();

  // Model selection
  const [selectedModel, setSelectedModel]     = useState("");
  const [compareModel, setCompareModel]       = useState(() => localStorage.getItem(LS_COMPARE_MODEL) ?? "");
  function setCompareModelP(v: string) { setCompareModel(v); localStorage.setItem(LS_COMPARE_MODEL, v); }

  // Scroll to bottom ref
  const bottomRef1 = useRef<HTMLDivElement>(null);
  const bottomRef2 = useRef<HTMLDivElement>(null);

  useEffect(() => { bottomRef1.current?.scrollIntoView({ behavior: "smooth" }); }, [primary.messages]);
  useEffect(() => { bottomRef2.current?.scrollIntoView({ behavior: "smooth" }); }, [secondary.messages]);

  // ── Model discovery ─────────────────────────────────────────────────────
  // 1. Always fetch the provider registry FIRST — it reports which providers
  //    are available (env var configured) and their known models.
  // 2. Only query Ollama when the registry marks it as available.
  // 3. As a last resort, check provider connections for supported_models.

  const { data: registryData, isLoading: registryLoading } = useQuery({
    queryKey: ["provider-registry"],
    queryFn: () => defaultApi.getProviderRegistry(),
    retry: false, staleTime: 120_000, refetchOnWindowFocus: false,
  });

  // Only query Ollama if the registry says it's available (OLLAMA_HOST set).
  const ollamaAvailable = (registryData ?? []).some(p => p.id === "ollama" && p.available);

  const { data: modelsData, isLoading: ollamaLoading } = useQuery({
    queryKey: ["ollama-models"],
    queryFn: () => defaultApi.getOllamaModels(),
    retry: false, staleTime: 60_000, refetchOnWindowFocus: false,
    enabled: ollamaAvailable,
  });

  // Fetch configured default models from the runtime settings.
  // These are set via CAIRN_DEFAULT_STREAM_MODEL, CAIRN_BRAIN_MODEL, etc.
  // Returns null on 404 (setting not configured), so we catch errors gracefully.
  const resolveSetting = async (key: string) => {
    try { return await defaultApi.resolveDefaultSetting(key); }
    catch { return null; }
  };
  const { data: streamModelSetting } = useQuery({
    queryKey: ["default-setting", "stream_model"],
    queryFn: () => resolveSetting("stream_model"),
    retry: false, staleTime: 120_000, refetchOnWindowFocus: false,
  });
  const { data: brainModelSetting } = useQuery({
    queryKey: ["default-setting", "brain_model"],
    queryFn: () => resolveSetting("brain_model"),
    retry: false, staleTime: 120_000, refetchOnWindowFocus: false,
  });
  const { data: generateModelSetting } = useQuery({
    queryKey: ["default-setting", "generate_model"],
    queryFn: () => resolveSetting("generate_model"),
    retry: false, staleTime: 120_000, refetchOnWindowFocus: false,
  });

  const { data: connectionsData } = useQuery({
    queryKey: ["provider-connections"],
    queryFn:  () => defaultApi.listProviderConnections("default"),
    retry: false, staleTime: 60_000, refetchOnWindowFocus: false,
  });

  const ollamaModels: string[] = modelsData?.models ?? [];

  // Configured models from runtime settings (e.g. CAIRN_DEFAULT_STREAM_MODEL,
  // CAIRN_BRAIN_MODEL).  These are the models the operator chose for this
  // deployment and should appear first in the selector.
  const configuredModels: string[] = [
    streamModelSetting?.value,
    brainModelSetting?.value,
    generateModelSetting?.value,
  ].filter((v): v is string => typeof v === "string" && v.length > 0);

  // Build model list from available providers in the registry.
  // Skip "ollama" (handled separately) and "anthropic" (non-OpenAI wire format).
  const registryModels: string[] = (registryData ?? [])
    .filter(p => p.available && p.id !== "ollama" && p.api_format !== "anthropic")
    .flatMap(p => {
      // If the provider has known models, list them. Otherwise expose the default.
      if (p.models.length > 0) return p.models.map(m => m.id);
      return p.default_model ? [p.default_model] : [];
    });

  const connectionModels: string[] = (connectionsData?.items ?? []).flatMap(c => c.supported_models ?? []);

  // Merge: configured models first, then Ollama, then registry, then connections.
  const allModels = [
    ...configuredModels,
    ...ollamaModels,
    ...registryModels,
    ...connectionModels,
  ];
  // Deduplicate while preserving order
  const uniqueModels = [...new Set(allModels)];

  // Sort free-tier models first; prefer models that support system prompts
  // (llama, qwen, deepseek) over those that don't (gemma).
  const models = [...uniqueModels].sort((a, b) => {
    const aFree = a.includes(':free');
    const bFree = b.includes(':free');
    if (aFree && !bFree) return -1;
    if (!aFree && bFree) return 1;
    // Among free models, prefer instruction-tuned (llama/qwen/deepseek) over gemma
    if (aFree && bFree) {
      const aGemma = a.includes('gemma');
      const bGemma = b.includes('gemma');
      if (!aGemma && bGemma) return -1;
      if (aGemma && !bGemma) return 1;
    }
    return 0;
  });

  const modelsLoading = registryLoading || ollamaLoading;

  // "No providers" only when there are zero models from any source.
  const noProviders = !modelsLoading && models.length === 0;
  const activeModel = selectedModel || models[0] || "";
  const cmpModel    = compareModel  || models[1] || models[0] || "";

  const anyStreaming = primary.streaming || (compareMode && secondary.streaming);

  // ── Conversation helpers ──────────────────────────────────────────────────

  function saveConversation(msgs: Message[]) {
    const convId = activeConvIdRef.current;
    if (!convId || compareMode) return;
    setConversations((prev) => {
      const existing = prev.find((c) => c.id === convId);
      let updated: Conversation[];
      if (existing) {
        updated = prev.map((c) =>
          c.id === convId ? { ...c, messages: msgs, timestamp: Date.now(), model: activeModel } : c,
        );
      } else {
        const title = autoTitle(msgs.find((m) => m.role === "user")?.content ?? "");
        updated = [{ id: convId, title, messages: msgs, timestamp: Date.now(), model: activeModel }, ...prev];
      }
      persistConversations(updated);
      return updated;
    });
  }

  function handleNew() {
    primary.clear();
    const newId = makeConvId();
    setActiveConvId(newId);
    localStorage.setItem(LS_ACTIVE_CONV, newId);
    activeConvIdRef.current = newId;
  }

  function handleSelectConv(id: string) {
    const conv = conversations.find((c) => c.id === id);
    if (!conv) return;
    primary.load(conv.messages);
    setActiveConvId(id);
    localStorage.setItem(LS_ACTIVE_CONV, id);
    if (conv.model) setSelectedModel(conv.model);
  }

  function handleDeleteConv(id: string) {
    setConversations((prev) => {
      const next = prev.filter((c) => c.id !== id);
      persistConversations(next);
      return next;
    });
    if (activeConvId === id) {
      primary.clear();
      setActiveConvId(null);
      localStorage.removeItem(LS_ACTIVE_CONV);
    }
  }

  function toggleSidebar() {
    setSidebarOpen((v) => { localStorage.setItem(LS_SIDEBAR_OPEN, String(!v)); return !v; });
  }

  // ── Submit ────────────────────────────────────────────────────────────────

  function handleSubmit(text: string) {
    if (!text || !activeModel || anyStreaming) return;

    // Ensure there's an active conv for single mode
    if (!compareMode && !activeConvIdRef.current) {
      const newId = makeConvId();
      setActiveConvId(newId);
      localStorage.setItem(LS_ACTIVE_CONV, newId);
      activeConvIdRef.current = newId;
    }

    primary.submit(activeModel, text, systemPrompt, temperature, maxTokens,
      compareMode ? undefined : saveConversation,
    );

    if (compareMode) {
      secondary.submit(cmpModel, text, systemPrompt, temperature, maxTokens);
    }
  }

  function handleStopAll() {
    primary.stop();
    if (compareMode) secondary.stop();
  }

  function handleToggleCompare() {
    if (!compareMode) {
      secondary.clear();
    }
    setCompareMode((v) => !v);
  }

  const turnCount = primary.messages.filter((m) => m.role === "user").length;
  const inputDisabled = !activeModel || noProviders;
  const inputPlaceholder =
    noProviders  ? "No LLM providers available" :
    !activeModel ? "No model selected" :
                   "Message… (⌘↵ to send)";

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-950">
      {/* ── Toolbar ─────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-3 px-4 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        {/* Sidebar toggle */}
        <button
          onClick={toggleSidebar}
          title={sidebarOpen ? "Close history" : "Open history"}
          aria-expanded={sidebarOpen}
          aria-label={sidebarOpen ? "Close conversation history" : "Open conversation history"}
          className="p-1 rounded text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors"
        >
          {sidebarOpen ? <PanelLeftClose size={14} /> : <PanelLeft size={14} />}
        </button>

        <Terminal size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">LLM Playground</span>

        {turnCount > 0 && !compareMode && (
          <span className="text-[10px] text-gray-300 dark:text-zinc-700">{turnCount} turn{turnCount !== 1 ? "s" : ""}</span>
        )}

        <div className="ml-auto flex items-center gap-3">
          {/* Provider status */}
          {modelsLoading ? (
            <span className="text-[11px] text-gray-400 dark:text-zinc-600 flex items-center gap-1">
              <Loader2 size={10} className="animate-spin" /> Checking…
            </span>
          ) : noProviders ? (
            <span
              className="text-[11px] text-amber-600 cursor-help"
              title="No LLM providers configured. Add one under Providers."
            >
              ⚠ No providers
            </span>
          ) : (
            <span className="text-[11px] text-gray-400 dark:text-zinc-600 flex items-center gap-1.5 hidden sm:flex">
              <Zap size={10} className="text-emerald-500" />
              {models.length} model{models.length !== 1 ? "s" : ""}
            </span>
          )}

          {/* Compare mode toggle */}
          <button
            onClick={handleToggleCompare}
            title={compareMode ? "Exit compare mode" : "Compare two models"}
            className={clsx(
              "flex items-center gap-1.5 rounded px-2 py-1 text-[11px] font-medium transition-colors",
              compareMode
                ? "bg-indigo-600/20 text-indigo-400 border border-indigo-700/40"
                : "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800",
            )}
          >
            <GitCompare size={12} />
            <span className="hidden sm:inline">Compare</span>
          </button>

          {/* Model selector (single mode) */}
          {!compareMode && (
            <div className="w-44">
              <ModelSelector value={activeModel} onChange={setSelectedModel} models={models} disabled={anyStreaming} />
            </div>
          )}

          {/* Export + Clear (single mode) */}
          {!compareMode && primary.messages.length > 0 && (
            <>
              <button
                onClick={() => exportConversation(primary.messages, activeModel, systemPrompt)}
                title="Export conversation as Markdown"
                disabled={primary.streaming}
                className="flex items-center gap-1.5 text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:text-zinc-300
                           disabled:opacity-30 transition-colors px-1.5 py-1 rounded hover:bg-gray-50 dark:bg-zinc-900">
                <Download size={12} />
                <span className="hidden sm:inline">Export</span>
              </button>
              <button onClick={() => { primary.stop(); primary.clear(); }} disabled={primary.streaming}
                title="Clear conversation"
                className="flex items-center gap-1.5 text-[11px] text-gray-400 dark:text-zinc-600 hover:text-red-400
                           disabled:opacity-30 transition-colors px-1.5 py-1 rounded hover:bg-gray-50 dark:bg-zinc-900">
                <Trash2 size={12} />
                <span className="hidden sm:inline">Clear</span>
              </button>
            </>
          )}
        </div>
      </div>

      {/* ── Shared settings ──────────────────────────────────────────────── */}
      <SystemPromptPanel
        value={systemPrompt} onChange={setSystemPrompt}
        open={systemOpen} onToggle={toggleSystemOpen}
        disabled={anyStreaming}
      />
      <ModelSettings
        temperature={temperature} onTemperature={setTemperature}
        maxTokens={maxTokens} onMaxTokens={setMaxTokens}
        disabled={anyStreaming}
      />

      {/* ── Body ─────────────────────────────────────────────────────────── */}
      <div className="flex flex-1 min-h-0 overflow-hidden">

        {/* Conversation history sidebar */}
        {sidebarOpen && !compareMode && (
          <ConversationSidebar
            conversations={conversations}
            activeId={activeConvId}
            onNew={handleNew}
            onSelect={handleSelectConv}
            onDelete={handleDeleteConv}
          />
        )}

        {/* Single chat panel */}
        {!compareMode && (
          <div className="flex flex-col flex-1 min-w-0 overflow-hidden">
            <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4 min-h-0">
              {primary.messages.length === 0
                ? <EmptyChat model={activeModel} noProviders={noProviders} />
                : primary.messages.map((msg, i) => <ChatBubble key={`msg-${i}-${msg.role}`} msg={msg} />)
              }
              <div ref={bottomRef1} />
            </div>
            <InputBar
              onSubmit={handleSubmit}
              disabled={inputDisabled || primary.streaming}
              placeholder={inputPlaceholder}
              streaming={primary.streaming}
              onStop={primary.stop}
            />
          </div>
        )}

        {/* Compare mode: two panels + shared input */}
        {compareMode && (
          <div className="flex flex-col flex-1 min-w-0 overflow-hidden">
            <div className="flex flex-1 min-h-0 overflow-hidden">
              {/* Panel A */}
              <div className="flex flex-col flex-1 min-w-0 overflow-hidden border-r border-gray-200 dark:border-zinc-800">
                <CompareHeader
                  label="A"
                  model={activeModel}
                  onModelChange={setSelectedModel}
                  models={models}
                  streaming={primary.streaming}
                  onStop={primary.stop}
                  onClear={primary.clear}
                />
                <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4 min-h-0">
                  {primary.messages.length === 0
                    ? <EmptyChat model={activeModel} noProviders={noProviders} />
                    : primary.messages.map((msg, i) => <ChatBubble key={`msg-${i}-${msg.role}`} msg={msg} />)
                  }
                  <div ref={bottomRef1} />
                </div>
              </div>

              {/* Panel B */}
              <div className="flex flex-col flex-1 min-w-0 overflow-hidden">
                <CompareHeader
                  label="B"
                  model={cmpModel}
                  onModelChange={setCompareModelP}
                  models={models}
                  streaming={secondary.streaming}
                  onStop={secondary.stop}
                  onClear={secondary.clear}
                />
                <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4 min-h-0">
                  {secondary.messages.length === 0
                    ? <EmptyChat model={cmpModel} noProviders={noProviders} />
                    : secondary.messages.map((msg, i) => <ChatBubble key={`msg-${i}-${msg.role}`} msg={msg} />)
                  }
                  <div ref={bottomRef2} />
                </div>
              </div>
            </div>

            {/* Shared input for compare mode */}
            <InputBar
              onSubmit={handleSubmit}
              disabled={inputDisabled || anyStreaming}
              placeholder={inputPlaceholder}
              streaming={anyStreaming}
              onStop={handleStopAll}
            />
          </div>
        )}
      </div>
    </div>
  );
}

export default PlaygroundPage;
