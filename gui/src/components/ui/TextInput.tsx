import type { InputHTMLAttributes } from "react";
import { forwardRef } from "react";
import { cn } from "../../lib/cn";

export const inputBaseClass
  = "h-9 w-full rounded-md border border-line-soft bg-surface px-3 text-sm text-ink outline-none transition-colors placeholder:text-ink-muted focus:border-focus focus:ring-2 focus:ring-accent-soft disabled:bg-surface-subtle disabled:text-ink-muted";

export const TextInput = forwardRef<HTMLInputElement, InputHTMLAttributes<HTMLInputElement>>(
  ({ className, ...props }, ref) => {
    return <input ref={ref} className={cn(inputBaseClass, className)} {...props} />;
  },
);
