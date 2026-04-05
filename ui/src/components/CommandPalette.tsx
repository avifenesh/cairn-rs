/**
 * CommandPalette — Cmd+K / Ctrl+K quick-navigation overlay.
 *
 * Features:
 *  - Opens on Cmd+K (Mac) or Ctrl+K (Win/Linux), closes on Escape
 *  - Fuzzy-filters all nav pages by label
 *  - Shows recent runs and sessions as quick-jump targets
 *  - Keyboard navigable (↑↓ to move, Enter to confirm)
 *  - Closes on backdrop click
 */

import {
  useState,
  useEffect,
  useRef,
  useCallback,
  type KeyboardEvent,
} from 'react';
import {
  Search,
  LayoutDashboard,
  Play,
  MonitorPlay,
  CheckSquare,
  Zap,
  Coins,
  Database,
  Settings,
  ArrowRight,
  Clock,
  Terminal,
} from 'lucide-react';
import { clsx } from 'clsx';
import { useQuery } from '@tanstack/react-query';
import { defaultApi } from '../lib/api';
import type { NavPage } from './Sidebar';

// ── Nav page definitions ──────────────────────────────────────────────────────

interface NavOption {
  kind: 'page';
  id: NavPage;
  label: string;
  description: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
}

interface QuickJumpOption {
  kind: 'run' | 'session';
  id: string;
  label: string;
  description: string;
  page: NavPage;
}

type Option = NavOption | QuickJumpOption;

const NAV_OPTIONS: NavOption[] = [
  { kind: 'page', id: 'dashboard',  label: 'Dashboard',  description: 'Overview, metrics, event stream', icon: LayoutDashboard },
  { kind: 'page', id: 'runs',       label: 'Runs',        description: 'Active and historical runs',       icon: Play            },
  { kind: 'page', id: 'sessions',   label: 'Sessions',    description: 'Conversation sessions',            icon: MonitorPlay     },
  { kind: 'page', id: 'approvals',  label: 'Approvals',   description: 'Pending operator approvals',       icon: CheckSquare     },
  { kind: 'page', id: 'providers',  label: 'Providers',   description: 'LLM provider health',              icon: Zap             },
  { kind: 'page', id: 'costs',      label: 'Costs',       description: 'Token spend and provider costs',   icon: Coins           },
  { kind: 'page', id: 'memory',     label: 'Memory',      description: 'Knowledge base and search',        icon: Database        },
  { kind: 'page', id: 'settings',   label: 'Settings',    description: 'Deployment configuration',         icon: Settings        },
];

// ── Fuzzy filter ──────────────────────────────────────────────────────────────

function matches(query: string, text: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  // Substring match first, then each char in order
  if (t.includes(q)) return true;
  let ti = 0;
  for (const ch of q) {
    while (ti < t.length && t[ti] !== ch) ti++;
    if (ti >= t.length) return false;
    ti++;
  }
  return true;
}

// ── Option row ────────────────────────────────────────────────────────────────

function OptionRow({
  option,
  active,
  onSelect,
}: {
  option: Option;
  active: boolean;
  onSelect: () => void;
}) {
  const ref = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (active) ref.current?.scrollIntoView({ block: 'nearest' });
  }, [active]);

  const isPage = option.kind === 'page';
  const Icon = isPage ? option.icon : (option.kind === 'run' ? Terminal : Clock);

  return (
    <button
      ref={ref}
      onClick={onSelect}
      className={clsx(
        'w-full flex items-center gap-3 px-3 py-2.5 text-left rounded-lg transition-colors',
        active
          ? 'bg-indigo-600/30 ring-1 ring-inset ring-indigo-500/40'
          : 'hover:bg-zinc-800/60'
      )}
    >
      {/* Icon */}
      <div
        className={clsx(
          'flex h-7 w-7 shrink-0 items-center justify-center rounded-md',
          isPage
            ? 'bg-zinc-800 text-zinc-300'
            : option.kind === 'run'
              ? 'bg-blue-950 text-blue-400'
              : 'bg-sky-950 text-sky-400'
        )}
      >
        <Icon size={14} />
      </div>

      {/* Label + description */}
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium text-zinc-100 truncate">{option.label}</p>
        <p className="text-xs text-zinc-500 truncate">{option.description}</p>
      </div>

      {/* Arrow indicator for active */}
      {active && <ArrowRight size={13} className="shrink-0 text-indigo-400" />}
    </button>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

interface CommandPaletteProps {
  onNavigate: (page: NavPage) => void;
}

export function CommandPalette({ onNavigate }: CommandPaletteProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery]   = useState('');
  const [activeIdx, setActiveIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Fetch recent runs + sessions for quick-jump (non-blocking)
  const { data: runs = [] }     = useQuery({ queryKey: ['runs'],     queryFn: () => defaultApi.getRuns({ limit: 5 }),    enabled: open, staleTime: 30_000 });
  const { data: sessions = [] } = useQuery({ queryKey: ['sessions'], queryFn: () => defaultApi.getSessions({ limit: 5 }), enabled: open, staleTime: 30_000 });

  // Build quick-jump options from recent data
  const recentRuns: QuickJumpOption[] = runs.slice(0, 5).map((r) => ({
    kind:        'run',
    id:          r.run_id,
    label:       r.run_id.length > 24 ? `${r.run_id.slice(0, 20)}…` : r.run_id,
    description: `Run · ${r.state}`,
    page:        'runs' as NavPage,
  }));
  const recentSessions: QuickJumpOption[] = sessions.slice(0, 3).map((s) => ({
    kind:        'session',
    id:          s.session_id,
    label:       s.session_id.length > 24 ? `${s.session_id.slice(0, 20)}…` : s.session_id,
    description: `Session · ${s.state}`,
    page:        'sessions' as NavPage,
  }));

  // Filter nav pages by query
  const filteredPages = NAV_OPTIONS.filter(
    (o) => matches(query, o.label) || matches(query, o.description)
  );

  // Quick-jump only when query is empty or matches id
  const filteredRecent = query
    ? [...recentRuns, ...recentSessions].filter(
        (o) => matches(query, o.label) || matches(query, o.description)
      )
    : [...recentRuns, ...recentSessions];

  const allOptions: Option[] = [...filteredPages, ...filteredRecent];

  // Clamp activeIdx when list changes
  const clampedIdx = allOptions.length > 0
    ? Math.min(activeIdx, allOptions.length - 1)
    : 0;

  // Open/close keybinding
  useEffect(() => {
    function handler(e: globalThis.KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        setOpen((v) => !v);
      }
      if (e.key === 'Escape') setOpen(false);
    }
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, []);

  // Focus input when opened; reset state on close
  useEffect(() => {
    if (open) {
      setQuery('');
      setActiveIdx(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const selectOption = useCallback(
    (opt: Option) => {
      onNavigate(opt.kind === 'page' ? opt.id : opt.page);
      setOpen(false);
    },
    [onNavigate]
  );

  function handleKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIdx((i) => (i + 1) % Math.max(1, allOptions.length));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIdx((i) => (i - 1 + Math.max(1, allOptions.length)) % Math.max(1, allOptions.length));
    } else if (e.key === 'Enter' && allOptions[clampedIdx]) {
      e.preventDefault();
      selectOption(allOptions[clampedIdx]);
    }
  }

  if (!open) return null;

  const hasRecent = filteredRecent.length > 0;
  const hasPages  = filteredPages.length > 0;

  return (
    /* Backdrop */
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh] px-4"
      style={{ background: 'rgba(0,0,0,0.65)', backdropFilter: 'blur(2px)' }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) setOpen(false); }}
    >
      {/* Panel */}
      <div className="w-full max-w-lg rounded-xl bg-zinc-900 ring-1 ring-zinc-700 shadow-2xl shadow-black/60 overflow-hidden">
        {/* Search row */}
        <div className="flex items-center gap-3 px-4 py-3 border-b border-zinc-800">
          <Search size={16} className="shrink-0 text-zinc-500" />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => { setQuery(e.target.value); setActiveIdx(0); }}
            onKeyDown={handleKeyDown}
            placeholder="Go to page, search runs…"
            className="flex-1 bg-transparent text-sm text-zinc-100 placeholder-zinc-600 outline-none"
          />
          <kbd className="hidden sm:inline-flex items-center gap-0.5 rounded bg-zinc-800 px-1.5 py-0.5 text-[10px] font-mono text-zinc-500 ring-1 ring-zinc-700">
            esc
          </kbd>
        </div>

        {/* Results */}
        <div className="max-h-96 overflow-y-auto p-2 space-y-0.5">
          {allOptions.length === 0 && (
            <p className="py-8 text-center text-sm text-zinc-600">No results for &ldquo;{query}&rdquo;</p>
          )}

          {/* Navigation pages */}
          {hasPages && (
            <>
              {query === '' && (
                <p className="px-3 pt-1 pb-0.5 text-[10px] font-semibold uppercase tracking-widest text-zinc-600">
                  Navigation
                </p>
              )}
              {filteredPages.map((opt, i) => (
                <OptionRow
                  key={opt.id}
                  option={opt}
                  active={i === clampedIdx}
                  onSelect={() => selectOption(opt)}
                />
              ))}
            </>
          )}

          {/* Recent runs & sessions */}
          {hasRecent && (
            <>
              <p className="px-3 pt-2 pb-0.5 text-[10px] font-semibold uppercase tracking-widest text-zinc-600">
                Recent
              </p>
              {filteredRecent.map((opt, i) => (
                <OptionRow
                  key={`${opt.kind}-${opt.id}`}
                  option={opt}
                  active={filteredPages.length + i === clampedIdx}
                  onSelect={() => selectOption(opt)}
                />
              ))}
            </>
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center gap-3 px-4 py-2 border-t border-zinc-800 text-[10px] text-zinc-600 font-mono">
          <span>↑↓ navigate</span>
          <span>↵ select</span>
          <span>esc close</span>
          <span className="ml-auto">⌘K</span>
        </div>
      </div>
    </div>
  );
}
