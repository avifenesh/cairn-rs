import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import {
  User,
  KeyRound,
  Eye,
  EyeOff,
  Check,
  LogOut,
  Monitor,
  Sun,
  Moon,
  AlignJustify,
  ChevronDown,
  Server,
} from 'lucide-react';
import { clsx } from 'clsx';
import {
  getStoredToken,
  setStoredToken,
  clearStoredToken,
  defaultApi,
} from '../lib/api';
import {
  usePreferences,
  applyTheme,
  type Preferences,
} from '../hooks/usePreferences';
import { useLocale } from '../hooks/useLocale';
import { LOCALE_LABELS, type Locale } from '../lib/i18n';

// ── Small components ──────────────────────────────────────────────────────────

function SectionCard({
  title,
  icon: Icon,
  children,
}: {
  title: string;
  icon: React.ComponentType<{ size?: number; className?: string }>;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-lg bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800">
      <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-zinc-800">
        <Icon size={14} className="text-gray-400 dark:text-zinc-500 shrink-0" />
        <h2 className="text-[13px] font-medium text-gray-800 dark:text-zinc-200">{title}</h2>
      </div>
      <div className="px-4 py-4 space-y-4">{children}</div>
    </div>
  );
}

function Row({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-4">
      <div className="min-w-0">
        <p className="text-[13px] text-gray-700 dark:text-zinc-300">{label}</p>
        {hint && <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">{hint}</p>}
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  );
}

function Toggle({ checked, onChange }: { checked: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      onClick={() => onChange(!checked)}
      className={clsx(
        'relative inline-flex h-5 w-9 items-center rounded-full transition-colors focus-visible:outline focus-visible:outline-2 focus-visible:outline-indigo-500',
        checked ? 'bg-indigo-500' : 'bg-zinc-700',
      )}
    >
      <span
        className={clsx(
          'inline-block h-3.5 w-3.5 transform rounded-full bg-white shadow transition-transform',
          checked ? 'translate-x-[18px]' : 'translate-x-[3px]',
        )}
      />
    </button>
  );
}

function SelectField<T extends string>({
  value,
  onChange,
  options,
}: {
  value: T;
  onChange: (v: T) => void;
  options: { value: T; label: string }[];
}) {
  return (
    <div className="relative">
      <select
        value={value}
        onChange={(e) => onChange(e.target.value as T)}
        className="appearance-none rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-gray-700 dark:text-zinc-300 text-[12px]
                   px-2.5 py-1.5 pr-7 focus:outline-none focus:border-indigo-500 cursor-pointer"
      >
        {options.map((o) => (
          <option key={o.value} value={o.value}>{o.label}</option>
        ))}
      </select>
      <ChevronDown size={11} className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
    </div>
  );
}

// ── Token management ──────────────────────────────────────────────────────────

function TokenSection() {
  const current = getStoredToken();
  const [editing, setEditing] = useState(false);
  const [draft, setDraft]     = useState('');
  const [shown, setShown]     = useState(false);
  const [saved, setSaved]     = useState(false);

  const masked = current.length > 8
    ? `${'•'.repeat(8)}${current.slice(-4)}`
    : '•'.repeat(current.length);

  function handleSave() {
    if (!draft.trim()) return;
    setStoredToken(draft.trim());
    setDraft('');
    setEditing(false);
    setSaved(true);
    setTimeout(() => setSaved(false), 2000);
  }

  function handleLogout() {
    clearStoredToken();
    window.location.reload();
  }

  return (
    <SectionCard title="Token Management" icon={KeyRound}>
      {/* Current token */}
      <Row label="Current token" hint="Used for all API requests">
        <div className="flex items-center gap-2">
          <span className="font-mono text-[12px] text-gray-500 dark:text-zinc-400">
            {shown ? current : masked}
          </span>
          <button
            onClick={() => setShown((v) => !v)}
            className="p-1 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
            title={shown ? 'Hide' : 'Reveal'}
          >
            {shown ? <EyeOff size={13} /> : <Eye size={13} />}
          </button>
        </div>
      </Row>

      {/* Change token */}
      {editing ? (
        <div className="flex items-center gap-2">
          <input
            autoFocus
            type="password"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter') handleSave(); if (e.key === 'Escape') setEditing(false); }}
            placeholder="Paste new token…"
            className="flex-1 rounded border border-gray-200 dark:border-zinc-700 bg-gray-100 dark:bg-zinc-800 text-gray-800 dark:text-zinc-200 text-[12px]
                       px-3 py-1.5 placeholder-zinc-600 focus:outline-none focus:border-indigo-500"
          />
          <button
            onClick={handleSave}
            disabled={!draft.trim()}
            className="flex items-center gap-1.5 rounded bg-indigo-600 hover:bg-indigo-500
                       disabled:opacity-40 text-white text-[12px] font-medium px-3 py-1.5 transition-colors"
          >
            <Check size={12} /> Save
          </button>
          <button
            onClick={() => setEditing(false)}
            className="rounded border border-gray-200 dark:border-zinc-700 text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 text-[12px] px-2.5 py-1.5 transition-colors"
          >
            Cancel
          </button>
        </div>
      ) : (
        <div className="flex items-center gap-2">
          <button
            onClick={() => setEditing(true)}
            className="rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 text-gray-500 dark:text-zinc-400
                       text-[12px] px-3 py-1.5 transition-colors"
          >
            Change token
          </button>
          {saved && (
            <span className="flex items-center gap-1 text-[11px] text-emerald-400">
              <Check size={11} /> Saved
            </span>
          )}
          <button
            onClick={handleLogout}
            className="ml-auto flex items-center gap-1.5 rounded border border-red-900/50 text-red-400
                       hover:bg-red-950/40 text-[12px] px-3 py-1.5 transition-colors"
          >
            <LogOut size={12} /> Sign out
          </button>
        </div>
      )}
    </SectionCard>
  );
}

// ── Display preferences ───────────────────────────────────────────────────────

const THEME_OPTIONS: { value: Preferences['theme']; icon: typeof Sun; label: string }[] = [
  { value: 'dark',   icon: Moon,    label: 'Dark'   },
  { value: 'light',  icon: Sun,     label: 'Light'  },
  { value: 'system', icon: Monitor, label: 'System' },
];

function PreferencesSection() {
  const [prefs, setPrefs] = usePreferences();
  const { locale, setLocale } = useLocale();

  function handleTheme(theme: Preferences['theme']) {
    setPrefs({ theme });
    applyTheme(theme);
  }

  return (
    <SectionCard title="Display Preferences" icon={AlignJustify}>
      {/* Theme */}
      <Row label="Theme" hint="Overrides the OS preference when set explicitly">
        <div className="flex items-center gap-1 rounded border border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 p-0.5">
          {THEME_OPTIONS.map(({ value, icon: Icon, label }) => (
            <button
              key={value}
              onClick={() => handleTheme(value)}
              title={label}
              className={clsx(
                'flex items-center gap-1.5 rounded px-2.5 py-1 text-[12px] font-medium transition-colors',
                prefs.theme === value
                  ? 'bg-gray-100 dark:bg-zinc-800 text-gray-900 dark:text-zinc-100'
                  : 'text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300',
              )}
            >
              <Icon size={12} />
              {label}
            </button>
          ))}
        </div>
      </Row>

      {/* Items per page */}
      <Row label="Items per page" hint="Rows shown in data tables">
        <SelectField
          value={String(prefs.itemsPerPage) as '25' | '50' | '100'}
          onChange={(v) => setPrefs({ itemsPerPage: Number(v) as Preferences['itemsPerPage'] })}
          options={[
            { value: '25',  label: '25 rows'  },
            { value: '50',  label: '50 rows'  },
            { value: '100', label: '100 rows' },
          ]}
        />
      </Row>

      {/* Date format */}
      <Row label="Date format" hint="How timestamps are displayed">
        <SelectField
          value={prefs.dateFormat}
          onChange={(v) => setPrefs({ dateFormat: v })}
          options={[
            { value: 'relative', label: 'Relative (2m ago)'      },
            { value: 'absolute', label: 'Absolute (2026-04-05)'  },
          ]}
        />
      </Row>

      {/* Timezone */}
      <Row label="Timezone" hint="For absolute timestamps; blank = browser default">
        <input
          type="text"
          value={prefs.timezone}
          onChange={(e) => setPrefs({ timezone: e.target.value })}
          placeholder="e.g. America/New_York"
          className="w-44 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-gray-700 dark:text-zinc-300 text-[12px]
                     px-2.5 py-1.5 placeholder-zinc-600 focus:outline-none focus:border-indigo-500"
        />
      </Row>

      {/* Compact mode */}
      <Row
        label="Compact mode"
        hint="Reduces padding in tables and cards for data-dense views"
      >
        <Toggle
          checked={prefs.compactMode}
          onChange={(v) => setPrefs({ compactMode: v })}
        />
      </Row>
      <Row
        label="Auto-refresh"
        hint="Master switch for per-page automatic data polling. When off, pages only update on manual refresh."
      >
        <Toggle
          checked={prefs.autoRefresh ?? true}
          onChange={(v) => {
            setPrefs({ autoRefresh: v });
            try { localStorage.setItem('cairn_refresh_global', String(v)); } catch { /* ignore */ }
          }}
        />
      </Row>
      <Row label="Language" hint="UI display language">
        <SelectField<Locale>
          value={locale}
          onChange={setLocale}
          options={(Object.entries(LOCALE_LABELS) as [Locale, string][]).map(
            ([value, label]) => ({ value, label }),
          )}
        />
      </Row>
    </SectionCard>
  );
}

// ── About ─────────────────────────────────────────────────────────────────────

function AboutSection() {
  const { data: status } = useQuery({
    queryKey: ['status'],
    queryFn:  () => defaultApi.getStatus(),
    refetchInterval: 30_000,
  });
  const { data: info } = useQuery({
    queryKey: ['systemInfo'],
    queryFn:  () => defaultApi.getSystemInfo(),
    // Version/store/deployment rarely change; no refetch interval.
  });

  function fmtUptime(secs: number): string {
    if (secs < 60)    return `${secs}s`;
    if (secs < 3600)  return `${Math.floor(secs / 60)}m ${secs % 60}s`;
    const h = Math.floor(secs / 3600);
    const m = Math.floor((secs % 3600) / 60);
    return `${h}h ${m}m`;
  }

  return (
    <SectionCard title="About" icon={Server}>
      <div className="space-y-0">
        {[
          { label: 'Version',        value: info?.version ? `v${info.version} (cairn-rs)` : '—' },
          { label: 'API endpoint',   value: import.meta.env.VITE_API_URL || 'localhost:3000' },
          { label: 'Deployment',     value: info?.environment?.deployment_mode ?? '—' },
          { label: 'Runtime',        value: status ? (status.status === 'ok' ? 'Healthy' : 'Degraded') : '—',
            ok: status?.status === 'ok' },
          { label: 'Store',          value: info?.features?.store_type ?? '—' },
          { label: 'Store health',   value: status ? ((status.components?.find(c => c.name === 'event_store')?.status === 'ok') ? 'Healthy' : 'Degraded') : '—',
            ok: status?.components?.find(c => c.name === 'event_store')?.status === 'ok' },
          { label: 'Uptime',         value: status ? fmtUptime(status.uptime_secs) : '—' },
        ].map(({ label, value, ok }) => (
          <div key={label} className="flex items-center justify-between py-2 border-b border-gray-200 dark:border-zinc-800 last:border-0">
            <span className="text-[12px] text-gray-400 dark:text-zinc-500">{label}</span>
            <span className={clsx(
              'text-[12px] font-mono',
              ok === true  && 'text-emerald-400',
              ok === false && 'text-red-400',
              ok === undefined && 'text-gray-700 dark:text-zinc-300',
            )}>
              {value}
            </span>
          </div>
        ))}
      </div>
    </SectionCard>
  );
}

// ── Main page ─────────────────────────────────────────────────────────────────

export function ProfilePage() {
  return (
    <div className="h-full overflow-y-auto bg-white dark:bg-zinc-950">
      <div className="max-w-2xl mx-auto px-6 py-6 space-y-5">
        {/* Header */}
        <div className="flex items-center gap-3">
          <div className="w-9 h-9 rounded-full bg-indigo-500/15 border border-indigo-500/30 flex items-center justify-center shrink-0">
            <User size={16} className="text-indigo-400" />
          </div>
          <div>
            <h1 className="text-[14px] font-semibold text-gray-900 dark:text-zinc-100">Account</h1>
            <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">Token, display preferences, and system information</p>
          </div>
        </div>

        <TokenSection />
        <PreferencesSection />
        <AboutSection />
      </div>
    </div>
  );
}

export default ProfilePage;
