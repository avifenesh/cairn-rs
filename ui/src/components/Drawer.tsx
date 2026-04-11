/**
 * Drawer — slide-in side panel.
 *
 * Extracts the pattern from RunsPage DetailPanel and ProvidersPage AddProviderModal.
 * Right-anchored by default, supports configurable width.
 */

import { useEffect } from "react";
import { X } from "lucide-react";
import { clsx } from "clsx";

type DrawerSide = "right" | "left";

interface DrawerProps {
  /** Whether the drawer is open. */
  open: boolean;
  /** Called when the user requests close (backdrop click or X button). */
  onClose: () => void;
  /** Drawer title. */
  title?: string;
  /** Which side the drawer slides in from. */
  side?: DrawerSide;
  /** Width class (default: "w-80"). */
  width?: string;
  /** Show backdrop overlay. */
  backdrop?: boolean;
  /** Drawer body content. */
  children: React.ReactNode;
  /** Footer content (e.g. action buttons). */
  footer?: React.ReactNode;
  className?: string;
}

const SIDE_CLASSES: Record<DrawerSide, string> = {
  right: "right-0 top-0 bottom-0 border-l",
  left:  "left-0 top-0 bottom-0 border-r",
};

export function Drawer({
  open,
  onClose,
  title,
  side = "right",
  width = "w-80",
  backdrop = true,
  children,
  footer,
  className,
}: DrawerProps) {
  // Close on Escape key
  useEffect(() => {
    if (!open) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") onClose();
    }
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <>
      {/* Backdrop */}
      {backdrop && (
        <div
          className="fixed inset-0 z-40 bg-black/60"
          onClick={onClose}
          aria-hidden="true"
        />
      )}

      {/* Panel */}
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title ?? "Drawer"}
        className={clsx(
          "fixed z-50 flex flex-col shadow-2xl",
          "bg-white dark:bg-zinc-950 border-gray-200 dark:border-zinc-800",
          SIDE_CLASSES[side],
          width,
          className,
        )}
      >
        {/* Header */}
        {title && (
          <div className="flex items-center justify-between px-4 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0 bg-white dark:bg-zinc-950">
            <span className="text-[13px] font-medium text-gray-800 dark:text-zinc-200 truncate">
              {title}
            </span>
            <button
              onClick={onClose}
              aria-label="Close"
              className="p-1 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors"
            >
              <X size={14} />
            </button>
          </div>
        )}

        {/* Body */}
        <div className="flex-1 overflow-y-auto">
          {children}
        </div>

        {/* Footer */}
        {footer && (
          <div className="flex items-center justify-end gap-2 px-4 py-3 border-t border-gray-200 dark:border-zinc-800 shrink-0 bg-white dark:bg-zinc-950">
            {footer}
          </div>
        )}
      </div>
    </>
  );
}
