/**
 * NotificationCenter — bell icon in TopBar with dropdown panel.
 *
 * - Polls GET /v1/notifications every 15 s
 * - Red badge shows unread count (max 99)
 * - Dropdown: list with icon, message, timestamp, read state
 * - "Mark all read" button
 * - Click a notification → navigate to its href
 */

import { useState, useRef, useEffect } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  Bell, CheckCircle2, XCircle, AlertTriangle,
  Clock, CheckCheck, Play,
} from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import type { Notification, NotifType } from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

const fmtRelative = (ms: number): string => {
  const d = Date.now() - ms;
  if (d < 60_000)      return 'just now';
  if (d < 3_600_000)   return `${Math.floor(d / 60_000)}m ago`;
  if (d < 86_400_000)  return `${Math.floor(d / 3_600_000)}h ago`;
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
};

// ── Icon + color per notification type ───────────────────────────────────────

const TYPE_CONFIG: Record<NotifType, {
  icon: React.ComponentType<{ size?: number; className?: string }>;
  iconClass: string;
  dotClass: string;
}> = {
  approval_requested: { icon: AlertTriangle, iconClass: 'text-amber-400',   dotClass: 'bg-amber-500'   },
  approval_resolved:  { icon: CheckCircle2,  iconClass: 'text-emerald-400', dotClass: 'bg-emerald-500' },
  run_completed:      { icon: CheckCircle2,  iconClass: 'text-emerald-400', dotClass: 'bg-emerald-500' },
  run_failed:         { icon: XCircle,       iconClass: 'text-red-400',     dotClass: 'bg-red-500'     },
  task_stuck:         { icon: Clock,         iconClass: 'text-amber-400',   dotClass: 'bg-amber-500'   },
};

// ── Notification row ──────────────────────────────────────────────────────────

function NotifRow({
  notif,
  onRead,
}: {
  notif: Notification;
  onRead: (id: string) => void;
}) {
  const cfg = TYPE_CONFIG[notif.type] ?? TYPE_CONFIG.run_completed;

  function handleClick() {
    if (!notif.read) onRead(notif.id);
    window.location.hash = notif.href;
  }

  return (
    <button
      onClick={handleClick}
      className={clsx(
        'w-full flex items-start gap-3 px-4 py-3 text-left transition-colors',
        'hover:bg-gray-100 dark:hover:bg-zinc-800/50 border-b border-gray-200 dark:border-zinc-800/60 last:border-0',
        !notif.read && 'bg-gray-50 dark:bg-zinc-900/60',
      )}
    >
      {/* Type icon */}
      <div className={clsx(
        'shrink-0 w-7 h-7 rounded-full flex items-center justify-center mt-0.5',
        notif.read ? 'bg-gray-100 dark:bg-zinc-800/60' : 'bg-gray-200 dark:bg-zinc-800',
      )}>
        <cfg.icon size={13} className={notif.read ? 'text-gray-400 dark:text-zinc-600' : cfg.iconClass} />
      </div>

      {/* Message + time */}
      <div className="flex-1 min-w-0">
        <p className={clsx(
          'text-[12px] leading-snug',
          notif.read ? 'text-gray-400 dark:text-zinc-500' : 'text-gray-800 dark:text-zinc-200',
        )}>
          {notif.message}
        </p>
        <p className="text-[10px] text-gray-400 dark:text-zinc-600 mt-0.5 tabular-nums">
          {fmtRelative(notif.created_at)}
        </p>
      </div>

      {/* Unread indicator */}
      {!notif.read && (
        <span className={clsx('shrink-0 w-1.5 h-1.5 rounded-full mt-1.5', cfg.dotClass)} />
      )}
    </button>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export function NotificationCenter() {
  const [open, setOpen] = useState(false);
  const containerRef    = useRef<HTMLDivElement>(null);
  const qc              = useQueryClient();

  const { data } = useQuery({
    queryKey: ['notifications'],
    queryFn:  () => defaultApi.getNotifications(50),
    refetchInterval: 15_000,
    retry: false,
    staleTime: 10_000,
  });

  const rawNotifications = data?.notifications ?? [];
  // Sort newest first so the most recent notification is at the top.
  const notifications = [...rawNotifications].sort((a, b) => b.created_at - a.created_at);
  const unread        = data?.unread_count  ?? 0;

  // Close on outside click.
  useEffect(() => {
    if (!open) return;
    function handler(e: PointerEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener('pointerdown', handler);
    return () => document.removeEventListener('pointerdown', handler);
  }, [open]);

  // Close on Escape.
  useEffect(() => {
    if (!open) return;
    function handler(e: KeyboardEvent) {
      if (e.key === 'Escape') setOpen(false);
    }
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [open]);

  const markRead = useMutation({
    mutationFn: (id: string) => defaultApi.markNotificationRead(id),
    onSuccess:  () => void qc.invalidateQueries({ queryKey: ['notifications'] }),
  });

  const markAll = useMutation({
    mutationFn: () => defaultApi.markAllNotificationsRead(),
    onSuccess:  () => void qc.invalidateQueries({ queryKey: ['notifications'] }),
  });

  function handleMarkRead(id: string) {
    markRead.mutate(id);
  }

  function handleMarkAll() {
    markAll.mutate();
  }

  const badgeCount = Math.min(unread, 99);

  return (
    <div ref={containerRef} className="relative">
      {/* Bell button */}
      <button
        onClick={() => setOpen(v => !v)}
        aria-label={`Notifications${unread > 0 ? ` (${unread} unread)` : ''}`}
        aria-expanded={open}
        className={clsx(
          'relative p-1.5 rounded transition-colors',
          open
            ? 'bg-gray-100 dark:bg-zinc-800 text-gray-700 dark:text-zinc-200'
            : 'text-gray-400 dark:text-zinc-400 hover:text-gray-700 dark:hover:text-zinc-200 hover:bg-gray-100 dark:hover:bg-zinc-800',
        )}
      >
        <Bell size={15} />
        {/* Unread badge */}
        {badgeCount > 0 && (
          <span className="absolute -top-0.5 -right-0.5 min-w-[14px] h-[14px] rounded-full
                           bg-red-500 text-white text-[9px] font-bold leading-none
                           flex items-center justify-center px-0.5 select-none">
            {badgeCount > 9 ? '9+' : badgeCount}
          </span>
        )}
      </button>

      {/* Dropdown panel */}
      {open && (
        <div
          className="absolute right-0 top-full mt-1.5 w-[340px] rounded-xl
                     bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800
                     shadow-2xl shadow-black/20 dark:shadow-black/40 z-50 overflow-hidden"
          role="dialog"
          aria-label="Notifications"
        >
          {/* Header */}
          <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-zinc-800">
            <div className="flex items-center gap-2">
              <Bell size={13} className="text-gray-400 dark:text-zinc-500" />
              <span className="text-[12px] font-semibold text-gray-800 dark:text-zinc-300">Notifications</span>
              {unread > 0 && (
                <span className="text-[10px] text-gray-400 dark:text-zinc-600">{unread} unread</span>
              )}
            </div>
            {unread > 0 && (
              <button
                onClick={handleMarkAll}
                disabled={markAll.isPending}
                className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors disabled:opacity-40"
              >
                <CheckCheck size={11} />
                Mark all read
              </button>
            )}
          </div>

          {/* List */}
          <div className="max-h-[360px] overflow-y-auto">
            {notifications.length === 0 ? (
              <div className="flex flex-col items-center justify-center py-10 gap-2 text-gray-400 dark:text-zinc-600">
                <Play size={20} className="text-gray-300 dark:text-zinc-600" />
                <p className="text-[12px]">No notifications yet</p>
                <p className="text-[11px] text-gray-300 dark:text-zinc-600">
                  Approvals, run completions, and stuck tasks appear here.
                </p>
              </div>
            ) : (
              notifications.map(n => (
                <NotifRow
                  key={n.id}
                  notif={n}
                  onRead={handleMarkRead}
                />
              ))
            )}
          </div>

          {/* Footer */}
          {notifications.length > 0 && (
            <div className="px-4 py-2 border-t border-gray-200 dark:border-zinc-800 flex items-center justify-between">
              <span className="text-[10px] text-gray-400 dark:text-zinc-600">
                {notifications.length} notification{notifications.length !== 1 ? 's' : ''}
              </span>
              <button
                onClick={() => { window.location.hash = 'audit-log'; setOpen(false); }}
                className="text-[11px] text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-400 transition-colors"
              >
                View audit log →
              </button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
