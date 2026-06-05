import type { PropsWithChildren } from "react";

interface TooltipProps {
  label: string;
}

export function Tooltip({ children, label }: PropsWithChildren<TooltipProps>) {
  return (
    <span className="inline-flex" title={label}>
      {children}
    </span>
  );
}
