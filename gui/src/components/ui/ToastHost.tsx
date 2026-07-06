import { useCallback, useEffect, useState } from "react";
import { cn } from "../../lib/cn";
import { onFutureEvent } from "../../lib/futureEvents";

interface Toast {
  id: number;
  message: string;
  tone: "error" | "info";
}

const DISMISS_MS = 3200;

/**
 * Global transient-message host. Listens for `toast` events on the typed bus and
 * renders a bottom-centered stack that auto-dismisses. Mounted once at the app
 * root so any component can `emitFutureEvent("toast", …)` without prop drilling.
 */
export function ToastHost() {
  const [toasts, setToasts] = useState<Toast[]>([]);

  useEffect(() => {
    let counter = 0;
    return onFutureEvent("toast", ({ message, tone }) => {
      setToasts(current => [...current, { id: counter++, message, tone: tone ?? "info" }]);
    });
  }, []);

  const dismiss = useCallback((id: number) => {
    setToasts(current => current.filter(toast => toast.id !== id));
  }, []);

  if (toasts.length === 0)
    return null;

  return (
    <div className="pointer-events-none fixed inset-x-0 top-16 z-50 flex flex-col items-center gap-2">
      {toasts.map(toast => (
        <ToastItem key={toast.id} onDismiss={dismiss} toast={toast} />
      ))}
    </div>
  );
}

function ToastItem({ toast, onDismiss }: { toast: Toast; onDismiss: (id: number) => void }) {
  useEffect(() => {
    const timer = window.setTimeout(onDismiss, DISMISS_MS, toast.id);
    return () => window.clearTimeout(timer);
  }, [toast.id, onDismiss]);

  return (
    <div
      className={cn(
        "pointer-events-auto max-w-[90vw] truncate rounded-lg border px-3 py-2 text-sm shadow-panel",
        toast.tone === "error"
          ? "border-danger-line bg-danger-soft text-danger"
          : "border-line-soft bg-surface text-ink",
      )}
    >
      {toast.message}
    </div>
  );
}
