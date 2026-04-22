import { useState, useEffect, useCallback, useRef } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Loader2, GitBranch, Play, Pause, SkipForward,
  RotateCcw, Search, CheckCircle2, XCircle, Clock, Zap,
  ExternalLink, AlertTriangle, Inbox, Cable,
} from "lucide-react";
import { clsx } from "clsx";
import { useToast } from "../components/Toast";
import { defaultApi } from "../lib/api";
import type { GitHubQueueEntry } from "../lib/api";
import { surface, border, text } from "../lib/design-system";
import { PageHeader } from "../components/PageHeader";
import { StatCard } from "../components/StatCard";
import { useEventStream } from "../hooks/useEventStream";

// ── Status helpers ──────────────────────────────────────────────────────────

function statusColor(status: string): string {
  if (status.includes("Pending")) return "text-zinc-400";
  if (status.includes("Processing")) return "text-blue-400";
  if (status.includes("WaitingApproval")) return "text-amber-400";
  if (status.includes("Completed")) return "text-emerald-400";
  if (status.includes("Failed")) return "text-red-400";
  return "text-zinc-500";
}

function statusIcon(status: string) {
  if (status.includes("Pending")) return <Clock size={14} className="text-zinc-400" />;
  if (status.includes("Processing")) return <Loader2 size={14} className="animate-spin text-blue-400" />;
  if (status.includes("WaitingApproval")) return <AlertTriangle size={14} className="text-amber-400" />;
  if (status.includes("Completed")) return <CheckCircle2 size={14} className="text-emerald-400" />;
  if (status.includes("Failed")) return <XCircle size={14} className="text-red-400" />;
  return <Clock size={14} className="text-zinc-500" />;
}

function statusLabel(status: string): string {
  if (status.includes("Pending")) return "Queued";
  if (status.includes("Processing")) return "Working...";
  if (status.includes("WaitingApproval")) return "Awaiting Review";
  if (status.includes("Completed")) return "Done";
  if (status.includes("Failed")) {
    const reason = status.match(/Failed\("(.+?)"\)/)?.[1];
    return reason ? `Failed: ${reason}` : "Failed";
  }
  return status;
}

// ── Scan dialog ─────────────────────────────────────────────────────────────

function ScanDialog({ onClose, onScan }: {
  onClose: () => void;
  onScan: (repo: string, labels?: string, limit?: number) => void;
}) {
  const [repo, setRepo] = useState("");
  const [labels, setLabels] = useState("");
  const [limit, setLimit] = useState(30);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className={clsx("w-full max-w-md rounded-xl border p-6 shadow-xl", surface.modal, border.default)}
        onClick={(e) => e.stopPropagation()}
      >
        <h3 className={clsx("text-lg font-semibold mb-4", text.heading)}>Scan Repository Issues</h3>

        <div className="space-y-4">
          <div>
            <label className={clsx("block text-sm font-medium mb-1", text.secondary)}>
              Repository
            </label>
            <input
              type="text"
              value={repo}
              onChange={(e) => setRepo(e.target.value)}
              placeholder="owner/repo"
              className={clsx(
                "w-full rounded-lg border px-3 py-2 text-sm",
                surface.elevated, border.default, text.body,
                "placeholder:text-zinc-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/40"
              )}
              autoFocus
            />
          </div>

          <div>
            <label className={clsx("block text-sm font-medium mb-1", text.secondary)}>
              Label filter <span className={text.muted}>(optional, comma-separated)</span>
            </label>
            <input
              type="text"
              value={labels}
              onChange={(e) => setLabels(e.target.value)}
              placeholder="bug, enhancement"
              className={clsx(
                "w-full rounded-lg border px-3 py-2 text-sm",
                surface.elevated, border.default, text.body,
                "placeholder:text-zinc-500 focus:outline-none focus:ring-2 focus:ring-indigo-500/40"
              )}
            />
          </div>

          <div>
            <label className={clsx("block text-sm font-medium mb-1", text.secondary)}>
              Max issues
            </label>
            <input
              type="number"
              value={limit}
              onChange={(e) => setLimit(Number(e.target.value))}
              min={1}
              max={100}
              className={clsx(
                "w-24 rounded-lg border px-3 py-2 text-sm",
                surface.elevated, border.default, text.body,
                "focus:outline-none focus:ring-2 focus:ring-indigo-500/40"
              )}
            />
          </div>
        </div>

        <div className="flex justify-end gap-2 mt-6">
          <button
            onClick={onClose}
            className={clsx(
              "px-4 py-2 rounded-lg text-sm font-medium border",
              border.default, text.secondary,
              "hover:bg-zinc-100 dark:hover:bg-zinc-800 transition-colors"
            )}
          >
            Cancel
          </button>
          <button
            onClick={() => {
              if (!repo.includes("/")) return;
              onScan(repo, labels || undefined, limit);
              onClose();
            }}
            disabled={!repo.includes("/")}
            className={clsx(
              "px-4 py-2 rounded-lg text-sm font-medium text-white transition-colors",
              "bg-indigo-600 hover:bg-indigo-700 disabled:opacity-40 disabled:cursor-not-allowed"
            )}
          >
            <Search size={14} className="inline mr-1.5 -mt-0.5" />
            Scan Issues
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Issue row ───────────────────────────────────────────────────────────────

function IssueRow({ entry, onSkip, onRetry, onNavigate }: {
  entry: GitHubQueueEntry;
  onSkip: () => void;
  onRetry: () => void;
  onNavigate: (hash: string) => void;
}) {
  const isFailed = entry.status.includes("Failed");
  const isPending = entry.status.includes("Pending");
  const isActive = entry.status.includes("Processing");
  const isWorking = isActive || entry.status.includes("WaitingApproval");

  return (
    <div
      className={clsx(
        "group flex items-center gap-3 px-4 py-3 border-b transition-colors cursor-pointer",
        border.subtle,
        isActive && "bg-blue-950/20",
        "hover:bg-zinc-50 dark:hover:bg-zinc-900/60"
      )}
      onClick={() => onNavigate(`run/${encodeURIComponent(entry.run_id)}`)}
    >
      <div className="shrink-0">{statusIcon(entry.status)}</div>

      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className={clsx("text-sm font-medium", text.heading)}>
            #{entry.issue_number}
          </span>
          <span className={clsx("text-sm truncate", text.body)}>
            {entry.title}
          </span>
          {isWorking && (
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-blue-600/20 text-blue-400 border border-blue-600/30">
              view run
            </span>
          )}
        </div>
        <div className="flex items-center gap-3 mt-0.5">
          <span className={clsx("text-[11px] font-mono", text.muted)}>{entry.repo}</span>
          <span className={clsx("text-[11px]", statusColor(entry.status))}>
            {statusLabel(entry.status)}
          </span>
          <span className={clsx("text-[10px] font-mono", text.muted)}>
            {entry.session_id.length > 30 ? `…${entry.session_id.slice(-25)}` : entry.session_id}
          </span>
        </div>
      </div>

      <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity shrink-0">
        {isPending && (
          <button
            onClick={onSkip}
            className="p-1.5 rounded-md text-zinc-400 hover:text-zinc-200 hover:bg-zinc-800 transition-colors"
            title="Skip this issue"
          >
            <SkipForward size={14} />
          </button>
        )}
        {isFailed && (
          <button
            onClick={onRetry}
            className="p-1.5 rounded-md text-zinc-400 hover:text-amber-400 hover:bg-zinc-800 transition-colors"
            title="Retry this issue"
          >
            <RotateCcw size={14} />
          </button>
        )}
        <a
          href={`https://github.com/${entry.repo}/issues/${entry.issue_number}`}
          target="_blank"
          rel="noopener noreferrer"
          className="p-1.5 rounded-md text-zinc-400 hover:text-indigo-400 hover:bg-zinc-800 transition-colors"
          title="Open on GitHub"
        >
          <ExternalLink size={14} />
        </a>
      </div>
    </div>
  );
}

// ── Main page ───────────────────────────────────────────────────────────────

export function IntegrationsPage() {
  const qc = useQueryClient();
  const toast = useToast();
  const [showScan, setShowScan] = useState(false);

  // Fetch GitHub config.
  const { data: ghConfig } = useQuery({
    queryKey: ["github-config"],
    queryFn: () => defaultApi.getGitHubInstallations(),
    refetchInterval: 30_000,
  });

  // Fetch queue with auto-refresh.
  const { data: queueData, isLoading: queueLoading } = useQuery({
    queryKey: ["github-queue"],
    queryFn: () => defaultApi.getGitHubQueue(),
    refetchInterval: 3_000,
  });

  // SSE-powered live updates — reuse shared stream (reconnect + replay).
  const { events: streamEvents } = useEventStream();
  const lastSeenProgressId = useRef<string | null>(null);
  useEffect(() => {
    const latest = streamEvents.find((ev) => ev.type === "github_progress");
    if (latest && latest.id !== lastSeenProgressId.current) {
      lastSeenProgressId.current = latest.id;
      void qc.invalidateQueries({ queryKey: ["github-queue"] });
    }
  }, [streamEvents, qc]);

  const queue = queueData?.queue ?? [];
  const pending = queue.filter((e) => e.status.includes("Pending")).length;
  const processing = queue.filter((e) => e.status.includes("Processing")).length;
  const completed = queue.filter((e) => e.status.includes("Completed")).length;
  const failed = queue.filter((e) => e.status.includes("Failed")).length;
  const waiting = queue.filter((e) => e.status.includes("WaitingApproval")).length;
  const maxConcurrent = queueData?.max_concurrent ?? 3;
  const dispatcherRunning = queueData?.dispatcher_running ?? false;

  // Mutations.
  const scanMut = useMutation({
    mutationFn: ({ repo, labels, limit }: { repo: string; labels?: string; limit?: number }) =>
      defaultApi.scanGitHubIssues(repo, { labels, limit }),
    onSuccess: (data) => {
      toast.success(`Queued ${data.queued} issues from ${data.repo}`);
      void qc.invalidateQueries({ queryKey: ["github-queue"] });
    },
    onError: () => toast.error("Scan failed"),
  });

  const pauseMut = useMutation({
    mutationFn: () => defaultApi.pauseGitHubQueue(),
    onSuccess: () => {
      toast.success("Queue paused");
      void qc.invalidateQueries({ queryKey: ["github-queue"] });
    },
    onError: (err: unknown) => {
      const msg = err instanceof Error ? err.message : String(err);
      toast.error(`Failed to pause: ${msg}`);
    },
  });

  const resumeMut = useMutation({
    mutationFn: () => defaultApi.resumeGitHubQueue(),
    onSuccess: () => {
      toast.success("Processing started");
      void qc.invalidateQueries({ queryKey: ["github-queue"] });
    },
    onError: (err: unknown) => {
      const msg = err instanceof Error ? err.message : String(err);
      if (msg.includes("no_brain_model") || msg.includes("412")) {
        toast.error("No LLM model configured. Go to Settings and set a brain/generate model first.");
      } else {
        toast.error(`Failed to start: ${msg}`);
      }
    },
  });

  const skipMut = useMutation({
    mutationFn: (issue: number) => defaultApi.skipGitHubIssue(issue),
    onSuccess: () => void qc.invalidateQueries({ queryKey: ["github-queue"] }),
  });

  const retryMut = useMutation({
    mutationFn: (issue: number) => defaultApi.retryGitHubIssue(issue),
    onSuccess: () => {
      toast.success("Issue re-queued");
      void qc.invalidateQueries({ queryKey: ["github-queue"] });
    },
  });

  const handleScan = useCallback((repo: string, labels?: string, limit?: number) => {
    scanMut.mutate({ repo, labels, limit });
  }, [scanMut]);

  const isConfigured = ghConfig?.configured ?? false;

  return (
    <div className={clsx("min-h-full p-6", surface.page)}>
      <PageHeader
        title="Integrations"
        subtitle="Connect external services and manage automated workflows"
      />

      {/* GitHub Integration Card */}
      <div className={clsx("rounded-xl border mb-6", surface.card, border.default)}>
        <div className="flex items-center justify-between px-5 py-4 border-b border-zinc-800/50">
          <div className="flex items-center gap-3">
            <div className={clsx(
              "flex items-center justify-center w-9 h-9 rounded-lg",
              isConfigured ? "bg-zinc-800" : "bg-zinc-900"
            )}>
              <GitBranch size={18} className={isConfigured ? "text-zinc-100" : "text-zinc-600"} />
            </div>
            <div>
              <h3 className={clsx("text-sm font-semibold", text.heading)}>GitHub</h3>
              <p className={clsx("text-[11px]", isConfigured ? "text-emerald-400" : text.muted)}>
                {isConfigured ? "Connected" : "Not configured"}
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2">
            {isConfigured && (
              <>
                {/* Concurrency selector. Option set unions the preset stops
                    with the current server value so a server-clamped value
                    outside the presets (server accepts 1..=20) still renders
                    as the selected option. */}
                <div className="flex items-center gap-1.5">
                  <span className={clsx("text-[10px]", text.muted)}>Parallel:</span>
                  <select
                    value={maxConcurrent}
                    onChange={(e) => {
                      const val = Number(e.target.value);
                      fetch("/v1/webhooks/github/queue/concurrency", {
                        method: "PUT",
                        headers: {
                          "Authorization": `Bearer ${localStorage.getItem("cairn_token") ?? ""}`,
                          "Content-Type": "application/json",
                        },
                        body: JSON.stringify({ max_concurrent: val }),
                      }).then(() => void qc.invalidateQueries({ queryKey: ["github-queue"] }));
                    }}
                    className={clsx(
                      "rounded border px-1.5 py-0.5 text-[11px]",
                      surface.elevated, border.default, text.body,
                      "focus:outline-none focus:ring-1 focus:ring-indigo-500/40"
                    )}
                  >
                    {Array.from(new Set([1, 2, 3, 5, 10, maxConcurrent]))
                      .filter((n) => n >= 1 && n <= 20)
                      .sort((a, b) => a - b)
                      .map((n) => (
                        <option key={n} value={n}>{n}</option>
                      ))}
                  </select>
                </div>

                {!dispatcherRunning ? (
                  <button
                    onClick={() => resumeMut.mutate()}
                    disabled={resumeMut.isPending || (pending === 0 && processing === 0)}
                    className={clsx(
                      "inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium transition-colors",
                      pending > 0 || processing > 0
                        ? "bg-emerald-600 hover:bg-emerald-700 text-white disabled:opacity-60"
                        : "bg-zinc-800 text-zinc-500 cursor-not-allowed"
                    )}
                  >
                    <Play size={12} /> {pending > 0 || processing > 0 ? "Resume" : "Start"}
                  </button>
                ) : (
                  <button
                    onClick={() => pauseMut.mutate()}
                    disabled={pauseMut.isPending}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-amber-600 hover:bg-amber-700 text-white transition-colors disabled:opacity-60"
                  >
                    <Pause size={12} /> Pause ({processing}/{maxConcurrent})
                  </button>
                )}

                <button
                  onClick={() => setShowScan(true)}
                  className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium bg-indigo-600 hover:bg-indigo-700 text-white transition-colors"
                >
                  <Search size={12} /> Scan Repo
                </button>
              </>
            )}
          </div>
        </div>

        {isConfigured && (
          <>
            {/* Stats row */}
            <div className="grid grid-cols-5 gap-px bg-zinc-800/30">
              <StatCard label="Queued" value={pending} compact />
              <StatCard label="Working" value={processing} compact />
              <StatCard label="Awaiting Review" value={waiting} compact />
              <StatCard label="Completed" value={completed} compact />
              <StatCard label="Failed" value={failed} compact />
            </div>

            {/* Queue list */}
            {queueLoading ? (
              <div className="flex items-center justify-center py-12">
                <Loader2 size={16} className="animate-spin text-zinc-500" />
              </div>
            ) : queue.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-16 text-center">
                <Inbox size={32} className="text-zinc-600 mb-3" />
                <p className={clsx("text-sm font-medium mb-1", text.secondary)}>
                  No issues in queue
                </p>
                <p className={clsx("text-xs max-w-xs", text.muted)}>
                  Click "Scan Repo" to fetch open issues from a GitHub repository and start processing them.
                </p>
              </div>
            ) : (
              <div className="max-h-[60vh] overflow-y-auto">
                {queue.map((entry) => (
                  <IssueRow
                    key={`${entry.repo}-${entry.issue_number}`}
                    entry={entry}
                    onSkip={() => skipMut.mutate(entry.issue_number)}
                    onRetry={() => retryMut.mutate(entry.issue_number)}
                    onNavigate={(hash) => { window.location.hash = hash; }}
                  />
                ))}
              </div>
            )}
          </>
        )}

        {!isConfigured && (
          <div className="flex flex-col items-center justify-center py-16 text-center">
            <Cable size={32} className="text-zinc-600 mb-3" />
            <p className={clsx("text-sm font-medium mb-1", text.secondary)}>
              GitHub App not connected
            </p>
            <p className={clsx("text-xs max-w-sm", text.muted)}>
              Set <code className="text-[10px] bg-zinc-800 px-1 py-0.5 rounded">GITHUB_APP_ID</code>,{" "}
              <code className="text-[10px] bg-zinc-800 px-1 py-0.5 rounded">GITHUB_PRIVATE_KEY_FILE</code>, and{" "}
              <code className="text-[10px] bg-zinc-800 px-1 py-0.5 rounded">GITHUB_WEBHOOK_SECRET</code>{" "}
              environment variables to enable the GitHub integration.
            </p>
          </div>
        )}
      </div>

      {/* Placeholder for future integrations */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 gap-4">
        {[
          { name: "Linear", desc: "Issue tracking & project management", icon: Zap },
          { name: "Slack", desc: "Notifications & approval channels", icon: Cable },
          { name: "Jira", desc: "Enterprise issue tracking", icon: Cable },
        ].map((integration) => (
          <div
            key={integration.name}
            className={clsx(
              "rounded-xl border px-5 py-4 opacity-50",
              surface.card, border.default
            )}
          >
            <div className="flex items-center gap-3">
              <div className="flex items-center justify-center w-9 h-9 rounded-lg bg-zinc-900">
                <integration.icon size={18} className="text-zinc-600" />
              </div>
              <div>
                <h3 className={clsx("text-sm font-semibold", text.heading)}>{integration.name}</h3>
                <p className={clsx("text-[11px]", text.muted)}>{integration.desc}</p>
              </div>
            </div>
            <p className={clsx("text-[11px] mt-3", text.muted)}>Coming soon</p>
          </div>
        ))}
      </div>

      {showScan && <ScanDialog onClose={() => setShowScan(false)} onScan={handleScan} />}
    </div>
  );
}
