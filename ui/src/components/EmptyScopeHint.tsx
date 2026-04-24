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
 */

import { useMemo } from 'react';
import { useQuery } from '@tanstack/react-query';
import { ArrowRight } from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import { useScope, isDefaultScope } from '../hooks/useScope';
import { scopeLabel, type ProjectScope } from '../lib/scope';

interface EmptyScopeHintProps {
  /** Whether the parent list is actually empty. */
  empty: boolean;
  /** Optional className overrides for positioning inside the caller. */
  className?: string;
}

/**
 * Discover the first non-default tenant/workspace/project that has any
 * content so we can offer "try switching to X" as a one-click jump.
 */
function useSuggestedScope(enabled: boolean): ProjectScope | null {
  const tenants = useQuery({
    queryKey: ['empty-scope-hint', 'tenants'],
    queryFn:  () => defaultApi.listTenants(),
    enabled,
    staleTime: 60_000,
  });

  const nonDefaultTenant = useMemo(
    () => tenants.data?.find((t) => t.tenant_id !== 'default_tenant'),
    [tenants.data],
  );

  const workspaces = useQuery({
    queryKey: ['empty-scope-hint', 'ws', nonDefaultTenant?.tenant_id],
    queryFn:  () => defaultApi.getWorkspaces(nonDefaultTenant!.tenant_id),
    enabled:  enabled && !!nonDefaultTenant,
    staleTime: 60_000,
  });

  const firstWorkspace = workspaces.data?.[0];

  const projects = useQuery({
    queryKey: ['empty-scope-hint', 'proj', firstWorkspace?.workspace_id],
    queryFn:  () => defaultApi.listProjects(firstWorkspace!.workspace_id),
    enabled:  enabled && !!firstWorkspace,
    staleTime: 60_000,
  });

  const firstProject = projects.data?.[0];

  if (!nonDefaultTenant || !firstWorkspace || !firstProject) return null;
  return {
    tenant_id:    nonDefaultTenant.tenant_id,
    workspace_id: firstWorkspace.workspace_id,
    project_id:   firstProject.project_id,
  };
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
