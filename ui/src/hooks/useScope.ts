/**
 * useScope — tenant/workspace/project scope management.
 *
 * The selected scope is persisted in localStorage and read on every API call
 * so that list endpoints automatically filter to the current context.
 */

import { useState, useCallback } from 'react';
import { DEFAULT_SCOPE, type ProjectScope } from '../lib/scope';

// ── Types ─────────────────────────────────────────────────────────────────────

// The canonical `ProjectScope` type and `DEFAULT_SCOPE` constant live in
// `../lib/scope` so non-React modules (e.g. `lib/api.ts`) can reference
// them without pulling React. Re-exported here so existing imports keep
// working.
export { DEFAULT_SCOPE };
export type { ProjectScope };

export const SCOPE_KEY = 'cairn_scope';

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
