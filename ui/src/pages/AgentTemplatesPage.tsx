import { useState } from "react";
import { useQuery, useMutation } from "@tanstack/react-query";
import {
  BookOpen, Code2, BarChart3, Loader2, Zap, Check,
  ChevronRight, Wrench, Shield, ShieldOff, Play, X,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { useToast } from "../components/Toast";
import type { AgentTemplate } from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

const ICONS: Record<string, React.ReactNode> = {
  BookOpen: <BookOpen size={20} />,
  Code2:    <Code2    size={20} />,
  BarChart3:<BarChart3 size={20} />,
};

const ICON_COLORS: Record<string, string> = {
  BookOpen: "text-sky-400 bg-sky-950/40 border-sky-800/40",
  Code2:    "text-indigo-400 bg-indigo-950/40 border-indigo-800/40",
  BarChart3:"text-emerald-400 bg-emerald-950/40 border-emerald-800/40",
};

const POLICY_LABEL: Record<string, { label: string; color: string; icon: React.ReactNode }> = {
  none:      { label: "No approval",    color: "text-gray-500 dark:text-zinc-400 bg-gray-100/60 dark:bg-zinc-800/60 border-gray-200 dark:border-zinc-700", icon: <ShieldOff size={10} /> },
  sensitive: { label: "Sensitive ops",  color: "text-amber-400 bg-amber-950/40 border-amber-800/40", icon: <Shield size={10} /> },
  all:       { label: "All ops",        color: "text-red-400 bg-red-950/40 border-red-800/40",  icon: <Shield size={10} /> },
};

// ── Instantiate modal ─────────────────────────────────────────────────────────

interface InstantiateModalProps {
  template: AgentTemplate;
  onClose:  () => void;
  onDone:   (runId: string) => void;
}

function InstantiateModal({ template, onClose, onDone }: InstantiateModalProps) {
  const [goal, setGoal] = useState("");
  const toast = useToast();

  const mutation = useMutation({
    mutationFn: () => defaultApi.instantiateAgentTemplate(template.id, { goal: goal.trim() }),
    onSuccess: (result) => {
      toast.success(`Agent started — Run ${result.run_id.slice(-8)}`);
      onDone(result.run_id);
    },
    onError: (e) => toast.error(e instanceof Error ? e.message : "Failed to instantiate"),
  });

  const placeholder = template.id === "knowledge-assistant"
    ? "e.g. Summarise recent papers on transformer architectures"
    : template.id === "code-reviewer"
    ? "e.g. Review the auth module in src/auth/ for security issues"
    : "e.g. Fetch Q1 sales data from the API and calculate growth";

  return (
    <>
      <div className="fixed inset-0 z-40 bg-black/70" onClick={onClose} />
      <div className="fixed inset-0 z-50 flex items-center justify-center p-4">
        <div className="w-full max-w-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-xl shadow-2xl flex flex-col">
          {/* Header */}
          <div className="flex items-center justify-between px-5 py-4 border-b border-gray-200 dark:border-zinc-800">
            <div className="flex items-center gap-3">
              <div className={clsx(
                "w-8 h-8 rounded-lg border flex items-center justify-center shrink-0",
                ICON_COLORS[template.icon] ?? "text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700",
              )}>
                {ICONS[template.icon] ?? <Zap size={16} />}
              </div>
              <div>
                <p className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">{template.name}</p>
                <p className="text-[11px] text-gray-400 dark:text-zinc-500">Enter a goal to start</p>
              </div>
            </div>
            <button onClick={onClose} className="text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors">
              <X size={16} />
            </button>
          </div>

          {/* Body */}
          <div className="px-5 py-4 space-y-4">
            <div>
              <label className="block text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wide mb-2">
                Goal / instruction <span className="text-red-400 normal-case">*</span>
              </label>
              <textarea
                value={goal}
                onChange={e => setGoal(e.target.value)}
                placeholder={placeholder}
                rows={3}
                className="w-full rounded-lg bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 px-3 py-2.5 text-[13px] text-gray-800 dark:text-zinc-200 placeholder-zinc-600 resize-none focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500/30 transition-colors"
              />
            </div>

            {/* Tools preview */}
            <div>
              <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wide mb-1.5">Pre-loaded tools</p>
              <div className="flex flex-wrap gap-1.5">
                {template.default_tools.map(t => (
                  <span key={t} className="flex items-center gap-1 text-[10px] font-mono text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-1.5 py-0.5">
                    <Wrench size={9} className="text-gray-400 dark:text-zinc-600" />{t}
                  </span>
                ))}
              </div>
            </div>
          </div>

          {/* Footer */}
          <div className="flex items-center justify-between px-5 py-3 border-t border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 rounded-b-xl">
            <button
              onClick={onClose}
              className="text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
            >Cancel</button>
            <button
              onClick={() => mutation.mutate()}
              disabled={mutation.isPending || !goal.trim()}
              className="flex items-center gap-1.5 px-4 py-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 text-white text-[12px] font-medium transition-colors shadow-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-indigo-400 focus-visible:ring-offset-2 focus-visible:ring-offset-white dark:focus-visible:ring-offset-zinc-950 disabled:bg-gray-300 dark:disabled:bg-zinc-700 disabled:text-gray-500 dark:disabled:text-zinc-500 disabled:cursor-not-allowed disabled:shadow-none"
            >
              {mutation.isPending
                ? <><Loader2 size={12} className="animate-spin" /> Starting…</>
                : <><Play size={12} /> Instantiate Agent</>}
            </button>
          </div>
        </div>
      </div>
    </>
  );
}

// ── Template card ─────────────────────────────────────────────────────────────

interface TemplateCardProps {
  template: AgentTemplate;
  onInstantiate: (t: AgentTemplate) => void;
  recentRunId?: string;
}

function TemplateCard({ template, onInstantiate, recentRunId }: TemplateCardProps) {
  const [expanded, setExpanded] = useState(false);
  const policy = POLICY_LABEL[template.approval_policy] ?? POLICY_LABEL.none;

  return (
    <div className="rounded-xl border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60 overflow-hidden flex flex-col hover:border-gray-200 dark:border-zinc-700 transition-colors">
      {/* Header */}
      <div className="px-5 pt-5 pb-4">
        <div className="flex items-start gap-4">
          <div className={clsx(
            "w-10 h-10 rounded-xl border flex items-center justify-center shrink-0",
            ICON_COLORS[template.icon] ?? "text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700",
          )}>
            {ICONS[template.icon] ?? <Zap size={20} />}
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-[14px] font-semibold text-gray-900 dark:text-zinc-100">{template.name}</p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-0.5 leading-relaxed">{template.description}</p>
          </div>
        </div>

        {/* Badges */}
        <div className="flex items-center gap-2 mt-3">
          <span className={clsx(
            "flex items-center gap-1 text-[10px] font-medium px-1.5 py-0.5 rounded border",
            policy.color,
          )}>
            {policy.icon} {policy.label}
          </span>
          <span className="text-[10px] text-gray-400 dark:text-zinc-600 font-mono bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-1.5 py-0.5">
            {template.agent_role}
          </span>
        </div>
      </div>

      {/* Tools */}
      <div className="px-5 py-3 border-t border-gray-200/60 dark:border-zinc-800/60">
        <div className="flex items-center justify-between mb-2">
          <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wide">Tools</p>
          <button
            onClick={() => setExpanded(v => !v)}
            className="text-[10px] text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 flex items-center gap-0.5 transition-colors"
          >
            {expanded ? "less" : "prompt"} <ChevronRight size={10} className={expanded ? "rotate-90" : ""} />
          </button>
        </div>
        <div className="flex flex-wrap gap-1.5">
          {template.default_tools.map(t => (
            <span key={t} className="flex items-center gap-1 text-[10px] font-mono text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-1.5 py-0.5">
              <Wrench size={9} className="text-gray-400 dark:text-zinc-600" />{t}
            </span>
          ))}
        </div>

        {/* Expandable prompt preview */}
        {expanded && (
          <div className="mt-3 rounded-lg bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 p-3">
            <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wide mb-1.5">Default system prompt</p>
            <p className="text-[11px] text-gray-500 dark:text-zinc-400 leading-relaxed">{template.default_prompt}</p>
          </div>
        )}
      </div>

      {/* Footer */}
      <div className="px-5 py-3 border-t border-gray-200/60 dark:border-zinc-800/60 mt-auto bg-gray-50/40 dark:bg-zinc-900/40 flex items-center justify-between">
        {recentRunId ? (
          <span className="flex items-center gap-1 text-[10px] text-emerald-400">
            <Check size={10} /> Instantiated
          </span>
        ) : (
          <span />
        )}
        <button
          onClick={() => onInstantiate(template)}
          className="flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-indigo-600 hover:bg-indigo-500 text-white text-[12px] font-medium transition-colors shadow-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-indigo-400 focus-visible:ring-offset-2 focus-visible:ring-offset-white dark:focus-visible:ring-offset-zinc-950"
        >
          <Play size={11} /> Instantiate
        </button>
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function AgentTemplatesPage() {
  const [modal, setModal]         = useState<AgentTemplate | null>(null);
  const [recentRuns, setRecentRuns] = useState<Record<string, string>>({});

  const { data: templates, isLoading, isError } = useQuery({
    queryKey: ["agent-templates"],
    queryFn:  () => defaultApi.listAgentTemplates(),
    staleTime: 5 * 60_000,
  });

  const handleDone = (templateId: string, runId: string) => {
    setRecentRuns(prev => ({ ...prev, [templateId]: runId }));
    setModal(null);
    // Navigate immediately: `instantiateAgentTemplate` is synchronous and the
    // 201 response already contains the created run_id (issue #161 — no race).
    window.location.hash = `run/${runId}`;
  };

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-5 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">Agent Templates</span>
        <span className="text-[11px] text-gray-400 dark:text-zinc-600">
          Pre-configured agents ready to instantiate
        </span>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-5">
        {isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading templates…</span>
          </div>
        ) : isError ? (
          <div className="text-center py-12">
            <p className="text-[13px] text-red-400">Failed to load templates.</p>
          </div>
        ) : (
          <div className="max-w-5xl space-y-6">
            {/* Intro */}
            <div className="rounded-xl border border-indigo-800/40 bg-indigo-950/20 px-5 py-4">
              <p className="text-[13px] font-medium text-indigo-300 mb-1">
                Start in seconds with a pre-built agent
              </p>
              <p className="text-[12px] text-gray-400 dark:text-zinc-500 leading-relaxed">
                Each template comes with a curated tool set and system prompt. Enter your goal,
                click Instantiate, and the agent runs immediately using your configured providers.
              </p>
            </div>

            {/* Template grid */}
            <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
              {(templates ?? []).map(t => (
                <TemplateCard
                  key={t.id}
                  template={t}
                  onInstantiate={setModal}
                  recentRunId={recentRuns[t.id]}
                />
              ))}
            </div>

            {/* Custom agent hint */}
            <div className="rounded-lg border border-gray-200/60 dark:border-zinc-800/60 bg-gray-50/40 dark:bg-zinc-900/40 px-5 py-4 flex items-start gap-3">
              <Zap size={14} className="text-gray-400 dark:text-zinc-600 mt-0.5 shrink-0" />
              <div>
                <p className="text-[12px] font-medium text-gray-500 dark:text-zinc-400">Need a custom configuration?</p>
                <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">
                  Use the Providers page to configure your tools, then create a run with your own agent_role and prompt.
                </p>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Instantiate modal */}
      {modal && (
        <InstantiateModal
          template={modal}
          onClose={() => setModal(null)}
          onDone={(runId) => handleDone(modal.id, runId)}
        />
      )}
    </div>
  );
}

export default AgentTemplatesPage;
