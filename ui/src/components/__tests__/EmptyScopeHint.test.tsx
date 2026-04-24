/**
 * EmptyScopeHint — F28 fix regression tests.
 *
 * Covers three buggy scenarios we must never ship again:
 *
 *  1. `isDefaultLikeScope` rejects canonical DEFAULT_SCOPE AND any triple
 *     containing a bare `"default"` segment or a `default_*` named segment.
 *     This is the core fix: the old filter only excluded `default_tenant`
 *     and cheerfully suggested "default / default / default" as the
 *     recommended scope to jump to.
 *
 *  2. Rendered end-to-end: given tenants
 *     `[default_tenant, default, acme]`, the hint must suggest `acme`, not
 *     the `default`-named tenant whose first-workspace/first-project are
 *     also named `default`.
 *
 *  3. Walker keeps going past tenants whose first workspace doesn't exist —
 *     returns the next valid triple, or null if none qualifies.
 */

import { render, screen, waitFor } from '@testing-library/react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// ── Mock the API layer so we don't hit the network ──────────────────────────
// Must be declared before importing the component under test.
const listTenants   = vi.fn();
const getWorkspaces = vi.fn();
const listProjects  = vi.fn();

vi.mock('../../lib/api', () => ({
  defaultApi: {
    listTenants:   (...args: unknown[]) => listTenants(...args),
    getWorkspaces: (...args: unknown[]) => getWorkspaces(...args),
    listProjects:  (...args: unknown[]) => listProjects(...args),
  },
}));

// Force `useScope` to always return the canonical DEFAULT_SCOPE so the hint
// becomes eligible to render. We also capture `setScope` to assert the
// correct scope gets applied on Switch.
const setScopeSpy = vi.fn();

vi.mock('../../hooks/useScope', async () => {
  const actual = await vi.importActual<typeof import('../../hooks/useScope')>(
    '../../hooks/useScope',
  );
  const scope = {
    tenant_id:    'default_tenant',
    workspace_id: 'default_workspace',
    project_id:   'default_project',
  };
  return {
    ...actual,
    // Force the hook to always report the canonical DEFAULT_SCOPE so the
    // hint is eligible to render. We intentionally DO NOT stub
    // `isDefaultScope` — we need the real impl to correctly classify the
    // canonical default and the non-default suggestion triples.
    useScope: () => [scope, setScopeSpy, () => setScopeSpy(scope)],
  };
});

import {
  EmptyScopeHint,
  isDefaultLikeScope,
  DEFAULT_LIKE_SEGMENTS,
} from '../EmptyScopeHint';

// ── Helpers ─────────────────────────────────────────────────────────────────

function renderHint(empty = true) {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={qc}>
      <EmptyScopeHint empty={empty} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  listTenants.mockReset();
  getWorkspaces.mockReset();
  listProjects.mockReset();
  setScopeSpy.mockReset();
});

// ── Unit — rejection predicate ──────────────────────────────────────────────

describe('isDefaultLikeScope', () => {
  it('rejects canonical DEFAULT_SCOPE', () => {
    expect(
      isDefaultLikeScope({
        tenant_id:    'default_tenant',
        workspace_id: 'default_workspace',
        project_id:   'default_project',
      }),
    ).toBe(true);
  });

  it('rejects a triple whose tenant is bare `default`', () => {
    expect(
      isDefaultLikeScope({
        tenant_id:    'default',
        workspace_id: 'default',
        project_id:   'default',
      }),
    ).toBe(true);
  });

  it('rejects a triple where ANY segment is a default sentinel', () => {
    expect(
      isDefaultLikeScope({
        tenant_id:    'acme',
        workspace_id: 'default_workspace',
        project_id:   'minecraft',
      }),
    ).toBe(true);
    expect(
      isDefaultLikeScope({
        tenant_id:    'acme',
        workspace_id: 'prod',
        project_id:   'default',
      }),
    ).toBe(true);
  });

  it('accepts a fully non-default triple', () => {
    expect(
      isDefaultLikeScope({
        tenant_id:    'acme',
        workspace_id: 'prod',
        project_id:   'minecraft',
      }),
    ).toBe(false);
  });

  it('enumerates the expected sentinel set', () => {
    // Regression guard: if someone renames the backend defaults they MUST
    // keep this set in lockstep or the hint will start suggesting the new
    // default shape again.
    expect([...DEFAULT_LIKE_SEGMENTS].sort()).toEqual(
      [
        'default',
        'default_project',
        'default_tenant',
        'default_workspace',
      ].sort(),
    );
  });
});

// ── Integration — rendered output ───────────────────────────────────────────

describe('<EmptyScopeHint />', () => {
  it('suggests `acme / prod / minecraft`, not `default / default / default`, when tenants include a legacy bare-default tenant', async () => {
    listTenants.mockResolvedValue([
      { tenant_id: 'default_tenant', name: 'Default',        created_at: 0, updated_at: 0 },
      { tenant_id: 'default',        name: 'Legacy default', created_at: 0, updated_at: 0 },
      { tenant_id: 'acme',           name: 'Acme',           created_at: 0, updated_at: 0 },
    ]);
    getWorkspaces.mockImplementation(async (tenantId: string) => {
      if (tenantId === 'default') return [{ workspace_id: 'default', tenant_id: 'default', name: 'default', created_at: 0, updated_at: 0 }];
      if (tenantId === 'acme')    return [{ workspace_id: 'prod',    tenant_id: 'acme',    name: 'prod',    created_at: 0, updated_at: 0 }];
      return [];
    });
    listProjects.mockImplementation(async (wsId: string) => {
      if (wsId === 'default') return [{ project_id: 'default',  workspace_id: 'default', tenant_id: 'default', name: 'default',  created_at: 0, updated_at: 0 }];
      if (wsId === 'prod')    return [{ project_id: 'minecraft', workspace_id: 'prod',    tenant_id: 'acme',    name: 'minecraft', created_at: 0, updated_at: 0 }];
      return [];
    });

    renderHint(true);

    // Wait for the hint to mount and report the suggested scope.
    const hint = await screen.findByTestId('empty-scope-hint');
    expect(hint).toHaveTextContent('acme / prod / minecraft');
    expect(hint).not.toHaveTextContent('default / default / default');
  });

  it('returns null (no render) when every candidate triple is default-shaped', async () => {
    listTenants.mockResolvedValue([
      { tenant_id: 'default_tenant', name: 'Default',        created_at: 0, updated_at: 0 },
      { tenant_id: 'default',        name: 'Legacy default', created_at: 0, updated_at: 0 },
    ]);
    getWorkspaces.mockImplementation(async () => [
      { workspace_id: 'default', tenant_id: 'default', name: 'default', created_at: 0, updated_at: 0 },
    ]);
    listProjects.mockImplementation(async () => [
      { project_id: 'default', workspace_id: 'default', tenant_id: 'default', name: 'default', created_at: 0, updated_at: 0 },
    ]);

    const { container } = renderHint(true);

    // Give the tenants fetch a chance to settle. In this scenario both
    // tenants are filtered out at the candidate stage (their tenant_id is
    // a default sentinel) so workspaces/projects are never fetched — hence
    // we key the wait on `listTenants`, not downstream calls.
    await waitFor(() => expect(listTenants).toHaveBeenCalled());
    expect(screen.queryByTestId('empty-scope-hint')).toBeNull();
    expect(container.textContent ?? '').not.toContain('default / default / default');
  });

  it('rejects a triple where tenant is non-default but workspace/project are bare `default`', async () => {
    listTenants.mockResolvedValue([
      { tenant_id: 'acme', name: 'Acme', created_at: 0, updated_at: 0 },
    ]);
    getWorkspaces.mockResolvedValue([
      { workspace_id: 'default', tenant_id: 'acme', name: 'default', created_at: 0, updated_at: 0 },
    ]);
    listProjects.mockResolvedValue([
      { project_id: 'default', workspace_id: 'default', tenant_id: 'acme', name: 'default', created_at: 0, updated_at: 0 },
    ]);

    renderHint(true);

    await waitFor(() => expect(listProjects).toHaveBeenCalled());
    expect(screen.queryByTestId('empty-scope-hint')).toBeNull();
  });

  it('walks past a tenant with no workspaces and returns the next valid triple', async () => {
    listTenants.mockResolvedValue([
      { tenant_id: 'empty_tenant', name: 'Empty', created_at: 0, updated_at: 0 },
      { tenant_id: 'acme',         name: 'Acme',  created_at: 0, updated_at: 0 },
    ]);
    getWorkspaces.mockImplementation(async (tenantId: string) => {
      if (tenantId === 'empty_tenant') return [];
      if (tenantId === 'acme')         return [{ workspace_id: 'prod', tenant_id: 'acme', name: 'prod', created_at: 0, updated_at: 0 }];
      return [];
    });
    listProjects.mockImplementation(async (wsId: string) => {
      if (wsId === 'prod') return [{ project_id: 'minecraft', workspace_id: 'prod', tenant_id: 'acme', name: 'minecraft', created_at: 0, updated_at: 0 }];
      return [];
    });

    renderHint(true);

    const hint = await screen.findByTestId('empty-scope-hint');
    expect(hint).toHaveTextContent('acme / prod / minecraft');
  });

  it('does not render when parent list is not empty', () => {
    listTenants.mockResolvedValue([
      { tenant_id: 'acme', name: 'Acme', created_at: 0, updated_at: 0 },
    ]);
    renderHint(false);
    expect(screen.queryByTestId('empty-scope-hint')).toBeNull();
    // The `enabled` flag also prevents the tenant fetch from firing at all.
    expect(listTenants).not.toHaveBeenCalled();
  });
});
