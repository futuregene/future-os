import type { ReactNode } from "react";
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useOverlayLayer } from "./overlayStack";

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
  const { t } = useTranslation("common");
  const { isTop } = useOverlayLayer(open);

  useEffect(() => {
    if (!open) {
      return;
    }
    function handleKeyDown(event: KeyboardEvent) {
      // Only the topmost overlay closes on Escape, so a nested dialog's Escape
      // doesn't also dismiss its parent.
      if (event.key === "Escape" && isTop()) {
        onClose();
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isTop, onClose, open]);

  if (!open) {
    return null;
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6">
      <button
        aria-label={t("close")}
        className="absolute inset-0 cursor-default bg-overlay backdrop-blur-[1px]"
        onClick={onClose}
        type="button"
      />
      {children}
    </div>
  );
}
