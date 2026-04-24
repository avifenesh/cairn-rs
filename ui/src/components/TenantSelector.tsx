/**
 * TenantSelector — cascading scope picker in the TopBar.
 *
 * Replaces the previous manual-text inputs (operator had to TYPE
 * tenant_id/workspace_id/project_id) with three discoverable dropdowns
 * populated from the backend. See fix quote:
 *
 *   "i dont see anything in tasks, runs, sessions, orchestration
 *   approvals, memory, prompts, agents, graphs, providers, tests still
 *   fails"
 *
 * Root cause: DEFAULT_SCOPE is `default_tenant/default_workspace/default_project`
 * but real data lives under `acme/prod/minecraft`. Operators had no
 * affordance to discover other scopes.
 *
 * The trigger shows the current scope as a breadcrumb and is always
 * visible in the TopBar so operators always know which scope they're
 * viewing.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import { ChevronDown, X, Check, RotateCcw } from 'lucide-react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { clsx } from 'clsx';
import {
  useScope,
  DEFAULT_SCOPE,
  SCOPE_NEEDS_PICK_EVENT,
  isDefaultScope,
  type ProjectScope,
} from '../hooks/useScope';
import { defaultApi, ApiError } from '../lib/api';
import type {
  TenantRecord,
  WorkspaceRecord,
  ProjectRecord,
} from '../lib/types';

// ── Helpers ───────────────────────────────────────────────────────────────────

function short(s: string, max = 16): string {
  return s.length > max ? `${s.slice(0, max - 3)}…` : s;
}

function slugify(s: string): string {
  return s.trim().toLowerCase().replace(/[^a-z0-9]+/g, '_').replace(/^_+|_+$/g, '').slice(0, 64);
}

// ── Create-new modal (shared for all three levels) ────────────────────────────

interface CreateModalProps {
  kind: 'tenant' | 'workspace' | 'project';
  onCancel: () => void;
  onCreate: (id: string, name: string) => Promise<void>;
}

function CreateModal({ kind, onCancel, onCreate }: CreateModalProps) {
  const [name, setName] = useState('');
  const [id, setId]     = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const ref = useRef<HTMLInputElement>(null);

  useEffect(() => { ref.current?.focus(); }, []);
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') onCancel();
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onCancel]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!name.trim()) return;
    setBusy(true); setError(null);
    try {
      await onCreate(id.trim() || slugify(name), name.trim());
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
      setBusy(false);
    }
  }

  return (
    <div
      data-testid={`scope-create-${kind}-modal`}
      className="fixed inset-0 z-[60] flex items-center justify-center bg-black/30"
      onClick={(e) => { if (e.target === e.currentTarget) onCancel(); }}
    >
      <form
        onSubmit={submit}
        className="w-full max-w-sm rounded-lg border border-gray-200 dark:border-zinc-800
                   bg-white dark:bg-zinc-900 shadow-2xl overflow-hidden"
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-100 dark:border-zinc-800">
          <h3 className="text-[13px] font-semibold text-gray-800 dark:text-zinc-200">
            Create {kind}
          </h3>
          <button
            type="button"
            onClick={onCancel}
            aria-label="Close"
            className="p-0.5 text-gray-400 hover:text-gray-700 dark:hover:text-zinc-300"
          >
            <X size={14} />
          </button>
        </div>
        <div className="p-4 space-y-3">
          <div>
            <label className="block text-[10px] font-medium text-gray-400 dark:text-zinc-600
                              uppercase tracking-wider mb-1">Name</label>
            <input
              ref={ref}
              value={name}
              onChange={(e) => setName(e.target.value)}
              data-testid={`scope-create-${kind}-name`}
              placeholder={kind === 'tenant' ? 'Acme Corp' : kind === 'workspace' ? 'Production' : 'Minecraft'}
              className="w-full rounded border border-gray-200 dark:border-zinc-800
                         bg-gray-50 dark:bg-zinc-950 text-[13px] text-gray-900 dark:text-zinc-200
                         px-2.5 py-1.5 focus:outline-none focus:border-indigo-500"
            />
          </div>
          <div>
            <label className="block text-[10px] font-medium text-gray-400 dark:text-zinc-600
                              uppercase tracking-wider mb-1">ID (optional)</label>
            <input
              value={id}
              onChange={(e) => setId(e.target.value)}
              data-testid={`scope-create-${kind}-id`}
              placeholder={name ? slugify(name) : 'auto'}
              className="w-full rounded border border-gray-200 dark:border-zinc-800
                         bg-gray-50 dark:bg-zinc-950 text-[12px] text-gray-900 dark:text-zinc-200
                         font-mono px-2.5 py-1.5 focus:outline-none focus:border-indigo-500"
            />
          </div>
          {error && (
            <p className="text-[11px] text-red-500 dark:text-red-400">{error}</p>
          )}
        </div>
        <div className="flex justify-end gap-2 px-4 pb-4">
          <button
            type="button"
            onClick={onCancel}
            className="px-3 py-1.5 rounded text-[12px] text-gray-500 dark:text-zinc-500
                       hover:text-gray-700 dark:hover:text-zinc-300"
          >
            Cancel
          </button>
          <button
            type="submit"
            disabled={busy || !name.trim()}
            data-testid={`scope-create-${kind}-submit`}
            className="flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium
                       bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 text-white transition-colors"
          >
            <Check size={11} /> {busy ? 'Creating…' : 'Create'}
          </button>
        </div>
      </form>
    </div>
  );
}

// ── Dropdown primitive ────────────────────────────────────────────────────────

interface DropdownOption {
  value: string;
  label: string;
  hint?: string;
}

interface ScopeDropdownProps {
  label: string;
  placeholder: string;
  value: string | null;
  options: DropdownOption[];
  disabled?: boolean;
  loading?: boolean;
  onChange: (v: string) => void;
  onCreateNew: () => void;
  testId: string;
}

function ScopeDropdown(props: ScopeDropdownProps) {
  const selected = props.options.find((o) => o.value === props.value);

  return (
    <div>
      <label className="block text-[10px] font-medium text-gray-400 dark:text-zinc-600
                        uppercase tracking-wider mb-1">
        {props.label}
      </label>
      <div className="relative">
        <select
          data-testid={props.testId}
          disabled={props.disabled || props.loading}
          value={props.value ?? ''}
          onChange={(e) => {
            if (e.target.value === '__create__') props.onCreateNew();
            else if (e.target.value) props.onChange(e.target.value);
          }}
          className={clsx(
            'w-full appearance-none rounded border bg-gray-50 dark:bg-zinc-950',
            'text-[12px] font-mono px-2.5 py-1.5 pr-7',
            'focus:outline-none focus:border-indigo-500 transition-colors',
            props.disabled || props.loading
              ? 'border-gray-100 dark:border-zinc-900 text-gray-300 dark:text-zinc-700 cursor-not-allowed'
              : 'border-gray-200 dark:border-zinc-800 text-gray-900 dark:text-zinc-200',
          )}
        >
          <option value="" disabled>
            {props.loading
              ? 'Loading…'
              : props.disabled
                ? props.placeholder
                : `Select ${props.label.toLowerCase()}…`}
          </option>
          {props.options.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}{o.hint ? ` — ${o.hint}` : ''}
            </option>
          ))}
          {!props.disabled && !props.loading && (
            <option value="__create__">+ Create new {props.label.toLowerCase()}…</option>
          )}
        </select>
        <ChevronDown
          size={12}
          className="absolute right-2 top-1/2 -translate-y-1/2 pointer-events-none
                     text-gray-400 dark:text-zinc-600"
        />
      </div>
      {selected?.hint && (
        <p className="mt-1 text-[10px] font-mono text-gray-400 dark:text-zinc-600">
          {selected.hint}
        </p>
      )}
    </div>
  );
}

// ── Scope popover with cascading dropdowns ────────────────────────────────────

interface ScopePopoverProps {
  current: ProjectScope;
  onApply: (s: ProjectScope) => void;
  onClose: () => void;
}

function ScopePopover({ current, onApply, onClose }: ScopePopoverProps) {
  const qc = useQueryClient();
  const [tenantId,    setTenantId]    = useState<string | null>(
    isDefaultScope(current) ? null : current.tenant_id,
  );
  const [workspaceId, setWorkspaceId] = useState<string | null>(
    isDefaultScope(current) ? null : current.workspace_id,
  );
  const [projectId,   setProjectId]   = useState<string | null>(
    isDefaultScope(current) ? null : current.project_id,
  );

  const [createFor, setCreateFor] = useState<'tenant' | 'workspace' | 'project' | null>(null);

  // Close popover on outside-key Escape.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape' && !createFor) onClose();
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [onClose, createFor]);

  // ── Queries ───────────────────────────────────────────────────────────────
  const tenantsQ = useQuery({
    queryKey: ['scope-picker', 'tenants'],
    queryFn:  () => defaultApi.listTenants(),
    staleTime: 30_000,
  });
  const workspacesQ = useQuery({
    queryKey: ['scope-picker', 'ws', tenantId],
    queryFn:  () => defaultApi.getWorkspaces(tenantId!),
    enabled:  !!tenantId,
    staleTime: 30_000,
  });
  const projectsQ = useQuery({
    queryKey: ['scope-picker', 'proj', workspaceId],
    queryFn:  () => defaultApi.listProjects(workspaceId!),
    enabled:  !!workspaceId,
    staleTime: 30_000,
  });

  // Auto-select single-option scopes to reduce friction.
  useEffect(() => {
    if (!tenantId && tenantsQ.data && tenantsQ.data.length === 1) {
      setTenantId(tenantsQ.data[0].tenant_id);
    }
  }, [tenantsQ.data, tenantId]);
  useEffect(() => {
    if (!workspaceId && workspacesQ.data && workspacesQ.data.length === 1) {
      setWorkspaceId(workspacesQ.data[0].workspace_id);
    }
  }, [workspacesQ.data, workspaceId]);
  useEffect(() => {
    if (!projectId && projectsQ.data && projectsQ.data.length === 1) {
      setProjectId(projectsQ.data[0].project_id);
    }
  }, [projectsQ.data, projectId]);

  const tenantOpts: DropdownOption[] = useMemo(
    () => (tenantsQ.data ?? []).map((t: TenantRecord) => ({
      value: t.tenant_id, label: t.name || t.tenant_id, hint: t.tenant_id,
    })),
    [tenantsQ.data],
  );
  const wsOpts: DropdownOption[] = useMemo(
    () => (workspacesQ.data ?? []).map((w: WorkspaceRecord) => ({
      value: w.workspace_id, label: w.name || w.workspace_id, hint: w.workspace_id,
    })),
    [workspacesQ.data],
  );
  const projOpts: DropdownOption[] = useMemo(
    () => (projectsQ.data ?? []).map((p: ProjectRecord) => ({
      value: p.project_id, label: p.name || p.project_id, hint: p.project_id,
    })),
    [projectsQ.data],
  );

  // ── Mutations via Create modal callbacks ──────────────────────────────────
  async function createTenant(id: string, name: string) {
    await defaultApi.createTenant({ tenant_id: id, name });
    await qc.invalidateQueries({ queryKey: ['scope-picker', 'tenants'] });
    setTenantId(id);
    setWorkspaceId(null);
    setProjectId(null);
    setCreateFor(null);
  }
  async function createWorkspace(id: string, name: string) {
    if (!tenantId) return;
    await defaultApi.createWorkspace(tenantId, { workspace_id: id, name });
    await qc.invalidateQueries({ queryKey: ['scope-picker', 'ws', tenantId] });
    setWorkspaceId(id);
    setProjectId(null);
    setCreateFor(null);
  }
  async function createProject(id: string, name: string) {
    if (!workspaceId) return;
    await defaultApi.createProject(workspaceId, { project_id: id, name });
    await qc.invalidateQueries({ queryKey: ['scope-picker', 'proj', workspaceId] });
    setProjectId(id);
    setCreateFor(null);
  }

  const canApply = !!tenantId && !!workspaceId && !!projectId;

  function apply() {
    if (!canApply) return;
    onApply({ tenant_id: tenantId!, workspace_id: workspaceId!, project_id: projectId! });
  }

  function resetToDefault() {
    setTenantId(null);
    setWorkspaceId(null);
    setProjectId(null);
    onApply({ ...DEFAULT_SCOPE });
  }

  return (
    <>
      <div
        className="absolute top-full right-0 mt-1 w-80 rounded-lg border border-gray-200 dark:border-zinc-800
                   bg-white dark:bg-zinc-900 shadow-xl shadow-black/20 z-50 overflow-hidden"
        role="dialog"
        aria-label="Scope selector"
        data-testid="scope-popover"
      >
        <div className="flex items-center justify-between px-3 py-2.5 border-b border-gray-100 dark:border-zinc-800">
          <p className="text-[12px] font-semibold text-gray-700 dark:text-zinc-300">Switch scope</p>
          <button
            onClick={onClose}
            aria-label="Close scope selector"
            className="p-0.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300
                       hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors"
          >
            <X size={13} />
          </button>
        </div>

        <div className="p-3 space-y-3">
          <ScopeDropdown
            label="Tenant"
            placeholder="No tenants"
            value={tenantId}
            options={tenantOpts}
            loading={tenantsQ.isLoading}
            onChange={(v) => { setTenantId(v); setWorkspaceId(null); setProjectId(null); }}
            onCreateNew={() => setCreateFor('tenant')}
            testId="scope-tenant-select"
          />
          <ScopeDropdown
            label="Workspace"
            placeholder="Select tenant first"
            value={workspaceId}
            options={wsOpts}
            disabled={!tenantId}
            loading={!!tenantId && workspacesQ.isLoading}
            onChange={(v) => { setWorkspaceId(v); setProjectId(null); }}
            onCreateNew={() => setCreateFor('workspace')}
            testId="scope-workspace-select"
          />
          <ScopeDropdown
            label="Project"
            placeholder="Select workspace first"
            value={projectId}
            options={projOpts}
            disabled={!workspaceId}
            loading={!!workspaceId && projectsQ.isLoading}
            onChange={(v) => setProjectId(v)}
            onCreateNew={() => setCreateFor('project')}
            testId="scope-project-select"
          />
        </div>

        <div className="flex items-center gap-2 px-3 pb-3">
          <button
            onClick={resetToDefault}
            title="Reset to default scope"
            className="flex items-center gap-1 text-[11px] text-gray-400 dark:text-zinc-600
                       hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
          >
            <RotateCcw size={11} /> Default
          </button>
          <div className="ml-auto flex items-center gap-2">
            <button
              onClick={onClose}
              className="px-3 py-1.5 rounded text-[12px] text-gray-500 dark:text-zinc-500
                         hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
            >
              Cancel
            </button>
            <button
              onClick={apply}
              disabled={!canApply}
              data-testid="scope-apply"
              className="flex items-center gap-1.5 rounded px-3 py-1.5 text-[12px] font-medium
                         bg-indigo-600 hover:bg-indigo-500 disabled:opacity-50 disabled:cursor-not-allowed
                         text-white transition-colors"
            >
              <Check size={11} /> Apply
            </button>
          </div>
        </div>
      </div>

      {createFor === 'tenant' && (
        <CreateModal kind="tenant" onCancel={() => setCreateFor(null)} onCreate={createTenant} />
      )}
      {createFor === 'workspace' && (
        <CreateModal kind="workspace" onCancel={() => setCreateFor(null)} onCreate={createWorkspace} />
      )}
      {createFor === 'project' && (
        <CreateModal kind="project" onCancel={() => setCreateFor(null)} onCreate={createProject} />
      )}
    </>
  );
}

// ── TenantSelector ────────────────────────────────────────────────────────────

export interface TenantSelectorProps {
  /** Force the popover open on mount — used when bootstrap detects the
   *  operator needs to pick between multiple tenants. */
  defaultOpen?: boolean;
}

export function TenantSelector({ defaultOpen = false }: TenantSelectorProps = {}) {
  const [scope, setScope] = useScope();
  const [open, setOpen]   = useState(defaultOpen);
  const containerRef      = useRef<HTMLDivElement>(null);
  const qc                = useQueryClient();

  // Listen for the bootstrap "needs-pick" event so we auto-open on multi-
  // tenant first login.
  useEffect(() => {
    function onNeedsPick() { setOpen(true); }
    window.addEventListener(SCOPE_NEEDS_PICK_EVENT, onNeedsPick);
    return () => window.removeEventListener(SCOPE_NEEDS_PICK_EVENT, onNeedsPick);
  }, []);

  // Close on outside click (but not when a <CreateModal> is open — its
  // overlay is rendered in a portal-like sibling and pointerdown on the
  // modal would otherwise collapse the popover underneath).
  useEffect(() => {
    if (!open) return;
    function onPointer(e: PointerEvent) {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        // Ignore clicks that landed inside a create modal (rendered as a
        // fixed-positioned sibling of the popover).
        const tgt = e.target as HTMLElement;
        if (tgt.closest('[data-testid$="-modal"]')) return;
        setOpen(false);
      }
    }
    document.addEventListener('pointerdown', onPointer);
    return () => document.removeEventListener('pointerdown', onPointer);
  }, [open]);

  function handleApply(next: ProjectScope) {
    setScope(next);
    setOpen(false);
    // Invalidate all queries so every page re-fetches with the new scope.
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
        data-testid="scope-trigger"
        className={clsx(
          'flex items-center gap-1 rounded px-2 py-1 text-[11px] font-mono transition-colors',
          'border max-w-[260px]',
          open
            ? 'bg-indigo-50 dark:bg-indigo-950/40 border-indigo-300 dark:border-indigo-700/60 text-indigo-700 dark:text-indigo-300'
            : isDefault
              ? 'border-amber-300 dark:border-amber-700/50 bg-amber-50/40 dark:bg-amber-950/20 text-amber-700 dark:text-amber-300 hover:border-amber-400'
              : 'border-indigo-200 dark:border-indigo-800/50 text-indigo-600 dark:text-indigo-400 bg-indigo-50/50 dark:bg-indigo-950/20',
        )}
      >
        <span className="truncate">
          {isDefault ? (
            <span>default scope — click to pick</span>
          ) : (
            <>
              <span className="text-gray-500 dark:text-zinc-500">{short(scope.tenant_id)}</span>
              <span className="mx-0.5 text-gray-300 dark:text-zinc-600">/</span>
              <span className="text-gray-500 dark:text-zinc-500">{short(scope.workspace_id)}</span>
              <span className="mx-0.5 text-gray-300 dark:text-zinc-600">/</span>
              <span className="text-indigo-600 dark:text-indigo-400 font-medium">
                {short(scope.project_id)}
              </span>
            </>
          )}
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
