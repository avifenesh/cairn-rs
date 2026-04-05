/**
 * CommandPalette — Cmd+K / Ctrl+K quick-navigation overlay.
 *
 * Global keyboard shortcuts (registered here):
 *  - Cmd/Ctrl+K  — toggle palette
 *  - Cmd/Ctrl+1..9 — navigate to the first 9 pages directly
 *  - ?  — show keyboard shortcut help overlay (when not in a text input)
 *  - Escape — close palette / help overlay
 */

import {
  useState,
  useEffect,
  useRef,
  useCallback,
  type KeyboardEvent,
} from 'react';
import { GlobalSearch } from './GlobalSearch';
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
  ListChecks,
  BookOpen,
  FileText,
  Network,
  Waves,
  Plus,
  Keyboard,
  X,
} from 'lucide-react';
import { clsx } from 'clsx';
import { useQuery } from '@tanstack/react-query';
import { defaultApi } from '../lib/api';
import type { NavPage } from './Sidebar';

// ── Platform ──────────────────────────────────────────────────────────────────

const IS_MAC = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
const MOD = IS_MAC ? '⌘' : 'Ctrl';

// ── Option types ──────────────────────────────────────────────────────────────

interface NavOption {
  kind: 'page';
  id: NavPage;
  label: string;
  description: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  shortcut?: string[];
}

interface ActionOption {
  kind: 'action';
  id: string;
  label: string;
  description: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  shortcut?: string[];
  action: () => void;
}

interface QuickJumpOption {
  kind: 'run' | 'session';
  id: string;
  label: string;
  description: string;
  page: NavPage;
}

type Option = NavOption | ActionOption | QuickJumpOption;

// ── Navigation pages (all 11) ─────────────────────────────────────────────────

const NAV_OPTIONS: NavOption[] = [
  { kind: 'page', id: 'dashboard',  label: 'Dashboard',  description: 'Overview and live metrics',          icon: LayoutDashboard, shortcut: [MOD, '1'] },
  { kind: 'page', id: 'sessions',   label: 'Sessions',    description: 'Conversation sessions',              icon: MonitorPlay,     shortcut: [MOD, '2'] },
  { kind: 'page', id: 'runs',       label: 'Runs',        description: 'Active and historical runs',         icon: Play,            shortcut: [MOD, '3'] },
  { kind: 'page', id: 'tasks',      label: 'Tasks',       description: 'Task queue and worker activity',     icon: ListChecks,      shortcut: [MOD, '4'] },
  { kind: 'page', id: 'approvals',  label: 'Approvals',   description: 'Pending operator approvals',         icon: CheckSquare,     shortcut: [MOD, '5'] },
  { kind: 'page', id: 'traces',     label: 'Traces',      description: 'LLM call traces and latency',        icon: Waves,           shortcut: [MOD, '6'] },
  { kind: 'page', id: 'memory',     label: 'Memory',      description: 'Knowledge base and search',          icon: Database,        shortcut: [MOD, '7'] },
  { kind: 'page', id: 'costs',      label: 'Costs',           description: 'Token spend and provider costs',     icon: Coins,    shortcut: [MOD, '8'] },
  { kind: 'page', id: 'providers',  label: 'Providers',       description: 'LLM provider health',                icon: Zap,      shortcut: [MOD, '9'] },
  { kind: 'page', id: 'prompts',    label: 'Prompts',         description: 'Prompt assets, versions, releases',  icon: FileText  },
  { kind: 'page', id: 'graph',      label: 'Knowledge Graph', description: 'Node/edge schema, RFC 004 graph',    icon: Network   },
  { kind: 'page', id: 'api-docs',   label: 'API Reference',   description: 'All endpoints with Try it',          icon: BookOpen  },
  { kind: 'page', id: 'playground', label: 'Playground',      description: 'Interactive LLM prompt testing',     icon: Terminal  },
  { kind: 'page', id: 'settings',   label: 'Settings',        description: 'Deployment configuration',           icon: Settings  },
];

// ── Shortcut help data ────────────────────────────────────────────────────────

const SHORTCUT_SECTIONS = [
  {
    title: 'Navigation',
    items: [
      { keys: [MOD, 'K'],  label: 'Open command palette' },
      { keys: [MOD, 'F'],  label: 'Global entity search'  },
      { keys: [MOD, '1'],  label: 'Dashboard'            },
      { keys: [MOD, '2'],  label: 'Sessions'             },
      { keys: [MOD, '3'],  label: 'Runs'                 },
      { keys: [MOD, '4'],  label: 'Tasks'                },
      { keys: [MOD, '5'],  label: 'Approvals'            },
      { keys: [MOD, '6'],  label: 'Traces'               },
      { keys: [MOD, '7'],  label: 'Memory'               },
      { keys: [MOD, '8'],  label: 'Costs'                },
      { keys: [MOD, '9'],  label: 'Providers'            },
    ],
  },
  {
    title: 'General',
    items: [
      { keys: ['?'],       label: 'Keyboard shortcuts'   },
      { keys: ['↑', '↓'],  label: 'Move through results' },
      { keys: ['↵'],       label: 'Confirm selection'    },
      { keys: ['Esc'],     label: 'Close palette / modal'},
    ],
  },
];

// ── Fuzzy filter ──────────────────────────────────────────────────────────────

function matches(query: string, text: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  const t = text.toLowerCase();
  if (t.includes(q)) return true;
  let ti = 0;
  for (const ch of q) {
    while (ti < t.length && t[ti] !== ch) ti++;
    if (ti >= t.length) return false;
    ti++;
  }
  return true;
}

// ── Kbd atom ──────────────────────────────────────────────────────────────────

function Kbd({ children }: { children: React.ReactNode }) {
  return (
    <kbd className="inline-flex items-center justify-center min-w-[1.375rem] h-5 rounded bg-zinc-800 px-1 text-[10px] font-mono text-zinc-400 ring-1 ring-inset ring-zinc-700">
      {children}
    </kbd>
  );
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

  const isPage   = option.kind === 'page';
  const isAction = option.kind === 'action';
  const isRun    = option.kind === 'run';

  const Icon = isPage || isAction
    ? option.icon
    : isRun ? Play : Clock;

  const shortcut: string[] | undefined =
    (isPage || isAction) ? option.shortcut : undefined;

  const iconBg = isPage
    ? 'bg-zinc-800 text-zinc-300'
    : isAction
      ? 'bg-indigo-950 text-indigo-400'
      : isRun
        ? 'bg-blue-950 text-blue-400'
        : 'bg-sky-950 text-sky-400';

  return (
    <button
      ref={ref}
      onClick={onSelect}
      className={clsx(
        'w-full flex items-center gap-3 px-3 py-2 text-left rounded-md transition-colors',
        active
          ? 'bg-zinc-800 ring-1 ring-inset ring-zinc-700'
          : 'hover:bg-zinc-800/60',
      )}
    >
      {/* Icon */}
      <div className={clsx(
        'flex h-6 w-6 shrink-0 items-center justify-center rounded',
        iconBg,
      )}>
        <Icon size={13} />
      </div>

      {/* Label + description */}
      <div className="flex-1 min-w-0">
        <p className="text-[13px] font-medium text-zinc-100 truncate">{option.label}</p>
        <p className="text-[11px] text-zinc-500 truncate">{option.description}</p>
      </div>

      {/* Shortcut hint */}
      {shortcut && shortcut.length > 0 ? (
        <span className="shrink-0 flex items-center gap-0.5">
          {shortcut.map((k, i) => <Kbd key={i}>{k}</Kbd>)}
        </span>
      ) : active ? (
        <ArrowRight size={13} className="shrink-0 text-zinc-500" />
      ) : null}
    </button>
  );
}

// ── Section heading ───────────────────────────────────────────────────────────

function SectionHeading({ children }: { children: React.ReactNode }) {
  return (
    <p className="px-3 pt-2 pb-1 text-[10px] font-semibold uppercase tracking-widest text-zinc-600 select-none">
      {children}
    </p>
  );
}

// ── Shortcuts help modal ──────────────────────────────────────────────────────

function ShortcutsHelp({ onClose }: { onClose: () => void }) {
  useEffect(() => {
    function handler(e: globalThis.KeyboardEvent) {
      if (e.key === 'Escape' || e.key === '?') { e.preventDefault(); onClose(); }
    }
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center px-4"
      style={{ background: 'rgba(0,0,0,0.70)', backdropFilter: 'blur(3px)' }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}
      aria-hidden="false"
    >
      <div
        className="w-full max-w-md rounded-xl bg-zinc-900 ring-1 ring-zinc-800 shadow-2xl shadow-black/60 overflow-hidden"
        role="dialog"
        aria-modal="true"
        aria-label="Keyboard shortcuts"
      >
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
          <div className="flex items-center gap-2">
            <Keyboard size={14} className="text-zinc-500" />
            <span className="text-[13px] font-medium text-zinc-200">Keyboard Shortcuts</span>
          </div>
          <button
            onClick={onClose}
            aria-label="Close shortcuts"
            className="p-1 rounded text-zinc-600 hover:text-zinc-300 hover:bg-zinc-800 transition-colors"
          >
            <X size={14} />
          </button>
        </div>

        {/* Shortcut grid */}
        <div className="p-4 space-y-4">
          {SHORTCUT_SECTIONS.map((section) => (
            <div key={section.title}>
              <p className="text-[10px] font-semibold uppercase tracking-widest text-zinc-600 mb-2">
                {section.title}
              </p>
              <div className="space-y-1">
                {section.items.map(({ keys, label }) => (
                  <div key={label} className="flex items-center justify-between py-1">
                    <span className="text-[13px] text-zinc-400">{label}</span>
                    <span className="flex items-center gap-0.5">
                      {keys.map((k, i) => <Kbd key={i}>{k}</Kbd>)}
                    </span>
                  </div>
                ))}
              </div>
            </div>
          ))}
        </div>

        {/* Footer */}
        <div className="px-4 py-2.5 border-t border-zinc-800 text-center">
          <span className="text-[11px] text-zinc-600">
            Press <Kbd>?</Kbd> or <Kbd>Esc</Kbd> to close
          </span>
        </div>
      </div>
    </div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

interface CommandPaletteProps {
  onNavigate: (page: NavPage) => void;
}

export function CommandPalette({ onNavigate }: CommandPaletteProps) {
  const [open,        setOpen]        = useState(false);
  const [showHelp,    setShowHelp]    = useState(false);
  const [searchOpen,  setSearchOpen]  = useState(false);
  const [searchQuery, setSearchQuery] = useState('');
  const [query,       setQuery]       = useState('');
  const [activeIdx, setActiveIdx] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  // Action commands (defined here to close over state setters).
  const ACTION_OPTIONS: ActionOption[] = [
    {
      kind:        'action',
      id:          'new-session',
      label:       'New Session',
      description: 'Create a new agent session and open Sessions',
      icon:        Plus,
      action:      () => { void defaultApi.createSession({}); onNavigate('sessions'); },
    },
    {
      kind:        'action',
      id:          'new-run',
      label:       'New Run',
      description: 'Start a new agent run and open Runs',
      icon:        Play,
      action:      () => { void defaultApi.createRun({}); onNavigate('runs'); },
    },
    {
      kind:        'action',
      id:          'show-shortcuts',
      label:       'Keyboard Shortcuts',
      description: 'Show all keyboard shortcut bindings',
      icon:        Keyboard,
      shortcut:    ['?'],
      action:      () => { setOpen(false); setShowHelp(true); },
    },
    // Dynamic search action — only visible when there's a query.
    ...(query.trim().length >= 2 ? [{
      kind:        'action' as const,
      id:          'search-entities',
      label:       `Search "${query.trim()}" across all entities`,
      description: 'Runs, sessions, tasks, approvals, traces, prompts',
      icon:        Search,
      action:      () => { setSearchQuery(query); setOpen(false); setSearchOpen(true); },
    }] : []),
  ];

  // Fetch recent runs + sessions for quick-jump (non-blocking).
  const { data: runs     = [] } = useQuery({ queryKey: ['runs'],     queryFn: () => defaultApi.getRuns({ limit: 5 }),     enabled: open, staleTime: 30_000 });
  const { data: sessions = [] } = useQuery({ queryKey: ['sessions'], queryFn: () => defaultApi.getSessions({ limit: 5 }), enabled: open, staleTime: 30_000 });

  const recentRuns: QuickJumpOption[] = runs.slice(0, 5).map((r) => ({
    kind:        'run' as const,
    id:          r.run_id,
    label:       r.run_id.length > 24 ? `${r.run_id.slice(0, 20)}…` : r.run_id,
    description: `Run · ${r.state}`,
    page:        'runs' as NavPage,
  }));
  const recentSessions: QuickJumpOption[] = sessions.slice(0, 3).map((s) => ({
    kind:        'session' as const,
    id:          s.session_id,
    label:       s.session_id.length > 24 ? `${s.session_id.slice(0, 20)}…` : s.session_id,
    description: `Session · ${s.state}`,
    page:        'sessions' as NavPage,
  }));

  // Filter all option groups.
  const filteredPages = NAV_OPTIONS.filter(
    (o) => matches(query, o.label) || matches(query, o.description),
  );
  const filteredActions = ACTION_OPTIONS.filter(
    (o) => matches(query, o.label) || matches(query, o.description),
  );
  const filteredRecent = query
    ? [...recentRuns, ...recentSessions].filter(
        (o) => matches(query, o.label) || matches(query, o.description),
      )
    : [...recentRuns, ...recentSessions];

  const allOptions: Option[] = [...filteredPages, ...filteredActions, ...filteredRecent];

  const clampedIdx = allOptions.length > 0
    ? Math.min(activeIdx, allOptions.length - 1)
    : 0;

  // Global keyboard shortcuts.
  useEffect(() => {
    function handler(e: globalThis.KeyboardEvent) {
      const mod = e.metaKey || e.ctrlKey;

      // Cmd+K — toggle palette.
      if (mod && e.key === 'k') {
        e.preventDefault();
        setOpen((v) => !v);
        return;
      }

      // Cmd+F — open global entity search directly.
      if (mod && e.key === 'f' && !open) {
        e.preventDefault();
        setSearchQuery('');
        setSearchOpen((v) => !v);
        return;
      }

      // Cmd+1..9 — navigate directly (only when palette/help not open).
      if (mod && !open && !showHelp && e.key >= '1' && e.key <= '9') {
        const page = NAV_OPTIONS[parseInt(e.key, 10) - 1];
        if (page) {
          e.preventDefault();
          onNavigate(page.id);
        }
        return;
      }

      // Escape — close whatever is open.
      if (e.key === 'Escape') {
        setOpen(false);
        setShowHelp(false);
        setSearchOpen(false);
        return;
      }

      // ? — toggle shortcuts help (skip when focus is inside a text field).
      if (e.key === '?' && !mod) {
        const tag = (e.target as HTMLElement).tagName;
        if (tag !== 'INPUT' && tag !== 'TEXTAREA' && tag !== 'SELECT') {
          e.preventDefault();
          setShowHelp((v) => !v);
        }
      }
    }
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [open, showHelp, onNavigate]);

  // Focus input when opened; reset on close.
  useEffect(() => {
    if (open) {
      setQuery('');
      setActiveIdx(0);
      requestAnimationFrame(() => inputRef.current?.focus());
    }
  }, [open]);

  const selectOption = useCallback(
    (opt: Option) => {
      if (opt.kind === 'action') {
        opt.action();
      } else {
        onNavigate(opt.kind === 'page' ? opt.id : opt.page);
      }
      setOpen(false);
    },
    [onNavigate],
  );

  function handleKeyDown(e: KeyboardEvent<HTMLInputElement>) {
    const len = Math.max(1, allOptions.length);
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setActiveIdx((i) => (i + 1) % len);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setActiveIdx((i) => (i - 1 + len) % len);
    } else if (e.key === 'Enter' && allOptions[clampedIdx]) {
      e.preventDefault();
      selectOption(allOptions[clampedIdx]);
    }
  }

  const hasPages   = filteredPages.length > 0;
  const hasActions = filteredActions.length > 0;
  const hasRecent  = filteredRecent.length > 0;

  return (
    <>
      {/* Shortcuts help overlay */}
      {showHelp && <ShortcutsHelp onClose={() => setShowHelp(false)} />}

      {/* Global entity search */}
      {searchOpen && (
        <GlobalSearch
          initialQuery={searchQuery}
          onClose={() => setSearchOpen(false)}
          onBack={() => { setSearchOpen(false); setOpen(true); }}
        />
      )}

      {/* Command palette */}
      {open && (
        <div
          className="fixed inset-0 z-50 flex items-start justify-center pt-[14vh] px-4"
          style={{ background: 'rgba(0,0,0,0.65)', backdropFilter: 'blur(2px)' }}
          onMouseDown={(e) => { if (e.target === e.currentTarget) setOpen(false); }}
        >
          <div
            className="w-full max-w-lg rounded-xl bg-zinc-900 ring-1 ring-zinc-800 shadow-2xl shadow-black/60 overflow-hidden"
            role="dialog"
            aria-modal="true"
            aria-label="Command palette"
          >
            {/* Search row */}
            <div className="flex items-center gap-3 px-3.5 py-3 border-b border-zinc-800">
              <Search size={15} className="shrink-0 text-zinc-500" />
              <input
                ref={inputRef}
                value={query}
                onChange={(e) => { setQuery(e.target.value); setActiveIdx(0); }}
                onKeyDown={handleKeyDown}
                placeholder="Go to page or run a command…"
                aria-label="Command search"
                aria-autocomplete="list"
                aria-expanded={allOptions.length > 0}
                role="combobox"
                className="flex-1 bg-transparent text-[13px] text-zinc-100 placeholder-zinc-600 outline-none"
              />
              {query ? (
                <button
                  onClick={() => { setQuery(''); setActiveIdx(0); inputRef.current?.focus(); }}
                  aria-label="Clear search"
                  className="shrink-0 p-0.5 rounded text-zinc-600 hover:text-zinc-400 transition-colors"
                >
                  <X size={13} />
                </button>
              ) : (
                <Kbd>esc</Kbd>
              )}
            </div>

            {/* Results */}
            <div className="max-h-[26rem] overflow-y-auto p-1.5">
              {allOptions.length === 0 && (
                <p className="py-8 text-center text-[13px] text-zinc-600">
                  No results for &ldquo;{query}&rdquo;
                </p>
              )}

              {/* Pages */}
              {hasPages && (
                <>
                  {query === '' && <SectionHeading>Navigation</SectionHeading>}
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

              {/* Actions */}
              {hasActions && (
                <>
                  <SectionHeading>Actions</SectionHeading>
                  {filteredActions.map((opt, i) => (
                    <OptionRow
                      key={opt.id}
                      option={opt}
                      active={filteredPages.length + i === clampedIdx}
                      onSelect={() => selectOption(opt)}
                    />
                  ))}
                </>
              )}

              {/* Recent quick-jump */}
              {hasRecent && (
                <>
                  <SectionHeading>Recent</SectionHeading>
                  {filteredRecent.map((opt, i) => (
                    <OptionRow
                      key={`${opt.kind}-${opt.id}`}
                      option={opt}
                      active={filteredPages.length + filteredActions.length + i === clampedIdx}
                      onSelect={() => selectOption(opt)}
                    />
                  ))}
                </>
              )}
            </div>

            {/* Footer */}
            <div className="flex items-center gap-3 px-4 py-2 border-t border-zinc-800 text-[10px] text-zinc-600 font-mono select-none">
              <span className="flex items-center gap-1"><Kbd>↑</Kbd><Kbd>↓</Kbd> navigate</span>
              <span className="flex items-center gap-1"><Kbd>↵</Kbd> select</span>
              <span className="flex items-center gap-1"><Kbd>?</Kbd> shortcuts</span>
              <span className="ml-auto flex items-center gap-0.5"><Kbd>{MOD}</Kbd><Kbd>K</Kbd></span>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
