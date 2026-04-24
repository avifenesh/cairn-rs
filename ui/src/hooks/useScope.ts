/**
 * useScope — tenant/workspace/project scope management.
 *
 * The selected scope is persisted in localStorage and read on every API call
 * so that list endpoints automatically filter to the current context.
 *
 * Bootstrap behaviour (added 2026-04-23):
 *
 *   On first mount the App shell calls `useBootstrapScope()` which:
 *     1. Reads localStorage. If a cached scope is present AND still resolves
 *        to an existing tenant / workspace / project on the backend, use it.
 *     2. Otherwise calls `listTenants`. If exactly one tenant exists, auto-
 *        drills to its single workspace + single project. This turns the
 *        "empty page because you're on default_tenant" bug into a silent
 *        auto-selection for the common single-tenant case.
 *     3. Otherwise leaves the scope unresolved so the App can render the
 *        StarterSetup flow (zero tenants) or force the operator to pick
 *        (multi-tenant first login).
 */

import { useCallback, useEffect, useRef, useState } from 'react';
import { DEFAULT_SCOPE, type ProjectScope, scopeIsDefault } from '../lib/scope';
import { defaultApi } from '../lib/api';

// ── Types ─────────────────────────────────────────────────────────────────────

// The canonical `ProjectScope` type and `DEFAULT_SCOPE` constant live in
// `../lib/scope` so non-React modules (e.g. `lib/api.ts`) can reference
// them without pulling React. Re-exported here so existing imports keep
// working.
export { DEFAULT_SCOPE };
export type { ProjectScope };

export const SCOPE_KEY = 'cairn_scope';

/** Fired by `ScopeBootstrapGate` when the operator must pick a scope
 *  (multi-tenant first login). `TenantSelector` listens and auto-opens. */
export const SCOPE_NEEDS_PICK_EVENT = 'cairn:scope-needs-pick';

/** Custom event name fired when the scope is mutated anywhere in the app.
 *  The TopBar listens for this so its breadcrumb updates instantly across
 *  tabs/components without re-mounting the tree. */
export const SCOPE_CHANGED_EVENT = 'cairn:scope-changed';

// ── Persistence helpers ───────────────────────────────────────────────────────

export function getStoredScope(): ProjectScope {
  try {
    const raw = localStorage.getItem(SCOPE_KEY);
    if (!raw) return { ...DEFAULT_SCOPE };
    return { ...DEFAULT_SCOPE, ...(JSON.parse(raw) as Partial<ProjectScope>) };
  } catch {
    return { ...DEFAULT_SCOPE };
  }
}

export function setStoredScope(scope: ProjectScope): void {
  try {
    localStorage.setItem(SCOPE_KEY, JSON.stringify(scope));
    window.dispatchEvent(new CustomEvent(SCOPE_CHANGED_EVENT, { detail: scope }));
  } catch { /* storage quota or private mode */ }
}

/** Returns true when the supplied scope equals the canonical DEFAULT_SCOPE. */
export function isDefaultScope(scope: ProjectScope): boolean {
  return scopeIsDefault(scope);
}

// ── Hook ─────────────────────────────────────────────────────────────────────

/**
 * Access and persist the current tenant/workspace/project scope.
 *
 * Returns `[scope, setScope, reset]`.  Calling `setScope` persists to
 * localStorage immediately and fires `SCOPE_CHANGED_EVENT`.  A `reset()`
 * helper restores the default scope.
 */
export function useScope(): [ProjectScope, (s: ProjectScope) => void, () => void] {
  const [scope, setScopeState] = useState<ProjectScope>(getStoredScope);

  // Keep local state in sync with cross-component scope changes.
  useEffect(() => {
    function onChanged() {
      setScopeState(getStoredScope());
    }
    window.addEventListener(SCOPE_CHANGED_EVENT, onChanged);
    return () => window.removeEventListener(SCOPE_CHANGED_EVENT, onChanged);
  }, []);

  const setScope = useCallback((s: ProjectScope) => {
    setStoredScope(s);
    setScopeState(s);
  }, []);

  const reset = useCallback(() => {
    setScope({ ...DEFAULT_SCOPE });
  }, [setScope]);

  return [scope, setScope, reset];
}

// ── Bootstrap resolution ─────────────────────────────────────────────────────

/** Resolution state returned by `useBootstrapScope`. */
export type BootstrapState =
  | { status: 'loading' }
  /** No tenants exist at all — render StarterSetup. */
  | { status: 'empty' }
  /** Scope resolved (either from cache, auto-select, or existing). */
  | { status: 'ready'; scope: ProjectScope }
  /** Multiple tenants exist and no cached scope — operator must pick. */
  | { status: 'needs-pick' }
  /** Backend errored out (e.g. 401/5xx). Fall back to cached/default scope. */
  | { status: 'error'; error: Error };

/** Internal: resolve scope against the backend. Exported for unit tests. */
export async function resolveBootstrapScope(deps: {
  listTenants:    () => Promise<{ tenant_id: string }[]>;
  listWorkspaces: (tenantId: string) => Promise<{ workspace_id: string }[]>;
  listProjects:   (workspaceId: string) => Promise<{ project_id: string }[]>;
  cached?: ProjectScope | null;
}): Promise<BootstrapState> {
  const tenants = await deps.listTenants();

  if (tenants.length === 0) {
    return { status: 'empty' };
  }

  // If we have a non-default cached scope, validate it still exists.
  if (deps.cached && !scopeIsDefault(deps.cached)) {
    const t = tenants.find((x) => x.tenant_id === deps.cached!.tenant_id);
    if (t) {
      const workspaces = await deps.listWorkspaces(deps.cached.tenant_id);
      const w = workspaces.find((x) => x.workspace_id === deps.cached!.workspace_id);
      if (w) {
        const projects = await deps.listProjects(deps.cached.workspace_id);
        const p = projects.find((x) => x.project_id === deps.cached!.project_id);
        if (p) {
          return { status: 'ready', scope: { ...deps.cached } };
        }
      }
    }
    // Cached scope no longer valid — fall through to auto-select.
  }

  // Auto-select when there's exactly one tenant.
  if (tenants.length === 1) {
    const tenant = tenants[0];
    const workspaces = await deps.listWorkspaces(tenant.tenant_id);
    if (workspaces.length === 1) {
      const ws = workspaces[0];
      const projects = await deps.listProjects(ws.workspace_id);
      if (projects.length === 1) {
        return {
          status: 'ready',
          scope: {
            tenant_id:    tenant.tenant_id,
            workspace_id: ws.workspace_id,
            project_id:   projects[0].project_id,
          },
        };
      }
    }
  }

  return { status: 'needs-pick' };
}

/**
 * App-shell hook that resolves the initial scope on first mount.
 *
 * Renders `loading` while talking to the backend, then transitions to
 * `ready` / `needs-pick` / `empty` / `error`. The App shell should:
 *   - `empty`      → show the <StarterSetup/> guided flow
 *   - `ready`      → scope is already persisted, render normal pages
 *   - `needs-pick` → open the <TenantSelector/> popover automatically
 *   - `error`      → render normal pages with stale cached scope
 */
export function useBootstrapScope(): BootstrapState {
  const [state, setState] = useState<BootstrapState>({ status: 'loading' });
  const ran = useRef(false);

  useEffect(() => {
    if (ran.current) return;
    ran.current = true;

    (async () => {
      try {
        const cached = (() => {
          try {
            const raw = localStorage.getItem(SCOPE_KEY);
            return raw ? (JSON.parse(raw) as ProjectScope) : null;
          } catch {
            return null;
          }
        })();

        const resolved = await resolveBootstrapScope({
          listTenants:    () => defaultApi.listTenants(),
          listWorkspaces: (t) => defaultApi.getWorkspaces(t),
          listProjects:   (w) => defaultApi.listProjects(w),
          cached,
        });

        if (resolved.status === 'ready') {
          setStoredScope(resolved.scope);
        }

        setState(resolved);
      } catch (err) {
        setState({ status: 'error', error: err as Error });
      }
    })();
  }, []);

  return state;
}
