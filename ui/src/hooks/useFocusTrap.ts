/**
 * useFocusTrap — trap keyboard focus inside a container while it is active.
 *
 * Attach the returned ref to any modal/dialog container.  While the element
 * is in the DOM:
 *   - Tab / Shift+Tab cycle only within focusable children.
 *   - Escape fires the onClose callback.
 *   - Focus is moved to the first focusable child on mount.
 *   - The element that had focus before the trap opened receives focus back
 *     on cleanup (important for screen-reader continuity).
 *
 * Usage:
 *   const trapRef = useFocusTrap({ onClose: () => setOpen(false) });
 *   <div ref={trapRef} role="dialog" aria-modal="true"> … </div>
 */

import { useEffect, useRef, useCallback } from 'react';

const FOCUSABLE = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
  'details > summary',
].join(', ');

interface UseFocusTrapOptions {
  /** Called when the user presses Escape. */
  onClose?: () => void;
  /** Set false to temporarily disable the trap without unmounting. */
  enabled?: boolean;
}

export function useFocusTrap<T extends HTMLElement = HTMLDivElement>({
  onClose,
  enabled = true,
}: UseFocusTrapOptions = {}): React.RefObject<T | null> {
  const ref = useRef<T | null>(null);
  // Remember what had focus before the trap opened so we can restore it.
  const previousFocus = useRef<Element | null>(null);

  const getFocusable = useCallback((): HTMLElement[] => {
    if (!ref.current) return [];
    return Array.from(ref.current.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
      el => !el.closest('[aria-hidden="true"]'),
    );
  }, []);

  useEffect(() => {
    if (!enabled || !ref.current) return;

    // Save current focus for restoration.
    previousFocus.current = document.activeElement;

    // Move focus to the first focusable element inside the trap.
    const firstFocusable = getFocusable()[0];
    if (firstFocusable) {
      firstFocusable.focus();
    } else {
      // Make the container itself focusable if nothing inside is.
      ref.current.setAttribute('tabindex', '-1');
      ref.current.focus();
    }

    function handleKeyDown(e: KeyboardEvent) {
      if (!ref.current) return;

      if (e.key === 'Escape') {
        e.preventDefault();
        onClose?.();
        return;
      }

      if (e.key !== 'Tab') return;

      const focusable = getFocusable();
      if (focusable.length === 0) { e.preventDefault(); return; }

      const first = focusable[0];
      const last  = focusable[focusable.length - 1];

      if (e.shiftKey) {
        // Shift+Tab: if focus is at the first element, wrap to last.
        if (document.activeElement === first || !ref.current.contains(document.activeElement)) {
          e.preventDefault();
          last.focus();
        }
      } else {
        // Tab: if focus is at the last element, wrap to first.
        if (document.activeElement === last || !ref.current.contains(document.activeElement)) {
          e.preventDefault();
          first.focus();
        }
      }
    }

    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      // Restore focus to the element that was active before the trap.
      if (previousFocus.current instanceof HTMLElement) {
        previousFocus.current.focus();
      }
    };
  }, [enabled, onClose, getFocusable]);

  return ref;
}
