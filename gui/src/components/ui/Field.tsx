import type { ReactNode } from "react";

export function Field({ children, label }: { children: ReactNode; label: ReactNode }) {
  return (
    <label className="block space-y-1">
      <span className="text-sm font-medium text-ink">{label}</span>
      {children}
    </label>
  );
}
