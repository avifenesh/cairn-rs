/**
 * ActionButton — standardized button component.
 *
 * Extracts the best pattern from toolbar buttons, modal actions, and CTA buttons
 * found across all 39 pages.
 */

import { Loader2 } from "lucide-react";
import { clsx } from "clsx";

export type ButtonVariant = "primary" | "secondary" | "danger" | "ghost";
export type ButtonSize = "sm" | "md" | "lg";

interface ActionButtonProps {
  children: React.ReactNode;
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Show a loading spinner and disable the button. */
  loading?: boolean;
  /** Leading icon element (rendered before children). */
  icon?: React.ReactNode;
  disabled?: boolean;
  onClick?: (e: React.MouseEvent<HTMLButtonElement>) => void;
  type?: "button" | "submit" | "reset";
  title?: string;
  className?: string;
}

const VARIANT_CLASSES: Record<ButtonVariant, string> = {
  primary:
    "bg-indigo-600 hover:bg-indigo-500 text-white font-medium border border-transparent",
  secondary:
    "bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-700 text-gray-500 dark:text-zinc-400 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600",
  danger:
    "bg-red-600 hover:bg-red-500 text-white font-medium border border-transparent",
  ghost:
    "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 border border-transparent",
};

const SIZE_CLASSES: Record<ButtonSize, string> = {
  sm: "text-[11px] px-2 py-1 gap-1",
  md: "text-[12px] px-3 py-1.5 gap-1.5",
  lg: "text-[13px] px-4 py-2 gap-2",
};

export function ActionButton({
  children,
  variant = "primary",
  size = "md",
  loading = false,
  icon,
  disabled = false,
  onClick,
  type = "button",
  title,
  className,
}: ActionButtonProps) {
  return (
    <button
      type={type}
      onClick={onClick}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      title={title}
      className={clsx(
        "inline-flex items-center justify-center rounded transition-colors disabled:opacity-50",
        VARIANT_CLASSES[variant],
        SIZE_CLASSES[size],
        className,
      )}
    >
      {loading ? (
        <Loader2 size={size === "sm" ? 10 : size === "lg" ? 14 : 12} className="animate-spin" />
      ) : icon ? (
        icon
      ) : null}
      {children}
    </button>
  );
}
