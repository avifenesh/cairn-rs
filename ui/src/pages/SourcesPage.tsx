import { useState, useEffect, type FormEvent } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  RefreshCw, Loader2, ServerCrash, Database, ChevronDown, ChevronRight,
  Plus, X, Edit2, Trash2, FileText, Clock, PlayCircle,
} from 'lucide-react';
import { clsx } from 'clsx';
import { StatCard } from '../components/StatCard';
import { defaultApi } from '../lib/api';
import { useToast } from '../components/Toast';
import { useScope } from '../hooks/useScope';
import { useFocusTrap } from '../hooks/useFocusTrap';
import { ds } from '../lib/design-system';
import type {
  SourceRecord, SourceQualityRecord, SourceChunkView, RefreshScheduleResponse,
} from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtDate(ms: number | null | undefined): string {
  if (!ms) return '—';
  const d = new Date(ms);
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' });
}

function fmtRelative(ms: number | null | undefined): string {
  if (!ms) return 'Never';
  const diff = Date.now() - ms;
  const mins  = Math.floor(diff / 60_000);
  const hours = Math.floor(mins / 60);
  const days  = Math.floor(hours / 24);
  if (days  > 0) return `${days}d ago`;
  if (hours > 0) return `${hours}h ago`;
  if (mins  > 0) return `${mins}m ago`;
  return 'Just now';
}

/** Infer a display type from the source_id prefix/pattern. */
function inferType(sourceId: string): string {
  const lower = sourceId.toLowerCase();
  if (lower.startsWith('web/')  || lower.includes('http'))  return 'Web';
  if (lower.startsWith('file/') || lower.includes('.md') || lower.includes('.txt')) return 'File';
  if (lower.startsWith('git/'))  return 'Git';
  if (lower.startsWith('db/') || lower.includes('sql'))     return 'Database';
  if (lower.startsWith('api/'))  return 'API';
  return 'Document';
}

/** Freshness status based on last_ingested_at. */
function sourceStatus(src: SourceRecord): { label: string; color: string } {
  if (!src.last_ingested_at_ms) {
    return { label: 'Pending', color: 'text-gray-400 dark:text-zinc-500 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700' };
  }
  const age = Date.now() - src.last_ingested_at_ms;
  const days = age / 86_400_000;
  if (days < 1)  return { label: 'Fresh',   color: 'text-emerald-400 bg-emerald-400/10 border-emerald-400/20' };
  if (days < 7)  return { label: 'Recent',  color: 'text-amber-400 bg-amber-400/10 border-amber-400/20' };
  return           { label: 'Stale',   color: 'text-red-400 bg-red-400/10 border-red-400/20' };
}

// ── Quality bar ───────────────────────────────────────────────────────────────

function QualityBar({ score }: { score: number }) {
  const pct   = Math.round(Math.min(score, 1) * 100);
  const color = pct >= 70 ? 'bg-emerald-500' : pct >= 40 ? 'bg-amber-500' : 'bg-red-500';
  return (
    <div className="flex items-center gap-2">
      <div className="w-16 h-1 rounded-full bg-gray-100 dark:bg-zinc-800">
        <div className={clsx('h-1 rounded-full', color)} style={{ width: `${pct}%` }} />
      </div>
      <span className="text-[11px] tabular-nums text-gray-400 dark:text-zinc-500 w-7 text-right">{pct}%</span>
    </div>
  );
}

// ── Quality detail panel ──────────────────────────────────────────────────────

function QualityPanel({ sourceId }: { sourceId: string }) {
  const { data, isLoading, isError } = useQuery<SourceQualityRecord>({
    queryKey: ['source-quality', sourceId],
    queryFn:  () => defaultApi.getSourceQuality(sourceId),
    staleTime: 30_000,
  });

  if (isLoading) return (
    <div className="flex items-center gap-2 px-6 py-3 text-gray-400 dark:text-zinc-600 text-[12px]">
      <Loader2 size={12} className="animate-spin" /> Loading quality metrics…
    </div>
  );
  if (isError || !data) return (
    <div className="px-6 py-3 text-[12px] text-gray-400 dark:text-zinc-600 italic">Quality data unavailable.</div>
  );

  const metrics: { label: string; value: string }[] = [
    { label: 'Chunks',          value: data.chunk_count.toLocaleString() },
    { label: 'Retrievals',      value: data.total_retrievals.toLocaleString() },
    { label: 'Credibility',     value: `${(data.credibility_score * 100).toFixed(0)}%` },
    { label: 'Avg Rating',      value: data.avg_rating != null ? data.avg_rating.toFixed(2) : '—' },
  ];

  return (
    <div className="px-6 py-3 border-t border-gray-200/60 dark:border-zinc-800/60 bg-white dark:bg-zinc-950/40">
      <div className="grid grid-cols-2 sm:grid-cols-4 gap-x-8 gap-y-2">
        {metrics.map(({ label, value }) => (
          <div key={label}>
            <p className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">{label}</p>
            <p className="text-[13px] font-semibold text-gray-800 dark:text-zinc-200 tabular-nums">{value}</p>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── Modal shell ───────────────────────────────────────────────────────────────

function Modal({
  title, icon, onClose, children,
}: {
  title: string;
  icon: React.ReactNode;
  onClose: () => void;
  children: React.ReactNode;
}) {
  const trapRef = useFocusTrap({ onClose });
  return (
    <div className={ds.modal.backdrop} onClick={onClose}>
      <div
        className={clsx(ds.modal.container, 'w-full max-w-lg mx-4 shadow-2xl')}
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-gray-200 dark:border-zinc-800">
          <div className="flex items-center gap-2">
            {icon}
            <span className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">{title}</span>
          </div>
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
            <X size={14} />
          </button>
        </div>
        {children}
      </div>
    </div>
  );
}

// ── Create source modal ───────────────────────────────────────────────────────

function CreateSourceModal({ onClose }: { onClose: () => void }) {
  const toast = useToast();
  const qc    = useQueryClient();
  const [sourceId,    setSourceId]    = useState('');
  const [name,        setName]        = useState('');
  const [description, setDescription] = useState('');

  const { mutate, isPending } = useMutation({
    mutationFn: () => defaultApi.createSource({
      source_id: sourceId.trim(),
      ...(name.trim()        ? { name:        name.trim() }        : {}),
      ...(description.trim() ? { description: description.trim() } : {}),
    }),
    onSuccess: () => {
      toast.success(`Source ${sourceId} created.`);
      qc.invalidateQueries({ queryKey: ['sources'] });
      onClose();
    },
    onError: (e: unknown) => toast.error(e instanceof Error ? e.message : 'Create failed.'),
  });

  function submit(e: FormEvent) {
    e.preventDefault();
    if (!sourceId.trim()) return;
    mutate();
  }

  return (
    <Modal title="New Source" icon={<Plus size={14} className="text-indigo-400" />} onClose={onClose}>
      <form onSubmit={submit} className="p-5 space-y-3">
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 font-medium">Source ID <span className="text-red-400">*</span></span>
          <input
            value={sourceId}
            onChange={e => setSourceId(e.target.value)}
            placeholder="docs/handbook"
            className="mt-1 w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 h-8 text-xs font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
            required
          />
        </label>
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 font-medium">Name</span>
          <input
            value={name}
            onChange={e => setName(e.target.value)}
            placeholder="Team Handbook"
            className="mt-1 w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 h-8 text-xs text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
          />
        </label>
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 font-medium">Description</span>
          <textarea
            value={description}
            onChange={e => setDescription(e.target.value)}
            rows={3}
            className="mt-1 w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 py-1.5 text-xs text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 resize-y"
          />
        </label>
        <div className="flex justify-end gap-2 pt-2">
          <button type="button" onClick={onClose} className="h-8 px-3 rounded-md border border-gray-200 dark:border-zinc-800 text-xs text-gray-600 dark:text-zinc-400 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors">Cancel</button>
          <button type="submit" disabled={isPending || !sourceId.trim()} className="h-8 px-3 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:bg-gray-200 dark:disabled:bg-zinc-800 disabled:text-gray-400 dark:disabled:text-zinc-600 text-white text-xs font-medium flex items-center gap-1.5 transition-colors">
            {isPending ? <Loader2 size={12} className="animate-spin" /> : null}
            Create
          </button>
        </div>
      </form>
    </Modal>
  );
}

// ── Edit source modal ─────────────────────────────────────────────────────────

function EditSourceModal({ sourceId, onClose }: { sourceId: string; onClose: () => void }) {
  const toast = useToast();
  const qc    = useQueryClient();

  const { data: detail } = useQuery({
    queryKey: ['source', sourceId],
    queryFn:  () => defaultApi.getSource(sourceId),
  });

  const [name,        setName]        = useState('');
  const [description, setDescription] = useState('');

  useEffect(() => {
    if (detail) {
      setName(detail.name ?? '');
      setDescription(detail.description ?? '');
    }
  }, [detail]);

  const { mutate, isPending } = useMutation({
    mutationFn: () => defaultApi.updateSource(sourceId, {
      name:        name.trim()        || undefined,
      description: description.trim() || undefined,
    }),
    onSuccess: () => {
      toast.success(`Source ${sourceId} updated.`);
      qc.invalidateQueries({ queryKey: ['sources'] });
      qc.invalidateQueries({ queryKey: ['source', sourceId] });
      onClose();
    },
    onError: (e: unknown) => toast.error(e instanceof Error ? e.message : 'Update failed.'),
  });

  function submit(e: FormEvent) {
    e.preventDefault();
    mutate();
  }

  return (
    <Modal title={`Edit ${sourceId}`} icon={<Edit2 size={14} className="text-indigo-400" />} onClose={onClose}>
      <form onSubmit={submit} className="p-5 space-y-3">
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 font-medium">Name</span>
          <input
            value={name}
            onChange={e => setName(e.target.value)}
            className="mt-1 w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 h-8 text-xs text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
          />
        </label>
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 font-medium">Description</span>
          <textarea
            value={description}
            onChange={e => setDescription(e.target.value)}
            rows={3}
            className="mt-1 w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 py-1.5 text-xs text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500 resize-y"
          />
        </label>
        <div className="flex justify-end gap-2 pt-2">
          <button type="button" onClick={onClose} className="h-8 px-3 rounded-md border border-gray-200 dark:border-zinc-800 text-xs text-gray-600 dark:text-zinc-400 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors">Cancel</button>
          <button type="submit" disabled={isPending} className="h-8 px-3 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:bg-gray-200 dark:disabled:bg-zinc-800 disabled:text-gray-400 dark:disabled:text-zinc-600 text-white text-xs font-medium flex items-center gap-1.5 transition-colors">
            {isPending ? <Loader2 size={12} className="animate-spin" /> : null}
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
}

// ── Delete confirm modal ──────────────────────────────────────────────────────

function DeleteSourceModal({ sourceId, onClose }: { sourceId: string; onClose: () => void }) {
  const toast = useToast();
  const qc    = useQueryClient();
  const { mutate, isPending } = useMutation({
    mutationFn: () => defaultApi.deleteSource(sourceId),
    onSuccess: () => {
      toast.success(`Source ${sourceId} deleted.`);
      qc.invalidateQueries({ queryKey: ['sources'] });
      onClose();
    },
    onError: (e: unknown) => toast.error(e instanceof Error ? e.message : 'Delete failed.'),
  });

  return (
    <Modal title="Delete Source" icon={<Trash2 size={14} className="text-red-400" />} onClose={onClose}>
      <div className="p-5 space-y-4">
        <p className="text-[13px] text-gray-700 dark:text-zinc-300">
          Delete source <span className="font-mono text-red-400">{sourceId}</span>? Its chunks stay in the
          store but the source is marked inactive and hidden from searches.
        </p>
        <div className="flex justify-end gap-2">
          <button type="button" onClick={onClose} className="h-8 px-3 rounded-md border border-gray-200 dark:border-zinc-800 text-xs text-gray-600 dark:text-zinc-400 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors">Cancel</button>
          <button type="button" onClick={() => mutate()} disabled={isPending} className="h-8 px-3 rounded-md bg-red-600 hover:bg-red-500 disabled:bg-gray-200 dark:disabled:bg-zinc-800 disabled:text-gray-400 dark:disabled:text-zinc-600 text-white text-xs font-medium flex items-center gap-1.5 transition-colors">
            {isPending ? <Loader2 size={12} className="animate-spin" /> : null}
            Delete
          </button>
        </div>
      </div>
    </Modal>
  );
}

// ── Chunks drawer modal ───────────────────────────────────────────────────────

function ChunksModal({ sourceId, onClose }: { sourceId: string; onClose: () => void }) {
  const { data, isLoading, isError, error } = useQuery({
    queryKey: ['source', sourceId, 'chunks'],
    queryFn:  () => defaultApi.getSourceChunks(sourceId, { limit: 100 }),
  });
  const items: SourceChunkView[] = data?.items ?? [];

  return (
    <Modal title={`Chunks — ${sourceId}`} icon={<FileText size={14} className="text-indigo-400" />} onClose={onClose}>
      <div className="p-5 max-h-[60vh] overflow-y-auto">
        {isLoading ? (
          <div className="flex items-center gap-2 text-xs text-gray-400 dark:text-zinc-500">
            <Loader2 size={12} className="animate-spin" /> Loading chunks…
          </div>
        ) : isError ? (
          <p className="text-xs text-red-400">{error instanceof Error ? error.message : 'Failed to load chunks.'}</p>
        ) : items.length === 0 ? (
          <p className="text-xs text-gray-400 dark:text-zinc-500 italic">No chunks for this source yet.</p>
        ) : (
          <ul className="space-y-2">
            {items.map(c => (
              <li key={c.chunk_id} className="border border-gray-200 dark:border-zinc-800 rounded-md p-2 bg-white dark:bg-zinc-950/40">
                <div className="flex items-center justify-between gap-2">
                  <span className="text-[10px] font-mono text-gray-400 dark:text-zinc-500 truncate" title={c.chunk_id}>{c.chunk_id}</span>
                  {c.credibility_score != null && (
                    <span className="text-[10px] tabular-nums text-gray-400 dark:text-zinc-500">
                      cred {(c.credibility_score * 100).toFixed(0)}%
                    </span>
                  )}
                </div>
                <p className="mt-1 text-[11px] text-gray-700 dark:text-zinc-300 leading-relaxed">
                  {c.text_preview}
                </p>
              </li>
            ))}
          </ul>
        )}
      </div>
    </Modal>
  );
}

// ── Schedule modal ────────────────────────────────────────────────────────────

function ScheduleModal({ sourceId, onClose }: { sourceId: string; onClose: () => void }) {
  const toast = useToast();
  const qc    = useQueryClient();

  const { data: current } = useQuery<RefreshScheduleResponse>({
    queryKey: ['source', sourceId, 'schedule'],
    queryFn:  () => defaultApi.getSourceRefreshSchedule(sourceId),
    retry: false,
  });

  const [intervalMin, setIntervalMin] = useState<number>(60);
  const [refreshUrl,  setRefreshUrl]  = useState<string>('');

  useEffect(() => {
    if (current) {
      setIntervalMin(Math.max(1, Math.round(current.interval_ms / 60_000)));
      setRefreshUrl(current.refresh_url ?? '');
    }
  }, [current]);

  const { mutate, isPending } = useMutation({
    mutationFn: () => defaultApi.setSourceRefreshSchedule(sourceId, {
      interval_ms: Math.max(1, intervalMin) * 60_000,
      refresh_url: refreshUrl.trim() || null,
    }),
    onSuccess: () => {
      toast.success(`Schedule saved for ${sourceId}.`);
      qc.invalidateQueries({ queryKey: ['source', sourceId, 'schedule'] });
      qc.invalidateQueries({ queryKey: ['sources'] });
      onClose();
    },
    onError: (e: unknown) => toast.error(e instanceof Error ? e.message : 'Save schedule failed.'),
  });

  function submit(e: FormEvent) {
    e.preventDefault();
    if (intervalMin < 1) return;
    mutate();
  }

  return (
    <Modal title={`Refresh Schedule — ${sourceId}`} icon={<Clock size={14} className="text-indigo-400" />} onClose={onClose}>
      <form onSubmit={submit} className="p-5 space-y-3">
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 font-medium">Interval (minutes) <span className="text-red-400">*</span></span>
          <input
            type="number"
            min={1}
            value={intervalMin}
            onChange={e => setIntervalMin(Number(e.target.value))}
            className="mt-1 w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 h-8 text-xs tabular-nums text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
            required
          />
        </label>
        <label className="block">
          <span className="text-[11px] text-gray-500 dark:text-zinc-400 font-medium">Refresh URL</span>
          <input
            value={refreshUrl}
            onChange={e => setRefreshUrl(e.target.value)}
            placeholder="https://example.com/feed (optional)"
            className="mt-1 w-full rounded-md bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 px-3 h-8 text-xs font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500"
          />
        </label>
        {current && (
          <p className="text-[11px] text-gray-400 dark:text-zinc-500">
            Last refresh: {fmtRelative(current.last_refresh_ms)} · schedule id <span className="font-mono">{current.schedule_id}</span>
          </p>
        )}
        <div className="flex justify-end gap-2 pt-2">
          <button type="button" onClick={onClose} className="h-8 px-3 rounded-md border border-gray-200 dark:border-zinc-800 text-xs text-gray-600 dark:text-zinc-400 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors">Cancel</button>
          <button type="submit" disabled={isPending || intervalMin < 1} className="h-8 px-3 rounded-md bg-indigo-600 hover:bg-indigo-500 disabled:bg-gray-200 dark:disabled:bg-zinc-800 disabled:text-gray-400 dark:disabled:text-zinc-600 text-white text-xs font-medium flex items-center gap-1.5 transition-colors">
            {isPending ? <Loader2 size={12} className="animate-spin" /> : null}
            Save
          </button>
        </div>
      </form>
    </Modal>
  );
}

// ── Source row ────────────────────────────────────────────────────────────────

function SourceRow({ source, even, expanded, onToggle, onEdit, onDelete, onChunks, onSchedule }: {
  source: SourceRecord;
  even: boolean;
  expanded: boolean;
  onToggle: () => void;
  onEdit:     (sid: string) => void;
  onDelete:   (sid: string) => void;
  onChunks:   (sid: string) => void;
  onSchedule: (sid: string) => void;
}) {
  const status = sourceStatus(source);

  return (
    <div className={clsx('border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0', even ? 'bg-gray-50 dark:bg-zinc-900' : 'bg-gray-50/50 dark:bg-zinc-900/50')}>
      {/* Main row */}
      <div
        className="flex items-center gap-3 px-4 h-10 cursor-pointer hover:bg-white/[0.02] transition-colors"
        onClick={onToggle}
      >
        {/* Expand icon */}
        <span className="shrink-0 text-gray-400 dark:text-zinc-600">
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </span>

        {/* Source ID */}
        <span className="flex-1 min-w-0 text-[12px] font-mono text-gray-700 dark:text-zinc-300 truncate" title={source.source_id}>
          {source.source_id}
        </span>

        {/* Type */}
        <span className="w-20 shrink-0 text-[11px] text-gray-400 dark:text-zinc-500">
          {inferType(source.source_id)}
        </span>

        {/* Status */}
        <span className={clsx('w-16 shrink-0 inline-flex items-center px-1.5 py-0.5 rounded border text-[10px] font-medium', status.color)}>
          {status.label}
        </span>

        {/* Last sync */}
        <span className="w-24 shrink-0 text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums" title={fmtDate(source.last_ingested_at_ms)}>
          {fmtRelative(source.last_ingested_at_ms)}
        </span>

        {/* Records */}
        <span className="w-16 shrink-0 text-right text-[12px] tabular-nums text-gray-700 dark:text-zinc-300">
          {source.document_count.toLocaleString()}
        </span>

        {/* Quality bar */}
        <div className="w-28 shrink-0 flex justify-end">
          <QualityBar score={source.avg_quality_score} />
        </div>

        {/* Actions */}
        <div className="shrink-0 flex items-center gap-1" onClick={e => e.stopPropagation()}>
          <button title="View chunks" onClick={() => onChunks(source.source_id)}
            className="p-1 rounded hover:bg-gray-100 dark:hover:bg-zinc-800 text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
            <FileText size={12} />
          </button>
          <button title="Refresh schedule" onClick={() => onSchedule(source.source_id)}
            className="p-1 rounded hover:bg-gray-100 dark:hover:bg-zinc-800 text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
            <Clock size={12} />
          </button>
          <button title="Edit" onClick={() => onEdit(source.source_id)}
            className="p-1 rounded hover:bg-gray-100 dark:hover:bg-zinc-800 text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
            <Edit2 size={12} />
          </button>
          <button title="Delete" onClick={() => onDelete(source.source_id)}
            className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-500/20 text-gray-400 dark:text-zinc-500 hover:text-red-500 transition-colors">
            <Trash2 size={12} />
          </button>
        </div>
      </div>

      {/* Expanded quality panel */}
      {expanded && <QualityPanel sourceId={source.source_id} />}
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function SourcesPage() {
  const toast = useToast();
  const qc    = useQueryClient();
  const [scope] = useScope();
  const [expanded, setExpanded] = useState<string | null>(null);
  const [creating,       setCreating]       = useState(false);
  const [editing,        setEditing]        = useState<string | null>(null);
  const [deleting,       setDeleting]       = useState<string | null>(null);
  const [chunksFor,      setChunksFor]      = useState<string | null>(null);
  const [scheduleFor,    setScheduleFor]    = useState<string | null>(null);

  // Scope travels in the queryKey so changing tenant/workspace/project does
  // not serve stale sources from a different scope's cache.
  const { data: sources, isLoading, isError, error, refetch, isFetching } = useQuery<SourceRecord[]>({
    queryKey: ['sources', scope.tenant_id, scope.workspace_id, scope.project_id],
    queryFn:  () => defaultApi.getSources(),
    refetchInterval: 60_000,
  });

  const processRefresh = useMutation({
    mutationFn: () => defaultApi.processSourceRefresh(),
    onSuccess: (res) => {
      toast.success(`Processed ${res.processed_count} due schedule${res.processed_count === 1 ? '' : 's'}.`);
      qc.invalidateQueries({ queryKey: ['sources'] });
    },
    onError: (e: unknown) => toast.error(e instanceof Error ? e.message : 'Refresh failed.'),
  });

  const rows = sources ?? [];

  const totalDocs   = rows.reduce((s, r) => s + r.document_count, 0);
  const avgQuality  = rows.length > 0
    ? rows.reduce((s, r) => s + r.avg_quality_score, 0) / rows.length
    : 0;

  if (isError) return (
    <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
      <ServerCrash size={32} className="text-red-500" />
      <p className="text-[13px] text-gray-700 dark:text-zinc-300 font-medium">Failed to load sources</p>
      <p className="text-[12px] text-gray-400 dark:text-zinc-500">
        {error instanceof Error ? error.message : 'Unknown error'}
      </p>
      <button onClick={() => refetch()} className="mt-1 px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-700 dark:text-zinc-300 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors">
        Retry
      </button>
    </div>
  );

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Sources
          {!isLoading && (
            <span className="ml-2 text-[12px] text-gray-400 dark:text-zinc-500 font-normal">{rows.length}</span>
          )}
        </span>
        <button
          onClick={() => setCreating(true)}
          className="ml-auto flex items-center gap-1 h-7 px-2.5 rounded-md bg-indigo-600 hover:bg-indigo-500 text-white text-[12px] font-medium transition-colors"
        >
          <Plus size={12} /> New Source
        </button>
        <button
          onClick={() => processRefresh.mutate()}
          disabled={processRefresh.isPending}
          className="flex items-center gap-1 h-7 px-2.5 rounded-md border border-gray-200 dark:border-zinc-800 text-[12px] text-gray-600 dark:text-zinc-400 hover:bg-gray-100 dark:hover:bg-zinc-800 disabled:opacity-40 transition-colors"
          title="Process due refresh schedules"
        >
          {processRefresh.isPending ? <Loader2 size={11} className="animate-spin" /> : <PlayCircle size={11} />}
          Process Due
        </button>
        <button
          onClick={() => refetch()}
          disabled={isFetching}
          className="flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={11} className={isFetching ? 'animate-spin' : ''} />
          Refresh
        </button>
      </div>

      {/* Stat strip */}
      {!isLoading && rows.length > 0 && (
        <div className="grid grid-cols-3 gap-x-6 px-5 py-3 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <StatCard compact variant="info" label="Sources"     value={rows.length} />
          <StatCard compact variant="info" label="Total Docs"  value={totalDocs.toLocaleString()} />
          <StatCard compact variant="info" label="Avg Quality" value={`${(avgQuality * 100).toFixed(0)}%`} />
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-x-auto overflow-y-auto">
        {isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading…</span>
          </div>
        ) : rows.length === 0 ? (
          <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
              <Database size={24} className="text-gray-400 dark:text-zinc-500" />
            </div>
            <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No sources registered</p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-xs">
              Click "New Source" to register one, or ingest a document on the Memory page.
            </p>
          </div>
        ) : (
          <div>
            {/* Column headers */}
            <div className="flex items-center gap-3 px-4 h-8 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 sticky top-0">
              <span className="w-4 shrink-0" />
              <span className="flex-1 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Source ID</span>
              <span className="w-20 shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Type</span>
              <span className="w-16 shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Status</span>
              <span className="w-24 shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Last Sync</span>
              <span className="w-16 shrink-0 text-right text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Records</span>
              <span className="w-28 shrink-0 text-right text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Quality</span>
              <span className="shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Actions</span>
            </div>

            {rows.map((source, i) => (
              <SourceRow
                key={source.source_id}
                source={source}
                even={i % 2 === 0}
                expanded={expanded === source.source_id}
                onToggle={() => setExpanded(v => v === source.source_id ? null : source.source_id)}
                onEdit={setEditing}
                onDelete={setDeleting}
                onChunks={setChunksFor}
                onSchedule={setScheduleFor}
              />
            ))}
          </div>
        )}
      </div>

      {/* Modals */}
      {creating    && <CreateSourceModal onClose={() => setCreating(false)} />}
      {editing     && <EditSourceModal   sourceId={editing}     onClose={() => setEditing(null)} />}
      {deleting    && <DeleteSourceModal sourceId={deleting}    onClose={() => setDeleting(null)} />}
      {chunksFor   && <ChunksModal       sourceId={chunksFor}   onClose={() => setChunksFor(null)} />}
      {scheduleFor && <ScheduleModal     sourceId={scheduleFor} onClose={() => setScheduleFor(null)} />}
    </div>
  );
}

export default SourcesPage;
