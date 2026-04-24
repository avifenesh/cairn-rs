/**
 * Unit tests for `resolveBootstrapScope` — covers the decision table the
 * App shell relies on (cached scope valid → use it; single tenant/ws/proj
 * → auto-select; zero tenants → empty; multi-tenant → needs-pick).
 */
import { describe, it, expect } from 'vitest';
import { resolveBootstrapScope } from '../useScope';

function mkDeps(opts: {
  tenants:    { tenant_id: string }[];
  workspaces: Record<string, { workspace_id: string }[]>;
  projects:   Record<string, { project_id: string }[]>;
}) {
  return {
    listTenants:    async () => opts.tenants,
    listWorkspaces: async (t: string) => opts.workspaces[t] ?? [],
    listProjects:   async (w: string) => opts.projects[w]   ?? [],
  };
}

describe('resolveBootstrapScope', () => {
  it('returns `empty` when no tenants exist', async () => {
    const r = await resolveBootstrapScope({
      ...mkDeps({ tenants: [], workspaces: {}, projects: {} }),
      cached: null,
    });
    expect(r.status).toBe('empty');
  });

  it('auto-selects the single tenant / workspace / project', async () => {
    const r = await resolveBootstrapScope({
      ...mkDeps({
        tenants:    [{ tenant_id: 'acme' }],
        workspaces: { acme: [{ workspace_id: 'prod' }] },
        projects:   { prod: [{ project_id: 'minecraft' }] },
      }),
      cached: null,
    });
    expect(r).toEqual({
      status: 'ready',
      scope: { tenant_id: 'acme', workspace_id: 'prod', project_id: 'minecraft' },
    });
  });

  it('returns `needs-pick` for multi-tenant installs without cached scope', async () => {
    const r = await resolveBootstrapScope({
      ...mkDeps({
        tenants: [{ tenant_id: 'a' }, { tenant_id: 'b' }],
        workspaces: { a: [{ workspace_id: 'wa' }], b: [{ workspace_id: 'wb' }] },
        projects: { wa: [{ project_id: 'pa' }], wb: [{ project_id: 'pb' }] },
      }),
      cached: null,
    });
    expect(r.status).toBe('needs-pick');
  });

  it('uses cached scope when it still resolves', async () => {
    const r = await resolveBootstrapScope({
      ...mkDeps({
        tenants: [{ tenant_id: 'acme' }, { tenant_id: 'other' }],
        workspaces: { acme: [{ workspace_id: 'prod' }] },
        projects:   { prod: [{ project_id: 'minecraft' }] },
      }),
      cached: { tenant_id: 'acme', workspace_id: 'prod', project_id: 'minecraft' },
    });
    expect(r).toEqual({
      status: 'ready',
      scope: { tenant_id: 'acme', workspace_id: 'prod', project_id: 'minecraft' },
    });
  });

  it('discards stale cached scope when the tenant no longer exists', async () => {
    const r = await resolveBootstrapScope({
      ...mkDeps({
        tenants: [{ tenant_id: 'acme' }, { tenant_id: 'other' }],
        workspaces: {},
        projects: {},
      }),
      cached: { tenant_id: 'deleted', workspace_id: 'x', project_id: 'y' },
    });
    // Two tenants, cache invalid → needs-pick (not empty, not ready).
    expect(r.status).toBe('needs-pick');
  });

  it('treats DEFAULT_SCOPE cache as "no cache" (always re-resolve)', async () => {
    const r = await resolveBootstrapScope({
      ...mkDeps({
        tenants:    [{ tenant_id: 'acme' }],
        workspaces: { acme: [{ workspace_id: 'prod' }] },
        projects:   { prod: [{ project_id: 'minecraft' }] },
      }),
      cached: {
        tenant_id:    'default_tenant',
        workspace_id: 'default_workspace',
        project_id:   'default_project',
      },
    });
    expect(r.status).toBe('ready');
    if (r.status === 'ready') {
      expect(r.scope.tenant_id).toBe('acme');
    }
  });
});
