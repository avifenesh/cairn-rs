/**
 * EntityExplainer — quiet one-line explainer shown under page titles and
 * detail-page headers (F32).
 *
 * Style is deliberately unobtrusive: 11px, muted foreground, `leading-relaxed`.
 * Not a link, not collapsible, not a modal — the text itself is the
 * explanation, and it must be readable at a glance when an operator lands
 * on the page cold.
 *
 * Pair with strings from `ui/src/lib/entityExplainers.ts`. Never duplicate
 * the text inline in a page — use the constant so tests can assert it.
 */

import { clsx } from "clsx";

interface EntityExplainerProps {
  /** The explainer text. Should be one sentence, 60–160 chars. */
  children: string;
  /** Extra tailwind classes (e.g. `mt-0` to tighten spacing in a toolbar). */
  className?: string;
  /** When set, wraps the content in a <p> with this data-testid. */
  testId?: string;
}

export function EntityExplainer({ children, className, testId }: EntityExplainerProps) {
  return (
    <p
      data-testid={testId ?? "entity-explainer"}
      className={clsx(
        "text-[11px] text-gray-500 dark:text-zinc-500 leading-relaxed",
        className,
      )}
    >
      {children}
    </p>
  );
}
