import type { InputHTMLAttributes, Ref } from "react";
import { cn } from "../../lib/cn";

export const inputBaseClass
  = "h-9 w-full rounded-md border border-line-soft bg-surface px-3 text-sm text-ink outline-none transition-colors placeholder:text-ink-muted focus:border-focus focus:ring-2 focus:ring-focus disabled:bg-surface-subtle disabled:text-ink-muted";

// React 19: `ref` is an ordinary prop, so `forwardRef` is no longer needed.
export function TextInput({
  className,
  ref,
  ...props
}: InputHTMLAttributes<HTMLInputElement> & { ref?: Ref<HTMLInputElement> }) {
  return <input ref={ref} className={cn(inputBaseClass, className)} {...props} />;
}
