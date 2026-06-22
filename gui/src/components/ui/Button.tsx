import type { ButtonHTMLAttributes, PropsWithChildren } from "react";
import { cn } from "../../lib/cn";

type ButtonVariant = "danger" | "danger-soft" | "ghost" | "primary" | "secondary";
type ButtonSize = "md" | "sm";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

const variants: Record<ButtonVariant, string> = {
  "danger": "border-danger bg-danger text-white hover:opacity-90",
  "danger-soft": "border-danger-line bg-danger-soft text-danger hover:brightness-95",
  "ghost": "border-transparent bg-transparent text-ink-soft hover:bg-surface-subtle hover:text-ink",
  "primary": "border-accent bg-accent text-white hover:bg-accent-hover",
  "secondary": "border-line bg-surface text-ink hover:bg-surface-subtle",
};

const sizes: Record<ButtonSize, string> = {
  md: "h-9 px-3 text-sm",
  sm: "h-8 px-3 text-xs",
};

export function Button({
  children,
  className,
  size = "md",
  variant = "secondary",
  ...props
}: PropsWithChildren<ButtonProps>) {
  return (
    <button
      className={cn(
        "inline-flex items-center justify-center gap-2 rounded-md border font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50",
        sizes[size],
        variants[variant],
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}
