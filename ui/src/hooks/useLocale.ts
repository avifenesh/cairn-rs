import { useState, useCallback } from 'react';
import { type Locale, type TranslationKey, makeT } from '../lib/i18n';

const LS_KEY = 'cairn_locale';

function loadLocale(): Locale {
  try {
    const raw = localStorage.getItem(LS_KEY);
    if (raw === 'en' || raw === 'es' || raw === 'de' || raw === 'ja' || raw === 'zh') {
      return raw;
    }
  } catch { /* ignore */ }
  return 'en';
}

function saveLocale(locale: Locale) {
  try { localStorage.setItem(LS_KEY, locale); } catch { /* ignore */ }
}

export interface UseLocaleResult {
  locale:    Locale;
  setLocale: (l: Locale) => void;
  t:         (key: TranslationKey) => string;
}

export function useLocale(): UseLocaleResult {
  const [locale, setLocaleState] = useState<Locale>(loadLocale);

  const setLocale = useCallback((l: Locale) => {
    setLocaleState(l);
    saveLocale(l);
  }, []);

  const t = useCallback(makeT(locale), [locale]); // eslint-disable-line react-hooks/exhaustive-deps

  return { locale, setLocale, t };
}
