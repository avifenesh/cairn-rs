import { useState, useCallback } from 'react';

// ── Types ─────────────────────────────────────────────────────────────────────

export interface Preferences {
  /** Rows shown per page in data tables. */
  itemsPerPage: 25 | 50 | 100;
  /** How timestamps are displayed throughout the app. */
  dateFormat: 'relative' | 'absolute';
  /** IANA timezone name for absolute date formatting.  Empty = browser default. */
  timezone: string;
  /** Reduce padding and spacing in data-dense views. */
  compactMode: boolean;
  /** UI colour scheme. */
  theme: 'dark' | 'light' | 'system';
}

const STORAGE_KEY = 'cairn_preferences';

const DEFAULTS: Preferences = {
  itemsPerPage: 50,
  dateFormat:   'relative',
  timezone:     '',
  compactMode:  false,
  theme:        'dark',
};

// ── Helpers ───────────────────────────────────────────────────────────────────

function loadPrefs(): Preferences {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return { ...DEFAULTS };
    return { ...DEFAULTS, ...(JSON.parse(raw) as Partial<Preferences>) };
  } catch {
    return { ...DEFAULTS };
  }
}

function savePrefs(p: Preferences): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(p));
  } catch { /* storage quota or private mode */ }
}

/**
 * Apply the theme preference to the document root so Tailwind's `dark:` class
 * picks it up immediately.
 */
export function applyTheme(theme: Preferences['theme']): void {
  const root = document.documentElement;
  if (theme === 'dark') {
    root.classList.add('dark');
  } else if (theme === 'light') {
    root.classList.remove('dark');
  } else {
    // 'system': follow OS preference
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    root.classList.toggle('dark', prefersDark);
  }
}

// ── Hook ─────────────────────────────────────────────────────────────────────

type SetPrefs = (patch: Partial<Preferences>) => void;

/**
 * Access and persist user preferences.
 *
 * Returns `[prefs, setPrefs]`.  `setPrefs` accepts a partial object that is
 * merged into the current preferences and saved to localStorage immediately.
 * The theme is applied to the DOM on every update.
 *
 * @example
 *   const [prefs, setPrefs] = usePreferences();
 *   setPrefs({ compactMode: true });
 */
export function usePreferences(): [Preferences, SetPrefs] {
  const [prefs, setPrefsState] = useState<Preferences>(loadPrefs);

  const setPrefs = useCallback<SetPrefs>((patch) => {
    setPrefsState((prev) => {
      const next = { ...prev, ...patch };
      savePrefs(next);
      if (patch.theme !== undefined) applyTheme(next.theme);
      return next;
    });
  }, []);

  return [prefs, setPrefs];
}
