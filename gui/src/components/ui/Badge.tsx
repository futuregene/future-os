import type { PropsWithChildren } from "react";
import { cn } from "../../lib/cn";

interface BadgeProps {
  tone?: "neutral" | "accent" | "info" | "success" | "warning" | "danger";
  className?: string;
}

const tones = {
  neutral: "border-line bg-surface-subtle text-ink-soft",
  accent: "border-info-line bg-accent-soft text-accent",
  info: "border-info-line bg-info-soft text-info",
  success: "border-success-line bg-success-soft text-success",
  warning: "border-warning-line bg-warning-soft text-warning",
  danger: "border-danger-line bg-danger-soft text-danger",
};

export function Badge({
  children,
  tone = "neutral",
  className,
}: PropsWithChildren<BadgeProps>) {
  return (
    <span
      className={cn(
        "inline-flex h-6 items-center rounded-md border px-2 text-xs font-medium",
        tones[tone],
        className,
      )}
    >
      {children}
    </span>
  );
}
