import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  FileText, ChevronRight, ChevronDown, RefreshCw, Plus,
  GitCompare, Package, Loader2, AlertTriangle, X, Check,
  Play, Pause,
} from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { useToast } from "../components/Toast";
import type {
  PromptAssetRecord, PromptVersionRecord, PromptReleaseRecord, PromptVersionDiff,
} from "../lib/types";

// ── Helpers ───────────────────────────────────────────────────────────────────

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit",
  });

const shortId = (id: string) =>
  id.length > 20 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const shortHash = (h: string) => h.slice(0, 8);

function makeReleaseId(assetId: string): string {
  return `rel-${assetId.slice(0, 8)}-${Date.now().toString(36)}`;
}

// ── Kind badge ────────────────────────────────────────────────────────────────

const KIND_STYLE: Record<string, string> = {
  system:    "bg-blue-950/60 text-blue-300 border-blue-800/40",
  user:      "bg-indigo-950/60 text-indigo-300 border-indigo-800/40",
  assistant: "bg-teal-950/60 text-teal-300 border-teal-800/40",
  tool:      "bg-amber-950/60 text-amber-300 border-amber-800/40",
};

function KindBadge({ kind }: { kind: string }) {
  return (
    <span className={clsx(
      "text-[10px] font-mono font-medium rounded px-1.5 py-0.5 border",
      KIND_STYLE[kind] ?? "bg-zinc-800/60 text-zinc-400 border-zinc-700",
    )}>
      {kind}
    </span>
  );
}

// ── Release state badge ───────────────────────────────────────────────────────

const RELEASE_STYLE: Record<string, string> = {
  draft:              "bg-zinc-800/60 text-zinc-400 border-zinc-700",
  pending_approval:   "bg-amber-950/60 text-amber-300 border-amber-800/40",
  approved:           "bg-blue-950/60 text-blue-300 border-blue-800/40",
  released:           "bg-emerald-950/60 text-emerald-300 border-emerald-800/40",
  rolling_out:        "bg-indigo-950/60 text-indigo-300 border-indigo-800/40",
  archived:           "bg-zinc-800/40 text-zinc-600 border-zinc-800",
  rolled_back:        "bg-red-950/60 text-red-300 border-red-800/40",
};

function ReleaseBadge({ state }: { state: string }) {
  return (
    <span className={clsx(
      "text-[10px] font-medium rounded px-1.5 py-0.5 border whitespace-nowrap",
      RELEASE_STYLE[state] ?? RELEASE_STYLE.draft,
    )}>
      {state.replace(/_/g, " ")}
    </span>
  );
}

// ── Diff panel ────────────────────────────────────────────────────────────────

function DiffPanel({ diff, onClose }: { diff: PromptVersionDiff; onClose: () => void }) {
  const pct = Math.round(diff.similarity_score * 100);
  return (
    <div className="rounded-lg border border-zinc-800 bg-zinc-950 overflow-hidden mt-2">
      <div className="flex items-center justify-between px-3 py-2 border-b border-zinc-800">
        <div className="flex items-center gap-3">
          <GitCompare size={12} className="text-zinc-500" />
          <span className="text-[11px] font-medium text-zinc-400">Version diff</span>
          <span className={clsx(
            "text-[10px] font-mono rounded px-1.5 py-0.5",
            pct >= 80 ? "bg-emerald-950/60 text-emerald-400" :
            pct >= 50 ? "bg-amber-950/60 text-amber-400" :
                        "bg-red-950/60 text-red-400",
          )}>
            {pct}% similar
          </span>
        </div>
        <button onClick={onClose} className="p-0.5 rounded text-zinc-600 hover:text-zinc-300 transition-colors">
          <X size={12} />
        </button>
      </div>
      <div className="grid grid-cols-2 divide-x divide-zinc-800 max-h-48 overflow-y-auto">
        <div className="p-2">
          <p className="text-[10px] text-red-500 font-semibold mb-1 uppercase tracking-wider">
            — Removed ({diff.removed_lines.length})
          </p>
          {diff.removed_lines.length === 0
            ? <p className="text-[11px] text-zinc-700 italic">None</p>
            : diff.removed_lines.map((l, i) => (
              <div key={i} className="flex items-start gap-1 text-[11px] font-mono text-red-400 bg-red-950/20 rounded px-1 mb-0.5 leading-relaxed">
                <span className="text-red-700 shrink-0">−</span>
                <span className="break-all">{l}</span>
              </div>
            ))}
        </div>
        <div className="p-2">
          <p className="text-[10px] text-emerald-500 font-semibold mb-1 uppercase tracking-wider">
            + Added ({diff.added_lines.length})
          </p>
          {diff.added_lines.length === 0
            ? <p className="text-[11px] text-zinc-700 italic">None</p>
            : diff.added_lines.map((l, i) => (
              <div key={i} className="flex items-start gap-1 text-[11px] font-mono text-emerald-400 bg-emerald-950/20 rounded px-1 mb-0.5 leading-relaxed">
                <span className="text-emerald-700 shrink-0">+</span>
                <span className="break-all">{l}</span>
              </div>
            ))}
        </div>
      </div>
    </div>
  );
}

// ── Version row ───────────────────────────────────────────────────────────────

function VersionRow({
  version, prevVersionId, assetId, onCreateRelease,
}: {
  version: PromptVersionRecord;
  prevVersionId: string | null;
  assetId: string;
  onCreateRelease: (versionId: string) => void;
}) {
  const [diffOpen, setDiffOpen] = useState(false);
  const vNum = version.version_number ?? "—";

  const { data: diff, isLoading: diffLoading } = useQuery({
    queryKey: ["prompt-diff", assetId, version.prompt_version_id, prevVersionId],
    queryFn: () =>
      defaultApi.getVersionDiff(assetId, version.prompt_version_id, prevVersionId!),
    enabled: diffOpen && !!prevVersionId,
    staleTime: Infinity,
    retry: false,
  });

  return (
    <div className="space-y-1">
      <div className={clsx(
        "flex items-center gap-3 px-3 py-2 text-[12px] transition-colors rounded",
        "hover:bg-zinc-800/40",
      )}>
        {/* Version number */}
        <span className="shrink-0 font-mono text-zinc-500 w-6 text-right">v{vNum}</span>

        {/* Hash */}
        <code className="shrink-0 font-mono text-zinc-600 text-[10px]">
          {shortHash(version.content_hash)}
        </code>

        {/* Content preview */}
        {version.content && (
          <span className="flex-1 text-zinc-500 truncate text-[11px] font-mono">
            {version.content.slice(0, 60)}{version.content.length > 60 ? "…" : ""}
          </span>
        )}
        {!version.content && (
          <span className="flex-1 text-zinc-700 italic text-[11px]">content not loaded</span>
        )}

        <span className="shrink-0 text-zinc-700">{fmtTime(version.created_at)}</span>

        {/* Actions */}
        <div className="shrink-0 flex items-center gap-1">
          {prevVersionId && (
            <button
              onClick={() => setDiffOpen((v) => !v)}
              className={clsx(
                "flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium transition-colors",
                diffOpen
                  ? "bg-indigo-600/20 text-indigo-400 border border-indigo-700/40"
                  : "text-zinc-600 hover:text-zinc-300 hover:bg-zinc-800",
              )}
            >
              <GitCompare size={10} /> Diff
            </button>
          )}
          <button
            onClick={() => onCreateRelease(version.prompt_version_id)}
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                       text-zinc-600 hover:text-zinc-300 hover:bg-zinc-800 transition-colors"
          >
            <Package size={10} /> Release
          </button>
        </div>
      </div>

      {/* Template vars */}
      {version.template_vars && version.template_vars.length > 0 && (
        <div className="px-3 pb-1">
          <div className="flex items-center gap-1 flex-wrap">
            {version.template_vars.map((v) => (
              <span key={v.name} className="text-[10px] font-mono text-indigo-400 bg-indigo-950/40 rounded px-1.5 py-0.5">
                {`{{${v.name}}}`}{v.required && <span className="text-red-500 ml-0.5">*</span>}
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Diff panel */}
      {diffOpen && (
        <div className="px-3 pb-2">
          {diffLoading
            ? <div className="flex items-center gap-1.5 text-[11px] text-zinc-600 py-2">
                <Loader2 size={11} className="animate-spin" /> Loading diff…
              </div>
            : diff
              ? <DiffPanel diff={diff} onClose={() => setDiffOpen(false)} />
              : <p className="text-[11px] text-zinc-600 italic py-2">Diff unavailable</p>
          }
        </div>
      )}
    </div>
  );
}

// ── Release controls ──────────────────────────────────────────────────────────

function ReleaseControls({ release }: { release: PromptReleaseRecord }) {
  const qc    = useQueryClient();
  const toast = useToast();
  const [rollout, setRollout] = useState(release.rollout_percent ?? 0);

  const invalidate = () => {
    void qc.invalidateQueries({ queryKey: ["prompt-releases"] });
  };

  const activate = useMutation({
    mutationFn: () => defaultApi.activatePromptRelease(release.prompt_release_id),
    onSuccess: () => { toast.success("Release activated."); invalidate(); },
    onError:   () => toast.error("Failed to activate release."),
  });

  const applyRollout = useMutation({
    mutationFn: () => defaultApi.rolloutPromptRelease(release.prompt_release_id, rollout),
    onSuccess: () => { toast.success(`Rollout set to ${rollout}%.`); invalidate(); },
    onError:   () => toast.error("Failed to update rollout."),
  });

  const reqApproval = useMutation({
    mutationFn: () => defaultApi.requestPromptReleaseApproval(release.prompt_release_id),
    onSuccess: () => { toast.success("Approval requested."); invalidate(); },
    onError:   () => toast.error("Failed to request approval."),
  });

  return (
    <div className="flex items-center gap-2 flex-wrap">
      <ReleaseBadge state={release.state} />

      {release.rollout_percent != null && (
        <span className="text-[10px] font-mono text-zinc-500">
          {release.rollout_percent}% rollout
        </span>
      )}

      {release.release_tag && (
        <span className="text-[10px] font-mono text-zinc-600 bg-zinc-800 rounded px-1.5 py-0.5">
          {release.release_tag}
        </span>
      )}

      {/* State-driven action buttons */}
      {release.state === "draft" && (
        <button
          onClick={() => reqApproval.mutate()}
          disabled={reqApproval.isPending}
          className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                     bg-amber-900/40 text-amber-300 border border-amber-800/40
                     hover:bg-amber-900/70 transition-colors disabled:opacity-40"
        >
          {reqApproval.isPending ? <Loader2 size={9} className="animate-spin" /> : null}
          Request Approval
        </button>
      )}

      {release.state === "approved" && (
        <button
          onClick={() => activate.mutate()}
          disabled={activate.isPending}
          className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                     bg-emerald-900/40 text-emerald-300 border border-emerald-800/40
                     hover:bg-emerald-900/70 transition-colors disabled:opacity-40"
        >
          {activate.isPending
            ? <Loader2 size={9} className="animate-spin" />
            : <Play size={9} />}
          Activate
        </button>
      )}

      {(release.state === "released" || release.state === "rolling_out") && (
        <div className="flex items-center gap-2">
          <input
            type="range" min={0} max={100} step={5}
            value={rollout}
            onChange={(e) => setRollout(Number(e.target.value))}
            className="w-24 accent-indigo-500 cursor-pointer"
          />
          <span className="text-[10px] font-mono text-zinc-400 w-8 text-right tabular-nums">
            {rollout}%
          </span>
          <button
            onClick={() => applyRollout.mutate()}
            disabled={applyRollout.isPending || rollout === (release.rollout_percent ?? 0)}
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                       bg-indigo-900/40 text-indigo-300 border border-indigo-800/40
                       hover:bg-indigo-900/70 transition-colors disabled:opacity-40"
          >
            {applyRollout.isPending ? <Loader2 size={9} className="animate-spin" /> : <Check size={9} />}
            Apply
          </button>
        </div>
      )}
    </div>
  );
}

// ── Asset item ────────────────────────────────────────────────────────────────

function AssetItem({
  asset, releases,
}: {
  asset: PromptAssetRecord;
  releases: PromptReleaseRecord[];
}) {
  const [expanded, setExpanded] = useState(false);
  const qc    = useQueryClient();
  const toast = useToast();

  const assetReleases = releases
    .filter((r) => r.prompt_asset_id === asset.prompt_asset_id)
    .sort((a, b) => b.created_at - a.created_at);

  const latestRelease = assetReleases[0];

  const { data: versionsData, isLoading: versionsLoading } = useQuery({
    queryKey: ["prompt-versions", asset.prompt_asset_id],
    queryFn: () => defaultApi.getPromptVersions(asset.prompt_asset_id, { limit: 20 }),
    enabled: expanded,
    staleTime: 60_000,
  });

  const versions = (versionsData?.items ?? []).sort(
    (a, b) => (b.version_number ?? 0) - (a.version_number ?? 0),
  );

  const createRelease = useMutation({
    mutationFn: (versionId: string) =>
      defaultApi.createPromptRelease({
        prompt_release_id: makeReleaseId(asset.prompt_asset_id),
        prompt_asset_id:   asset.prompt_asset_id,
        prompt_version_id: versionId,
      }),
    onSuccess: () => {
      toast.success("Release created.");
      void qc.invalidateQueries({ queryKey: ["prompt-releases"] });
    },
    onError: () => toast.error("Failed to create release."),
  });

  return (
    <div className={clsx(
      "rounded-lg border transition-colors",
      expanded ? "border-zinc-700 bg-zinc-900/60" : "border-zinc-800 bg-zinc-900 hover:border-zinc-700",
    )}>
      {/* Collapsed header */}
      <button
        onClick={() => setExpanded((v) => !v)}
        className="w-full flex items-center gap-3 px-4 py-3 text-left"
      >
        {expanded
          ? <ChevronDown  size={13} className="text-zinc-500 shrink-0" />
          : <ChevronRight size={13} className="text-zinc-600 shrink-0" />}

        <FileText size={13} className="text-zinc-600 shrink-0" />

        <span className="font-medium text-[13px] text-zinc-200 truncate flex-1">
          {asset.name}
        </span>

        <KindBadge kind={asset.kind} />

        {latestRelease && <ReleaseBadge state={latestRelease.state} />}

        <span className="text-[10px] text-zinc-600 font-mono shrink-0">
          {assetReleases.length} release{assetReleases.length !== 1 ? "s" : ""}
        </span>

        <span className="text-[11px] text-zinc-700 shrink-0 hidden sm:block">
          {fmtTime(asset.updated_at ?? asset.created_at)}
        </span>
      </button>

      {/* Expanded body */}
      {expanded && (
        <div className="border-t border-zinc-800 divide-y divide-zinc-800/60">
          {/* Asset metadata */}
          <div className="px-4 py-2 flex items-center gap-4 text-[11px] text-zinc-600">
            <span className="font-mono">{asset.prompt_asset_id}</span>
            {asset.scope && <span>scope: <code className="text-zinc-500">{asset.scope}</code></span>}
          </div>

          {/* Version history */}
          <div className="px-4 py-3">
            <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-2">
              Version History
            </p>
            {versionsLoading ? (
              <div className="flex items-center gap-1.5 text-[12px] text-zinc-600 py-2">
                <Loader2 size={12} className="animate-spin" /> Loading versions…
              </div>
            ) : versions.length === 0 ? (
              <p className="text-[12px] text-zinc-700 italic py-2">No versions yet.</p>
            ) : (
              <div className="rounded-lg border border-zinc-800 overflow-hidden divide-y divide-zinc-800/50">
                {versions.map((v, i) => (
                  <VersionRow
                    key={v.prompt_version_id}
                    version={v}
                    assetId={asset.prompt_asset_id}
                    prevVersionId={versions[i + 1]?.prompt_version_id ?? null}
                    onCreateRelease={(vid) => createRelease.mutate(vid)}
                  />
                ))}
              </div>
            )}
          </div>

          {/* Releases */}
          {assetReleases.length > 0 && (
            <div className="px-4 py-3">
              <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-2">
                Releases
              </p>
              <div className="space-y-2">
                {assetReleases.map((rel) => (
                  <div key={rel.prompt_release_id} className="rounded-lg border border-zinc-800 bg-zinc-900 px-3 py-2">
                    <div className="flex items-center justify-between gap-3 mb-1.5">
                      <code className="text-[11px] font-mono text-zinc-500">
                        {shortId(rel.prompt_release_id)}
                      </code>
                      <span className="text-[10px] text-zinc-700">{fmtTime(rel.updated_at)}</span>
                    </div>
                    <ReleaseControls release={rel} />
                  </div>
                ))}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ── New Prompt form ───────────────────────────────────────────────────────────

function NewPromptForm({ onClose }: { onClose: () => void }) {
  const qc    = useQueryClient();
  const toast = useToast();
  const [id,   setId]   = useState("");
  const [name, setName] = useState("");
  const [kind, setKind] = useState("system");

  const create = useMutation({
    mutationFn: () => defaultApi.createPromptAsset({ prompt_asset_id: id.trim(), name: name.trim(), kind }),
    onSuccess: () => {
      toast.success("Prompt asset created.");
      void qc.invalidateQueries({ queryKey: ["prompt-assets"] });
      onClose();
    },
    onError: () => toast.error("Failed to create prompt asset."),
  });

  const valid = id.trim().length > 0 && name.trim().length > 0;

  return (
    <div className="rounded-lg border border-zinc-700 bg-zinc-900 px-4 py-3 space-y-3">
      <div className="flex items-center justify-between">
        <span className="text-[12px] font-medium text-zinc-300">New Prompt Asset</span>
        <button onClick={onClose} className="p-0.5 rounded text-zinc-600 hover:text-zinc-300 transition-colors">
          <X size={13} />
        </button>
      </div>
      <div className="grid grid-cols-3 gap-3">
        <div>
          <label className="text-[10px] text-zinc-500 block mb-1">ID <span className="text-red-500">*</span></label>
          <input
            value={id} onChange={(e) => setId(e.target.value)}
            placeholder="my-system-prompt"
            className="w-full rounded border border-zinc-800 bg-zinc-950 text-[12px] text-zinc-200
                       font-mono px-2 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors"
          />
        </div>
        <div>
          <label className="text-[10px] text-zinc-500 block mb-1">Name <span className="text-red-500">*</span></label>
          <input
            value={name} onChange={(e) => setName(e.target.value)}
            placeholder="My System Prompt"
            className="w-full rounded border border-zinc-800 bg-zinc-950 text-[12px] text-zinc-200
                       px-2 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors"
          />
        </div>
        <div>
          <label className="text-[10px] text-zinc-500 block mb-1">Kind</label>
          <select
            value={kind} onChange={(e) => setKind(e.target.value)}
            className="w-full rounded border border-zinc-800 bg-zinc-950 text-[12px] text-zinc-300
                       px-2 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors"
          >
            {["system", "user", "assistant", "tool"].map((k) => (
              <option key={k} value={k}>{k}</option>
            ))}
          </select>
        </div>
      </div>
      <div className="flex items-center gap-2 justify-end">
        <button onClick={onClose}
          className="px-3 py-1.5 rounded text-[12px] text-zinc-500 hover:text-zinc-300 transition-colors">
          Cancel
        </button>
        <button
          onClick={() => create.mutate()}
          disabled={!valid || create.isPending}
          className="flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium
                     bg-indigo-600 hover:bg-indigo-500 text-white
                     disabled:bg-zinc-800 disabled:text-zinc-600 disabled:cursor-not-allowed transition-colors"
        >
          {create.isPending ? <Loader2 size={11} className="animate-spin" /> : <Plus size={11} />}
          Create
        </button>
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function PromptsPage() {
  const [showNew, setShowNew] = useState(false);
  const [filter, setFilter]  = useState<"all" | "released" | "draft">("all");

  const { data: assetsData, isLoading: assetsLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["prompt-assets"],
    queryFn: () => defaultApi.getPromptAssets({ limit: 100 }),
    refetchInterval: 60_000,
  });

  const { data: releasesData } = useQuery({
    queryKey: ["prompt-releases"],
    queryFn: () => defaultApi.getPromptReleases({ limit: 200 }),
    refetchInterval: 60_000,
    retry: false,
  });

  const assets   = assetsData?.items   ?? [];
  const releases = releasesData?.items ?? [];

  // Derive latest release state per asset for filtering
  const latestState = (assetId: string): string | null => {
    const rels = releases
      .filter((r) => r.prompt_asset_id === assetId)
      .sort((a, b) => b.created_at - a.created_at);
    return rels[0]?.state ?? null;
  };

  const filtered = assets.filter((a) => {
    if (filter === "all")      return true;
    if (filter === "released") return latestState(a.prompt_asset_id) === "released" || latestState(a.prompt_asset_id) === "rolling_out";
    if (filter === "draft")    return !latestState(a.prompt_asset_id) || latestState(a.prompt_asset_id) === "draft";
    return true;
  });

  if (isError) return (
    <div className="flex flex-col items-center justify-center h-full gap-3 p-8 text-center">
      <AlertTriangle size={28} className="text-red-500" />
      <p className="text-[13px] font-medium text-zinc-300">Failed to load prompts</p>
      <p className="text-[12px] text-zinc-500">{error instanceof Error ? error.message : "Unknown"}</p>
      <button onClick={() => refetch()}
        className="px-3 py-1.5 rounded bg-zinc-800 text-zinc-300 text-[12px] hover:bg-zinc-700 transition-colors">
        Retry
      </button>
    </div>
  );

  return (
    <div className="flex flex-col h-full bg-zinc-950">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-5 h-11 border-b border-zinc-800 shrink-0">
        <FileText size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
          Prompts
        </span>

        {!assetsLoading && (
          <span className="text-[10px] text-zinc-700">
            {assets.length} asset{assets.length !== 1 ? "s" : ""} · {releases.length} release{releases.length !== 1 ? "s" : ""}
          </span>
        )}

        {/* Filter tabs */}
        <div className="flex items-center gap-0 ml-4">
          {(["all", "released", "draft"] as const).map((f) => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              className={clsx(
                "px-3 h-11 text-[11px] font-medium transition-colors border-b-2 capitalize",
                filter === f
                  ? "text-zinc-100 border-indigo-500"
                  : "text-zinc-500 border-transparent hover:text-zinc-300",
              )}
            >
              {f}
            </button>
          ))}
        </div>

        <div className="ml-auto flex items-center gap-2">
          <button onClick={() => refetch()} disabled={isFetching}
            className="flex items-center gap-1 text-[12px] text-zinc-500 hover:text-zinc-300 disabled:opacity-40 transition-colors">
            <RefreshCw size={11} className={isFetching ? "animate-spin" : ""} />
          </button>
          <button
            onClick={() => setShowNew((v) => !v)}
            className={clsx(
              "flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium transition-colors",
              showNew
                ? "bg-indigo-600/20 text-indigo-400 border border-indigo-700/40"
                : "bg-indigo-600 hover:bg-indigo-500 text-white",
            )}
          >
            {showNew ? <Pause size={11} /> : <Plus size={11} />}
            {showNew ? "Cancel" : "New Prompt"}
          </button>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-5 py-4 space-y-3 max-w-4xl mx-auto w-full">
        {/* New Prompt form */}
        {showNew && <NewPromptForm onClose={() => setShowNew(false)} />}

        {/* Asset list */}
        {assetsLoading ? (
          <div className="flex items-center justify-center gap-2 py-16 text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading prompts…</span>
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 gap-3 text-center">
            <FileText size={28} className="text-zinc-700" />
            <p className="text-[13px] text-zinc-600">
              {assets.length === 0
                ? "No prompt assets yet — create one to get started."
                : `No prompts match the "${filter}" filter.`}
            </p>
          </div>
        ) : (
          filtered.map((asset) => (
            <AssetItem
              key={asset.prompt_asset_id}
              asset={asset}
              releases={releases}
            />
          ))
        )}
      </div>
    </div>
  );
}

export default PromptsPage;
