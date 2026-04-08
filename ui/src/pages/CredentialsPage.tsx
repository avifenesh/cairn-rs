/**
 * CredentialsPage — operator view for managing encrypted provider credentials.
 *
 * RFC 011: credentials are stored encrypted at rest. This page shows ONLY
 * metadata (never the secret value). The plaintext_value field in the "Add"
 * form is transmitted once over TLS and then discarded — the backend stores
 * only the encrypted form.
 *
 * Routes:
 *   GET    /v1/admin/tenants/:tenantId/credentials
 *   POST   /v1/admin/tenants/:tenantId/credentials
 *   DELETE /v1/admin/tenants/:tenantId/credentials/:id
 */

import { useState, useId } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  RefreshCw, Loader2, ServerCrash, Plus, Trash2, X,
  KeyRound, Lock, Eye, EyeOff, AlertTriangle,
} from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import { useFocusTrap } from '../hooks/useFocusTrap';
import type { CredentialSummary, StoreCredentialRequest } from '../lib/types';

// ── Constants ─────────────────────────────────────────────────────────────────

const DEFAULT_TENANT = 'default';

const CREDENTIAL_TYPES = [
  { value: 'api_key',           label: 'API Key' },
  { value: 'oauth_token',       label: 'OAuth Token' },
  { value: 'connection_string', label: 'Connection String' },
  { value: 'service_account',   label: 'Service Account' },
  { value: 'bearer_token',      label: 'Bearer Token' },
  { value: 'basic_auth',        label: 'Basic Auth' },
] as const;

type CredentialTypeValue = typeof CREDENTIAL_TYPES[number]['value'];

// ── Helpers ───────────────────────────────────────────────────────────────────

function fmtDate(ms: number): string {
  return new Date(ms).toLocaleDateString(undefined, {
    month: 'short', day: 'numeric', year: 'numeric',
  });
}

function fmtRelative(ms: number | null | undefined): string {
  if (!ms) return '—';
  const diff = Date.now() - ms;
  const days  = Math.floor(diff / 86_400_000);
  if (days === 0) return 'Today';
  if (days === 1) return 'Yesterday';
  if (days < 30)  return `${days}d ago`;
  const months = Math.floor(days / 30);
  return months === 1 ? '1 month ago' : `${months} months ago`;
}

/** Badge colours per credential type */
function typeColors(credType: string): string {
  switch (credType) {
    case 'api_key':           return 'text-indigo-400 bg-indigo-400/10 border-indigo-400/20';
    case 'oauth_token':       return 'text-purple-400 bg-purple-400/10 border-purple-400/20';
    case 'connection_string': return 'text-sky-400 bg-sky-400/10 border-sky-400/20';
    case 'service_account':   return 'text-amber-400 bg-amber-400/10 border-amber-400/20';
    case 'bearer_token':      return 'text-emerald-400 bg-emerald-400/10 border-emerald-400/20';
    default:                  return 'text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800 border-gray-200 dark:border-zinc-700';
  }
}

function typeLabel(credType: string): string {
  return CREDENTIAL_TYPES.find(t => t.value === credType)?.label ?? credType;
}

// ── Stat card ─────────────────────────────────────────────────────────────────

function StatCard({ label, value, sub }: { label: string; value: string | number; sub?: string }) {
  return (
    <div className="border-l-2 border-indigo-500 pl-3 py-0.5">
      <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[20px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight">{value}</p>
      {sub && <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">{sub}</p>}
    </div>
  );
}

// ── Delete confirmation dialog ────────────────────────────────────────────────

function DeleteDialog({
  credential,
  onConfirm,
  onCancel,
  isPending,
}: {
  credential: CredentialSummary;
  onConfirm: () => void;
  onCancel: () => void;
  isPending: boolean;
}) {
  const trapRef = useFocusTrap({ onClose: onCancel });
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60" onClick={onCancel}>
      <div
        className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg w-full max-w-md mx-4 shadow-2xl"
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        onClick={e => e.stopPropagation()}
      >
        <div className="flex items-start gap-3 p-5">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-red-500/10 border border-red-500/20">
            <AlertTriangle size={14} className="text-red-400" />
          </div>
          <div className="flex-1 min-w-0">
            <p className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">Revoke credential?</p>
            <p className="text-[12px] text-gray-500 dark:text-zinc-400 mt-1">
              <span className="font-mono text-gray-700 dark:text-zinc-300">{credential.name || credential.provider_id}</span>
              {' '}will be revoked. The record is retained for audit history but the credential
              will no longer be usable by any provider.
            </p>
            <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-2">This action cannot be undone.</p>
          </div>
        </div>

        <div className="flex justify-end gap-2 px-5 pb-4">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors"
          >
            Cancel
          </button>
          <button
            onClick={onConfirm}
            disabled={isPending}
            className="px-3 py-1.5 rounded bg-red-600 text-white text-[12px] hover:bg-red-500 disabled:opacity-50 transition-colors flex items-center gap-1.5"
          >
            {isPending && <Loader2 size={11} className="animate-spin" />}
            {isPending ? 'Revoking…' : 'Revoke'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Add credential modal ──────────────────────────────────────────────────────

interface AddCredentialFormState {
  tenant_id: string;
  provider_id: string;
  cred_type: CredentialTypeValue;
  plaintext_value: string;
  key_id: string;
}

function AddCredentialModal({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: () => void;
}) {
  const formId = useId();

  const [form, setForm] = useState<AddCredentialFormState>({
    tenant_id:       DEFAULT_TENANT,
    provider_id:     '',
    cred_type:       'api_key',
    plaintext_value: '',
    key_id:          '',
  });
  const [showValue, setShowValue] = useState(false);
  const [fieldErr,  setFieldErr]  = useState<Partial<Record<keyof AddCredentialFormState, string>>>({});

  const { mutate, isPending, error: mutErr } = useMutation({
    mutationFn: ({ tenantId, body }: { tenantId: string; body: StoreCredentialRequest }) =>
      defaultApi.storeCredential(tenantId, body),
    onSuccess: () => {
      onCreated();
      onClose();
    },
  });

  function set<K extends keyof AddCredentialFormState>(key: K, value: AddCredentialFormState[K]) {
    setForm(f => ({ ...f, [key]: value }));
    setFieldErr(e => { const copy = { ...e }; delete copy[key]; return copy; });
  }

  function validate(): boolean {
    const errs: Partial<Record<keyof AddCredentialFormState, string>> = {};
    if (!form.tenant_id.trim())     errs.tenant_id     = 'Tenant ID is required';
    if (!form.provider_id.trim())   errs.provider_id   = 'Provider ID is required';
    if (!form.plaintext_value.trim()) errs.plaintext_value = 'Secret value is required';
    setFieldErr(errs);
    return Object.keys(errs).length === 0;
  }

  function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!validate()) return;
    const body: StoreCredentialRequest = {
      provider_id:     form.provider_id.trim(),
      plaintext_value: form.plaintext_value,
      ...(form.key_id.trim() ? { key_id: form.key_id.trim() } : {}),
    };
    mutate({ tenantId: form.tenant_id.trim(), body });
  }

  const displayErr = mutErr instanceof Error ? mutErr.message : null;

  const trapRef = useFocusTrap({ onClose: onClose });
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60"
      onClick={onClose}
    >
      <div
        className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg w-full max-w-lg mx-4 shadow-2xl"
        ref={trapRef}
        role="dialog"
        aria-modal="true"
        onClick={e => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-gray-200 dark:border-zinc-800">
          <div className="flex items-center gap-2">
            <KeyRound size={14} className="text-indigo-400" />
            <span className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">Add Credential</span>
          </div>
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:text-zinc-300 transition-colors">
            <X size={14} />
          </button>
        </div>

        {/* Security notice */}
        <div className="mx-5 mt-4 flex items-start gap-2 rounded-md border border-amber-500/20 bg-amber-500/5 px-3 py-2">
          <Lock size={12} className="mt-0.5 shrink-0 text-amber-400" />
          <p className="text-[11px] text-amber-300/80 leading-relaxed">
            The secret value is transmitted once over TLS and encrypted at rest (RFC 011).
            It is <strong>never returned</strong> by the API after creation.
          </p>
        </div>

        {/* Form */}
        <form id={formId} onSubmit={submit} className="p-5 space-y-4">
          {/* Scope row: tenant_id */}
          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">
              Tenant <span className="text-red-400">*</span>
            </label>
            <input
              type="text"
              value={form.tenant_id}
              onChange={e => set('tenant_id', e.target.value)}
              placeholder="default"
              className={clsx(
                'w-full h-8 bg-white dark:bg-zinc-950 border rounded-md px-3 text-[12px] text-gray-800 dark:text-zinc-200',
                'placeholder-zinc-600 focus:outline-none transition-colors',
                fieldErr.tenant_id
                  ? 'border-red-500/60 focus:border-red-500'
                  : 'border-gray-200 dark:border-zinc-800 focus:border-indigo-500',
              )}
            />
            {fieldErr.tenant_id && (
              <p className="mt-1 text-[11px] text-red-400">{fieldErr.tenant_id}</p>
            )}
          </div>

          {/* Two-column: Provider ID + Type */}
          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">
                Provider ID <span className="text-red-400">*</span>
              </label>
              <input
                type="text"
                value={form.provider_id}
                onChange={e => set('provider_id', e.target.value)}
                placeholder="openai-production"
                className={clsx(
                  'w-full h-8 bg-white dark:bg-zinc-950 border rounded-md px-3 text-[12px] text-gray-800 dark:text-zinc-200',
                  'placeholder-zinc-600 focus:outline-none transition-colors',
                  fieldErr.provider_id
                    ? 'border-red-500/60 focus:border-red-500'
                    : 'border-gray-200 dark:border-zinc-800 focus:border-indigo-500',
                )}
              />
              {fieldErr.provider_id && (
                <p className="mt-1 text-[11px] text-red-400">{fieldErr.provider_id}</p>
              )}
            </div>

            <div>
              <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">Type</label>
              <select
                value={form.cred_type}
                onChange={e => set('cred_type', e.target.value as CredentialTypeValue)}
                className="w-full h-8 bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-md px-2 text-[12px] text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 transition-colors"
              >
                {CREDENTIAL_TYPES.map(({ value, label }) => (
                  <option key={value} value={value}>{label}</option>
                ))}
              </select>
              <p className="mt-1 text-[10px] text-gray-300 dark:text-zinc-700">Informational — backend derives type from provider_id</p>
            </div>
          </div>

          {/* Secret value */}
          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">
              Secret Value <span className="text-red-400">*</span>
            </label>
            <div className="relative">
              <input
                type={showValue ? 'text' : 'password'}
                value={form.plaintext_value}
                onChange={e => set('plaintext_value', e.target.value)}
                placeholder="sk-…"
                autoComplete="new-password"
                className={clsx(
                  'w-full h-8 bg-white dark:bg-zinc-950 border rounded-md pl-3 pr-9 text-[12px] text-gray-800 dark:text-zinc-200',
                  'placeholder-zinc-600 focus:outline-none transition-colors font-mono',
                  fieldErr.plaintext_value
                    ? 'border-red-500/60 focus:border-red-500'
                    : 'border-gray-200 dark:border-zinc-800 focus:border-indigo-500',
                )}
              />
              <button
                type="button"
                onClick={() => setShowValue(v => !v)}
                className="absolute right-2.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:text-zinc-400 transition-colors"
                tabIndex={-1}
              >
                {showValue ? <EyeOff size={12} /> : <Eye size={12} />}
              </button>
            </div>
            {fieldErr.plaintext_value && (
              <p className="mt-1 text-[11px] text-red-400">{fieldErr.plaintext_value}</p>
            )}
            <p className="mt-1 text-[10px] text-gray-300 dark:text-zinc-700">
              Entered once — not retrievable after creation.
            </p>
          </div>

          {/* Optional key_id */}
          <div>
            <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1.5">
              Encryption Key ID <span className="text-gray-300 dark:text-zinc-700">(optional)</span>
            </label>
            <input
              type="text"
              value={form.key_id}
              onChange={e => set('key_id', e.target.value)}
              placeholder="key_prod_v2"
              className="w-full h-8 bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-md px-3 text-[12px] text-gray-800 dark:text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-indigo-500 transition-colors font-mono"
            />
            <p className="mt-1 text-[10px] text-gray-300 dark:text-zinc-700">
              Leave blank to use the default tenant encryption key.
            </p>
          </div>

          {displayErr && (
            <p className="text-[11px] text-red-400 font-mono">{displayErr}</p>
          )}
        </form>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-5 pb-5">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors"
          >
            Cancel
          </button>
          <button
            type="submit"
            form={formId}
            disabled={isPending}
            className="px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 disabled:opacity-50 transition-colors flex items-center gap-1.5"
          >
            {isPending && <Loader2 size={11} className="animate-spin" />}
            {isPending ? 'Storing…' : 'Store Credential'}
          </button>
        </div>
      </div>
    </div>
  );
}

// ── Credential row ────────────────────────────────────────────────────────────

function CredentialRow({
  cred,
  even,
  onRevoke,
}: {
  cred: CredentialSummary;
  even: boolean;
  onRevoke: (cred: CredentialSummary) => void;
}) {
  const encrypted = !!cred.encrypted_at_ms;

  return (
    <div
      className={clsx(
        'flex items-center gap-0 border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0 h-10',
        even ? 'bg-gray-50 dark:bg-zinc-900' : 'bg-gray-50/50 dark:bg-zinc-900/50',
        !cred.active && 'opacity-50',
      )}
    >
      {/* Name / provider_id */}
      <div className="flex-1 min-w-0 flex items-center gap-2 px-4">
        <span className="text-[12px] font-medium text-gray-800 dark:text-zinc-200 truncate">
          {cred.name || cred.provider_id}
        </span>
        {!cred.active && (
          <span className="shrink-0 text-[10px] text-red-400 bg-red-400/10 border border-red-400/20 px-1.5 py-0.5 rounded">
            revoked
          </span>
        )}
      </div>

      {/* Type badge */}
      <div className="w-36 shrink-0 px-2">
        <span className={clsx(
          'inline-flex items-center px-1.5 py-0.5 rounded border text-[10px] font-medium',
          typeColors(cred.credential_type),
        )}>
          {typeLabel(cred.credential_type)}
        </span>
      </div>

      {/* Scope (tenant) */}
      <div className="w-28 shrink-0 px-2">
        <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-500 truncate" title={cred.tenant_id}>
          {cred.tenant_id}
        </span>
      </div>

      {/* Encrypted indicator */}
      <div className="w-24 shrink-0 px-2 flex items-center gap-1.5">
        {encrypted ? (
          <>
            <Lock size={10} className="text-emerald-400 shrink-0" />
            <span className="text-[11px] text-emerald-400">Encrypted</span>
          </>
        ) : (
          <span className="text-[11px] text-amber-400">Plaintext</span>
        )}
      </div>

      {/* Created at */}
      <div className="w-28 shrink-0 px-2">
        <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">
          {fmtDate(cred.created_at)}
        </span>
      </div>

      {/* Last rotated */}
      <div className="w-28 shrink-0 px-2">
        <span className="text-[11px] text-gray-400 dark:text-zinc-600 tabular-nums">
          {fmtRelative(cred.revoked_at_ms ?? (encrypted ? cred.encrypted_at_ms : null))}
        </span>
      </div>

      {/* Actions */}
      <div className="w-20 shrink-0 px-2 flex justify-end">
        {cred.active && (
          <button
            onClick={() => onRevoke(cred)}
            title="Revoke credential"
            className="flex items-center gap-1 px-2 py-1 rounded text-[11px] text-red-500/70 hover:bg-red-500/10 hover:text-red-400 transition-colors"
          >
            <Trash2 size={10} /> Revoke
          </button>
        )}
      </div>
    </div>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function CredentialsPage() {
  const [tenantId,    setTenantId]    = useState(DEFAULT_TENANT);
  const [showAdd,     setShowAdd]     = useState(false);
  const [revokeTarget, setRevokeTarget] = useState<CredentialSummary | null>(null);
  const queryClient = useQueryClient();

  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ['credentials', tenantId],
    queryFn:  () => defaultApi.getCredentials(tenantId, { limit: 200 }),
    refetchInterval: 60_000,
  });

  const { mutate: revoke, isPending: isRevoking } = useMutation({
    mutationFn: (cred: CredentialSummary) =>
      defaultApi.revokeCredential(cred.tenant_id, cred.id),
    onSuccess: () => {
      setRevokeTarget(null);
      queryClient.invalidateQueries({ queryKey: ['credentials', tenantId] });
    },
  });

  const creds      = data?.items ?? [];
  const active     = creds.filter(c => c.active);
  const encrypted  = active.filter(c => !!c.encrypted_at_ms);
  const typeSet    = new Set(active.map(c => c.credential_type));

  if (isError) return (
    <div className="flex flex-col items-center justify-center min-h-64 gap-3 p-8 text-center">
      <ServerCrash size={32} className="text-red-500" />
      <p className="text-[13px] text-gray-700 dark:text-zinc-300 font-medium">Failed to load credentials</p>
      <p className="text-[12px] text-gray-400 dark:text-zinc-500">
        {error instanceof Error ? error.message : 'Unknown error'}
      </p>
      <button
        onClick={() => refetch()}
        className="mt-1 px-3 py-1.5 rounded bg-gray-100 dark:bg-zinc-800 text-gray-700 dark:text-zinc-300 text-[12px] hover:bg-gray-200 dark:hover:bg-zinc-700 transition-colors"
      >
        Retry
      </button>
    </div>
  );

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-zinc-900">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-gray-50 dark:bg-zinc-900">
        <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">
          Credentials
          {!isLoading && (
            <span className="ml-2 text-[12px] text-gray-400 dark:text-zinc-500 font-normal">
              {active.length} active
            </span>
          )}
        </span>

        {/* Tenant selector */}
        <div className="flex items-center gap-1.5 ml-4">
          <span className="text-[11px] text-gray-400 dark:text-zinc-600">Tenant:</span>
          <input
            type="text"
            value={tenantId}
            onChange={e => setTenantId(e.target.value || DEFAULT_TENANT)}
            className="h-6 w-32 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-2 text-[11px] font-mono text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500 transition-colors"
          />
        </div>

        <button
          onClick={() => setShowAdd(true)}
          className="ml-auto flex items-center gap-1.5 px-2.5 py-1 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 transition-colors"
        >
          <Plus size={11} /> Add Credential
        </button>
        <button
          onClick={() => refetch()}
          disabled={isFetching}
          className="flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:text-zinc-300 disabled:opacity-40 transition-colors"
        >
          <RefreshCw size={11} className={isFetching ? 'animate-spin' : ''} />
          Refresh
        </button>
      </div>

      {/* Stat strip */}
      {!isLoading && (
        <div className="grid grid-cols-3 gap-x-6 px-5 py-3 border-b border-gray-200 dark:border-zinc-800 shrink-0">
          <StatCard label="Total"     value={active.length}    sub={`${creds.length - active.length} revoked`} />
          <StatCard label="Encrypted" value={encrypted.length} sub={active.length > 0 ? `${Math.round(encrypted.length / active.length * 100)}% of active` : undefined} />
          <StatCard label="Types"     value={typeSet.size}     sub={Array.from(typeSet).join(', ') || '—'} />
        </div>
      )}

      {/* Table */}
      <div className="flex-1 overflow-x-auto overflow-y-auto">
        {isLoading ? (
          <div className="flex items-center justify-center min-h-48 gap-2 text-gray-400 dark:text-zinc-600">
            <Loader2 size={16} className="animate-spin" />
            <span className="text-[13px]">Loading…</span>
          </div>
        ) : creds.length === 0 ? (
          <div className="flex flex-col items-center justify-center min-h-64 gap-3 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-xl bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
              <KeyRound size={24} className="text-gray-400 dark:text-zinc-500" />
            </div>
            <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No credentials stored</p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-xs">
              Add a credential to give providers access to external APIs and services.
              Secrets are encrypted at rest (RFC 011).
            </p>
            <button
              onClick={() => setShowAdd(true)}
              className="mt-1 flex items-center gap-1.5 px-3 py-1.5 rounded bg-indigo-600 text-white text-[12px] hover:bg-indigo-500 transition-colors"
            >
              <Plus size={11} /> Add Credential
            </button>
          </div>
        ) : (
          <div className="min-w-[700px]">
            {/* Column headers */}
            <div className="flex items-center gap-0 h-8 border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 sticky top-0">
              <div className="flex-1 min-w-0 px-4">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Name</span>
              </div>
              <div className="w-36 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Type</span>
              </div>
              <div className="w-28 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Scope</span>
              </div>
              <div className="w-24 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Encrypted</span>
              </div>
              <div className="w-28 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Created</span>
              </div>
              <div className="w-28 shrink-0 px-2">
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Last Rotated</span>
              </div>
              <div className="w-20 shrink-0 px-2" />
            </div>

            {creds.map((cred, i) => (
              <CredentialRow
                key={cred.id}
                cred={cred}
                even={i % 2 === 0}
                onRevoke={setRevokeTarget}
              />
            ))}
          </div>
        )}
      </div>

      {/* Modals */}
      {showAdd && (
        <AddCredentialModal
          onClose={() => setShowAdd(false)}
          onCreated={() => queryClient.invalidateQueries({ queryKey: ['credentials', tenantId] })}
        />
      )}

      {revokeTarget && (
        <DeleteDialog
          credential={revokeTarget}
          onConfirm={() => revoke(revokeTarget)}
          onCancel={() => setRevokeTarget(null)}
          isPending={isRevoking}
        />
      )}
    </div>
  );
}

export default CredentialsPage;
