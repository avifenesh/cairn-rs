/**
 * StarterSetup — guided first-login flow for truly empty installs.
 *
 * Rendered by the App shell when `useBootstrapScope` resolves to `empty`
 * (zero tenants exist on the server). Walks the operator through creating
 * tenant → workspace → project, then sets the active scope automatically
 * so the next page render already has data to show.
 */

import { useState, type FormEvent } from 'react';
import { defaultApi } from '../lib/api';
import { setStoredScope, type ProjectScope } from '../hooks/useScope';
import { ApiError } from '../lib/api';

type Step = 'tenant' | 'workspace' | 'project' | 'done';

interface StarterSetupProps {
  /** Called once scope is created. Parent should re-run the bootstrap. */
  onComplete: (scope: ProjectScope) => void;
}

function slugify(s: string): string {
  return s
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '_')
    .replace(/^_+|_+$/g, '')
    .slice(0, 64);
}

export function StarterSetup({ onComplete }: StarterSetupProps) {
  const [step, setStep] = useState<Step>('tenant');

  const [tenantName,  setTenantName]  = useState('');
  const [tenantId,    setTenantId]    = useState('');
  const [workspaceName, setWorkspaceName] = useState('');
  const [workspaceId,   setWorkspaceId]   = useState('');
  const [projectName, setProjectName] = useState('');
  const [projectId,   setProjectId]   = useState('');

  const [busy, setBusy]   = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleCreateTenant(e: FormEvent) {
    e.preventDefault();
    if (!tenantName.trim()) return;
    setBusy(true); setError(null);
    try {
      const id = tenantId.trim() || slugify(tenantName);
      await defaultApi.createTenant({ tenant_id: id, name: tenantName.trim() });
      setTenantId(id);
      setStep('workspace');
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function handleCreateWorkspace(e: FormEvent) {
    e.preventDefault();
    if (!workspaceName.trim()) return;
    setBusy(true); setError(null);
    try {
      const id = workspaceId.trim() || slugify(workspaceName);
      await defaultApi.createWorkspace(tenantId, { workspace_id: id, name: workspaceName.trim() });
      setWorkspaceId(id);
      setStep('project');
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function handleCreateProject(e: FormEvent) {
    e.preventDefault();
    if (!projectName.trim()) return;
    setBusy(true); setError(null);
    try {
      const id = projectId.trim() || slugify(projectName);
      await defaultApi.createProject(workspaceId, { project_id: id, name: projectName.trim() });
      const scope = { tenant_id: tenantId, workspace_id: workspaceId, project_id: id };
      setStoredScope(scope);
      setStep('done');
      onComplete(scope);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  const stepNum = step === 'tenant' ? 1 : step === 'workspace' ? 2 : step === 'project' ? 3 : 3;

  return (
    <div
      data-testid="starter-setup"
      className="min-h-screen flex items-center justify-center bg-gray-50 dark:bg-zinc-950 px-4"
    >
      <div className="w-full max-w-md rounded-xl border border-gray-200 dark:border-zinc-800
                      bg-white dark:bg-zinc-900 shadow-xl overflow-hidden">
        <div className="px-6 py-5 border-b border-gray-100 dark:border-zinc-800">
          <h1 className="text-[15px] font-semibold text-gray-900 dark:text-zinc-100">
            Welcome to cairn
          </h1>
          <p className="text-[12px] text-gray-500 dark:text-zinc-400 mt-1 leading-relaxed">
            Let&apos;s create your first tenant, workspace, and project so you have somewhere
            to put your runs, tasks, and memory.
          </p>
          <div className="mt-3 flex items-center gap-1.5">
            {[1, 2, 3].map((n) => (
              <div
                key={n}
                className={`h-1 flex-1 rounded-full ${
                  n <= stepNum ? 'bg-indigo-500' : 'bg-gray-200 dark:bg-zinc-800'
                }`}
              />
            ))}
          </div>
        </div>

        {error && (
          <div className="mx-6 mt-4 rounded-md border border-red-200 dark:border-red-900/50
                          bg-red-50 dark:bg-red-950/30 px-3 py-2 text-[12px]
                          text-red-700 dark:text-red-300">
            {error}
          </div>
        )}

        {step === 'tenant' && (
          <form onSubmit={handleCreateTenant} className="p-6 space-y-3" data-testid="starter-step-tenant">
            <LabeledInput
              label="Tenant name"
              placeholder="Acme Corp"
              value={tenantName}
              onChange={setTenantName}
              autoFocus
              testId="starter-tenant-name"
            />
            <LabeledInput
              label="Tenant ID (optional)"
              placeholder={tenantName ? slugify(tenantName) : 'acme'}
              value={tenantId}
              onChange={setTenantId}
              mono
              testId="starter-tenant-id"
            />
            <SubmitRow busy={busy} label="Create tenant →" />
          </form>
        )}

        {step === 'workspace' && (
          <form onSubmit={handleCreateWorkspace} className="p-6 space-y-3" data-testid="starter-step-workspace">
            <LabeledInput
              label="Workspace name"
              placeholder="Production"
              value={workspaceName}
              onChange={setWorkspaceName}
              autoFocus
              testId="starter-workspace-name"
            />
            <LabeledInput
              label="Workspace ID (optional)"
              placeholder={workspaceName ? slugify(workspaceName) : 'prod'}
              value={workspaceId}
              onChange={setWorkspaceId}
              mono
              testId="starter-workspace-id"
            />
            <SubmitRow busy={busy} label="Create workspace →" />
          </form>
        )}

        {step === 'project' && (
          <form onSubmit={handleCreateProject} className="p-6 space-y-3" data-testid="starter-step-project">
            <LabeledInput
              label="Project name"
              placeholder="Minecraft Ops"
              value={projectName}
              onChange={setProjectName}
              autoFocus
              testId="starter-project-name"
            />
            <LabeledInput
              label="Project ID (optional)"
              placeholder={projectName ? slugify(projectName) : 'minecraft'}
              value={projectId}
              onChange={setProjectId}
              mono
              testId="starter-project-id"
            />
            <SubmitRow busy={busy} label="Finish setup" />
          </form>
        )}

        {step === 'done' && (
          <div className="p-6 text-center" data-testid="starter-done">
            <p className="text-[13px] text-gray-700 dark:text-zinc-300">
              All set. Loading your dashboard…
            </p>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Primitives ────────────────────────────────────────────────────────────────

function LabeledInput(props: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  autoFocus?: boolean;
  mono?: boolean;
  testId?: string;
}) {
  return (
    <label className="block">
      <span className="block text-[10px] font-medium text-gray-400 dark:text-zinc-600
                       uppercase tracking-wider mb-1">
        {props.label}
      </span>
      <input
        data-testid={props.testId}
        autoFocus={props.autoFocus}
        value={props.value}
        onChange={(e) => props.onChange(e.target.value)}
        placeholder={props.placeholder}
        spellCheck={false}
        className={`w-full rounded border border-gray-200 dark:border-zinc-800
                    bg-gray-50 dark:bg-zinc-950 text-[13px]
                    text-gray-900 dark:text-zinc-200 px-3 py-2
                    focus:outline-none focus:border-indigo-500 transition-colors
                    placeholder-gray-400 dark:placeholder-zinc-600
                    ${props.mono ? 'font-mono text-[12px]' : ''}`}
      />
    </label>
  );
}

function SubmitRow({ busy, label }: { busy: boolean; label: string }) {
  return (
    <div className="pt-2">
      <button
        type="submit"
        disabled={busy}
        data-testid="starter-submit"
        className="w-full rounded-md bg-indigo-600 hover:bg-indigo-500
                   disabled:opacity-50 text-white px-3 py-2 text-[13px]
                   font-medium transition-colors"
      >
        {busy ? 'Creating…' : label}
      </button>
    </div>
  );
}
