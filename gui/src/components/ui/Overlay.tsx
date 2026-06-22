import type { ReactNode } from "react";
import { useEffect } from "react";

/**
 * Full-screen modal scaffold: a dimmed backdrop that closes on click or
 * Escape, with the dialog content centered on top. Shared by every modal so
 * the backdrop tint, z-index, and escape handling live in one place.
 */
export function Overlay({
  children,
  onClose,
  open,
}: {
  children: ReactNode;
  onClose: () => void;
  open: boolean;
}) {
  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose, open]);

  if (!open) {
    return null;
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6">
      <button
        aria-label="关闭"
        className="absolute inset-0 cursor-default bg-ink-strong/20 backdrop-blur-[1px]"
        onClick={onClose}
        type="button"
      />
      {children}
    </div>
  );
}
