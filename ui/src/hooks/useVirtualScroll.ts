/**
 * useVirtualScroll — window-based virtual rendering for large lists.
 *
 * Only mounts DOM nodes for rows currently visible in the scroll container
 * plus a configurable overscan buffer (default 20 rows above/below).
 *
 * Usage:
 *   const { containerRef, visibleItems, totalHeight, offsetY } =
 *     useVirtualScroll({ items: traces, rowHeight: 36, overscan: 20 });
 *
 *   return (
 *     <div ref={containerRef} style={{ overflowY: 'auto', height: '100%' }}>
 *       <table style={{ width: '100%' }}>
 *         <tbody>
 *           {offsetY > 0 && <tr><td style={{ height: offsetY }} /></tr>}
 *           {visibleItems.map(({ item, index }) => <Row key={index} item={item} />)}
 *           {(totalHeight - offsetY - visibleItems.length * rowHeight) > 0 && (
 *             <tr><td style={{ height: totalHeight - offsetY - visibleItems.length * rowHeight }} /></tr>
 *           )}
 *         </tbody>
 *       </table>
 *     </div>
 *   );
 */

import { useRef, useState, useEffect, type RefObject } from 'react';

export const DEFAULT_ROW_HEIGHT = 36;
export const DEFAULT_OVERSCAN   = 20;

export interface VirtualItem<T> {
  /** Original item from the source array. */
  item:  T;
  /** Absolute index in the source array. */
  index: number;
}

export interface UseVirtualScrollResult<T> {
  /** Attach to the scrollable container element. */
  containerRef: RefObject<HTMLDivElement | null>;
  /** Rows to render — already windowed to the visible range + overscan. */
  visibleItems: VirtualItem<T>[];
  /** Total pixel height of all rows (use as `height` of an inner spacer). */
  totalHeight: number;
  /** Pixel offset from the top of the list to the first visible item. */
  offsetY: number;
  /** Index of the first visible item (after overscan subtraction). */
  startIndex: number;
}

interface Options<T> {
  /** Full item array (pre-filtered / pre-sorted). */
  items: T[];
  /** Fixed row height in pixels. Default: 36. */
  rowHeight?: number;
  /** Extra rows to render above and below the visible window. Default: 20. */
  overscan?: number;
}

export function useVirtualScroll<T>({
  items,
  rowHeight = DEFAULT_ROW_HEIGHT,
  overscan  = DEFAULT_OVERSCAN,
}: Options<T>): UseVirtualScrollResult<T> {
  const containerRef = useRef<HTMLDivElement>(null);
  const [scrollTop,        setScrollTop]        = useState(0);
  const [containerHeight,  setContainerHeight]  = useState(600);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    // Measure on first attach.
    setContainerHeight(el.clientHeight || 600);

    const onScroll = () => setScrollTop(el.scrollTop);
    el.addEventListener('scroll', onScroll, { passive: true });

    // Keep height up to date when the window is resized.
    const ro = new ResizeObserver(() => setContainerHeight(el.clientHeight || 600));
    ro.observe(el);

    return () => {
      el.removeEventListener('scroll', onScroll);
      ro.disconnect();
    };
  }, []);

  const count       = items.length;
  const totalHeight = count * rowHeight;

  // First row that intersects the viewport (clamped by overscan).
  const startIndex = Math.max(
    0,
    Math.floor(scrollTop / rowHeight) - overscan,
  );

  // Last row that intersects the viewport (clamped by overscan).
  const endIndex = Math.min(
    count - 1,
    Math.ceil((scrollTop + containerHeight) / rowHeight) + overscan,
  );

  const visibleItems: VirtualItem<T>[] = [];
  for (let i = startIndex; i <= endIndex; i++) {
    visibleItems.push({ item: items[i], index: i });
  }

  const offsetY = startIndex * rowHeight;

  return { containerRef, visibleItems, totalHeight, offsetY, startIndex };
}
