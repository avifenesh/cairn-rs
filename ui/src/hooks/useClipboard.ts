/**
 * useClipboard — copy-to-clipboard with toast feedback.
 *
 * Uses the modern Clipboard API with a textarea fallback for older browsers
 * that don't support navigator.clipboard (e.g. HTTP contexts, Safari < 13.1).
 *
 * Usage:
 *   const { copy, copied, CopyButton } = useClipboard();
 *   <button onClick={() => copy(someId)}>Copy</button>
 *   // or use the returned CopyButton JSX helper directly
 */

import { useState, useCallback } from 'react';
import { useToast } from '../components/Toast';

// ── Core copy function (no React) ─────────────────────────────────────────────

/** Copy `text` to the clipboard.  Returns true on success. */
async function writeToClipboard(text: string): Promise<boolean> {
  // Modern path — works in HTTPS and localhost contexts.
  if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to legacy path.
    }
  }

  // Legacy fallback — creates a temporary <textarea>, selects it, and uses
  // the deprecated document.execCommand('copy').  Works in HTTP contexts and
  // older Safari versions.
  try {
    const el = document.createElement('textarea');
    el.value = text;
    // Keep off-screen so it doesn't cause a layout shift.
    el.style.cssText = 'position:fixed;top:-9999px;left:-9999px;opacity:0';
    document.body.appendChild(el);
    el.focus();
    el.select();
    const ok = document.execCommand('copy');
    document.body.removeChild(el);
    return ok;
  } catch {
    return false;
  }
}

// ── Hook ──────────────────────────────────────────────────────────────────────

interface UseClipboardOptions {
  /** Toast message on success (default: "Copied!"). */
  successMessage?: string;
  /** Duration the `copied` flag stays true in ms (default: 1500). */
  resetAfter?: number;
}

export interface UseClipboardResult {
  /** Copy `text` to the clipboard and show a toast. */
  copy: (text: string) => Promise<void>;
  /** True for `resetAfter` ms after the last successful copy. */
  copied: boolean;
}

export function useClipboard({
  successMessage = 'Copied!',
  resetAfter     = 1_500,
}: UseClipboardOptions = {}): UseClipboardResult {
  const [copied, setCopied] = useState(false);
  const toast               = useToast();

  const copy = useCallback(async (text: string) => {
    const ok = await writeToClipboard(text);
    if (ok) {
      toast.success(successMessage);
      setCopied(true);
      setTimeout(() => setCopied(false), resetAfter);
    } else {
      toast.error('Copy failed — please copy manually.');
    }
  }, [toast, successMessage, resetAfter]);

  return { copy, copied };
}
