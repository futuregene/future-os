import type { PropsWithChildren } from "react";
import { cn } from "../../lib/cn";

interface PanelProps {
  className?: string;
}

export function Panel({ children, className }: PropsWithChildren<PanelProps>) {
  return (
    <section className={cn("rounded-lg border border-line bg-surface shadow-sm", className)}>
      {children}
    </section>
  );
}
