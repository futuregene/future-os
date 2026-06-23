import type { PropsWithChildren, ReactNode } from "react";
import { cn } from "../../lib/cn";
import { Overlay } from "./Overlay";

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
  return (
    <Overlay onClose={onClose} open={open}>
      <section
        aria-labelledby="dialog-title"
        aria-modal="true"
        className={cn(
          "relative z-10 w-full max-w-md rounded-xl border border-line-soft bg-surface p-5 shadow-dialog",
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
    </Overlay>
  );
}
