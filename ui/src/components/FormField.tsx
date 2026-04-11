/**
 * FormField — labeled form input wrapper.
 *
 * Extracts the best pattern from PromptsPage, MemoryPage, ChannelsPage forms.
 * Wraps a label + input/select/textarea with consistent spacing and styling.
 */

import { clsx } from "clsx";

interface FormFieldProps {
  /** Field label text. */
  label: string;
  /** Mark as required (shows red asterisk). */
  required?: boolean;
  /** Helper text below the input. */
  helper?: string;
  /** Error message (replaces helper, shown in red). */
  error?: string;
  /** The input element(s). */
  children: React.ReactNode;
  className?: string;
}

export function FormField({
  label,
  required = false,
  helper,
  error,
  children,
  className,
}: FormFieldProps) {
  return (
    <div className={className}>
      <label className="block text-[11px] text-gray-400 dark:text-zinc-500 mb-1">
        {label}
        {required && <span className="text-red-500 ml-0.5">*</span>}
      </label>
      {children}
      {error ? (
        <p className="text-[10px] text-red-400 mt-1">{error}</p>
      ) : helper ? (
        <p className="text-[10px] text-gray-300 dark:text-zinc-600 mt-1">{helper}</p>
      ) : null}
    </div>
  );
}

// ── Preset input classes (for use without the wrapper) ───────────────────────

export const fieldInput =
  "w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-950 text-[12px] text-gray-800 dark:text-zinc-200 px-3 py-2 focus:outline-none focus:border-indigo-500 transition-colors";

export const fieldInputMono = clsx(fieldInput, "font-mono");

export const fieldSelect =
  "w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[12px] text-gray-700 dark:text-zinc-300 px-2 py-1.5 focus:outline-none focus:border-indigo-500 transition-colors";

export const fieldTextarea = clsx(fieldInput, "font-mono resize-y leading-relaxed");
