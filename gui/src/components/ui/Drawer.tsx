import type { PropsWithChildren } from "react";
import { cn } from "../../lib/cn";

interface DrawerProps {
  expanded: boolean;
  className?: string;
}

export function Drawer({ children, expanded, className }: PropsWithChildren<DrawerProps>) {
  return (
    <aside
      className={cn(
        "border-t border-line bg-surface transition-[height] duration-200",
        expanded ? "h-64" : "h-12",
        className,
      )}
    >
      {children}
    </aside>
  );
}
