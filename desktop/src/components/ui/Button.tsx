import type { ButtonHTMLAttributes, PropsWithChildren, ReactNode } from "react";
import { cn } from "../../lib/cn";

type ButtonVariant = "danger" | "danger-soft" | "ghost" | "primary" | "secondary" | "toolbar";
type ButtonSize = "md" | "sm" | "xs";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Icon rendered before the label (already sized, e.g. `size-3.5`). */
  leftIcon?: ReactNode;
}

const variants: Record<ButtonVariant, string> = {
  "danger": "border-danger bg-danger text-white hover:opacity-90",
  "danger-soft": "border-danger-line bg-danger-soft text-danger hover:brightness-95",
  "ghost": "border-transparent bg-transparent text-ink-soft hover:bg-surface-subtle hover:text-ink",
  "primary": "border-accent bg-accent text-white hover:bg-accent-hover disabled:bg-accent-disabled",
  "secondary": "border-line bg-surface text-ink hover:bg-surface-subtle",
  // Muted outlined action button used in toolbars/embeds (icon + short label).
  "toolbar": "border-line bg-surface text-ink-soft hover:bg-surface-subtle hover:text-ink",
};

const sizes: Record<ButtonSize, string> = {
  md: "h-9 px-3 text-sm",
  sm: "h-8 px-3 text-xs",
  xs: "h-7 gap-1.5 px-2 text-xs",
};

export function Button({
  children,
  className,
  leftIcon,
  size = "md",
  variant = "secondary",
  ...props
}: PropsWithChildren<ButtonProps>) {
  return (
    <button
      className={cn(
        // `whitespace-nowrap`: a control's label must not wrap. A two-character
        // button in a squeezed flex row was rendering its label as two stacked
        // lines (the host-settings "Save" button, 44px wide) — a button whose
        // text is unreadable is not a button anyone can use, and the layout
        // should make room instead.
        "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md border font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50",
        sizes[size],
        variants[variant],
        className,
      )}
      {...props}
    >
      {leftIcon}
      {children}
    </button>
  );
}
