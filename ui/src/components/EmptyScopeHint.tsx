/**
 * EmptyScopeHint — renders a "you might be on the wrong scope" nudge when
 * a list page shows zero rows AND the active scope is the canonical default
 * AND there are other tenants/workspaces/projects the operator could switch
 * to.
 *
 * Fixes the onboarding bug where operators land on
 * `default_tenant/default_workspace/default_project`, see empty pages across
 * the entire app, and have no discoverable way to realise the data lives in
 * a different scope (e.g. `acme/prod/minecraft`).
 *
 * F28 fix (2026-04-24) — the previous implementation rejected only the
 * literal `default_tenant` string. The system also auto-creates a tenant
 * named bare `default` (legacy seeded fixture), whose first workspace and
 * first project are also named `default`. That produced the nonsense
 * suggestion "switch to default / default / default" — an operator on the
 * canonical default was told to jump to another default-shaped scope, and
 * doing so dropped them into an equally-empty cell. We now:
 *
 *   1. Reject any candidate triple whose (tenant, workspace, project) is
 *      the canonical DEFAULT_SCOPE via `isDefaultScope`.
 *   2. Reject any triple where *any* segment is the bare string `"default"`
 *      — not covered by DEFAULT_SCOPE but still useless as a suggestion.
 *   3. Walk further: iterate up to MAX_CANDIDATE_TENANTS tenants (cheap —
 *      users rarely have more than a handful) until a genuinely non-default
 *      triple is found, otherwise return null (don't render the hint).
 */

import { useMemo } from 'react';
import { useQuery, useQueries } from '@tanstack/react-query';
import { ArrowRight } from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import { useScope, isDefaultScope } from '../hooks/useScope';
import { scopeLabel, type ProjectScope, DEFAULT_SCOPE } from '../lib/scope';

interface EmptyScopeHintProps {
  /** Whether the parent list is actually empty. */
  empty: boolean;
  /** Optional className overrides for positioning inside the caller. */
  className?: string;
}

/**
 * Cap the walker at a small number of tenants. The hint is a best-effort
 * onboarding nudge, not a full discovery API — if an install has more than
 * MAX_CANDIDATE_TENANTS tenants the operator has already been through the
 * scope picker.
 */
const MAX_CANDIDATE_TENANTS = 10;

/** Segment values that would render as a degenerate "default-ish" scope. */
const DEFAULT_LIKE_SEGMENTS = new Set<string>([
  'default',
  DEFAULT_SCOPE.tenant_id,
  DEFAULT_SCOPE.workspace_id,
  DEFAULT_SCOPE.project_id,
]);

/** True when the given scope is either canonical default or contains any
 *  bare-`default`-shaped segment — both are useless as a "switch to" hint. */
function isDefaultLikeScope(scope: ProjectScope): boolean {
  if (isDefaultScope(scope)) return true;
  return (
    DEFAULT_LIKE_SEGMENTS.has(scope.tenant_id) ||
    DEFAULT_LIKE_SEGMENTS.has(scope.workspace_id) ||
    DEFAULT_LIKE_SEGMENTS.has(scope.project_id)
  );
}

/**
 * Discover the first non-default tenant/workspace/project that has any
 * content so we can offer "try switching to X" as a one-click jump.
 *
 * Algorithm:
 *   1. List tenants, cap at MAX_CANDIDATE_TENANTS.
 *   2. For each, fan out a parallel workspaces query.
 *   3. For each tenant's first workspace, fan out a parallel projects query.
 *   4. Walk the resulting (tenant, firstWs, firstProj) triples in order and
 *      return the first one that passes `isDefaultLikeScope === false`.
 *   5. If none qualify, return null — don't render the hint.
 */
function useSuggestedScope(enabled: boolean): ProjectScope | null {
  const tenants = useQuery({
    queryKey: ['empty-scope-hint', 'tenants'],
    queryFn:  () => defaultApi.listTenants(),
    enabled,
    staleTime: 60_000,
  });

  // Candidate tenant IDs — anything that isn't plainly default-shaped at the
  // tenant level. We still fetch workspaces/projects for these because the
  // final accept/reject decision is on the full triple, not tenant alone.
  const candidateTenants = useMemo(() => {
    const list = tenants.data ?? [];
    return list
      .filter((t) => !DEFAULT_LIKE_SEGMENTS.has(t.tenant_id))
      .slice(0, MAX_CANDIDATE_TENANTS);
  }, [tenants.data]);

  const workspaceQueries = useQueries({
    queries: candidateTenants.map((t) => ({
      queryKey: ['empty-scope-hint', 'ws', t.tenant_id],
      queryFn:  () => defaultApi.getWorkspaces(t.tenant_id),
      enabled,
      staleTime: 60_000,
    })),
  });

  // For each candidate tenant, the first workspace (if any). Keyed by
  // tenant_id so projectQueries can be ordered consistently below.
  const firstWorkspaces = useMemo(() => {
    return candidateTenants.map((t, i) => {
      const ws = workspaceQueries[i]?.data?.[0];
      return ws ? { tenantId: t.tenant_id, workspaceId: ws.workspace_id } : null;
    });
    // workspaceQueries is an array of query results; use a stable fingerprint.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [candidateTenants, workspaceQueries.map((q) => q.data?.[0]?.workspace_id).join('|')]);

  const projectQueries = useQueries({
    queries: firstWorkspaces.map((fw) => ({
      queryKey: ['empty-scope-hint', 'proj', fw?.tenantId, fw?.workspaceId],
      queryFn:  () => defaultApi.listProjects(fw!.workspaceId),
      enabled:  enabled && !!fw,
      staleTime: 60_000,
    })),
  });

  return useMemo<ProjectScope | null>(() => {
    for (let i = 0; i < candidateTenants.length; i++) {
      const fw = firstWorkspaces[i];
      if (!fw) continue;
      const firstProject = projectQueries[i]?.data?.[0];
      if (!firstProject) continue;
      const triple: ProjectScope = {
        tenant_id:    fw.tenantId,
        workspace_id: fw.workspaceId,
        project_id:   firstProject.project_id,
      };
      if (isDefaultLikeScope(triple)) continue;
      return triple;
    }
    return null;
    // projectQueries is an array of query results; fingerprint the slice we
    // actually read (the first project_id per entry).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    candidateTenants,
    firstWorkspaces,
    projectQueries.map((q) => q.data?.[0]?.project_id ?? '').join('|'),
  ]);
}

export function EmptyScopeHint({ empty, className }: EmptyScopeHintProps) {
  const [scope, setScope] = useScope();
  const onDefault = isDefaultScope(scope);

  // Only run the queries when we might actually render something.
  const suggested = useSuggestedScope(empty && onDefault);

  if (!empty || !onDefault || !suggested) return null;

  function jump() {
    if (suggested) setScope(suggested);
  }

  return (
    <div
      data-testid="empty-scope-hint"
      className={clsx(
        'mt-3 rounded-lg border border-amber-300/50 dark:border-amber-700/40',
        'bg-amber-50/60 dark:bg-amber-950/20 px-4 py-3 text-[12px]',
        'text-amber-900 dark:text-amber-200 flex items-center gap-3',
        className,
      )}
      role="status"
    >
      <span className="shrink-0 text-[10px] font-semibold uppercase tracking-wider
                       text-amber-700 dark:text-amber-300">
        Tip
      </span>
      <span className="flex-1 leading-relaxed">
        No data here. You may be on the default scope — try switching to{' '}
        <span className="font-mono font-medium">{scopeLabel(suggested)}</span>.
      </span>
      <button
        onClick={jump}
        data-testid="empty-scope-hint-jump"
        className="shrink-0 inline-flex items-center gap-1 rounded-md
                   bg-amber-600 hover:bg-amber-500 text-white
                   px-2.5 py-1 text-[11px] font-medium transition-colors"
      >
        Switch <ArrowRight size={11} />
      </button>
    </div>
  );
}

// Re-exported for unit tests so they don't duplicate the rejection rules.
export { isDefaultLikeScope, DEFAULT_LIKE_SEGMENTS, MAX_CANDIDATE_TENANTS };
