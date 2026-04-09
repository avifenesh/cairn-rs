/**
 * CopyButton — an inline icon-only button that copies text to the clipboard.
 *
 * Shows a Check icon briefly after a successful copy.
 *
 * Usage:
 *   <CopyButton text={run.run_id} />
 *   <CopyButton text={run.run_id} label="Copy run ID" size={12} />
 */

import { useState } from 'react';
import { Copy, Check } from 'lucide-react';
import { clsx } from 'clsx';
import { useClipboard } from '../hooks/useClipboard';

interface CopyButtonProps {
  /** The text that will be written to the clipboard. */
  text:      string;
  /** Accessible label (used as `aria-label` and `title`). Default: "Copy". */
  label?:    string;
  /** Icon size in px. Default: 11. */
  size?:     number;
  /** Extra class names on the <button>. */
  className?: string;
}

export function CopyButton({
  text,
  label     = 'Copy',
  size      = 11,
  className,
}: CopyButtonProps) {
  const { copy }       = useClipboard({ successMessage: 'Copied!' });
  const [ok, setOk]    = useState(false);

  async function handleClick(e: React.MouseEvent) {
    e.stopPropagation();   // don't bubble to row-click handlers
    e.preventDefault();
    await copy(text);
    setOk(true);
    setTimeout(() => setOk(false), 1_500);
  }

  return (
    <button
      onClick={handleClick}
      aria-label={label}
      title={label}
      className={clsx(
        'inline-flex items-center justify-center rounded transition-colors',
        'text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:text-zinc-300 hover:bg-gray-200 dark:hover:bg-zinc-700/40',
        'p-0.5',
        className,
      )}
    >
      {ok
        ? <Check size={size} className="text-emerald-400" />
        : <Copy  size={size} />
      }
    </button>
  );
}
