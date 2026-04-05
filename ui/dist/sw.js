/**
 * cairn service worker — offline support and asset caching.
 *
 * Strategy:
 *   - Static assets (JS, CSS, fonts, icons):  Cache-first with network fallback.
 *   - API calls (/v1/*, /health):              Network-first with no caching.
 *   - Navigation requests (HTML):             Network-first, fallback to offline page.
 *
 * No workbox, no build-step compilation — pure Service Worker API.
 */

const CACHE_VERSION   = 'cairn-v1';
const STATIC_CACHE    = `${CACHE_VERSION}-static`;
const OFFLINE_URL     = '/offline.html';

// Static assets to pre-cache on install.
// The hashed JS/CSS filenames change on each build, so we match by pattern
// in the fetch handler rather than listing them here.  We do pre-cache the
// offline fallback page.
const PRECACHE_URLS = [
  '/',
  '/offline.html',
];

// ── Install ──────────────────────────────────────────────────────────────────

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(STATIC_CACHE).then((cache) =>
      cache.addAll(PRECACHE_URLS).catch(() => {
        // offline.html may not exist yet on first deploy; that's fine.
      })
    ).then(() => self.skipWaiting())
  );
});

// ── Activate ─────────────────────────────────────────────────────────────────

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches.keys().then((keys) =>
      Promise.all(
        keys
          .filter((key) => key.startsWith('cairn-') && key !== STATIC_CACHE)
          .map((key) => caches.delete(key))
      )
    ).then(() => self.clients.claim())
  );
});

// ── Fetch ─────────────────────────────────────────────────────────────────────

self.addEventListener('fetch', (event) => {
  const { request } = event;
  const url = new URL(request.url);

  // Only handle same-origin requests.
  if (url.origin !== self.location.origin) return;

  // ── API calls: network-first, no caching ─────────────────────────────────
  if (url.pathname.startsWith('/v1/') || url.pathname === '/health') {
    event.respondWith(networkOnly(request));
    return;
  }

  // ── Static assets: cache-first ───────────────────────────────────────────
  if (isStaticAsset(url.pathname)) {
    event.respondWith(cacheFirst(request));
    return;
  }

  // ── Navigation (HTML): network-first, offline fallback ───────────────────
  if (request.mode === 'navigate') {
    event.respondWith(networkFirstWithOfflineFallback(request));
    return;
  }
});

// ── Strategy helpers ──────────────────────────────────────────────────────────

/** Network-only: used for API calls. Never cache. */
async function networkOnly(request) {
  try {
    return await fetch(request);
  } catch {
    return new Response(
      JSON.stringify({ error: 'offline', message: 'No network connection' }),
      { status: 503, headers: { 'Content-Type': 'application/json' } }
    );
  }
}

/** Cache-first: serve from cache; on miss, fetch, cache, and return. */
async function cacheFirst(request) {
  const cached = await caches.match(request);
  if (cached) return cached;

  try {
    const response = await fetch(request);
    if (response.ok) {
      const cache = await caches.open(STATIC_CACHE);
      cache.put(request, response.clone());
    }
    return response;
  } catch {
    // No network and no cache — nothing we can do.
    return new Response('', { status: 503 });
  }
}

/** Network-first for navigation: try network, fall back to /offline.html. */
async function networkFirstWithOfflineFallback(request) {
  try {
    const response = await fetch(request);
    if (response.ok) {
      // Cache the shell so future navigations work offline too.
      const cache = await caches.open(STATIC_CACHE);
      cache.put(request, response.clone());
    }
    return response;
  } catch {
    const cached = await caches.match(request);
    if (cached) return cached;

    // Last resort: offline page.
    const offline = await caches.match(OFFLINE_URL);
    if (offline) return offline;

    return new Response(
      `<!doctype html><html><head><title>Offline</title></head><body>
       <h1>You are offline</h1>
       <p>cairn cannot be reached. Please check your network connection.</p>
      </body></html>`,
      { status: 503, headers: { 'Content-Type': 'text/html' } }
    );
  }
}

/** True for files that should be cached aggressively. */
function isStaticAsset(pathname) {
  return (
    pathname.startsWith('/assets/') ||   // Vite-built JS/CSS (hashed)
    pathname.endsWith('.js')          ||
    pathname.endsWith('.css')         ||
    pathname.endsWith('.woff2')       ||
    pathname.endsWith('.woff')        ||
    pathname.endsWith('.ttf')         ||
    pathname.endsWith('.svg')         ||
    pathname.endsWith('.png')         ||
    pathname.endsWith('.ico')         ||
    pathname === '/favicon.svg'       ||
    pathname === '/icons.svg'
  );
}

// ── Update notification ───────────────────────────────────────────────────────

/** Tell all open tabs that a new SW version is waiting. */
self.addEventListener('message', (event) => {
  if (event.data?.type === 'SKIP_WAITING') {
    self.skipWaiting();
  }
});
