import type { ButtonHTMLAttributes, ReactNode } from "react";
import { cn } from "../../lib/cn";

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  icon: ReactNode;
  label: string;
  active?: boolean;
}

export function IconButton({ icon, label, active, className, ...props }: IconButtonProps) {
  return (
    <button
      aria-label={label}
      title={label}
      className={cn(
        "inline-flex size-9 items-center justify-center rounded-md border border-transparent text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink",
        active && "border-accent bg-accent-soft text-accent",
        className,
      )}
      {...props}
    >
      {icon}
    </button>
  );
}
