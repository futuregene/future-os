import { cn } from "../../lib/cn";

export interface SegmentItem<T extends string> {
  value: T;
  label: string;
}

interface SegmentedControlProps<T extends string> {
  items: SegmentItem<T>[];
  value: T;
  onChange: (value: T) => void;
  className?: string;
}

export function SegmentedControl<T extends string>({
  items,
  value,
  onChange,
  className,
}: SegmentedControlProps<T>) {
  return (
    <div className={cn("inline-flex rounded-lg border border-line bg-surface-subtle p-1", className)}>
      {items.map(item => (
        <button
          key={item.value}
          className={cn(
            "h-8 min-w-20 rounded-md px-3 text-sm font-medium text-ink-soft transition-colors",
            value === item.value && "bg-surface text-ink shadow-sm",
          )}
          onClick={() => onChange(item.value)}
          type="button"
        >
          {item.label}
        </button>
      ))}
    </div>
  );
}
