import type { PropsWithChildren, SelectHTMLAttributes } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "../../lib/cn";

type SelectSize = "md" | "sm" | "xs";

interface SelectProps extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "size"> {
  size?: SelectSize;
  /** Applied to the relative wrapper (e.g. width constraints). */
  wrapperClassName?: string;
}

const sizes: Record<SelectSize, string> = {
  md: "h-9 text-sm",
  sm: "h-8 text-sm",
  xs: "h-7 text-xs",
};

export function Select({
  children,
  className,
  size = "md",
  wrapperClassName,
  ...props
}: PropsWithChildren<SelectProps>) {
  return (
    <div className={cn("relative", wrapperClassName)}>
      <select
        className={cn(
          "w-full appearance-none rounded-md border border-line-soft bg-surface pl-3 pr-8 text-ink outline-none transition-colors focus:border-focus focus:ring-2 focus:ring-focus",
          sizes[size],
          className,
        )}
        {...props}
      >
        {children}
      </select>
      <ChevronDown className="pointer-events-none absolute right-2.5 top-1/2 size-4 -translate-y-1/2 text-ink-muted" />
    </div>
  );
}
