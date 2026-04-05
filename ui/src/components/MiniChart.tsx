/**
 * MiniChart — pure-SVG sparkline with a filled area.
 *
 * Draws a smooth polyline through the data points with a filled region below.
 * No external chart library.
 *
 * Usage:
 *   <MiniChart data={[4, 7, 3, 9, 5, 11]} color="#6366f1" />
 */

import { useMemo } from "react";
import { clsx } from "clsx";

export interface MiniChartProps {
  /** Y-axis data values. Must have at least 2 points to draw a line. */
  data: number[];
  /**
   * Fixed pixel width. When omitted the SVG expands to fill its container
   * via width="100%" — useful inside a flex/grid parent.
   */
  width?: number;
  height?: number;
  /** Stroke + fill colour. Accepts any CSS colour string. */
  color?: string;
  /** Extra class for the outer <svg>. */
  className?: string;
  /** Show a dashed baseline at y=0. */
  baseline?: boolean;
}

/** Map a value in [min, max] to SVG y-coordinate in [0, height]. */
function scaleY(value: number, min: number, max: number, height: number, padding = 4): number {
  if (max === min) return height / 2;
  return padding + ((1 - (value - min) / (max - min)) * (height - padding * 2));
}

/** Build a smooth SVG path from a series of [x, y] points using cubic Bézier curves. */
function smoothPath(points: [number, number][]): string {
  if (points.length < 2) return "";
  if (points.length === 2) {
    return `M ${points[0][0]} ${points[0][1]} L ${points[1][0]} ${points[1][1]}`;
  }

  const d: string[] = [`M ${points[0][0]} ${points[0][1]}`];
  for (let i = 0; i < points.length - 1; i++) {
    const [x0, y0] = points[i];
    const [x1, y1] = points[i + 1];
    // Control-point tension: 1/3 of horizontal distance
    const cx = (x1 - x0) / 3;
    d.push(`C ${x0 + cx} ${y0}, ${x1 - cx} ${y1}, ${x1} ${y1}`);
  }
  return d.join(" ");
}

/** Internal canvas width used for coordinate calculations. */
const CANVAS_W = 200;

export function MiniChart({
  data,
  width,
  height = 36,
  color = "#6366f1",
  className,
  baseline = false,
}: MiniChartProps) {
  const points = useMemo<[number, number][]>(() => {
    if (data.length < 2) return [];
    const min  = Math.min(...data);
    const max  = Math.max(...data);
    const step = (CANVAS_W - 2) / (data.length - 1);
    return data.map((v, i) => [1 + i * step, scaleY(v, min, max, height)]);
  }, [data, height]);

  // SVG width attribute: fixed number or "100%" for fluid layout.
  const svgWidth: number | string = width ?? "100%";

  if (points.length < 2) {
    return (
      <svg width={svgWidth} height={height} viewBox={`0 0 ${CANVAS_W} ${height}`}
           className={clsx("overflow-visible", className)}>
        <line x1={0} y1={height / 2} x2={CANVAS_W} y2={height / 2}
          stroke={color} strokeWidth={1} strokeDasharray="3 3" opacity={0.3} />
      </svg>
    );
  }

  const linePath = smoothPath(points);

  // Closed area path: follow the line then drop to baseline
  const areaPath =
    linePath +
    ` L ${points[points.length - 1][0]} ${height}` +
    ` L ${points[0][0]} ${height} Z`;

  // Gradient ID must not collide between instances.
  const gradId = `mg-${color.replace(/[^a-z0-9]/gi, "")}-${height}`;

  return (
    <svg
      width={svgWidth}
      height={height}
      viewBox={`0 0 ${CANVAS_W} ${height}`}
      preserveAspectRatio="none"
      className={clsx("overflow-visible", className)}
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={gradId} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%"   stopColor={color} stopOpacity={0.25} />
          <stop offset="100%" stopColor={color} stopOpacity={0.02} />
        </linearGradient>
      </defs>

      {baseline && (
        <line x1={0} y1={height - 1} x2={width} y2={height - 1}
          stroke={color} strokeWidth={0.5} strokeDasharray="2 2" opacity={0.2} />
      )}

      {/* Filled area */}
      <path d={areaPath} fill={`url(#${gradId})`} />

      {/* Sparkline */}
      <path d={linePath} fill="none" stroke={color} strokeWidth={1.5}
        strokeLinecap="round" strokeLinejoin="round" />

      {/* Endpoint dot */}
      <circle
        cx={points[points.length - 1][0]}
        cy={points[points.length - 1][1]}
        r={2.5}
        fill={color}
      />
    </svg>
  );
}

export default MiniChart;
