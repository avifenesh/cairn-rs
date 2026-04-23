import { useState, useEffect } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  FileText, ChevronRight, ChevronDown, RefreshCw, Plus,
  GitCompare, Package, Loader2, AlertTriangle, X, Check,
  Play, Pause,
} from "lucide-react";
import { clsx } from "clsx";
import { Card } from "../components/Card";
import { defaultApi } from "../lib/api";
import { sectionLabel } from "../lib/design-system";
import { useToast } from "../components/Toast";
import type {
  PromptAssetRecord, PromptVersionRecord, PromptReleaseRecord, PromptVersionDiff,
  PromptKind, PromptReleaseState,
} from "../lib/types";

/** Human-readable label for a `PromptKind`. */
const KIND_LABEL: Record<PromptKind, string> = {
  system:        "system",
  user_template: "user template",
  tool_prompt:   "tool prompt",
  critic:        "critic",
  router:        "router",
};

const PROMPT_KINDS = Object.keys(KIND_LABEL) as PromptKind[];

/** Runtime guard — DOM `<select>` values are untrusted strings. */
function isPromptKind(value: string): value is PromptKind {
  return (PROMPT_KINDS as string[]).includes(value);
}

/** Human-readable label for a `PromptReleaseState`. */
const RELEASE_LABEL: Record<PromptReleaseState, string> = {
  draft:    "draft",
  proposed: "proposed",
  approved: "approved",
  active:   "active",
  rejected: "rejected",
  archived: "archived",
};

// ── Helpers ───────────────────────────────────────────────────────────────────

const fmtTime = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: "short", day: "numeric",
    hour: "2-digit", minute: "2-digit",
  });

const shortId = (id: string) =>
  id.length > 20 ? `${id.slice(0, 10)}…${id.slice(-6)}` : id;

const shortHash = (h: string) => h.slice(0, 8);

/** SHA-256 hex digest via SubtleCrypto — matches backend `content_hash` format. */
async function sha256Hex(text: string): Promise<string> {
  const bytes = new TextEncoder().encode(text);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
}

// ── Kind badge ────────────────────────────────────────────────────────────────

const KIND_STYLE: Record<PromptKind, string> = {
  system:        "bg-blue-950/60 text-blue-300 border-blue-800/40",
  user_template: "bg-indigo-950/60 text-indigo-300 border-indigo-800/40",
  tool_prompt:   "bg-amber-950/60 text-amber-300 border-amber-800/40",
  critic:        "bg-teal-950/60 text-teal-300 border-teal-800/40",
  router:        "bg-purple-950/60 text-purple-300 border-purple-800/40",
};

function KindBadge({ kind }: { kind: PromptKind }) {
  return (
    <span className={clsx(
      "text-[10px] font-mono font-medium rounded px-1.5 py-0.5 border",
      KIND_STYLE[kind] ?? "bg-gray-100/60 dark:bg-zinc-800/60 text-gray-500 dark:text-zinc-400 border-gray-200 dark:border-zinc-700",
    )}>
      {KIND_LABEL[kind] ?? kind}
    </span>
  );
}

// ── Release state badge ───────────────────────────────────────────────────────

const RELEASE_STYLE: Record<PromptReleaseState, string> = {
  draft:    "bg-gray-100/60 dark:bg-zinc-800/60 text-gray-500 dark:text-zinc-400 border-gray-200 dark:border-zinc-700",
  proposed: "bg-amber-950/60 text-amber-300 border-amber-800/40",
  approved: "bg-blue-950/60 text-blue-300 border-blue-800/40",
  active:   "bg-emerald-950/60 text-emerald-300 border-emerald-800/40",
  rejected: "bg-red-950/60 text-red-300 border-red-800/40",
  archived: "bg-gray-100/40 dark:bg-zinc-800/40 text-gray-400 dark:text-zinc-600 border-gray-200 dark:border-zinc-800",
};

function ReleaseBadge({ state }: { state: PromptReleaseState }) {
  return (
    <span className={clsx(
      "text-[10px] font-medium rounded px-1.5 py-0.5 border whitespace-nowrap",
      RELEASE_STYLE[state] ?? RELEASE_STYLE.draft,
    )}>
      {RELEASE_LABEL[state] ?? state}
    </span>
  );
}

// ── Diff panel ────────────────────────────────────────────────────────────────

function DiffPanel({ diff, onClose }: { diff: PromptVersionDiff; onClose: () => void }) {
  const pct = Math.round(diff.similarity_score * 100);
  return (
    <Card variant="inner" className="mt-2">
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-zinc-800">
        <div className="flex items-center gap-3">
          <GitCompare size={12} className="text-gray-400 dark:text-zinc-500" />
          <span className="text-[11px] font-medium text-gray-500 dark:text-zinc-400">Version diff</span>
          <span className={clsx(
            "text-[10px] font-mono rounded px-1.5 py-0.5",
            pct >= 80 ? "bg-emerald-950/60 text-emerald-400" :
            pct >= 50 ? "bg-amber-950/60 text-amber-400" :
                        "bg-red-950/60 text-red-400",
          )}>
            {pct}% similar
          </span>
        </div>
        <button onClick={onClose} className="p-0.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
          <X size={12} />
        </button>
      </div>
      <div className="grid grid-cols-2 divide-x divide-gray-200 dark:divide-zinc-800 max-h-48 overflow-y-auto">
        <div className="p-2">
          <p className="text-[10px] text-red-500 font-semibold mb-1 uppercase tracking-wider">
            — Removed ({diff.removed_lines.length})
          </p>
          {diff.removed_lines.length === 0
            ? <p className="text-[11px] text-gray-300 dark:text-zinc-600 italic">None</p>
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
            ? <p className="text-[11px] text-gray-300 dark:text-zinc-600 italic">None</p>
            : diff.added_lines.map((l, i) => (
              <div key={i} className="flex items-start gap-1 text-[11px] font-mono text-emerald-400 bg-emerald-950/20 rounded px-1 mb-0.5 leading-relaxed">
                <span className="text-emerald-700 shrink-0">+</span>
                <span className="break-all">{l}</span>
              </div>
            ))}
        </div>
      </div>
    </Card>
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
        "hover:bg-gray-100/40 dark:hover:bg-gray-100/40 dark:bg-zinc-800/40",
      )}>
        {/* Version number */}
        <span className="shrink-0 font-mono text-gray-400 dark:text-zinc-500 w-6 text-right">v{vNum}</span>

        {/* Hash */}
        <code className="shrink-0 font-mono text-gray-400 dark:text-zinc-600 text-[10px]">
          {shortHash(version.content_hash)}
        </code>

        {/* Content preview */}
        {version.content && (
          <span className="flex-1 text-gray-400 dark:text-zinc-500 truncate text-[11px] font-mono">
            {version.content.slice(0, 60)}{version.content.length > 60 ? "…" : ""}
          </span>
        )}
        {!version.content && (
          <span className="flex-1 text-gray-300 dark:text-zinc-600 italic text-[11px]">content not loaded</span>
        )}

        <span className="shrink-0 text-gray-300 dark:text-zinc-600">{fmtTime(version.created_at)}</span>

        {/* Actions */}
        <div className="shrink-0 flex items-center gap-1">
          {prevVersionId && (
            <button
              onClick={() => setDiffOpen((v) => !v)}
              className={clsx(
                "flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium transition-colors",
                diffOpen
                  ? "bg-indigo-600/20 text-indigo-400 border border-indigo-700/40"
                  : "text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800",
              )}
            >
              <GitCompare size={10} /> Diff
            </button>
          )}
          <button
            onClick={() => onCreateRelease(version.prompt_version_id)}
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                       text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors"
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
            ? <div className="flex items-center gap-1.5 text-[11px] text-gray-400 dark:text-zinc-600 py-2">
                <Loader2 size={11} className="animate-spin" /> Loading diff…
              </div>
            : diff
              ? <DiffPanel diff={diff} onClose={() => setDiffOpen(false)} />
              : <p className="text-[11px] text-gray-400 dark:text-zinc-600 italic py-2">Diff unavailable</p>
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

  const approve = useMutation({
    mutationFn: () => defaultApi.transitionPromptRelease(release.prompt_release_id, "approved"),
    onSuccess: () => { toast.success("Release approved."); invalidate(); },
    onError:   () => toast.error("Failed to approve release."),
  });

  const reject = useMutation({
    mutationFn: () => defaultApi.transitionPromptRelease(release.prompt_release_id, "rejected"),
    onSuccess: () => { toast.success("Release rejected."); invalidate(); },
    onError:   () => toast.error("Failed to reject release."),
  });

  const demote = useMutation({
    mutationFn: () => defaultApi.transitionPromptRelease(release.prompt_release_id, "approved"),
    onSuccess: () => { toast.success("Release demoted to approved."); invalidate(); },
    onError:   () => toast.error("Failed to demote release."),
  });

  const archive = useMutation({
    mutationFn: () => defaultApi.transitionPromptRelease(release.prompt_release_id, "archived"),
    onSuccess: () => { toast.success("Release archived."); invalidate(); },
    onError:   () => toast.error("Failed to archive release."),
  });

  // Any pending mutation on this release locks out competing buttons
  // so we never fire overlapping requests into the same release —
  // including rollout, which shares the `state` field with state
  // transitions on the server.
  const anyPending =
    reqApproval.isPending
    || approve.isPending
    || reject.isPending
    || activate.isPending
    || demote.isPending
    || archive.isPending
    || applyRollout.isPending;

  return (
    <div className="flex items-center gap-2 flex-wrap">
      <ReleaseBadge state={release.state} />

      {release.rollout_percent != null && (
        <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-500">
          {release.rollout_percent}% rollout
        </span>
      )}

      {release.release_tag && (
        <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-600 bg-gray-100 dark:bg-zinc-800 rounded px-1.5 py-0.5">
          {release.release_tag}
        </span>
      )}

      {/* State-driven action buttons. Every transition fired here is
          one `PromptReleaseState::can_transition_to` permits in cairn-evals:
          draft       -> proposed (Request Approval) / approved (Approve) / archived
          proposed    -> approved / rejected / archived
          approved    -> active (Activate) / archived
          active      -> approved (Demote) / archived
          rejected    -> archived
          A single `anyPending` gate prevents overlapping mutations. */}
      {release.state === "draft" && (
        <>
          <button
            onClick={() => reqApproval.mutate()}
            disabled={anyPending}
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                       bg-amber-900/40 text-amber-300 border border-amber-800/40
                       hover:bg-amber-900/70 transition-colors disabled:opacity-40"
          >
            {reqApproval.isPending ? <Loader2 size={9} className="animate-spin" /> : null}
            Request Approval
          </button>
          {/* Rust matrix permits draft -> approved directly; expose it
              for operators who self-approve without a separate reviewer. */}
          <button
            onClick={() => approve.mutate()}
            disabled={anyPending}
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                       bg-blue-900/40 text-blue-300 border border-blue-800/40
                       hover:bg-blue-900/70 transition-colors disabled:opacity-40"
          >
            {approve.isPending ? <Loader2 size={9} className="animate-spin" /> : <Check size={9} />}
            Approve
          </button>
        </>
      )}

      {release.state === "proposed" && (
        <>
          <button
            onClick={() => approve.mutate()}
            disabled={anyPending}
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                       bg-blue-900/40 text-blue-300 border border-blue-800/40
                       hover:bg-blue-900/70 transition-colors disabled:opacity-40"
          >
            {approve.isPending ? <Loader2 size={9} className="animate-spin" /> : <Check size={9} />}
            Approve
          </button>
          <button
            onClick={() => reject.mutate()}
            disabled={anyPending}
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                       bg-red-900/40 text-red-300 border border-red-800/40
                       hover:bg-red-900/70 transition-colors disabled:opacity-40"
          >
            {reject.isPending ? <Loader2 size={9} className="animate-spin" /> : <X size={9} />}
            Reject
          </button>
        </>
      )}

      {release.state === "approved" && (
        <button
          onClick={() => activate.mutate()}
          disabled={anyPending}
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

      {release.state === "active" && (
        <>
          <div className="flex items-center gap-2">
            <input
              type="range" min={0} max={100} step={5}
              value={rollout}
              onChange={(e) => setRollout(Number(e.target.value))}
              className="w-24 accent-indigo-500 cursor-pointer"
            />
            <span className="text-[10px] font-mono text-gray-500 dark:text-zinc-400 w-8 text-right tabular-nums">
              {rollout}%
            </span>
            <button
              onClick={() => applyRollout.mutate()}
              disabled={anyPending || rollout === (release.rollout_percent ?? 0)}
              className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                         bg-indigo-900/40 text-indigo-300 border border-indigo-800/40
                         hover:bg-indigo-900/70 transition-colors disabled:opacity-40"
            >
              {applyRollout.isPending ? <Loader2 size={9} className="animate-spin" /> : <Check size={9} />}
              Apply
            </button>
          </div>
          {/* Rust permits active -> approved to pull a release out of
              rotation without archiving it. */}
          <button
            onClick={() => demote.mutate()}
            disabled={anyPending}
            className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                       bg-blue-900/40 text-blue-300 border border-blue-800/40
                       hover:bg-blue-900/70 transition-colors disabled:opacity-40"
          >
            {demote.isPending ? <Loader2 size={9} className="animate-spin" /> : <Pause size={9} />}
            Demote
          </button>
        </>
      )}

      {(release.state === "draft"
        || release.state === "proposed"
        || release.state === "approved"
        || release.state === "active"
        || release.state === "rejected") && (
        <button
          onClick={() => archive.mutate()}
          disabled={anyPending}
          className="flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-medium
                     bg-zinc-800/60 text-zinc-400 border border-zinc-700/40
                     hover:bg-zinc-800 transition-colors disabled:opacity-40"
        >
          {archive.isPending ? <Loader2 size={9} className="animate-spin" /> : <Package size={9} />}
          Archive
        </button>
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
      expanded ? "border-gray-200 dark:border-zinc-700 bg-gray-50/60 dark:bg-zinc-900/60" : "border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 hover:border-gray-200 dark:border-zinc-700",
    )}>
      {/* Collapsed header */}
      <button
        onClick={() => setExpanded((v) => !v)}
        className="w-full flex items-center gap-3 px-4 py-3 text-left"
      >
        {expanded
          ? <ChevronDown  size={13} className="text-gray-400 dark:text-zinc-500 shrink-0" />
          : <ChevronRight size={13} className="text-gray-400 dark:text-zinc-600 shrink-0" />}

        <FileText size={13} className="text-gray-400 dark:text-zinc-600 shrink-0" />

        <span className="font-medium text-[13px] text-gray-800 dark:text-zinc-200 truncate flex-1">
          {asset.name}
        </span>

        <KindBadge kind={asset.kind} />

        {latestRelease && <ReleaseBadge state={latestRelease.state} />}

        <span className="text-[10px] text-gray-400 dark:text-zinc-600 font-mono shrink-0">
          {assetReleases.length} release{assetReleases.length !== 1 ? "s" : ""}
        </span>

        <span className="text-[11px] text-gray-300 dark:text-zinc-600 shrink-0 hidden sm:block">
          {fmtTime(asset.updated_at ?? asset.created_at)}
        </span>
      </button>

      {/* Expanded body */}
      {expanded && (
        <div className="border-t border-gray-200 dark:border-zinc-800 divide-y divide-gray-200 dark:divide-zinc-800/60">
          {/* Asset metadata */}
          <div className="px-4 py-2 flex items-center gap-4 text-[11px] text-gray-400 dark:text-zinc-600">
            <span className="font-mono">{asset.prompt_asset_id}</span>
            {asset.scope && <span>scope: <code className="text-gray-400 dark:text-zinc-500">{asset.scope}</code></span>}
          </div>

          {/* Version history */}
          <div className="px-4 py-3">
            <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">
              Version History
            </p>
            {versionsLoading ? (
              <div className="flex items-center gap-1.5 text-[12px] text-gray-400 dark:text-zinc-600 py-2">
                <Loader2 size={12} className="animate-spin" /> Loading versions…
              </div>
            ) : versions.length === 0 ? (
              <p className="text-[12px] text-gray-300 dark:text-zinc-600 italic py-2">No versions yet.</p>
            ) : (
              <Card variant="shell" className="overflow-x-auto divide-y divide-gray-200 dark:divide-zinc-800/50">
                {versions.map((v, i) => (
                  <VersionRow
                    key={v.prompt_version_id}
                    version={v}
                    assetId={asset.prompt_asset_id}
                    prevVersionId={versions[i + 1]?.prompt_version_id ?? null}
                    onCreateRelease={(vid) => createRelease.mutate(vid)}
                  />
                ))}
              </Card>
            )}
          </div>

          {/* Releases */}
          {assetReleases.length > 0 && (
            <div className="px-4 py-3">
              <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">
                Releases
              </p>
              <div className="space-y-2">
                {assetReleases.map((rel) => (
                  <Card key={rel.prompt_release_id} className="px-3 py-2">
                    <div className="flex items-center justify-between gap-3 mb-1.5">
                      <code className="text-[11px] font-mono text-gray-400 dark:text-zinc-500">
                        {shortId(rel.prompt_release_id)}
                      </code>
                      <span className="text-[10px] text-gray-300 dark:text-zinc-600">{fmtTime(rel.updated_at)}</span>
                    </div>
                    <ReleaseControls release={rel} />
                  </Card>
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
  const [kind, setKind] = useState<PromptKind>("system");
  const [initialBody, setInitialBody] = useState("");

  // Close on Escape
  useEffect(() => {
    function onKey(e: KeyboardEvent) { if (e.key === "Escape") onClose(); }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Single mutation: create asset, then (if body non-empty) create initial version.
  // Sequential: version depends on the returned asset_id, and a failure mid-flow
  // must not block asset discovery — we toast a partial-success message.
  const create = useMutation({
    mutationFn: async () => {
      const asset = await defaultApi.createPromptAsset({
        prompt_asset_id: id.trim(),
        name: name.trim(),
        kind,
      });
      const body = initialBody.trim();
      if (body.length === 0) return { asset, version: null as null };
      try {
        const content_hash = await sha256Hex(body);
        const version = await defaultApi.createPromptVersion(asset.prompt_asset_id, {
          content: body,
          content_hash,
        });
        return { asset, version };
      } catch (err) {
        // Asset is created; bubble a partial-success so the operator knows.
        throw Object.assign(new Error("version_failed"), { assetCreated: true, cause: err });
      }
    },
    onSuccess: (result) => {
      if (result.version) {
        toast.success(`Prompt asset “${result.asset.prompt_asset_id}” created with initial version.`);
      } else {
        toast.success(`Prompt asset “${result.asset.prompt_asset_id}” created.`);
      }
      void qc.invalidateQueries({ queryKey: ["prompt-assets"] });
      void qc.invalidateQueries({ queryKey: ["prompt-versions", result.asset.prompt_asset_id] });
      onClose();
    },
    onError: (err: unknown) => {
      if (err && typeof err === "object" && "assetCreated" in err) {
        toast.error("Asset created, but initial version failed. Add a version from the asset view.");
        void qc.invalidateQueries({ queryKey: ["prompt-assets"] });
        onClose();
      } else {
        toast.error("Failed to create prompt asset.");
      }
    },
  });

  const valid = id.trim().length > 0 && name.trim().length > 0;

  return (
    <Card className="px-4 py-3 space-y-3">
      <div className="flex items-center justify-between">
        <span className="text-[12px] font-medium text-gray-700 dark:text-zinc-300">New Prompt Asset</span>
        <button onClick={onClose} className="p-0.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
          <X size={13} />
        </button>
      </div>
      <div className="grid grid-cols-3 gap-3">
        <div>
          <label className="text-[10px] text-gray-400 dark:text-zinc-500 block mb-1">ID <span className="text-red-500">*</span></label>
          <input
            value={id} onChange={(e) => setId(e.target.value)}
            placeholder="my-system-prompt"
            className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 text-[12px] text-gray-800 dark:text-zinc-200
                       font-mono px-2 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors"
          />
        </div>
        <div>
          <label className="text-[10px] text-gray-400 dark:text-zinc-500 block mb-1">Name <span className="text-red-500">*</span></label>
          <input
            value={name} onChange={(e) => setName(e.target.value)}
            placeholder="My System Prompt"
            className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 text-[12px] text-gray-800 dark:text-zinc-200
                       px-2 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors"
          />
        </div>
        <div>
          <label className="text-[10px] text-gray-400 dark:text-zinc-500 block mb-1">Kind</label>
          <select
            value={kind}
            onChange={(e) => {
              const next = e.target.value;
              if (isPromptKind(next)) setKind(next);
            }}
            className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 text-[12px] text-gray-700 dark:text-zinc-300
                       px-2 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors"
          >
            {PROMPT_KINDS.map((k) => (
              <option key={k} value={k}>{KIND_LABEL[k]}</option>
            ))}
          </select>
        </div>
      </div>
      <div>
        <label className="text-[10px] text-gray-400 dark:text-zinc-500 block mb-1">
          Initial version <span className="text-gray-300 dark:text-zinc-600">(optional — leave blank to author later)</span>
        </label>
        <textarea
          value={initialBody}
          onChange={(e) => setInitialBody(e.target.value)}
          placeholder="You are a helpful assistant…"
          rows={5}
          className="w-full rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 text-[12px] text-gray-800 dark:text-zinc-200
                     font-mono px-2 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors resize-y"
        />
      </div>
      <div className="flex items-center gap-2 justify-end">
        <button onClick={onClose}
          className="px-3 py-1.5 rounded text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
          Cancel
        </button>
        <button
          onClick={() => create.mutate()}
          disabled={!valid || create.isPending}
          className="flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium
                     bg-indigo-600 hover:bg-indigo-500 text-white
                     disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 disabled:cursor-not-allowed transition-colors"
        >
          {create.isPending ? <Loader2 size={11} className="animate-spin" /> : <Plus size={11} />}
          Create
        </button>
      </div>
    </Card>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function PromptsPage() {
  const [showNew, setShowNew] = useState(false);
  const [filter, setFilter]  = useState<"all" | "active" | "draft">("all");

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
  const latestState = (assetId: string): PromptReleaseState | null => {
    const rels = releases
      .filter((r) => r.prompt_asset_id === assetId)
      .sort((a, b) => b.created_at - a.created_at);
    return rels[0]?.state ?? null;
  };

  const filtered = assets.filter((a) => {
    if (filter === "all")    return true;
    if (filter === "active") return latestState(a.prompt_asset_id) === "active";
    if (filter === "draft")  return !latestState(a.prompt_asset_id) || latestState(a.prompt_asset_id) === "draft";
    return true;
  });

  if (isError) return (
    <div className="flex flex-col items-center justify-center h-full gap-3 p-8 text-center">
      <AlertTriangle size={28} className="text-red-500" />
      <p className="text-[13px] font-medium text-gray-700 dark:text-zinc-300">Failed to load prompts</p>
      <p className="text-[12px] text-gray-400 dark:text-zinc-500">{error instanceof Error ? error.message : "Unknown"}</p>
      <button onClick={() => refetch()}
        className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-700 dark:text-zinc-300 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors">
        Retry
      </button>
    </div>
  );

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-950">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-5 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        <FileText size={13} className="text-indigo-400 shrink-0" />
        <span className={`${sectionLabel} mb-0`}>
          Prompts
        </span>

        {!assetsLoading && (
          <span className="text-[10px] text-gray-300 dark:text-zinc-600">
            {assets.length} asset{assets.length !== 1 ? "s" : ""} · {releases.length} release{releases.length !== 1 ? "s" : ""}
          </span>
        )}

        {/* Filter tabs */}
        <div className="flex items-center gap-0 ml-4">
          {(["all", "active", "draft"] as const).map((f) => (
            <button
              key={f}
              onClick={() => setFilter(f)}
              className={clsx(
                "px-3 h-11 text-[11px] font-medium transition-colors border-b-2 capitalize",
                filter === f
                  ? "text-gray-900 dark:text-zinc-100 border-indigo-500"
                  : "text-gray-400 dark:text-zinc-500 border-transparent hover:text-gray-700 dark:hover:text-zinc-300",
              )}
            >
              {f}
            </button>
          ))}
        </div>

        <div className="ml-auto flex items-center gap-2">
          <button onClick={() => refetch()} disabled={isFetching}
            className="flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 disabled:opacity-40 transition-colors">
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
          <div className="flex items-center justify-center gap-2 py-16 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading prompts…</span>
          </div>
        ) : filtered.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 gap-3 text-center">
            <FileText size={28} className="text-gray-300 dark:text-zinc-600" />
            <p className="text-[13px] text-gray-400 dark:text-zinc-600">
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
