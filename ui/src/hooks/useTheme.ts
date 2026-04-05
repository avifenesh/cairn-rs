/**
 * useTheme — dark/light/system theme management.
 *
 * Persists selection in localStorage. Applies/removes the `dark` class on
 * `document.documentElement` so Tailwind's `dark:` variants activate.
 *
 * Cycle: dark → light → system → dark
 */

import { useState, useEffect } from 'react';

export type Theme = 'dark' | 'light' | 'system';

export const THEME_KEY = 'cairn_theme';

const CYCLE: Record<Theme, Theme> = { dark: 'light', light: 'system', system: 'dark' };

function resolvedDark(t: Theme): boolean {
  if (t === 'system') return window.matchMedia('(prefers-color-scheme: dark)').matches;
  return t === 'dark';
}

function applyTheme(t: Theme) {
  document.documentElement.classList.toggle('dark', resolvedDark(t));
}

export function useTheme() {
  const [theme, setThemeState] = useState<Theme>(
    () => (localStorage.getItem(THEME_KEY) as Theme) ?? 'dark',
  );

  // Apply whenever preference changes.
  useEffect(() => { applyTheme(theme); }, [theme]);

  // Re-apply when OS preference changes (only matters in 'system' mode).
  useEffect(() => {
    if (theme !== 'system') return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => applyTheme('system');
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, [theme]);

  function setTheme(next: Theme) {
    localStorage.setItem(THEME_KEY, next);
    setThemeState(next);
  }

  const cycleTheme = () => setTheme(CYCLE[theme]);

  return { theme, setTheme, cycleTheme };
}
