import { LayoutDashboard } from 'lucide-react';

interface NotFoundPageProps {
  onNavigate?: () => void;
}

export function NotFoundPage({ onNavigate }: NotFoundPageProps) {
  function goHome() {
    window.location.hash = 'dashboard';
    onNavigate?.();
  }

  return (
    <div className="flex h-full items-center justify-center bg-zinc-950">
      <div className="flex flex-col items-center gap-6 text-center select-none">
        {/* Large 404 */}
        <div className="relative">
          <span className="text-[120px] font-black leading-none text-zinc-900 tracking-tighter tabular-nums">
            404
          </span>
          <span className="absolute inset-0 flex items-center justify-center text-[120px] font-black leading-none tracking-tighter tabular-nums bg-gradient-to-b from-zinc-600 to-zinc-800 bg-clip-text text-transparent">
            404
          </span>
        </div>

        {/* Message */}
        <div className="space-y-1.5">
          <p className="text-[16px] font-semibold text-zinc-300">Page not found</p>
          <p className="text-[13px] text-zinc-600 max-w-xs">
            The page you're looking for doesn't exist or was moved.
          </p>
        </div>

        {/* CTA */}
        <button
          onClick={goHome}
          className="flex items-center gap-2 rounded-md bg-indigo-600 hover:bg-indigo-500
                     text-white text-[13px] font-medium px-4 py-2 transition-colors"
        >
          <LayoutDashboard size={14} />
          Go to Dashboard
        </button>
      </div>
    </div>
  );
}

export default NotFoundPage;
