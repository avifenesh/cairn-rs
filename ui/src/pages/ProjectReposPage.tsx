import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { RefreshCw, Trash2, FolderGit2, Plus, X } from "lucide-react";
import { clsx } from "clsx";
import { DataTable } from "../components/DataTable";
import { StatCard } from "../components/StatCard";
import { CopyButton } from "../components/CopyButton";
import { HelpTooltip } from "../components/HelpTooltip";
import { ErrorFallback } from "../components/ErrorFallback";
import { useToast } from "../components/Toast";
import { useScope } from "../hooks/useScope";
import { sectionLabel } from "../lib/design-system";
import { defaultApi, ApiError } from "../lib/api";
import type { ProjectRepoEntry } from "../lib/types";

// ── Helpers ──────────────────────────────────────────────────────────────────

function fmtRelative(ms?: number | null): string {
  if (!ms) return "—";
  const d = Date.now() - ms;
  if (d < 60_000) return "just now";
  if (d < 3_600_000) return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000) return `${Math.floor(d / 3_600_000)}h ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

function parseRepoId(repoId: string): { owner: string; repo: string } | null {
  const slash = repoId.indexOf("/");
  if (slash <= 0 || slash === repoId.length - 1) return null;
  return { owner: repoId.slice(0, slash), repo: repoId.slice(slash + 1) };
}

function errorMessage(e: unknown, fallback: string): string {
  // `apiFetch` normalizes both `{code, message}` and `{error: string}` body
  // shapes into `ApiError.message`, so the decoded backend reason is
  // available here regardless of which envelope `repo_routes.rs` emits.
  if (e instanceof ApiError) return e.message || fallback;
  if (e instanceof Error) return e.message || fallback;
  return fallback;
}

// ── Clone status pill ────────────────────────────────────────────────────────

const STATUS_PILL: Record<string, string> = {
  present: "bg-emerald-500/10 text-emerald-400 border-emerald-500/20",
  missing: "bg-amber-500/10 text-amber-400 border-amber-500/20",
};
const STATUS_DOT: Record<string, string> = {
  present: "bg-emerald-400",
  missing: "bg-amber-400",
};

function StatusPill({ status }: { status: string }) {
  const cls = STATUS_PILL[status] ?? "bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-500 border-gray-200 dark:border-zinc-700";
  const dot = STATUS_DOT[status] ?? "bg-zinc-600";
  return (
    <span className={clsx(
      "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium border whitespace-nowrap",
      cls,
    )}>
      <span className={clsx("w-1 h-1 rounded-full shrink-0", dot)} />
      {status}
    </span>
  );
}

// ── Attach form ──────────────────────────────────────────────────────────────

interface AttachFormProps {
  onCancel: () => void;
  onSubmit: (repoId: string) => void;
  submitting: boolean;
}

function AttachForm({ onCancel, onSubmit, submitting }: AttachFormProps) {
  const [owner, setOwner] = useState("");
  const [repo, setRepo] = useState("");

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    const o = owner.trim();
    const r = repo.trim();
    if (!o || !r) return;
    onSubmit(`${o}/${r}`);
  }

  const disabled = submitting || owner.trim() === "" || repo.trim() === "";

  return (
    <form
      onSubmit={handleSubmit}
      className="rounded-md border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 p-4 space-y-3"
    >
      <div className="flex items-center justify-between">
        <p className="text-[12px] font-medium text-gray-800 dark:text-zinc-200">Attach a GitHub repo</p>
        <button
          type="button"
          onClick={onCancel}
          className="p-1 rounded hover:bg-white/5 text-gray-400 dark:text-zinc-500"
          aria-label="Close attach form"
        >
          <X size={12} />
        </button>
      </div>
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-500">Owner</span>
          <input
            value={owner}
            onChange={e => setOwner(e.target.value)}
            placeholder="avifenesh"
            autoComplete="off"
            spellCheck={false}
            className="mt-1 w-full rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 px-2 py-1.5 text-[12px] font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500"
          />
        </label>
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-500">Repo</span>
          <input
            value={repo}
            onChange={e => setRepo(e.target.value)}
            placeholder="cairn-rs"
            autoComplete="off"
            spellCheck={false}
            className="mt-1 w-full rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 px-2 py-1.5 text-[12px] font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500"
          />
        </label>
      </div>
      <div className="flex items-center justify-end gap-2">
        <button
          type="button"
          onClick={onCancel}
          className="px-2.5 py-1.5 text-[11px] rounded-md border border-gray-200 dark:border-zinc-800 text-gray-600 dark:text-zinc-400 hover:bg-white/5"
        >
          Cancel
        </button>
        <button
          type="submit"
          disabled={disabled}
          className={clsx(
            "px-2.5 py-1.5 text-[11px] rounded-md border font-medium transition-colors",
            disabled
              ? "border-gray-200 dark:border-zinc-800 text-gray-400 dark:text-zinc-600 cursor-not-allowed"
              : "border-indigo-500 bg-indigo-500/10 text-indigo-400 hover:bg-indigo-500/20",
          )}
        >
          {submitting ? "Attaching…" : "Attach repo"}
        </button>
      </div>
    </form>
  );
}

// ── Page ─────────────────────────────────────────────────────────────────────

export function ProjectReposPage() {
  const [scope] = useScope();
  const qc = useQueryClient();
  const toast = useToast();
  const [showForm, setShowForm] = useState(false);

  // Same slash-path encoding used by TriggersPage (PR #132). Axum 0.7 captures
  // `:project` as one segment, so `/` must be `%2F` on the wire.
  const projectPath = encodeURIComponent(
    `${scope.tenant_id}/${scope.workspace_id}/${scope.project_id}`,
  );

  const reposQ = useQuery<ProjectRepoEntry[]>({
    queryKey: ["project-repos", projectPath],
    queryFn: () => defaultApi.listProjectRepos(scope),
    refetchInterval: 30_000,
  });

  const attachMut = useMutation({
    mutationFn: (repoId: string) => defaultApi.attachProjectRepo({ repo_id: repoId }, scope),
    onSuccess: (res) => {
      toast.success(
        res.clone_created
          ? `Attached ${res.repo_id} (clone created).`
          : `Attached ${res.repo_id}.`,
      );
      setShowForm(false);
      void qc.invalidateQueries({ queryKey: ["project-repos", projectPath] });
    },
    onError: (e) => toast.error(errorMessage(e, "Failed to attach repo.")),
  });

  const detachMut = useMutation({
    mutationFn: ({ owner, repo }: { owner: string; repo: string }) =>
      defaultApi.detachProjectRepo(owner, repo, scope),
    onSuccess: (_, vars) => {
      toast.success(`Detached ${vars.owner}/${vars.repo}.`);
      void qc.invalidateQueries({ queryKey: ["project-repos", projectPath] });
    },
    onError: (e) => toast.error(errorMessage(e, "Failed to detach repo.")),
  });

  if (reposQ.isError) {
    return (
      <ErrorFallback
        error={reposQ.error}
        resource="project repos"
        onRetry={() => void reposQ.refetch()}
      />
    );
  }

  const repos = reposQ.data ?? [];
  const present = repos.filter(r => r.clone_status === "present").length;

  return (
    <div className="p-6 space-y-5">
      {/* Toolbar */}
      <div className="flex items-center justify-between">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <p className={clsx(sectionLabel, "mb-0")}>Project Repos</p>
            <HelpTooltip
              text="GitHub repos allowlisted for this project. Runs can only clone or mutate repos that appear here. RFC 016."
              placement="right"
            />
          </div>
          <p className="text-[11px] text-gray-500 dark:text-zinc-400">
            Scope: <span className="font-mono">{scope.tenant_id}/{scope.workspace_id}/{scope.project_id}</span>
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => reposQ.refetch()}
            className="flex items-center gap-1.5 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-2.5 py-1.5 text-[11px] text-gray-400 dark:text-zinc-500 hover:bg-white/5 transition-colors"
          >
            <RefreshCw size={11} className={clsx(reposQ.isFetching && "animate-spin")} /> Refresh
          </button>
          <button
            onClick={() => setShowForm(v => !v)}
            className="flex items-center gap-1.5 rounded-md border border-indigo-500 bg-indigo-500/10 px-2.5 py-1.5 text-[11px] font-medium text-indigo-400 hover:bg-indigo-500/20 transition-colors"
          >
            <Plus size={11} /> Attach repo
          </button>
        </div>
      </div>

      {/* Stat cards */}
      <div className="grid grid-cols-2 sm:grid-cols-3 gap-3">
        <StatCard label="Attached" value={repos.length} />
        <StatCard label="Clone present" value={present} variant="success" />
        <StatCard label="Clone missing" value={repos.length - present} />
      </div>

      {/* Inline attach form */}
      {showForm && (
        <AttachForm
          onCancel={() => setShowForm(false)}
          onSubmit={repoId => attachMut.mutate(repoId)}
          submitting={attachMut.isPending}
        />
      )}

      {/* Table */}
      <DataTable<ProjectRepoEntry>
        data={repos}
        getRowId={r => r.repo_id}
        columns={[
          {
            key: "repo",
            header: "Repo",
            render: r => (
              <span className="flex items-center gap-1 font-medium text-gray-800 dark:text-zinc-200 text-[12px] whitespace-nowrap group/id">
                <FolderGit2 size={11} className="text-violet-400 shrink-0" />
                <span className="font-mono">{r.repo_id}</span>
                <CopyButton text={r.repo_id} label="Copy repo id" size={10} className="opacity-0 group-hover/id:opacity-100" />
              </span>
            ),
            sortValue: r => r.repo_id,
          },
          {
            key: "clone",
            header: "Clone",
            render: r => <StatusPill status={r.clone_status} />,
            sortValue: r => r.clone_status,
          },
          {
            key: "added",
            header: "Added",
            render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{fmtRelative(r.added_at)}</span>,
            sortValue: r => r.added_at ?? 0,
          },
          {
            key: "last_used",
            header: "Last used",
            render: r => <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{fmtRelative(r.last_used_at)}</span>,
            sortValue: r => r.last_used_at ?? 0,
          },
          {
            key: "actions",
            header: "",
            render: r => {
              const parts = parseRepoId(r.repo_id);
              if (!parts) return null;
              // `DataTable` doesn't put a `group` class on the row, so
              // `group-hover:opacity-100` would hide this button forever.
              // Keep it always visible — this is a destructive action the
              // operator always needs to be able to reach.
              return (
                <div className="flex items-center gap-1">
                  <button
                    onClick={() => {
                      if (window.confirm(`Detach ${r.repo_id} from this project?`)) {
                        detachMut.mutate(parts);
                      }
                    }}
                    title="Detach"
                    className="p-1 rounded hover:bg-gray-100 dark:hover:bg-zinc-800 text-red-400 transition-colors"
                  >
                    <Trash2 size={12} />
                  </button>
                </div>
              );
            },
          },
        ]}
        // `DataTable` lowercases the query before calling this predicate,
        // so compare in lowercase too — otherwise mixed-case repo ids
        // (e.g. `Microsoft/TypeScript`) would never match.
        filterFn={(r, q) => r.repo_id.toLowerCase().includes(q)}
        csvRow={r => [r.repo_id, r.clone_status, r.added_at ?? "", r.last_used_at ?? ""]}
        csvHeaders={["Repo", "Clone", "Added", "Last Used"]}
        filename="project-repos"
        emptyText="No repos attached. Click 'Attach repo' to link a GitHub repo to this project."
      />
    </div>
  );
}
