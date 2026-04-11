/**
 * ChannelsPage — webhook / notification channel management.
 *
 * Backed by:
 *   GET  /v1/admin/operators/:id/notifications  → NotificationPreference
 *   POST /v1/admin/operators/:id/notifications  → upsert channels + event subscriptions
 *   GET  /v1/admin/notifications/failed         → delivery history / error log
 *   POST /v1/admin/notifications/:id/retry      → re-dispatch a failed delivery
 *   POST /v1/notifications/send                 → test-fire a notification
 */

import { useState, useId } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  RefreshCw, Loader2, Plus, Trash2, X,
  Bell, ChevronDown, ChevronRight, Webhook, Mail,
  AlertTriangle, CheckCircle2, Send, RotateCcw, Clock,
} from 'lucide-react';
import { clsx } from 'clsx';
import { StatCard } from '../components/StatCard';
import { defaultApi } from '../lib/api';
import { useFocusTrap } from '../hooks/useFocusTrap';
import { ErrorFallback } from '../components/ErrorFallback';
import type { NotificationChannel, NotificationRecord } from '../lib/types';
import { useScope } from '../hooks/useScope';

// ── Constants ─────────────────────────────────────────────────────────────────

const CHANNEL_TYPES = [
  { value: 'webhook',    label: 'Webhook',   placeholder: 'https://hooks.example.com/…' },
  { value: 'slack',      label: 'Slack',     placeholder: 'https://hooks.slack.com/services/…' },
  { value: 'email',      label: 'Email',     placeholder: 'alerts@example.com' },
  { value: 'pagerduty',  label: 'PagerDuty', placeholder: 'Routing key or service URL' },
  { value: 'telegram',   label: 'Telegram',  placeholder: 'Chat ID or bot webhook URL' },
] as const;

const ALL_EVENTS = [
  { value: 'run.failed',           label: 'Run Failed' },
  { value: 'run.completed',        label: 'Run Completed' },
  { value: 'run.paused',           label: 'Run Paused' },
  { value: 'task.failed',          label: 'Task Failed' },
  { value: 'task.completed',       label: 'Task Completed' },
  { value: 'approval.required',    label: 'Approval Required' },
  { value: 'approval.resolved',    label: 'Approval Resolved' },
  { value: 'provider.error',       label: 'Provider Error' },
  { value: 'provider.degraded',    label: 'Provider Degraded' },
  { value: 'budget.alert',         label: 'Budget Alert' },
  { value: 'agent.progress',       label: 'Agent Progress' },
  { value: 'memory.ingested',      label: 'Memory Ingested' },
  { value: 'credential.rotated',   label: 'Credential Rotated' },
];

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTime(ms: number): string {
  return new Date(ms).toLocaleString(undefined, {
    month: 'short', day: 'numeric',
    hour: '2-digit', minute: '2-digit', second: '2-digit',
  });
}

function fmtRelative(ms: number): string {
  const diff = Date.now() - ms;
  const m = Math.floor(diff / 60_000);
  const h = Math.floor(m / 60);
  const d = Math.floor(h / 24);
  if (d > 0)  return `${d}d ago`;
  if (h > 0)  return `${h}h ago`;
  if (m > 0)  return `${m}m ago`;
  return 'Just now';
}

function channelLabel(kind: string): string {
  return CHANNEL_TYPES.find(t => t.value === kind)?.label ?? kind;
}

function channelDisplayName(ch: NotificationChannel): string {
  const target = ch.target;
  try {
    if (ch.kind === 'webhook' || ch.kind === 'slack') {
      const url = new URL(target);
      return url.hostname + (url.pathname.length > 1 ? url.pathname.slice(0, 24) + '…' : '');
    }
  } catch { /* not a URL */ }
  return target.length > 36 ? target.slice(0, 34) + '…' : target;
}

type ChannelStatus = 'active' | 'inactive' | 'error';

function statusColors(s: ChannelStatus): string {
  if (s === 'active')   return 'text-emerald-400 bg-emerald-400/10 border-emerald-400/20';
  if (s === 'error')    return 'text-red-400 bg-red-400/10 border-red-400/20';
  return 'text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700';
}

function kindColors(kind: string): string {
  switch (kind) {
    case 'webhook':   return 'text-indigo-400 bg-indigo-400/10 border-indigo-400/20';
    case 'slack':     return 'text-purple-400 bg-purple-400/10 border-purple-400/20';
    case 'email':     return 'text-sky-400 bg-sky-400/10 border-sky-400/20';
    case 'pagerduty': return 'text-amber-400 bg-amber-400/10 border-amber-400/20';
    case 'telegram':  return 'text-cyan-400 bg-cyan-400/10 border-cyan-400/20';
    default:          return 'text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700';
  }
}

function KindIcon({ kind, size = 12 }: { kind: string; size?: number }) {
  if (kind === 'email')  return <Mail size={size} />;
  if (kind === 'slack')  return <Bell size={size} />;
  return <Webhook size={size} />;
}

// ── Channel detail panel ──────────────────────────────────────────────────────

function DeliveryRow({ rec }: { rec: NotificationRecord }) {
  return (
    <div className="flex items-center gap-3 px-3 py-1.5 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0 hover:bg-white/[0.02] transition-colors">
      <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-600 shrink-0 tabular-nums w-36">
        {fmtTime(rec.sent_at_ms)}
      </span>
      <span className="flex-1 min-w-0 text-[11px] text-gray-500 dark:text-zinc-400 font-mono truncate">
        {rec.event_type}
      </span>
      <span className={clsx('shrink-0 flex items-center gap-1 text-[10px]',
        rec.delivered ? 'text-emerald-400' : 'text-red-400')}>
        {rec.delivered
          ? <><CheckCircle2 size={10} /> delivered</>
          : <><AlertTriangle size={10} /> failed</>}
      </span>
      {rec.delivery_error && (
        <span className="shrink-0 text-[10px] font-mono text-red-400 truncate max-w-[180px]"
          title={rec.delivery_error}>
          {rec.delivery_error.slice(0, 40)}{rec.delivery_error.length > 40 ? '…' : ''}
        </span>
      )}
    </div>
  );
}

function ChannelDetail({
  channel,
  deliveries,
  tenantId,
  operatorId,
}: {
  channel: NotificationChannel;
  deliveries: NotificationRecord[];
  tenantId: string;
  operatorId: string;
}) {
  const queryClient = useQueryClient();
  const [testState, setTestState] = useState<'idle' | 'sending' | 'ok' | 'err'>('idle');
  const [testMsg,   setTestMsg]   = useState('');

  // Filter deliveries for this specific channel target
  const myDeliveries = deliveries
    .filter(d => d.channel_target === channel.target)
    .sort((a, b) => b.sent_at_ms - a.sent_at_ms)
    .slice(0, 20);

  const { mutate: retryRecord } = useMutation({
    mutationFn: (id: string) => defaultApi.retryNotification(id, tenantId),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['channels-failed', tenantId] }),
  });

  async function testConnection() {
    setTestState('sending');
    setTestMsg('');
    try {
      const res = await defaultApi.sendTestNotification(tenantId, {
        event_type:  'test.connection',
        message:     `Test from cairn dashboard (channel: ${channel.kind} → ${channel.target})`,
        severity:    'info',
        operator_id: operatorId,
      });
      setTestMsg(`Dispatched to ${res.dispatched} channel(s)`);
      setTestState('ok');
    } catch (e) {
      setTestMsg(e instanceof Error ? e.message : 'Test failed');
      setTestState('err');
    }
  }

  return (
    <div className="border-t border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950/30">
      <div className="flex items-center justify-between px-4 py-2.5">
        <span className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
          Recent Deliveries
          {myDeliveries.length > 0 && (
            <span className="ml-1.5 font-normal normal-case text-gray-300 dark:text-zinc-600">({myDeliveries.length})</span>
          )}
        </span>

        {/* Test connection */}
        <div className="flex items-center gap-2">
          {testState !== 'idle' && (
            <span className={clsx('text-[11px]',
              testState === 'ok'  ? 'text-emerald-400' :
              testState === 'err' ? 'text-red-400' : 'text-gray-400 dark:text-zinc-500')}>
              {testState === 'sending' ? 'Sending…' : testMsg}
            </span>
          )}
          <button
            onClick={testConnection}
            disabled={testState === 'sending'}
            className="flex items-center gap-1.5 px-2.5 py-1 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[11px] hover:bg-gray-200 dark:hover:bg-zinc-700 hover:text-gray-800 dark:hover:text-zinc-200 disabled:opacity-40 transition-colors"
          >
            {testState === 'sending'
              ? <Loader2 size={10} className="animate-spin" />
              : <Send size={10} />}
            Test Connection
          </button>
        </div>
      </div>

      {myDeliveries.length === 0 ? (
        <div className="px-4 pb-4 text-[12px] text-gray-400 dark:text-zinc-600 italic">
          No delivery records for this channel yet.
        </div>
      ) : (
        <div className="mx-4 mb-3 rounded-md border border-gray-200 dark:border-zinc-800 overflow-hidden bg-white dark:bg-zinc-950">
          {/* Table header */}
          <div className="flex items-center gap-3 px-3 h-7 border-b border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900">
            <span className="w-36 shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Timestamp</span>
            <span className="flex-1 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Event</span>
            <span className="shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Status</span>
            <span className="w-44 shrink-0 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Error</span>
          </div>
          {myDeliveries.map(rec => (
            <div key={rec.record_id} className="group relative">
              <DeliveryRow rec={rec} />
              {!rec.delivered && (
                <button
                  onClick={() => retryRecord(rec.record_id)}
                  className="absolute right-2 top-1/2 -translate-y-1/2 opacity-0 group-hover:opacity-100 flex items-center gap-1 px-1.5 py-0.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-400 dark:text-zinc-500 text-[10px] hover:text-gray-700 dark:hover:text-zinc-300 transition-all"
                >
                  <RotateCcw size={9} /> Retry
                </button>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Channel row ───────────────────────────────────────────────────────────────

function ChannelRow({
  channel,
  eventCount,
  status,
  lastTriggeredMs,
  deliveries,
  tenantId,
  operatorId,
  even,
  expanded,
  onToggle,
  onDelete,
}: {
  channel: NotificationChannel;
  eventCount: number;
  status: ChannelStatus;
  lastTriggeredMs: number | null;
  deliveries: NotificationRecord[];
  tenantId: string;
  operatorId: string;
  even: boolean;
  expanded: boolean;
  onToggle: () => void;
  onDelete: () => void;
}) {
  return (
    <div className={clsx(
      'border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0',
      even ? 'bg-gray-50 dark:bg-zinc-900' : 'bg-gray-50/50 dark:bg-zinc-900/50',
    )}>
      {/* Main row */}
      <div
        className="flex items-center gap-0 h-10 cursor-pointer hover:bg-white/[0.02] transition-colors select-none"
        onClick={onToggle}
      >
        {/* Expand chevron */}
        <div className="w-8 shrink-0 flex justify-center text-gray-400 dark:text-zinc-600">
          {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        </div>

        {/* Name */}
        <div className="flex-1 min-w-0 flex items-center gap-2 pr-2">
          <KindIcon kind={channel.kind} size={12} />
          <span className="text-[12px] font-medium text-gray-800 dark:text-zinc-200 truncate" title={channel.target}>
            {channelDisplayName(channel)}
          </span>
        </div>

        {/* Type badge */}
        <div className="w-28 shrink-0 px-2">
          <span className={clsx(
            'inline-flex items-center gap-1 px-1.5 py-0.5 rounded border text-[10px] font-medium',
            kindColors(channel.kind),
          )}>
            <KindIcon kind={channel.kind} size={9} />
            {channelLabel(channel.kind)}
          </span>
        </div>

        {/* URL/Target */}
        <div className="w-52 shrink-0 px-2">
          <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-500 truncate" title={channel.target}>
            {channel.target}
          </span>
        </div>

        {/* Status */}
        <div className="w-24 shrink-0 px-2">
          <span className={clsx(
            'inline-flex items-center px-1.5 py-0.5 rounded border text-[10px] font-medium',
            statusColors(status),
          )}>
            {status}
          </span>
        </div>

        {/* Last triggered */}
        <div className="w-28 shrink-0 px-2 flex items-center gap-1">
          {lastTriggeredMs ? (
            <>
              <Clock size={10} className="text-gray-400 dark:text-zinc-600 shrink-0" />
              <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">{fmtRelative(lastTriggeredMs)}</span>
            </>
          ) : (
            <span className="text-[11px] text-gray-300 dark:text-zinc-600">—</span>
          )}
        </div>

        {/* Events subscribed */}
        <div className="w-24 shrink-0 px-2">
          <span className="text-[11px] tabular-nums text-gray-500 dark:text-zinc-400">
            {eventCount} event{eventCount !== 1 ? 's' : ''}
          </span>
        </div>

        {/* Delete */}
        <div className="w-16 shrink-0 px-2 flex justify-end">
          <button
            onClick={e => { e.stopPropagation(); onDelete(); }}
            title="Remove channel"
            className="flex items-center gap-1 px-1.5 py-1 rounded text-gray-400 dark:text-zinc-600 text-[11px] hover:bg-red-500/10 hover:text-red-400 transition-colors"
          >
            <Trash2 size={10} />
          </button>
        </div>
      </div>

      {/* Expanded detail */}
      {expanded && (
        <ChannelDetail
          channel={channel}
          deliveries={deliveries}
          tenantId={tenantId}
          operatorId={operatorId}
        />
      )}
    </div>
  );
}

// ── Add channel modal ─────────────────────────────────────────────────────────

interface AddChannelForm {
  kind: string;
  target: string;
  selectedEvents: Set<string>;
}

function AddChannelModal({
  existingEvents,
  onClose,
  onAdd,
  isPending,
  error,
}: {
  existingEvents: string[];
  onClose: () => void;
  onAdd: (ch: NotificationChannel, events: string[]) => void;
  isPending: boolean;
  error: string | null;
}) {
  const formId = useId();
  const [form, setForm] = useState<AddChannelForm>({
    kind:           'webhook',
    target:         '',
    selectedEvents: new Set(existingEvents),
  });
  const [fieldErr, setFieldErr] = useState<{ kind?: string; target?: string }>({});

  const placeholder = CHANNEL_TYPES.find(t => t.value === form.kind)?.placeholder ?? '';

  function toggleEvent(ev: string) {
    setForm(f => {
      const s = new Set(f.selectedEvents);
      s.has(ev) ? s.delete(ev) : s.add(ev);
      return { ...f, selectedEvents: s };
    });
  }

  function validate(): boolean {
    const errs: { kind?: string; target?: string } = {};
    if (!form.kind)   errs.kind   = 'Type is required';
    if (!form.target.trim()) errs.target = 'Target is required';
    setFieldErr(errs);
    return Object.keys(errs).length === 0;
  }

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!validate()) return;
    onAdd(
      { kind: form.kind, target: form.target.trim() },
      Array.from(form.selectedEvents),
    );
  }

  const trapRef = useFocusTrap({ onClose: onClose });
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg w-full max-w-lg mx-4 shadow-2xl max-h-[90vh] flex flex-col"
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <div className="flex items-center gap-2">
            <Bell size={14} className="text-indigo-400" />
            <span className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">Add Channel</span>
          </div>
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
            <X size={14} />
          </button>
        </div>

        {/* Body — scrollable */}
        <form id={formId} onSubmit={submit} className="p-5 space-y-4 overflow-y-auto">
          {/* Type + Target side-by-side */}
          <div className="grid grid-cols-5 gap-3">
            <div className="col-span-2">
              <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">
                Type <span className="text-red-400">*</span>
              </label>
              <select
                value={form.kind}
                onChange={e => setForm(f => ({ ...f, kind: e.target.value }))}
                className="w-full h-8 bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-md px-2 text-[12px] text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 transition-colors"
              >
                {CHANNEL_TYPES.map(({ value, label }) => (
                  <option key={value} value={value}>{label}</option>
                ))}
              </select>
              {fieldErr.kind && <p className="mt-1 text-[11px] text-red-400">{fieldErr.kind}</p>}
            </div>

            <div className="col-span-3">
              <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">
                {form.kind === 'email' ? 'Email Address' : 'Target URL'}{' '}
                <span className="text-red-400">*</span>
              </label>
              <input
                type="text"
                value={form.target}
                onChange={e => { setForm(f => ({ ...f, target: e.target.value })); setFieldErr(v => ({ ...v, target: undefined })); }}
                placeholder={placeholder}
                className={clsx(
                  'w-full h-8 bg-white dark:bg-zinc-950 border rounded-md px-3 text-[12px] text-gray-800 dark:text-zinc-200 font-mono',
                  'placeholder-zinc-600 focus:outline-none transition-colors',
                  fieldErr.target ? 'border-red-500/60 focus:border-red-500' : 'border-gray-200 dark:border-zinc-800 focus:border-indigo-500',
                )}
              />
              {fieldErr.target && <p className="mt-1 text-[11px] text-red-400">{fieldErr.target}</p>}
            </div>
          </div>

          {/* Event subscriptions */}
          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-2">
              Event Subscriptions
              <span className="ml-1.5 text-gray-300 dark:text-zinc-600 font-normal">(applies to all channels)</span>
            </label>
            <div className="rounded-md border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 p-3 grid grid-cols-2 gap-x-4 gap-y-1.5">
              {ALL_EVENTS.map(({ value, label }) => (
                <label
                  key={value}
                  className="flex items-center gap-2 cursor-pointer group"
                >
                  <input
                    type="checkbox"
                    checked={form.selectedEvents.has(value)}
                    onChange={() => toggleEvent(value)}
                    className="w-3.5 h-3.5 rounded bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700 text-indigo-500 focus:ring-indigo-500 focus:ring-offset-0 accent-indigo-500"
                  />
                  <span className="text-[12px] text-gray-500 dark:text-zinc-400 group-hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
                    {label}
                  </span>
                </label>
              ))}
            </div>
            <p className="mt-1.5 text-[10px] text-gray-300 dark:text-zinc-600">
              {form.selectedEvents.size} of {ALL_EVENTS.length} events selected
            </p>
          </div>

          {error && (
            <p className="text-[11px] text-red-400 font-mono">{error}</p>
          )}
        </form>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-5 pb-5 shrink-0">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors"
          >
            Cancel
          </button>
          <button
            type="submit"
            form={formId}
            disabled={isPending}
            className="px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 disabled:opacity-50 transition-colors flex items-center gap-1.5"
          >
            {isPending && <Loader2 size={11} className="animate-spin" />}
            {isPending ? 'Adding…' : 'Add Channel'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Delete confirmation ───────────────────────────────────────────────────────

function DeleteDialog({
  channel,
  onConfirm,
  onCancel,
  isPending,
}: {
  channel: NotificationChannel;
  onConfirm: () => void;
  onCancel: () => void;
  isPending: boolean;
}) {
  const trapRef = useFocusTrap({ onClose: onCancel });
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onCancel}>
      <div
        className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg w-full max-w-sm mx-4 shadow-2xl"
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-start gap-3 p-5">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-red-500/10 border border-red-500/20">
            <AlertTriangle size={14} className="text-red-400" />
          </div>
          <div>
            <p className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">Remove channel?</p>
            <p className="text-[12px] text-gray-500 dark:text-zinc-400 mt-1">
              <span className="font-mono text-gray-700 dark:text-zinc-300">{channelLabel(channel.kind)}</span>
              {' → '}<span className="font-mono text-gray-700 dark:text-zinc-300">{channel.target}</span>{' '}
              will stop receiving notifications.
            </p>
          </div>
        </div>
        <div className="flex justify-end gap-2 px-5 pb-4">
          <button onClick={onCancel} className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors">Cancel</button>
          <button onClick={onConfirm} disabled={isPending} className="px-3 py-1.5 rounded bg-red-600 text-white text-[12px] hover:bg-red-500 disabled:opacity-50 transition-colors flex items-center gap-1.5">
            {isPending && <Loader2 size={11} className="animate-spin" />}
            {isPending ? 'Removing…' : 'Remove'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function ChannelsPage() {
  const [globalScope] = useScope();
  const [tenantId,   setTenantId]   = useState(globalScope.tenant_id);
  const [operatorId, setOperatorId] = useState('admin');
  const [expanded,   setExpanded]   = useState<string | null>(null);
  const [showAdd,    setShowAdd]    = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<NotificationChannel | null>(null);
  const queryClient = useQueryClient();

  // Fetch notification preferences (channels + event subscriptions)
  const prefsQuery = useQuery({
    queryKey: ['channels-prefs', tenantId, operatorId],
    queryFn:  () => defaultApi.getNotificationPreferences(operatorId, tenantId),
    retry: 1,
    staleTime: 30_000,
  });

  // Fetch failed notifications for delivery history
  const failedQuery = useQuery({
    queryKey: ['channels-failed', tenantId],
    queryFn:  () => defaultApi.getFailedNotifications(tenantId),
    retry: 1,
    staleTime: 30_000,
  });

  // Upsert preferences
  const { mutate: savePrefs, isPending: isSaving, error: saveError } = useMutation({
    mutationFn: (body: { event_types: string[]; channels: NotificationChannel[] }) =>
      defaultApi.setNotificationPreferences(operatorId, { tenant_id: tenantId, ...body }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['channels-prefs', tenantId, operatorId] });
      setShowAdd(false);
      setDeleteTarget(null);
    },
  });

  const channels    = prefsQuery.data?.channels   ?? [];
  const eventTypes  = prefsQuery.data?.event_types ?? [];
  const deliveries  = failedQuery.data?.items      ?? [];

  // Compute per-channel status from failure records
  function channelStatus(ch: NotificationChannel): ChannelStatus {
    const hasError = deliveries.some(d => d.channel_target === ch.target && !d.delivered);
    if (hasError) return 'error';
    if (eventTypes.length === 0) return 'inactive';
    return 'active';
  }

  function lastTriggered(ch: NotificationChannel): number | null {
    const hits = deliveries
      .filter(d => d.channel_target === ch.target)
      .map(d => d.sent_at_ms);
    return hits.length > 0 ? Math.max(...hits) : null;
  }

  function handleAdd(ch: NotificationChannel, events: string[]) {
    const updated = [...channels.filter(c => !(c.kind === ch.kind && c.target === ch.target)), ch];
    savePrefs({ channels: updated, event_types: events });
  }

  function handleDelete(ch: NotificationChannel) {
    const updated = channels.filter(c => !(c.kind === ch.kind && c.target === ch.target));
    savePrefs({ channels: updated, event_types: eventTypes });
  }

  const isError   = prefsQuery.isError && prefsQuery.error;
  const isLoading = prefsQuery.isLoading;

  // Compute stats
  const activeCount  = channels.filter(c => channelStatus(c) === 'active').length;
  const errorCount   = channels.filter(c => channelStatus(c) === 'error').length;

  if (isError) return <ErrorFallback error={prefsQuery.error} resource="channels" onRetry={() => void prefsQuery.refetch()} />;

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Channels
          {!isLoading && channels.length > 0 && (
            <span className="ml-2 text-[12px] text-gray-400 dark:text-zinc-500 font-normal">{channels.length}</span>
          )}
        </span>

        {/* Scope selectors */}
        <div className="flex items-center gap-3 ml-4">
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-gray-400 dark:text-zinc-600">Tenant:</span>
            <input
              value={tenantId}
              onChange={e => setTenantId(e.target.value || 'default')}
              className="h-6 w-24 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-2 text-[11px] font-mono text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500 transition-colors"
            />
          </div>
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-gray-400 dark:text-zinc-600">Operator:</span>
            <input
              value={operatorId}
              onChange={e => setOperatorId(e.target.value || 'admin')}
              className="h-6 w-24 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-2 text-[11px] font-mono text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500 transition-colors"
            />
          </div>
        </div>

        <button
          onClick={() => setShowAdd(true)}
          className="ml-auto flex items-center gap-1.5 px-2.5 py-1 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 transition-colors"
        >
          <Plus size={11} /> Add Channel
        </button>
        <button
          onClick={() => { prefsQuery.refetch(); failedQuery.refetch(); }}
          disabled={prefsQuery.isFetching}
          className="flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={11} className={prefsQuery.isFetching ? 'animate-spin' : ''} />
          Refresh
        </button>
      </div>

      {/* Stat strip */}
      {!isLoading && (
        <div className="grid grid-cols-4 gap-x-6 px-5 py-3 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <StatCard compact variant="info" label="Channels"    value={channels.length} />
          <StatCard compact variant="success" label="Active"      value={activeCount} />
          <StatCard compact variant="danger" label="Errors"      value={errorCount} description={errorCount > 0 ? 'check delivery log' : undefined} />
          <StatCard compact variant="info" label="Events"      value={eventTypes.length} description={eventTypes.length > 0 ? 'subscribed' : 'none configured'} />
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-x-auto overflow-y-auto">
        {isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading…</span>
          </div>
        ) : channels.length === 0 ? (
          <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
              <Bell size={24} className="text-gray-400 dark:text-zinc-500" />
            </div>
            <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No channels configured</p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-xs">
              Add a webhook, Slack, email, or PagerDuty channel to receive real-time notifications
              when runs fail, approvals are required, or providers degrade.
            </p>
            <button
              onClick={() => setShowAdd(true)}
              className="mt-1 flex items-center gap-1.5 px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 transition-colors"
            >
              <Plus size={11} /> Add Channel
            </button>
          </div>
        ) : (
          <div className="min-w-[860px]">
            {/* Column headers */}
            <div className="flex items-center h-8 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 sticky top-0">
              <div className="w-8 shrink-0" />
              <div className="flex-1 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Name / Target</span>
              </div>
              <div className="w-28 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Type</span>
              </div>
              <div className="w-52 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">URL / Target</span>
              </div>
              <div className="w-24 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Status</span>
              </div>
              <div className="w-28 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Last Triggered</span>
              </div>
              <div className="w-24 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Events</span>
              </div>
              <div className="w-16 shrink-0 px-2" />
            </div>

            {channels.map((ch, i) => {
              const key = `${ch.kind}:${ch.target}`;
              return (
                <ChannelRow
                  key={key}
                  channel={ch}
                  eventCount={eventTypes.length}
                  status={channelStatus(ch)}
                  lastTriggeredMs={lastTriggered(ch)}
                  deliveries={deliveries}
                  tenantId={tenantId}
                  operatorId={operatorId}
                  even={i % 2 === 0}
                  expanded={expanded === key}
                  onToggle={() => setExpanded(v => v === key ? null : key)}
                  onDelete={() => setDeleteTarget(ch)}
                />
              );
            })}
          </div>
        )}
      </div>

      {/* Event subscriptions footer — shown when channels exist */}
      {channels.length > 0 && eventTypes.length > 0 && (
        <div className="px-5 py-2.5 border-t border-gray-200 dark:border-zinc-800 shrink-0">
          <p className="text-[11px] text-gray-400 dark:text-zinc-600 mb-1">
            Subscribed events ({eventTypes.length}):
          </p>
          <div className="flex flex-wrap gap-1">
            {eventTypes.map(ev => (
              <span key={ev} className="inline-flex px-1.5 py-0.5 rounded bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 text-[10px] font-mono text-gray-400 dark:text-zinc-500">
                {ev}
              </span>
            ))}
          </div>
        </div>
      )}

      {/* Modals */}
      {showAdd && (
        <AddChannelModal
          existingEvents={eventTypes}
          onClose={() => setShowAdd(false)}
          onAdd={handleAdd}
          isPending={isSaving}
          error={saveError instanceof Error ? saveError.message : null}
        />
      )}

      {deleteTarget && (
        <DeleteDialog
          channel={deleteTarget}
          onConfirm={() => handleDelete(deleteTarget)}
          onCancel={() => setDeleteTarget(null)}
          isPending={isSaving}
        />
      )}
    </div>
  );
}

export default ChannelsPage;
