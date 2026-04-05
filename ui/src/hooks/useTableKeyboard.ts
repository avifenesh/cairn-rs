/**
 * useTableKeyboard — keyboard navigation and multi-selection for data tables.
 *
 * Keys:
 *   ArrowDown / j   — move focus down one row
 *   ArrowUp   / k   — move focus up one row
 *   Enter           — open/navigate to the active row (calls onOpen)
 *   Escape          — clear both focus and selection
 *   x               — toggle selection on the active row (multi-select)
 *
 * Returns containerProps to spread onto the table wrapper div so it can
 * receive keyboard events via tabIndex={0}.
 */

import { useState, useCallback, useEffect, type KeyboardEvent } from 'react';

interface Options<T> {
  /** The (possibly filtered/sorted) rows visible in the table. */
  items: T[];
  /** Returns a stable string key for a row (used for multi-selection). */
  getKey: (item: T) => string;
  /** Called when the user presses Enter on the active row. */
  onOpen?: (item: T, index: number) => void;
  /** Whether keyboard navigation is active. Defaults to true. */
  enabled?: boolean;
}

export interface TableKeyboardState {
  /** 0-based index of the currently highlighted row. -1 = none. */
  activeIndex: number;
  /** Set of row keys currently checked for bulk actions. */
  selectedKeys: Set<string>;
  /** Spread onto the scrollable container div. */
  containerProps: {
    tabIndex: number;
    onKeyDown: (e: KeyboardEvent) => void;
    className: string;
  };
  /** Highlight/check a row by key. */
  toggleSelect: (key: string) => void;
  /** Clear both activeIndex and selectedKeys. */
  clearSelection: () => void;
  /** Move active index programmatically (e.g. on mouse hover). */
  setActiveIndex: (i: number) => void;
}

const TEXT_TAGS = new Set(['INPUT', 'TEXTAREA', 'SELECT']);

export function useTableKeyboard<T>({
  items,
  getKey,
  onOpen,
  enabled = true,
}: Options<T>): TableKeyboardState {
  const [activeIndex, setActiveIndex] = useState(-1);
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());

  // Clamp active index if the item list shrinks.
  useEffect(() => {
    if (items.length === 0) {
      setActiveIndex(-1);
    } else {
      setActiveIndex(i => (i >= items.length ? items.length - 1 : i));
    }
  }, [items.length]);

  const toggleSelect = useCallback((key: string) => {
    setSelectedKeys(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const clearSelection = useCallback(() => {
    setSelectedKeys(new Set());
    setActiveIndex(-1);
  }, []);

  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (!enabled || items.length === 0) return;

    const inTextField = TEXT_TAGS.has((e.target as HTMLElement).tagName);
    const n = items.length;

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault();
        setActiveIndex(i => Math.min(i + 1, n - 1));
        break;

      case 'ArrowUp':
        e.preventDefault();
        setActiveIndex(i => (i <= 0 ? 0 : i - 1));
        break;

      case 'j':
        if (!inTextField) {
          e.preventDefault();
          setActiveIndex(i => Math.min(i + 1, n - 1));
        }
        break;

      case 'k':
        if (!inTextField) {
          e.preventDefault();
          setActiveIndex(i => (i <= 0 ? 0 : i - 1));
        }
        break;

      case 'Enter':
        if (activeIndex >= 0 && activeIndex < n) {
          e.preventDefault();
          onOpen?.(items[activeIndex], activeIndex);
        }
        break;

      case 'Escape':
        e.preventDefault();
        clearSelection();
        break;

      case 'x':
        if (!inTextField && activeIndex >= 0 && activeIndex < n) {
          e.preventDefault();
          toggleSelect(getKey(items[activeIndex]));
        }
        break;
    }
  }, [enabled, items, activeIndex, onOpen, clearSelection, toggleSelect, getKey]);

  return {
    activeIndex,
    selectedKeys,
    containerProps: {
      tabIndex:   0,
      onKeyDown:  handleKeyDown,
      className:  'focus:outline-none',
    },
    toggleSelect,
    clearSelection,
    setActiveIndex,
  };
}
