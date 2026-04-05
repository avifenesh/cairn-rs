import { Component, type ReactNode, type ErrorInfo } from 'react';
import { AlertTriangle, RotateCcw, ChevronDown } from 'lucide-react';
import { clsx } from 'clsx';

interface Props {
  children: ReactNode;
  /** Optional page name shown in the error card header. */
  name?: string;
}

interface State {
  error: Error | null;
  showStack: boolean;
}

/**
 * React error boundary that catches render errors in its subtree.
 *
 * Shows a clean error card with a 'Try Again' reset button and a
 * collapsible stack trace for debugging.
 */
export class ErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { error: null, showStack: false };
  }

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // Forward to an error reporting service in production; log in dev.
    if (import.meta.env.DEV) {
      // eslint-disable-next-line no-console
      console.error('[ErrorBoundary]', error, info.componentStack);
    }
  }

  reset = () => this.setState({ error: null, showStack: false });

  toggleStack = () => this.setState(s => ({ showStack: !s.showStack }));

  render() {
    if (!this.state.error) return this.props.children;

    const { error, showStack } = this.state;
    const { name } = this.props;

    return (
      <div className="flex items-start justify-center min-h-48 pt-12 px-6">
        <div className="w-full max-w-xl">
          {/* Card */}
          <div className="rounded-lg bg-zinc-900 border border-zinc-800 border-l-2 border-l-red-500 overflow-hidden">
            {/* Header */}
            <div className="flex items-start gap-3 px-4 py-3.5 border-b border-zinc-800">
              <AlertTriangle size={15} className="text-red-400 shrink-0 mt-0.5" />
              <div className="min-w-0 flex-1">
                <p className="text-[13px] font-medium text-zinc-200">
                  {name ? `${name} failed to render` : 'Something went wrong'}
                </p>
                <p className="text-[12px] text-zinc-500 mt-0.5 break-words">
                  {error.message || 'An unexpected error occurred.'}
                </p>
              </div>
            </div>

            {/* Stack trace (collapsible) */}
            {error.stack && (
              <div className="border-b border-zinc-800">
                <button
                  onClick={this.toggleStack}
                  className="w-full flex items-center gap-1.5 px-4 py-2 text-[11px] text-zinc-600
                             hover:text-zinc-400 hover:bg-zinc-800/40 transition-colors text-left"
                >
                  <ChevronDown
                    size={11}
                    className={clsx('transition-transform', showStack ? 'rotate-180' : '')}
                  />
                  {showStack ? 'Hide' : 'Show'} stack trace
                </button>
                {showStack && (
                  <pre className="px-4 pb-3 text-[10px] font-mono text-zinc-600 leading-relaxed
                                  overflow-x-auto whitespace-pre-wrap break-all">
                    {error.stack}
                  </pre>
                )}
              </div>
            )}

            {/* Actions */}
            <div className="flex items-center gap-3 px-4 py-3">
              <button
                onClick={this.reset}
                className="flex items-center gap-1.5 rounded bg-zinc-800 hover:bg-zinc-700
                           border border-zinc-700 text-zinc-300 text-[12px] font-medium
                           px-3 py-1.5 transition-colors"
              >
                <RotateCcw size={12} />
                Try again
              </button>
              <p className="text-[11px] text-zinc-700 ml-auto">
                Reload the page if the problem persists.
              </p>
            </div>
          </div>
        </div>
      </div>
    );
  }
}
