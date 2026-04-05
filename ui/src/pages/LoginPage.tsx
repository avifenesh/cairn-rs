import { useState, type FormEvent } from 'react';
import { KeyRound, ChevronRight, Loader2, AlertCircle } from 'lucide-react';
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
      // Verify the token works before saving.
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

  return (
    <div className="flex h-screen w-screen items-center justify-center bg-zinc-950">
      <div className="w-full max-w-sm px-4">
        {/* Logo / wordmark */}
        <div className="flex items-center gap-2 mb-8 justify-center">
          <div className="w-8 h-8 rounded-lg bg-indigo-500 flex items-center justify-center">
            <ChevronRight size={18} className="text-white" />
          </div>
          <span className="text-zinc-100 font-semibold text-lg tracking-tight">cairn</span>
        </div>

        {/* Card */}
        <div className="rounded-xl bg-zinc-900 ring-1 ring-zinc-800 p-6 space-y-5">
          <div>
            <h1 className="text-sm font-semibold text-zinc-100">Sign in</h1>
            <p className="text-xs text-zinc-500 mt-1">
              Enter the admin token set via{' '}
              <code className="font-mono text-zinc-400">CAIRN_ADMIN_TOKEN</code>.
            </p>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4">
            {/* Token field */}
            <div className="space-y-1.5">
              <label
                htmlFor="token"
                className="block text-xs font-medium text-zinc-400"
              >
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
                  onChange={(e) => setToken(e.target.value)}
                  placeholder="cairn-demo-token"
                  className="w-full rounded-lg bg-zinc-800 border border-zinc-700
                             pl-8 pr-3 py-2 text-sm text-zinc-100 placeholder-zinc-600
                             focus:outline-none focus:ring-2 focus:ring-indigo-500
                             focus:border-transparent transition"
                />
              </div>
            </div>

            {/* Error message */}
            {error && (
              <div className="flex items-center gap-2 rounded-lg bg-red-950/60 ring-1 ring-red-800/60 px-3 py-2">
                <AlertCircle size={13} className="text-red-400 shrink-0" />
                <p className="text-xs text-red-300">{error}</p>
              </div>
            )}

            {/* Submit */}
            <button
              type="submit"
              disabled={loading || !token.trim()}
              className="w-full flex items-center justify-center gap-2 rounded-lg
                         bg-indigo-600 hover:bg-indigo-500 disabled:bg-zinc-800
                         disabled:text-zinc-600 text-white text-sm font-medium
                         py-2 transition focus:outline-none focus:ring-2
                         focus:ring-indigo-500 focus:ring-offset-1
                         focus:ring-offset-zinc-900"
            >
              {loading ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <ChevronRight size={14} />
              )}
              {loading ? 'Verifying…' : 'Connect'}
            </button>
          </form>
        </div>

        <p className="mt-4 text-center text-xs text-zinc-700">
          Server: {import.meta.env.VITE_API_URL || 'http://localhost:3000'}
        </p>
      </div>
    </div>
  );
}
