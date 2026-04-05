/**
 * Service worker registration and update lifecycle.
 *
 * Call registerSW() once on app startup (from main.tsx).
 * It will:
 *   1. Register /sw.js when the browser supports service workers.
 *   2. On activation, take control of all open tabs immediately.
 *   3. When a new SW is waiting, broadcast a 'sw-update-ready' event
 *      so the UI can show an update notification (handled in ConnectionStatus).
 *   4. Expose applyUpdate() to trigger the update immediately.
 */

// ── Types ─────────────────────────────────────────────────────────────────────

export interface SWUpdateEvent extends Event {
  type: 'sw-update-ready';
}

declare global {
  interface WindowEventMap {
    'sw-update-ready': SWUpdateEvent;
  }
}

// ── State ─────────────────────────────────────────────────────────────────────

let waitingWorker: ServiceWorker | null = null;

/** True after a new SW version has downloaded and is waiting to activate. */
export function hasPendingUpdate(): boolean {
  return waitingWorker !== null;
}

/**
 * Tell the waiting service worker to skip waiting and activate immediately.
 * The page will automatically reload once the new SW takes control.
 */
export function applyUpdate(): void {
  if (!waitingWorker) return;
  waitingWorker.postMessage({ type: 'SKIP_WAITING' });
}

// ── Registration ──────────────────────────────────────────────────────────────

export function registerSW(): void {
  if (!('serviceWorker' in navigator)) return;
  if (import.meta.env.DEV) {
    // Don't register the SW in dev mode — Vite's HMR and SW caching conflict.
    return;
  }

  window.addEventListener('load', () => {
    navigator.serviceWorker
      .register('/sw.js', { scope: '/' })
      .then(registration => {
        // Already has a waiting worker (e.g. hard-refresh over an update).
        if (registration.waiting) {
          waitingWorker = registration.waiting;
          window.dispatchEvent(new Event('sw-update-ready'));
        }

        // New worker installed while the page is open.
        registration.addEventListener('updatefound', () => {
          const installing = registration.installing;
          if (!installing) return;

          installing.addEventListener('statechange', () => {
            if (installing.state === 'installed' && navigator.serviceWorker.controller) {
              // A new version is ready — save reference and notify the UI.
              waitingWorker = installing;
              window.dispatchEvent(new Event('sw-update-ready'));
            }
          });
        });

        // When the SW takes control, reload to serve the latest assets.
        let refreshing = false;
        navigator.serviceWorker.addEventListener('controllerchange', () => {
          if (refreshing) return;
          refreshing = true;
          window.location.reload();
        });
      })
      .catch(err => {
        // SW registration failure is non-fatal; app works without it.
        if (import.meta.env.DEV) {
          // eslint-disable-next-line no-console
          console.warn('[SW] Registration failed:', err);
        }
      });
  });
}

// ── Online/offline helpers ────────────────────────────────────────────────────

/** Subscribe to online/offline events. Returns an unsubscribe function. */
export function onNetworkChange(cb: (online: boolean) => void): () => void {
  const handleOnline  = () => cb(true);
  const handleOffline = () => cb(false);
  window.addEventListener('online',  handleOnline);
  window.addEventListener('offline', handleOffline);
  return () => {
    window.removeEventListener('online',  handleOnline);
    window.removeEventListener('offline', handleOffline);
  };
}

/** Current browser online/offline state. */
export function isOnline(): boolean {
  return navigator.onLine;
}
