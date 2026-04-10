/**
 * useScope — tenant/workspace/project scope management.
 *
 * The selected scope is persisted in localStorage and read on every API call
 * so that list endpoints automatically filter to the current context.
 */

import { useState, useCallback } from 'react';

// ── Types ─────────────────────────────────────────────────────────────────────

export interface ProjectScope {
  tenant_id: string;
  workspace_id: string;
  project_id: string;
}

export const SCOPE_KEY = 'cairn_scope';

export const DEFAULT_SCOPE: ProjectScope = {
  tenant_id:    'default_tenant',
  workspace_id: 'default_workspace',
  project_id:   'default_project',
};

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
  } catch { /* storage quota or private mode */ }
}

/** True when scope equals the default (i.e. no custom scope is active). */
export function isDefaultScope(scope: ProjectScope): boolean {
  return (
    scope.tenant_id    === DEFAULT_SCOPE.tenant_id &&
    scope.workspace_id === DEFAULT_SCOPE.workspace_id &&
    scope.project_id   === DEFAULT_SCOPE.project_id
  );
}

// ── Hook ─────────────────────────────────────────────────────────────────────

/**
 * Access and persist the current tenant/workspace/project scope.
 *
 * Returns `[scope, setScope]`.  Calling `setScope` persists to localStorage
 * immediately.  A `reset()` helper restores the default scope.
 */
export function useScope(): [ProjectScope, (s: ProjectScope) => void, () => void] {
  const [scope, setScopeState] = useState<ProjectScope>(getStoredScope);

  const setScope = useCallback((s: ProjectScope) => {
    setStoredScope(s);
    setScopeState(s);
  }, []);

  const reset = useCallback(() => {
    setScope({ ...DEFAULT_SCOPE });
  }, [setScope]);

  return [scope, setScope, reset];
}
