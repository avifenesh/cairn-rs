/**
 * RequestLogContext — lightweight in-memory log of API calls made from the
 * API Docs "Try it" panel.
 *
 * Pattern: simple React context wrapping a useState array (no extra deps).
 * The log is capped at MAX_ENTRIES to avoid unbounded growth.
 */

import { createContext, useContext, useState, useCallback, type ReactNode } from 'react';

export interface RequestLogEntry {
  id:          string;
  timestamp:   number;        // Date.now()
  method:      string;
  url:         string;
  path:        string;
  reqHeaders:  Record<string, string>;
  reqBody:     string | null;
  status:      number | null; // null = in-flight or error
  resBody:     unknown;
  latency:     number | null;
  error:       string | null;
}

interface RequestLogCtx {
  entries:   RequestLogEntry[];
  add:       (entry: RequestLogEntry) => void;
  update:    (id: string, patch: Partial<RequestLogEntry>) => void;
  clear:     () => void;
}

const MAX_ENTRIES = 20;

const Ctx = createContext<RequestLogCtx>({
  entries: [],
  add:     () => undefined,
  update:  () => undefined,
  clear:   () => undefined,
});

export function RequestLogProvider({ children }: { children: ReactNode }) {
  const [entries, setEntries] = useState<RequestLogEntry[]>([]);

  const add = useCallback((entry: RequestLogEntry) => {
    setEntries(prev => [entry, ...prev].slice(0, MAX_ENTRIES));
  }, []);

  const update = useCallback((id: string, patch: Partial<RequestLogEntry>) => {
    setEntries(prev => prev.map(e => e.id === id ? { ...e, ...patch } : e));
  }, []);

  const clear = useCallback(() => setEntries([]), []);

  return <Ctx.Provider value={{ entries, add, update, clear }}>{children}</Ctx.Provider>;
}

export function useRequestLog() {
  return useContext(Ctx);
}
