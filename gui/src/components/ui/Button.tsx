import type { ButtonHTMLAttributes, PropsWithChildren } from "react";
import { cn } from "../../lib/cn";

type ButtonVariant = "danger" | "ghost" | "primary" | "secondary";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
}

const variants: Record<ButtonVariant, string> = {
  danger: "border-red-600 bg-red-600 text-white hover:bg-red-700",
  ghost: "border-transparent bg-transparent text-ink-soft hover:bg-surface-subtle hover:text-ink",
  primary: "border-accent bg-accent text-white hover:bg-blue-700",
  secondary: "border-line bg-surface text-ink hover:bg-surface-subtle",
};

export function Button({
  children,
  className,
  variant = "secondary",
  ...props
}: PropsWithChildren<ButtonProps>) {
  return (
    <button
      className={cn(
        "inline-flex h-9 items-center justify-center gap-2 rounded-md border px-3 text-sm font-medium transition-colors disabled:cursor-not-allowed disabled:opacity-50",
        variants[variant],
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}
