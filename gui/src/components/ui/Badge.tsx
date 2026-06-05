import type { PropsWithChildren } from "react";
import { cn } from "../../lib/cn";

interface BadgeProps {
  tone?: "neutral" | "accent" | "success" | "warning" | "danger";
  className?: string;
}

const tones = {
  neutral: "border-line bg-surface-subtle text-ink-soft",
  accent: "border-blue-200 bg-accent-soft text-accent",
  success: "border-green-200 bg-green-50 text-green-700",
  warning: "border-amber-200 bg-amber-50 text-amber-700",
  danger: "border-red-200 bg-red-50 text-red-700",
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
