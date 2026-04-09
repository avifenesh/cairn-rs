/**
 * HelpTooltip — small (?) icon that shows a descriptive tooltip on hover.
 *
 * Uses CSS positioning only; no JS calculations or portals.
 * The tooltip fades in on hover via a CSS animation defined in index.css.
 */

import { clsx } from 'clsx';

type Placement = 'top' | 'right' | 'bottom' | 'left';

interface HelpTooltipProps {
  text: string;
  placement?: Placement;
  className?: string;
}

// Arrow and tooltip position classes per placement.
const PLACEMENT: Record<Placement, {
  tooltip: string;
  arrow:   string;
}> = {
  top: {
    tooltip: 'bottom-full left-1/2 -translate-x-1/2 mb-2',
    arrow:   'top-full left-1/2 -translate-x-1/2 border-t-zinc-800 border-x-transparent border-b-transparent border-[5px]',
  },
  bottom: {
    tooltip: 'top-full left-1/2 -translate-x-1/2 mt-2',
    arrow:   'bottom-full left-1/2 -translate-x-1/2 border-b-zinc-800 border-x-transparent border-t-transparent border-[5px]',
  },
  right: {
    tooltip: 'left-full top-1/2 -translate-y-1/2 ml-2',
    arrow:   'right-full top-1/2 -translate-y-1/2 border-r-zinc-800 border-y-transparent border-l-transparent border-[5px]',
  },
  left: {
    tooltip: 'right-full top-1/2 -translate-y-1/2 mr-2',
    arrow:   'left-full top-1/2 -translate-y-1/2 border-l-zinc-800 border-y-transparent border-r-transparent border-[5px]',
  },
};

export function HelpTooltip({ text, placement = 'top', className }: HelpTooltipProps) {
  const p = PLACEMENT[placement];

  return (
    <span
      className={clsx('relative inline-flex items-center group/tooltip', className)}
      aria-label={text}
    >
      {/* (?) trigger */}
      <span
        className="inline-flex items-center justify-center w-3.5 h-3.5 rounded-full
                   text-[9px] font-bold leading-none cursor-help select-none
                   bg-zinc-700/60 text-gray-400 dark:text-zinc-500
                   group-hover/tooltip:bg-indigo-500/20 group-hover/tooltip:text-indigo-400
                   transition-colors"
        aria-hidden="true"
      >
        ?
      </span>

      {/* Tooltip panel */}
      <span
        className={clsx(
          'absolute z-50 w-max max-w-[220px] px-2.5 py-1.5',
          'rounded-md bg-gray-100 dark:bg-zinc-800 border border-gray-200 dark:border-zinc-700/60',
          'text-[11px] text-gray-700 dark:text-zinc-300 leading-snug shadow-lg shadow-black/40',
          'pointer-events-none select-none whitespace-normal',
          // Visibility — hidden until hover, then fade in.
          'opacity-0 group-hover/tooltip:opacity-100',
          'transition-opacity duration-150 delay-100',
          p.tooltip,
        )}
        role="tooltip"
      >
        {text}
        {/* Arrow */}
        <span className={clsx('absolute w-0 h-0 border-solid', p.arrow)} />
      </span>
    </span>
  );
}
