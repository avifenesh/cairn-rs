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

import { useEffect, useState, useId } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import {
  RefreshCw, Loader2, ServerCrash, Plus, Trash2, X,
  KeyRound, Lock, Eye, EyeOff, AlertTriangle,
} from 'lucide-react';
import { clsx } from 'clsx';
import { defaultApi } from '../lib/api';
import { useScope } from '../hooks/useScope';
import { DEFAULT_SCOPE } from '../lib/scope';
import { useFocusTrap } from '../hooks/useFocusTrap';
import type { CredentialSummary, StoreCredentialRequest } from '../lib/types';
import { Badge } from '../components/Badge';
import { FormField, fieldInputMono } from '../components/FormField';
import { StatCard } from '../components/StatCard';
import { ds } from '../lib/design-system';
import { useToast } from '../components/Toast';

// ── Constants ─────────────────────────────────────────────────────────────────

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


function typeLabel(credType: string): string {
  return CREDENTIAL_TYPES.find(t => t.value === credType)?.label ?? credType;
}

/** Maps credential type to a Badge variant. */
function credTypeBadgeVariant(credType: string): "info" | "purple" | "sky" | "warning" | "success" | "neutral" {
  switch (credType) {
    case 'api_key':           return 'info';
    case 'oauth_token':       return 'purple';
    case 'connection_string': return 'sky';
    case 'service_account':   return 'warning';
    case 'bearer_token':      return 'success';
    default:                  return 'neutral';
  }
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
    <div className={ds.modal.backdrop} onClick={onCancel}>
      <div
        className={clsx(ds.modal.container, "w-full max-w-md mx-4 shadow-2xl")}
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
  initialTenantId,
  onClose,
  onCreated,
}: {
  initialTenantId: string;
  onClose: () => void;
  onCreated: () => void;
}) {
  const formId = useId();
  const toast = useToast();

  const [form, setForm] = useState<AddCredentialFormState>({
    tenant_id:       initialTenantId,
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
    onError: (e: unknown) => toast.error(e instanceof Error ? e.message : 'Failed to store credential.'),
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
      className={ds.modal.backdrop}
      onClick={onClose}
    >
      <div
        className={clsx(ds.modal.container, "w-full max-w-lg mx-4 shadow-2xl")}
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
          <button onClick={onClose} className="text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors">
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
          <FormField label="Tenant" required error={fieldErr.tenant_id}>
            <input
              type="text"
              value={form.tenant_id}
              onChange={e => set('tenant_id', e.target.value)}
              placeholder={DEFAULT_SCOPE.tenant_id}
              className={clsx(
                fieldInputMono,
                fieldErr.tenant_id && 'border-red-500/60 focus:border-red-500',
              )}
            />
          </FormField>

          {/* Two-column: Provider ID + Type */}
          <div className="grid grid-cols-2 gap-3">
            <FormField label="Provider ID" required error={fieldErr.provider_id}>
              <input
                type="text"
                value={form.provider_id}
                onChange={e => set('provider_id', e.target.value)}
                placeholder="openai-production"
                className={clsx(
                  fieldInputMono,
                  fieldErr.provider_id && 'border-red-500/60 focus:border-red-500',
                )}
              />
            </FormField>

            <FormField label="Type" helper="Informational — backend derives type from provider_id">
              <select
                value={form.cred_type}
                onChange={e => set('cred_type', e.target.value as CredentialTypeValue)}
                className="w-full h-8 bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-md px-2 text-[12px] text-gray-800 dark:text-zinc-200 focus:outline-none focus:border-indigo-500 transition-colors"
              >
                {CREDENTIAL_TYPES.map(({ value, label }) => (
                  <option key={value} value={value}>{label}</option>
                ))}
              </select>
            </FormField>
          </div>

          {/* Secret value */}
          <FormField label="Secret Value" required error={fieldErr.plaintext_value} helper="Entered once — not retrievable after creation.">
            <div className="relative">
              <input
                type={showValue ? 'text' : 'password'}
                value={form.plaintext_value}
                onChange={e => set('plaintext_value', e.target.value)}
                placeholder="sk-…"
                autoComplete="new-password"
                className={clsx(
                  fieldInputMono, 'pr-9',
                  fieldErr.plaintext_value && 'border-red-500/60 focus:border-red-500',
                )}
              />
              <button
                type="button"
                onClick={() => setShowValue(v => !v)}
                className="absolute right-2.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors"
                tabIndex={-1}
              >
                {showValue ? <EyeOff size={12} /> : <Eye size={12} />}
              </button>
            </div>
          </FormField>

          {/* Optional key_id */}
          <FormField label="Encryption Key ID" helper="Leave blank to use the default tenant encryption key.">
            <input
              type="text"
              value={form.key_id}
              onChange={e => set('key_id', e.target.value)}
              placeholder="key_prod_v2"
              className={fieldInputMono}
            />
          </FormField>

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
        'flex items-center gap-0 h-10',
        ds.table.rowBorder,
        even ? ds.table.rowEven : ds.table.rowOdd,
        !cred.active && 'opacity-50',
      )}
    >
      {/* Name / provider_id */}
      <div className="flex-1 min-w-0 flex items-center gap-2 px-4">
        <span className="text-[12px] font-medium text-gray-800 dark:text-zinc-200 truncate">
          {cred.name || cred.provider_id}
        </span>
        {!cred.active && (
          <Badge variant="danger" outlined compact>revoked</Badge>
        )}
      </div>

      {/* Type badge */}
      <div className="w-36 shrink-0 px-2">
        <Badge variant={credTypeBadgeVariant(cred.credential_type)} outlined compact>
          {typeLabel(cred.credential_type)}
        </Badge>
      </div>

      {/* Scope (tenant) */}
      <div className="w-28 shrink-0 px-2">
        <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-500 truncate" title={cred.tenant_id}>
          {cred.tenant_id}
        </span>
      </div>

      {/* Encrypted indicator — both variants use a leading dot so the
          visual weight matches, preventing the "one has a dot, the other
          doesn't" inconsistency flagged in #251. */}
      <div className="w-24 shrink-0 px-2 flex items-center gap-1.5">
        {encrypted ? (
          <Badge variant="success" dot compact>Encrypted</Badge>
        ) : (
          <Badge variant="warning" dot compact>Plaintext</Badge>
        )}
      </div>

      {/* Created at */}
      <div className="w-28 shrink-0 px-2">
        <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums">
          {fmtDate(cred.created_at)}
        </span>
      </div>

      {/* Rotated / revoked — two distinct timestamps. Pre-fix the column
          conflated `revoked_at_ms` with `encrypted_at_ms` so a revoked
          credential would show a misleading "rotation" time. Now each is
          rendered only when its timestamp is set. */}
      <div className="w-28 shrink-0 px-2 flex flex-col gap-0.5">
        {encrypted && cred.encrypted_at_ms && (
          <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums" title="Last time the credential was encrypted or rotated">
            Rotated: {fmtRelative(cred.encrypted_at_ms)}
          </span>
        )}
        {cred.revoked_at_ms && (
          <span className="text-[11px] text-red-400 tabular-nums" title="Time the credential was revoked">
            Revoked: {fmtRelative(cred.revoked_at_ms)}
          </span>
        )}
        {!cred.encrypted_at_ms && !cred.revoked_at_ms && (
          <span className="text-[11px] text-gray-300 dark:text-zinc-600">—</span>
        )}
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
  const [scope] = useScope();
  const [tenantId,    setTenantId]    = useState(scope.tenant_id || DEFAULT_SCOPE.tenant_id);
  const [showAdd,     setShowAdd]     = useState(false);
  const [revokeTarget, setRevokeTarget] = useState<CredentialSummary | null>(null);
  const queryClient = useQueryClient();

  useEffect(() => {
    setTenantId(scope.tenant_id || DEFAULT_SCOPE.tenant_id);
  }, [scope.tenant_id]);

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
    <div className={clsx("flex flex-col h-full", ds.surface.pageDense)}>
      {/* Toolbar */}
      <div className={clsx(ds.toolbar.base, ds.surface.pageDense)}>
        <span className={ds.toolbar.title}>
          Credentials
          {!isLoading && (
            <span className={ds.toolbar.count}>
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
            onChange={e => setTenantId(e.target.value || DEFAULT_SCOPE.tenant_id)}
            className="h-6 w-32 bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700 rounded px-2 text-[11px] font-mono text-gray-700 dark:text-zinc-300 focus:outline-none focus:border-indigo-500 transition-colors"
          />
        </div>

        <button
          onClick={() => setShowAdd(true)}
          className={clsx(ds.btn.primary, "ml-auto")}
        >
          <Plus size={11} /> Add Credential
        </button>
        <button
          onClick={() => refetch()}
          disabled={isFetching}
          className={ds.btn.ghost}
        >
          <RefreshCw size={11} className={isFetching ? 'animate-spin' : ''} />
          Refresh
        </button>
      </div>

      {/* Stat strip */}
      {!isLoading && (
        <div className={clsx(ds.spacing.statGrid3, "px-5 py-3 border-b border-gray-200 dark:border-zinc-800 shrink-0")}>
          <StatCard label="Total"     value={active.length}    description={`${creds.length - active.length} revoked`} variant="info" />
          <StatCard label="Encrypted" value={encrypted.length} description={active.length > 0 ? `${Math.round(encrypted.length / active.length * 100)}% of active` : undefined} variant="success" />
          <StatCard label="Types"     value={typeSet.size}     description={Array.from(typeSet).join(', ') || '—'} variant="default" />
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
            <div className="flex h-14 w-14 items-center justify-center rounded-lg bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700">
              <KeyRound size={24} className="text-gray-400 dark:text-zinc-500" />
            </div>
            <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No credentials stored</p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-600 max-w-xs">
              Add a credential to give providers access to external APIs and services.
              Secrets are encrypted at rest (RFC 011).
            </p>
            <button
              onClick={() => setShowAdd(true)}
              className={clsx(ds.btn.primary, "mt-1")}
            >
              <Plus size={11} /> Add Credential
            </button>
          </div>
        ) : (
          <div className="min-w-[700px]">
            {/* Column headers */}
            <div className={clsx("flex items-center gap-0 h-8 border-b border-gray-200 dark:border-zinc-800 sticky top-0", ds.table.headBg)}>
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
                <span className="text-[10px] text-gray-400 dark:text-zinc-600 uppercase tracking-wider">Rotated / Revoked</span>
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
          initialTenantId={tenantId}
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
