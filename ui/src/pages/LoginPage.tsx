import { useState, type FormEvent } from 'react';
import { KeyRound, Loader2, AlertCircle } from 'lucide-react';
import { setStoredToken, createApiClient } from '../lib/api';

interface LoginPageProps {
  onLogin: () => void;
}

export function LoginPage({ onLogin }: LoginPageProps) {
  const [token, setToken] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
      await client.getHealth();
      setStoredToken(trimmed);
      onLogin();
    } catch {
      setError('Token rejected — check the value and try again.');
    } finally {
      setLoading(false);
    }
  }

  const server = import.meta.env.VITE_API_URL || 'http://localhost:3000';

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-zinc-950">
      <div className="w-full max-w-sm px-4 flex flex-col items-center">

        {/* Logo */}
        <div className="flex items-center gap-2.5 mb-8">
          <span className="inline-flex h-8 w-8 items-center justify-center rounded-lg bg-indigo-600">
            <span className="block h-2.5 w-2.5 rounded-full bg-white opacity-90" />
          </span>
          <span className="text-zinc-100 font-semibold text-xl tracking-tight">cairn</span>
        </div>

        {/* Card */}
        <div className="w-full rounded-xl bg-zinc-900 border border-zinc-800 p-6 space-y-5">
          <div>
            <h1 className="text-sm font-semibold text-zinc-100">Connect to server</h1>
            <p className="text-xs text-zinc-500 mt-1">
              Enter the admin bearer token to authenticate.
            </p>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4">
            {/* Token input */}
            <div className="space-y-1.5">
              <label htmlFor="token" className="block text-xs font-medium text-zinc-400">
                Admin token
              </label>
              <div className="relative">
                <KeyRound
                  size={13}
                  className="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-600 pointer-events-none"
                />
                <input
                  id="token"
                  type="password"
                  autoComplete="current-password"
                  autoFocus
                  value={token}
                  onChange={(e) => { setToken(e.target.value); setError(null); }}
                  placeholder="cairn-demo-token"
                  className="w-full h-10 rounded-lg bg-zinc-950 border border-zinc-700
                             pl-9 pr-3 text-sm text-zinc-100 placeholder-zinc-600
                             focus:outline-none focus:border-indigo-500 focus:ring-1
                             focus:ring-indigo-500 transition-colors"
                />
              </div>
            </div>

            {/* Error */}
            {error && (
              <div className="flex items-center gap-2 rounded-lg bg-red-950/50 border border-red-800/50 px-3 py-2">
                <AlertCircle size={13} className="text-red-400 shrink-0" />
                <p className="text-xs text-red-300">{error}</p>
              </div>
            )}

            {/* Submit */}
            <button
              type="submit"
              disabled={loading || !token.trim()}
              className="w-full h-10 flex items-center justify-center gap-2 rounded-lg
                         bg-indigo-600 hover:bg-indigo-500 active:bg-indigo-700
                         disabled:bg-zinc-800 disabled:text-zinc-600
                         text-white text-sm font-medium
                         transition-colors focus:outline-none focus:ring-2
                         focus:ring-indigo-500 focus:ring-offset-1 focus:ring-offset-zinc-900"
            >
              {loading
                ? <Loader2 size={14} className="animate-spin" />
                : null}
              {loading ? 'Connecting…' : 'Sign in'}
            </button>
          </form>
        </div>

        {/* Footer */}
        <p className="mt-5 text-xs text-zinc-700">
          Connecting to{' '}
          <span className="text-zinc-600 font-mono">
            {server.replace(/^https?:\/\//, '')}
          </span>
        </p>
      </div>
    </div>
  );
}
