/**
 * scope — shared ProjectScope type and canonical default constants.
 *
 * Lives outside the React `useScope` hook so that non-React modules
 * (e.g. `lib/api.ts`) can reference the canonical defaults without
 * pulling in React. The `useScope` hook re-exports these for hook
 * consumers that already import from `../hooks/useScope`.
 *
 * IMPORTANT — the default strings MUST match the Rust constants at
 * `crates/cairn-app/src/handlers/feed.rs` (`DEFAULT_TENANT_ID`,
 * `DEFAULT_WORKSPACE_ID`, `DEFAULT_PROJECT_ID`). Using `'default'`
 * here instead of `'default_tenant'` routes operators into the wrong
 * tenant/workspace/project cell.
 */

export interface ProjectScope {
  tenant_id: string;
  workspace_id: string;
  project_id: string;
}

export const DEFAULT_SCOPE: ProjectScope = {
  tenant_id:    'default_tenant',
  workspace_id: 'default_workspace',
  project_id:   'default_project',
};

/** True when every component equals the canonical DEFAULT_SCOPE. */
export function scopeIsDefault(s: ProjectScope): boolean {
  return (
    s.tenant_id    === DEFAULT_SCOPE.tenant_id &&
    s.workspace_id === DEFAULT_SCOPE.workspace_id &&
    s.project_id   === DEFAULT_SCOPE.project_id
  );
}

/** Compact display string: `tenant / workspace / project`. */
export function scopeLabel(s: ProjectScope): string {
  return `${s.tenant_id} / ${s.workspace_id} / ${s.project_id}`;
}
