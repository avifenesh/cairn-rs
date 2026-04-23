/**
 * ChannelsPage — runtime message channels CRUD.
 *
 * Operator UI for the `/v1/channels` API (cairn-runtime::ChannelService):
 * named, capacity-bounded, project-scoped pub/sub channels used by agent
 * sessions for inter-agent messaging. Each channel holds a ring of
 * ChannelMessage records with sender_id / body / consumed_by metadata.
 *
 * Backed by:
 *   GET    /v1/channels                     → ListResponse<Channel>
 *   POST   /v1/channels                     → Channel (create)
 *   POST   /v1/channels/:id/send            → { message_id }
 *   GET    /v1/channels/:id/messages        → ChannelMessage[]
 *   POST   /v1/channels/:id/consume         → ChannelMessage | null
 *
 * NOTE: notification preferences live on NotificationsPage.tsx.
 */

import { useState, useId } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  RefreshCw, Loader2, Plus, X, Send, MessageSquare, Radio, Inbox,
} from 'lucide-react';
import { clsx } from 'clsx';
import { StatCard } from '../components/StatCard';
import { defaultApi } from '../lib/api';
import { useFocusTrap } from '../hooks/useFocusTrap';
import { ErrorFallback } from '../components/ErrorFallback';
import { useToast } from '../components/Toast';
import type { Channel } from '../lib/types';
import { useScope } from '../hooks/useScope';

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtTime(ms: number): string {
  if (!ms) return '—';
  return new Date(ms).toLocaleString(undefined, {
    month: 'short', day: 'numeric',
    hour: '2-digit', minute: '2-digit', second: '2-digit',
  });
}

function fmtRelative(ms: number): string {
  if (!ms) return '—';
  const diff = Date.now() - ms;
  const s = Math.floor(diff / 1000);
  if (s < 60)  return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60)  return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 24)  return `${h}h ago`;
  return `${Math.floor(h / 24)}d ago`;
}

// ── Create-channel modal ──────────────────────────────────────────────────────

function CreateChannelModal({
  onClose,
  onCreate,
  isPending,
  error,
}: {
  onClose: () => void;
  onCreate: (name: string, capacity: number) => void;
  isPending: boolean;
  error: string | null;
}) {
  const formId = useId();
  const [name, setName] = useState('');
  const [capacity, setCapacity] = useState('100');
  const [fieldErr, setFieldErr] = useState<{ name?: string; capacity?: string }>({});

  function submit(e: React.FormEvent) {
    e.preventDefault();
    const errs: { name?: string; capacity?: string } = {};
    if (!name.trim()) errs.name = 'Name is required';
    const cap = Number.parseInt(capacity, 10);
    if (!Number.isFinite(cap) || cap <= 0) errs.capacity = 'Must be a positive integer';
    setFieldErr(errs);
    if (Object.keys(errs).length > 0) return;
    onCreate(name.trim(), cap);
  }

  const trapRef = useFocusTrap({ onClose });
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg w-full max-w-md mx-4 shadow-2xl"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-gray-200 dark:border-zinc-800">
          <div className="flex items-center gap-2">
            <Radio size={14} className="text-indigo-400" />
            <span className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">New Channel</span>
          </div>
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300">
            <X size={14} />
          </button>
        </div>
        <form id={formId} onSubmit={submit} className="p-5 space-y-4">
          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">
              Name <span className="text-red-400">*</span>
            </label>
            <input
              value={name}
              onChange={e => setName(e.target.value)}
              placeholder="alerts"
              className={clsx(
                'w-full h-8 bg-white dark:bg-zinc-950 border rounded-md px-3 text-[12px] font-mono',
                'text-gray-800 dark:text-zinc-200 focus:outline-none transition-colors',
                fieldErr.name ? 'border-red-500/60 focus:border-red-500' : 'border-gray-200 dark:border-zinc-800 focus:border-indigo-500',
              )}
              autoFocus
            />
            {fieldErr.name && <p className="mt-1 text-[11px] text-red-400">{fieldErr.name}</p>}
          </div>
          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">
              Capacity <span className="text-red-400">*</span>
            </label>
            <input
              value={capacity}
              onChange={e => setCapacity(e.target.value)}
              type="number"
              min={1}
              className={clsx(
                'w-full h-8 bg-white dark:bg-zinc-950 border rounded-md px-3 text-[12px] font-mono',
                'text-gray-800 dark:text-zinc-200 focus:outline-none transition-colors',
                fieldErr.capacity ? 'border-red-500/60 focus:border-red-500' : 'border-gray-200 dark:border-zinc-800 focus:border-indigo-500',
              )}
            />
            {fieldErr.capacity && <p className="mt-1 text-[11px] text-red-400">{fieldErr.capacity}</p>}
            <p className="mt-1 text-[10px] text-gray-400 dark:text-zinc-600">Maximum messages retained before oldest drops.</p>
          </div>
          {error && <p className="text-[11px] text-red-400 font-mono">{error}</p>}
        </form>
        <div className="flex justify-end gap-2 px-5 pb-5">
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
            {isPending ? 'Creating…' : 'Create'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Send-message modal ────────────────────────────────────────────────────────

function SendMessageModal({
  channel,
  onClose,
  onSend,
  isPending,
  error,
}: {
  channel: Channel;
  onClose: () => void;
  onSend: (senderId: string, body: string) => void;
  isPending: boolean;
  error: string | null;
}) {
  const formId = useId();
  const [senderId, setSenderId] = useState('operator');
  const [body, setBody] = useState('');

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!senderId.trim() || !body.trim()) return;
    onSend(senderId.trim(), body.trim());
  }

  const trapRef = useFocusTrap({ onClose });
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onClose}>
      <div
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg w-full max-w-lg mx-4 shadow-2xl"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-gray-200 dark:border-zinc-800">
          <div className="flex items-center gap-2">
            <Send size={14} className="text-indigo-400" />
            <span className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">
              Send to {channel.name}
            </span>
          </div>
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300">
            <X size={14} />
          </button>
        </div>
        <form id={formId} onSubmit={submit} className="p-5 space-y-4">
          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">Sender ID</label>
            <input
              value={senderId}
              onChange={e => setSenderId(e.target.value)}
              className="w-full h-8 bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-md px-3 text-[12px] font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 transition-colors"
            />
          </div>
          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">Body</label>
            <textarea
              value={body}
              onChange={e => setBody(e.target.value)}
              rows={5}
              placeholder="Message body (plain text or JSON)…"
              className="w-full bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-md px-3 py-2 text-[12px] font-mono text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 transition-colors"
              autoFocus
            />
          </div>
          {error && <p className="text-[11px] text-red-400 font-mono">{error}</p>}
        </form>
        <div className="flex justify-end gap-2 px-5 pb-5">
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
            disabled={isPending || !body.trim() || !senderId.trim()}
            className="px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 disabled:opacity-50 transition-colors flex items-center gap-1.5"
          >
            {isPending && <Loader2 size={11} className="animate-spin" />}
            {isPending ? 'Sending…' : 'Send'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Messages drawer ───────────────────────────────────────────────────────────

function MessagesDrawer({
  channel,
  onClose,
}: {
  channel: Channel;
  onClose: () => void;
}) {
  const trapRef = useFocusTrap({ onClose });
  const messagesQuery = useQuery({
    queryKey: ['channel-messages', channel.channel_id],
    queryFn: () => defaultApi.getChannelMessages(channel.channel_id, 100),
    retry: 1,
    staleTime: 5_000,
    refetchInterval: 10_000,
  });

  const messages = messagesQuery.data ?? [];

  return (
    <div className="fixed inset-0 z-40 flex" onClick={onClose}>
      <div className="flex-1 bg-black/40" />
      <div
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        className="w-[540px] max-w-full h-full bg-white dark:bg-zinc-950 border-l border-gray-200 dark:border-zinc-800 flex flex-col shadow-2xl"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 h-12 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <div className="flex items-center gap-2 min-w-0">
            <Inbox size={14} className="text-indigo-400 shrink-0" />
            <span className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100 truncate">
              {channel.name}
            </span>
            <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums shrink-0">
              ({messages.length} message{messages.length === 1 ? '' : 's'})
            </span>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={() => messagesQuery.refetch()}
              disabled={messagesQuery.isFetching}
              className="flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 disabled:opacity-40"
            >
              <RefreshCw size={11} className={messagesQuery.isFetching ? 'animate-spin' : ''} />
              Refresh
            </button>
            <button onClick={onClose} className="text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300">
              <X size={14} />
            </button>
          </div>
        </div>
        <div className="flex-1 overflow-y-auto">
          {messagesQuery.isLoading ? (
            <div className="flex items-center justify-center h-48 gap-2 text-gray-400 dark:text-zinc-600">
              <Loader2 size={14} className="animate-spin" />
              <span className="text-[12px]">Loading…</span>
            </div>
          ) : messages.length === 0 ? (
            <div className="flex flex-col items-center justify-center h-48 gap-2 text-center px-4">
              <MessageSquare size={20} className="text-gray-400 dark:text-zinc-600" />
              <p className="text-[12px] text-gray-500 dark:text-zinc-400">No messages yet.</p>
            </div>
          ) : (
            <div className="divide-y divide-gray-200/50 dark:divide-zinc-800/50">
              {messages.map(msg => (
                <div key={msg.message_id} className="px-4 py-2.5">
                  <div className="flex items-center gap-2 mb-1">
                    <span className="text-[11px] font-mono text-indigo-500 dark:text-indigo-300">
                      {msg.sender_id}
                    </span>
                    <span className="text-[10px] text-gray-400 dark:text-zinc-600 tabular-nums">
                      {fmtTime(msg.sent_at_ms)}
                    </span>
                    {msg.consumed_by && (
                      <span className="ml-auto text-[10px] text-emerald-400 font-mono">
                        consumed by {msg.consumed_by}
                      </span>
                    )}
                  </div>
                  <pre className="text-[12px] font-mono text-gray-800 dark:text-zinc-200 whitespace-pre-wrap break-words">
                    {msg.body}
                  </pre>
                </div>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function ChannelsPage() {
  const [scope] = useScope();
  const queryClient = useQueryClient();
  const toast = useToast();
  const [showCreate, setShowCreate] = useState(false);
  const [sendTarget, setSendTarget] = useState<Channel | null>(null);
  const [messagesTarget, setMessagesTarget] = useState<Channel | null>(null);

  // Pass scope explicitly so list/create are unambiguously project-scoped
  // and the query key ↔ network call coupling is obvious to reviewers.
  const listQuery = useQuery({
    queryKey: ['channels-crud', scope.tenant_id, scope.workspace_id, scope.project_id],
    queryFn: () => defaultApi.listChannels({
      tenant_id:    scope.tenant_id,
      workspace_id: scope.workspace_id,
      project_id:   scope.project_id,
    }),
    retry: 1,
    staleTime: 10_000,
  });

  const createMutation = useMutation({
    mutationFn: (args: { name: string; capacity: number }) =>
      defaultApi.createChannel(args.name, args.capacity, scope),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['channels-crud'] });
      setShowCreate(false);
      toast.success('Channel created.');
    },
    onError: (e: unknown) =>
      toast.error(e instanceof Error ? e.message : 'Failed to create channel.'),
  });

  const sendMutation = useMutation({
    mutationFn: (args: { channelId: string; senderId: string; body: string }) =>
      defaultApi.sendToChannel(args.channelId, args.senderId, args.body),
    onSuccess: (_, vars) => {
      queryClient.invalidateQueries({ queryKey: ['channel-messages', vars.channelId] });
      setSendTarget(null);
      toast.success('Message sent.');
    },
    onError: (e: unknown) =>
      toast.error(e instanceof Error ? e.message : 'Failed to send message.'),
  });

  if (listQuery.isError) {
    return (
      <ErrorFallback
        error={listQuery.error}
        resource="channels"
        onRetry={() => void listQuery.refetch()}
      />
    );
  }

  const channels = listQuery.data?.items ?? [];

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Channels
          {!listQuery.isLoading && (
            <span className="ml-2 text-[12px] text-gray-400 dark:text-zinc-500 font-normal">
              {channels.length}
            </span>
          )}
        </span>
        <span className="text-[11px] text-gray-400 dark:text-zinc-600 font-mono">
          {scope.tenant_id}/{scope.workspace_id}/{scope.project_id}
        </span>
        <button
          onClick={() => setShowCreate(true)}
          className="ml-auto flex items-center gap-1.5 px-2.5 py-1 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 transition-colors"
        >
          <Plus size={11} /> New Channel
        </button>
        <button
          onClick={() => listQuery.refetch()}
          disabled={listQuery.isFetching}
          className="flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={11} className={listQuery.isFetching ? 'animate-spin' : ''} />
          Refresh
        </button>
      </div>

      {/* Stat strip */}
      {!listQuery.isLoading && (
        <div className="grid grid-cols-3 gap-x-6 px-5 py-3 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <StatCard compact variant="info"    label="Channels"       value={channels.length} />
          <StatCard compact variant="success" label="Total Capacity"
            value={channels.reduce((sum, c) => sum + c.capacity, 0)} />
          <StatCard compact variant="info"    label="Project"
            value={scope.project_id} />
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-x-auto overflow-y-auto">
        {listQuery.isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading…</span>
          </div>
        ) : channels.length === 0 ? (
          <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
              <Radio size={24} className="text-gray-400 dark:text-zinc-500" />
            </div>
            <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No channels in this project</p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-xs">
              Create a named, capacity-bounded pub/sub channel for inter-agent messaging.
            </p>
            <button
              onClick={() => setShowCreate(true)}
              className="mt-1 flex items-center gap-1.5 px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 transition-colors"
            >
              <Plus size={11} /> New Channel
            </button>
          </div>
        ) : (
          <div className="min-w-[760px]">
            {/* Column headers */}
            <div className="flex items-center h-8 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 sticky top-0">
              <div className="flex-1 px-4 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Name</div>
              <div className="w-64 shrink-0 px-2 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Channel ID</div>
              <div className="w-24 shrink-0 px-2 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Capacity</div>
              <div className="w-28 shrink-0 px-2 text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Created</div>
              <div className="w-40 shrink-0 px-2 text-right text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Actions</div>
            </div>
            {channels.map((ch, i) => (
              <div
                key={ch.channel_id}
                className={clsx(
                  'flex items-center h-10 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0 hover:bg-white/[0.02] transition-colors',
                  i % 2 === 0 ? 'bg-gray-50 dark:bg-zinc-900' : 'bg-gray-50/50 dark:bg-zinc-900/50',
                )}
              >
                <div className="flex-1 min-w-0 px-4 flex items-center gap-2">
                  <Radio size={12} className="text-indigo-400 shrink-0" />
                  <span className="text-[12px] font-medium text-gray-800 dark:text-zinc-200 truncate">
                    {ch.name}
                  </span>
                </div>
                <div className="w-64 shrink-0 px-2">
                  <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-500 truncate block" title={ch.channel_id}>
                    {ch.channel_id}
                  </span>
                </div>
                <div className="w-24 shrink-0 px-2">
                  <span className="text-[11px] tabular-nums text-gray-500 dark:text-zinc-400">
                    {ch.capacity}
                  </span>
                </div>
                <div className="w-28 shrink-0 px-2">
                  <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums" title={fmtTime(ch.created_at)}>
                    {fmtRelative(ch.created_at)}
                  </span>
                </div>
                <div className="w-40 shrink-0 px-2 flex justify-end gap-1.5">
                  <button
                    onClick={() => setSendTarget(ch)}
                    title="Send test message"
                    className="flex items-center gap-1 px-1.5 py-1 rounded text-gray-500 dark:text-zinc-400 text-[11px] hover:bg-gray-100 dark:hover:bg-zinc-800 hover:text-indigo-400 transition-colors"
                  >
                    <Send size={10} /> Send
                  </button>
                  <button
                    onClick={() => setMessagesTarget(ch)}
                    title="View messages"
                    className="flex items-center gap-1 px-1.5 py-1 rounded text-gray-500 dark:text-zinc-400 text-[11px] hover:bg-gray-100 dark:hover:bg-zinc-800 hover:text-indigo-400 transition-colors"
                  >
                    <Inbox size={10} /> Messages
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Modals */}
      {showCreate && (
        <CreateChannelModal
          onClose={() => setShowCreate(false)}
          onCreate={(name, capacity) => createMutation.mutate({ name, capacity })}
          isPending={createMutation.isPending}
          error={createMutation.error instanceof Error ? createMutation.error.message : null}
        />
      )}
      {sendTarget && (
        <SendMessageModal
          channel={sendTarget}
          onClose={() => setSendTarget(null)}
          onSend={(senderId, body) =>
            sendMutation.mutate({ channelId: sendTarget.channel_id, senderId, body })
          }
          isPending={sendMutation.isPending}
          error={sendMutation.error instanceof Error ? sendMutation.error.message : null}
        />
      )}
      {messagesTarget && (
        <MessagesDrawer
          channel={messagesTarget}
          onClose={() => setMessagesTarget(null)}
        />
      )}
    </div>
  );
}

export default ChannelsPage;
