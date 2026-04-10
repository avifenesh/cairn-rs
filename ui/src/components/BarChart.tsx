/**
 * BarChart — compact horizontal bar chart using only CSS/SVG.
 *
 * Renders a labelled list of bars normalised to the maximum value.
 * No external chart library.
 *
 * Usage:
 *   <BarChart items={[
 *     { label: "gpt-4o",     value: 42000, color: "#6366f1" },
 *     { label: "claude-3-5", value: 18000 },
 *   ]} />
 */

import { clsx } from "clsx";

export interface BarChartItem {
  label: string;
  value: number;
  /** Optional sub-label shown in muted text after the bar. */
  sublabel?: string;
  /** Fill colour. Defaults to indigo-500. */
  color?: string;
}

export interface BarChartProps {
  items: BarChartItem[];
  /** Function to format the value label at the bar end. */
  formatValue?: (v: number) => string;
  /** Max items to show before truncating. */
  maxItems?: number;
  className?: string;
  /** Height of each bar in px. */
  barHeight?: number;
  /** Extra row spacing in px. */
  rowGap?: number;
}

const DEFAULT_COLOR = "#6366f1";

export function BarChart({
  items,
  formatValue = String,
  maxItems = 8,
  className,
  barHeight = 6,
  rowGap = 4,
}: BarChartProps) {
  const visible = items.slice(0, maxItems);
  const max     = Math.max(...visible.map((i) => i.value), 1);

  if (visible.length === 0) {
    return (
      <div className={clsx("text-[11px] text-gray-400 dark:text-zinc-600 italic py-2 text-center", className)}>
        No data
      </div>
    );
  }

  const labelW = Math.min(
    96,
    Math.max(48, ...visible.map((i) => i.label.length * 6.5)),
  );

  return (
    <div className={clsx("space-y-0", className)}>
      {visible.map((item, idx) => {
        const pct    = (item.value / max) * 100;
        const color  = item.color ?? DEFAULT_COLOR;

        return (
          <div
            key={`${item.label}-${idx}`}
            className="flex items-center gap-2"
            style={{ marginBottom: idx < visible.length - 1 ? rowGap : 0 }}
          >
            {/* Label */}
            <span
              className="text-[11px] text-gray-500 dark:text-zinc-400 truncate shrink-0 text-right"
              style={{ width: labelW }}
              title={item.label}
            >
              {item.label}
            </span>

            {/* Bar track */}
            <div className="flex-1 min-w-0 relative" style={{ height: barHeight }}>
              <div
                className="absolute inset-y-0 left-0 rounded-full transition-all duration-500"
                style={{
                  width:  `${Math.max(pct, 0.5)}%`,
                  background: color,
                  opacity: 0.85,
                }}
              />
              {/* Track background */}
              <div
                className="absolute inset-0 rounded-full bg-gray-100 dark:bg-zinc-800"
                style={{ zIndex: -1 }}
              />
            </div>

            {/* Value */}
            <span
              className="text-[11px] font-mono tabular-nums text-gray-400 dark:text-zinc-500 shrink-0 text-right"
              style={{ minWidth: 48 }}
            >
              {formatValue(item.value)}
              {item.sublabel && (
                <span className="text-gray-300 dark:text-zinc-600 ml-1">{item.sublabel}</span>
              )}
            </span>
          </div>
        );
      })}

      {items.length > maxItems && (
        <p className="text-[10px] text-gray-300 dark:text-zinc-600 text-right pt-1">
          +{items.length - maxItems} more
        </p>
      )}
    </div>
  );
}

export default BarChart;
