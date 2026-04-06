/**
 * usePresence — lightweight cross-tab presence tracking.
 *
 * Each browser tab publishes its current page to localStorage every
 * HEARTBEAT_MS milliseconds. Other tabs read the shared key on the
 * `storage` event (browsers only fire this in *other* tabs, not the writer).
 *
 * Entries older than EXPIRE_MS are pruned on every read so stale tabs
 * (closed without cleanup) disappear automatically.
 *
 * The hook returns a Map<page, PresenceEntry[]> of *other* active tabs
 * grouped by the page they are currently viewing.
 *
 * Tested with up to ~20 concurrent tabs; localStorage is synchronous so
 * there is no coordination overhead beyond a single JSON round-trip per
 * heartbeat/navigation.
 */

import { useState, useEffect, useRef, useMemo } from 'react';

// ── Constants ─────────────────────────────────────────────────────────────────

const LS_PRESENCE = 'cairn_presence';   // shared presence roster
const SS_TAB_ID   = 'cairn_tab_id';     // stable ID for this tab (sessionStorage)
const LS_COLOR    = 'cairn_tab_color_'; // per-tab color prefix (localStorage)

const HEARTBEAT_MS = 10_000;  // publish interval
const EXPIRE_MS    = 35_000;  // entries older than this are stale

// Distinct, readable colors on a dark background.
const COLOR_POOL = [
  '#ef4444', // red-500
  '#f97316', // orange-500
  '#eab308', // yellow-500
  '#22c55e', // green-500
  '#06b6d4', // cyan-500
  '#8b5cf6', // violet-500
  '#ec4899', // pink-500
  '#14b8a6', // teal-500
  '#f59e0b', // amber-500
  '#6366f1', // indigo-500
];

// ── Types ─────────────────────────────────────────────────────────────────────

export interface PresenceEntry {
  /** Unique opaque ID for this browser tab. */
  id:    string;
  /** NavPage slug the tab is currently showing. */
  page:  string;
  /** Hex color assigned once per tab, persisted across navigations. */
  color: string;
  /** Unix ms of last heartbeat. */
  ts:    number;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function getTabId(): string {
  let id = sessionStorage.getItem(SS_TAB_ID);
  if (!id) {
    id = Math.random().toString(36).slice(2, 10);
    try { sessionStorage.setItem(SS_TAB_ID, id); } catch { /* ignore */ }
  }
  return id;
}

function getTabColor(tabId: string): string {
  const key = LS_COLOR + tabId;
  try {
    const stored = localStorage.getItem(key);
    if (stored) return stored;
    const color = COLOR_POOL[Math.floor(Math.random() * COLOR_POOL.length)];
    localStorage.setItem(key, color);
    return color;
  } catch {
    return COLOR_POOL[0];
  }
}

function readRoster(): PresenceEntry[] {
  try {
    const raw = localStorage.getItem(LS_PRESENCE);
    if (!raw) return [];
    return JSON.parse(raw) as PresenceEntry[];
  } catch {
    return [];
  }
}

function writeRoster(entries: PresenceEntry[]) {
  try { localStorage.setItem(LS_PRESENCE, JSON.stringify(entries)); } catch { /* ignore */ }
}

function pruneExpired(entries: PresenceEntry[]): PresenceEntry[] {
  const cutoff = Date.now() - EXPIRE_MS;
  return entries.filter(e => e.ts > cutoff);
}

// ── Hook ──────────────────────────────────────────────────────────────────────

/**
 * @param currentPage  The NavPage slug this tab is currently showing.
 * @returns Map<page, PresenceEntry[]> — other active tabs grouped by page.
 */
export function usePresence(currentPage: string): Map<string, PresenceEntry[]> {
  const tabId    = useRef(getTabId());
  const tabColor = useRef(getTabColor(tabId.current));
  // pageRef lets the heartbeat effect always read the latest page without
  // needing to tear down and restart the interval on every navigation.
  const pageRef  = useRef(currentPage);

  const [others, setOthers] = useState<PresenceEntry[]>(() =>
    pruneExpired(readRoster()).filter(e => e.id !== tabId.current),
  );

  // ── Publish own entry ─────────────────────────────────────────────────────

  function publish(page: string) {
    const self: PresenceEntry = {
      id:    tabId.current,
      page,
      color: tabColor.current,
      ts:    Date.now(),
    };
    // Remove own stale entry, prune expired others, append fresh self.
    const roster = [
      ...pruneExpired(readRoster()).filter(e => e.id !== tabId.current),
      self,
    ];
    writeRoster(roster);
    // Update local state with the other tabs' slice.
    setOthers(roster.filter(e => e.id !== tabId.current));
  }

  // ── Sync pageRef and re-publish on page change ────────────────────────────

  useEffect(() => {
    pageRef.current = currentPage;
    publish(currentPage);
  // publish is stable (closure over refs); only re-run on page change.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentPage]);

  // ── Heartbeat ─────────────────────────────────────────────────────────────

  useEffect(() => {
    const timer = setInterval(() => publish(pageRef.current), HEARTBEAT_MS);
    return () => clearInterval(timer);
  // Run once; heartbeat reads pageRef.current so it's always up to date.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ── Cross-tab updates via storage event ───────────────────────────────────

  useEffect(() => {
    function onStorage(e: StorageEvent) {
      if (e.key !== LS_PRESENCE) return;
      setOthers(pruneExpired(readRoster()).filter(e => e.id !== tabId.current));
    }
    window.addEventListener('storage', onStorage);
    return () => window.removeEventListener('storage', onStorage);
  }, []); // tabId.current is stable

  // ── Cleanup on tab close ──────────────────────────────────────────────────

  useEffect(() => {
    function onUnload() {
      writeRoster(readRoster().filter(e => e.id !== tabId.current));
    }
    window.addEventListener('beforeunload', onUnload);
    return () => {
      window.removeEventListener('beforeunload', onUnload);
      // Also clean up on component unmount (e.g. logout).
      onUnload();
    };
  }, []); // stable

  // ── Derive page → entries map ─────────────────────────────────────────────

  return useMemo(() => {
    const map = new Map<string, PresenceEntry[]>();
    for (const entry of others) {
      const list = map.get(entry.page) ?? [];
      list.push(entry);
      map.set(entry.page, list);
    }
    return map;
  }, [others]);
}
