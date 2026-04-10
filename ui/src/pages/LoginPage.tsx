import { useState, type FormEvent } from 'react';
import { Loader2, AlertCircle, Eye, EyeOff } from 'lucide-react';
import { setStoredToken, createApiClient } from '../lib/api';

interface LoginPageProps {
  onLogin: () => void;
}

export function LoginPage({ onLogin }: LoginPageProps) {
  const [token,     setToken]     = useState('');
  const [loading,   setLoading]   = useState(false);
  const [error,     setError]     = useState<string | null>(null);
  const [showToken, setShowToken] = useState(false);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const trimmed = token.trim();
    if (!trimmed) return;

    setLoading(true);
    setError(null);

    try {
      const client = createApiClient({
        baseUrl: import.meta.env.VITE_API_URL ?? '',
        token: trimmed,
      });
      // Two-step validation: health probe (public) then status (requires auth).
      await client.getHealth();
      await client.getStatus();
      setStoredToken(trimmed);
      onLogin();
    } catch (err: unknown) {
      const status = (err as { status?: number }).status;
      if (status === 401 || status === 403) {
        setError('Invalid token');
      } else {
        setError('Cannot reach server — is cairn-app running?');
      }
    } finally {
      setLoading(false);
    }
  }

  const server = (import.meta.env.VITE_API_URL ?? 'http://localhost:3000').replace(/^https?:\/\//, '');
  const isDev  = server.startsWith('localhost') || server.startsWith('127.');

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-white dark:bg-zinc-950">
      <div className="w-full max-w-[360px] px-4 flex flex-col items-center">

        {/* Wordmark */}
        <div className="flex flex-col items-center gap-3 mb-10">
          <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-indigo-600 shadow-lg shadow-indigo-600/30">
            <svg width="18" height="18" viewBox="0 0 18 18" fill="none">
              <rect x="2"  y="2"  width="6" height="6" rx="1.5" fill="white" opacity="0.9"/>
              <rect x="10" y="2"  width="6" height="6" rx="1.5" fill="white" opacity="0.55"/>
              <rect x="2"  y="10" width="6" height="6" rx="1.5" fill="white" opacity="0.55"/>
              <rect x="10" y="10" width="6" height="6" rx="1.5" fill="white" opacity="0.9"/>
            </svg>
          </div>
          <div className="text-center">
            <h1 className="text-[24px] font-semibold text-gray-900 dark:text-zinc-100 tracking-tight leading-none">
              cairn
            </h1>
            <p className="text-[13px] text-gray-400 dark:text-zinc-500 mt-1.5">Operator Control Plane</p>
          </div>
        </div>

        {/* Card */}
        <div className="w-full rounded-xl bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 shadow-2xl shadow-black/40 overflow-hidden">

          {/* Card header */}
          <div className="px-6 pt-5 pb-4 border-b border-gray-200/60 dark:border-zinc-800/60">
            <p className="text-[13px] font-semibold text-gray-900 dark:text-zinc-100">Sign in to your workspace</p>
            <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-0.5">
              Enter your admin bearer token to continue.
            </p>
          </div>

          {/* Form */}
          <form onSubmit={handleSubmit} className="px-6 py-5 space-y-4">
            <div className="space-y-1.5">
              <label htmlFor="token" className="block text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">
                Admin Token
              </label>
              <div className="relative">
                <input
                  id="token"
                  type={showToken ? 'text' : 'password'}
                  autoComplete="current-password"
                  autoFocus
                  value={token}
                  onChange={e => { setToken(e.target.value); setError(null); }}
                  placeholder={isDev ? 'dev-admin-token' : 'Bearer token…'}
                  className="w-full h-9 rounded-lg bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-700/80
                             pl-3 pr-9 text-[13px] text-gray-900 dark:text-zinc-100 font-mono placeholder-zinc-700
                             focus:outline-none focus:border-indigo-500 focus:ring-1
                             focus:ring-indigo-500/50 transition-colors"
                />
                <button
                  type="button"
                  tabIndex={-1}
                  onClick={() => setShowToken(v => !v)}
                  className="absolute right-2.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors"
                  aria-label={showToken ? 'Hide token' : 'Show token'}
                >
                  {showToken ? <EyeOff size={13} /> : <Eye size={13} />}
                </button>
              </div>
            </div>

            {/* Error state */}
            {error && (
              <div className="flex items-start gap-2 rounded-lg bg-red-950/40 border border-red-800/40 px-3 py-2.5">
                <AlertCircle size={13} className="text-red-400 shrink-0 mt-0.5" />
                <p className="text-[12px] text-red-300 leading-snug">{error}</p>
              </div>
            )}

            {/* Dev shortcut */}
            {isDev && !token && !error && (
              <p className="text-[11px] text-gray-300 dark:text-zinc-600">
                Dev mode — try{' '}
                <button
                  type="button"
                  onClick={() => setToken('dev-admin-token')}
                  className="font-mono text-gray-400 dark:text-zinc-600 hover:text-indigo-400 underline underline-offset-2 transition-colors"
                >
                  dev-admin-token
                </button>
              </p>
            )}

            {/* Submit */}
            <button
              type="submit"
              disabled={loading || !token.trim()}
              className="w-full h-9 flex items-center justify-center gap-2 rounded-lg
                         bg-indigo-600 hover:bg-indigo-500 active:bg-indigo-700
                         disabled:bg-gray-100 dark:bg-zinc-800 disabled:text-gray-400 dark:text-zinc-600 disabled:cursor-not-allowed
                         text-white text-[13px] font-medium
                         transition-colors focus:outline-none focus:ring-2
                         focus:ring-indigo-500 focus:ring-offset-1 focus:ring-offset-zinc-900"
            >
              {loading && <Loader2 size={13} className="animate-spin" />}
              {loading ? 'Signing in…' : 'Sign In'}
            </button>
          </form>
        </div>

        {/* Server info */}
        <p className="mt-5 text-[11px] text-gray-300 dark:text-zinc-600">
          {server}
        </p>
      </div>
    </div>
  );
}

export default LoginPage;
