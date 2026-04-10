/**
 * TenantSelector — compact breadcrumb-style scope picker in the TopBar.
 *
 * Displays: tenant / workspace / project
 * Click opens a popover form to edit all three values.
 * Changes are persisted to localStorage and trigger a full React Query
 * invalidation so every page re-fetches with the new scope.
 */

import { useState, useRef, useEffect } from 'react';
import { ChevronDown, X, Check, RotateCcw } from 'lucide-react';
import { useQueryClient } from '@tanstack/react-query';
import { clsx } from 'clsx';
import { useScope, DEFAULT_SCOPE, isDefaultScope, type ProjectScope } from '../hooks/useScope';

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Shorten a long ID for display in the compact trigger. */
function short(s: string): string {
  return s.length > 16 ? `${s.slice(0, 13)}…` : s;
}

// ── Scope popover ─────────────────────────────────────────────────────────────

interface ScopePopoverProps {
  current: ProjectScope;
  onApply: (s: ProjectScope) => void;
  onClose: () => void;
}

function ScopePopover({ current, onApply, onClose }: ScopePopoverProps) {
  const [draft, setDraft] = useState<ProjectScope>({ ...current });
  const firstRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    firstRef.current?.focus();
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onClose();
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); handleApply(); }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  function handleApply() {
    const trimmed: ProjectScope = {
      tenant_id:    draft.tenant_id.trim()    || DEFAULT_SCOPE.tenant_id,
      workspace_id: draft.workspace_id.trim() || DEFAULT_SCOPE.workspace_id,
      project_id:   draft.project_id.trim()   || DEFAULT_SCOPE.project_id,
    };
    onApply(trimmed);
  }

  function handleReset() {
    setDraft({ ...DEFAULT_SCOPE });
  }

  const isModified =
    draft.tenant_id    !== DEFAULT_SCOPE.tenant_id    ||
    draft.workspace_id !== DEFAULT_SCOPE.workspace_id ||
    draft.project_id   !== DEFAULT_SCOPE.project_id;

  return (
    <div
      className="absolute top-full right-0 mt-1 w-72 rounded-lg border border-gray-200 dark:border-zinc-800
                 bg-white dark:bg-zinc-900 shadow-xl shadow-black/20 z-50 overflow-hidden"
      role="dialog"
      aria-label="Scope selector"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2.5 border-b border-gray-100 dark:border-zinc-800">
        <p className="text-[12px] font-semibold text-gray-700 dark:text-zinc-300">Scope</p>
        <button
          onClick={onClose}
          aria-label="Close scope selector"
          className="p-0.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300
                     hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors"
        >
          <X size={13} />
        </button>
      </div>

      {/* Fields */}
      <div className="p-3 space-y-2.5">
        {(
          [
            { field: 'tenant_id',    label: 'Tenant',    placeholder: DEFAULT_SCOPE.tenant_id    },
            { field: 'workspace_id', label: 'Workspace', placeholder: DEFAULT_SCOPE.workspace_id },
            { field: 'project_id',   label: 'Project',   placeholder: DEFAULT_SCOPE.project_id   },
          ] as { field: keyof ProjectScope; label: string; placeholder: string }[]
        ).map(({ field, label, placeholder }, i) => (
          <div key={field}>
            <label className="block text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-1">
              {label}
            </label>
            <input
              ref={i === 0 ? firstRef : undefined}
              value={draft[field]}
              onChange={(e) => setDraft((p) => ({ ...p, [field]: e.target.value }))}
              placeholder={placeholder}
              spellCheck={false}
              className="w-full rounded border border-gray-200 dark:border-zinc-800
                         bg-gray-50 dark:bg-zinc-950 text-[12px] text-gray-900 dark:text-zinc-200
                         font-mono px-2.5 py-1.5 focus:outline-none
                         focus:border-indigo-500 transition-colors placeholder-gray-400 dark:placeholder-zinc-600"
            />
          </div>
        ))}
      </div>

      {/* Footer */}
      <div className="flex items-center gap-2 px-3 pb-3">
        {isModified && (
          <button
            onClick={handleReset}
            title="Reset to defaults"
            className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-zinc-600
                       hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
          >
            <RotateCcw size={11} /> Reset
          </button>
        )}
        <div className="ml-auto flex items-center gap-2">
          <button
            onClick={onClose}
            className="px-3 py-1.5 rounded text-[12px] text-gray-500 dark:text-zinc-500
                       hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={handleApply}
            className="flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium
                       bg-indigo-600 hover:bg-indigo-500 text-white transition-colors"
          >
            <Check size={11} /> Apply
          </button>
        </div>
      </div>

      {/* Hint */}
      <p className="px-3 pb-2.5 text-[10px] text-gray-400 dark:text-zinc-600">
        ⌘↵ to apply · Esc to cancel · Empty fields use defaults
      </p>
    </div>
  );
}

// ── TenantSelector ────────────────────────────────────────────────────────────

export function TenantSelector() {
  const [scope, setScope] = useScope();
  const [open, setOpen]   = useState(false);
  const containerRef      = useRef<HTMLDivElement>(null);
  const qc                = useQueryClient();

  // Close on outside click.
  useEffect(() => {
    if (!open) return;
    function onPointer(e: PointerEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener('pointerdown', onPointer);
    return () => document.removeEventListener('pointerdown', onPointer);
  }, [open]);

  function handleApply(next: ProjectScope) {
    setScope(next);
    setOpen(false);
    // Invalidate all queries so they re-fetch with the new scope.
    void qc.invalidateQueries();
  }

  const isDefault = isDefaultScope(scope);

  return (
    <div ref={containerRef} className="relative">
      <button
        onClick={() => setOpen((v) => !v)}
        title="Change tenant / workspace / project scope"
        aria-label="Change scope"
        aria-expanded={open}
        className={clsx(
          'flex items-center gap-1 rounded px-2 py-1 text-[11px] font-mono transition-colors',
          'border max-w-[220px]',
          open
            ? 'bg-indigo-50 dark:bg-indigo-950/40 border-indigo-300 dark:border-indigo-700/60 text-indigo-700 dark:text-indigo-300'
            : isDefault
              ? 'border-gray-200 dark:border-zinc-800 text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:border-gray-300 dark:hover:border-zinc-700'
              : 'border-indigo-200 dark:border-indigo-800/50 text-indigo-600 dark:text-indigo-400 bg-indigo-50/50 dark:bg-indigo-950/20',
        )}
      >
        {/* Scope breadcrumb: tenant / workspace / project */}
        <span className="truncate">
          <span className="text-gray-500 dark:text-zinc-500">{short(scope.tenant_id)}</span>
          <span className="mx-0.5 text-gray-300 dark:text-zinc-600">/</span>
          <span className="text-gray-500 dark:text-zinc-500">{short(scope.workspace_id)}</span>
          <span className="mx-0.5 text-gray-300 dark:text-zinc-600">/</span>
          <span className={isDefault ? 'text-gray-400 dark:text-zinc-600' : 'text-indigo-600 dark:text-indigo-400 font-medium'}>
            {short(scope.project_id)}
          </span>
        </span>
        <ChevronDown
          size={11}
          className={clsx('shrink-0 transition-transform', open && 'rotate-180')}
        />
      </button>

      {open && (
        <ScopePopover
          current={scope}
          onApply={handleApply}
          onClose={() => setOpen(false)}
        />
      )}
    </div>
  );
}
