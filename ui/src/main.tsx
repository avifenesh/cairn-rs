import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import { QueryClient, QueryCache, MutationCache, QueryClientProvider } from '@tanstack/react-query'
import { ToastProvider } from './components/Toast'
import { registerSW } from './lib/registerSW'
import { ApiError, clearStoredToken, AUTH_EXPIRED_EVENT } from './lib/api'
import './index.css'
import App from './App.tsx'

// Register service worker for offline support (no-op in dev mode).
registerSW();

// Global 401 interceptor: when any query or mutation fails with an ApiError
// whose status is 401, the stored token is invalid/rotated — clear it and
// notify the App shell so the operator is bounced back to the login screen.
// Without this, a rotated token surfaces as a permanent red error badge on
// every page until the operator manually logs out.
function handleMaybeAuthError(err: unknown) {
  if (err instanceof ApiError && err.status === 401) {
    clearStoredToken();
    window.dispatchEvent(new CustomEvent(AUTH_EXPIRED_EVENT));
  }
}

const queryClient = new QueryClient({
  queryCache: new QueryCache({
    onError: handleMaybeAuthError,
  }),
  mutationCache: new MutationCache({
    onError: handleMaybeAuthError,
  }),
})

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <App />
      </ToastProvider>
    </QueryClientProvider>
  </StrictMode>,
)
