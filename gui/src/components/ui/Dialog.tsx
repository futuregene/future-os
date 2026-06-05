import type { PropsWithChildren, ReactNode } from "react";
import { useEffect } from "react";
import { cn } from "../../lib/cn";

interface DialogProps {
  children: ReactNode;
  className?: string;
  description?: ReactNode;
  footer?: ReactNode;
  onClose: () => void;
  open: boolean;
  title: string;
}

export function Dialog({
  children,
  className,
  description,
  footer,
  onClose,
  open,
  title,
}: PropsWithChildren<DialogProps>) {
  useEffect(() => {
    if (!open)
      return;

    function handleKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        onClose();
      }
    }

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [onClose, open]);

  if (!open)
    return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-6">
      <button
        aria-label="Close dialog"
        className="absolute inset-0 cursor-default bg-slate-900/20 backdrop-blur-[1px]"
        onClick={onClose}
        type="button"
      />
      <section
        aria-labelledby="dialog-title"
        aria-modal="true"
        className={cn(
          "relative z-10 w-full max-w-md rounded-xl border border-line-soft bg-white p-5 shadow-[0_24px_60px_rgba(15,23,42,0.18)]",
          className,
        )}
        role="dialog"
      >
        <div className="space-y-1">
          <h2 id="dialog-title" className="text-lg font-semibold text-ink">
            {title}
          </h2>
          {description ? <div className="text-sm leading-5 text-ink-soft">{description}</div> : null}
        </div>
        <div className="mt-5">{children}</div>
        {footer ? <div className="mt-5 flex justify-end gap-2">{footer}</div> : null}
      </section>
    </div>
  );
}
