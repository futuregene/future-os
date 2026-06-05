import { cn } from "../../lib/cn";

export interface TabItem<T extends string> {
  value: T;
  label: string;
}

interface TabsProps<T extends string> {
  items: TabItem<T>[];
  value: T;
  onChange: (value: T) => void;
  className?: string;
}

export function Tabs<T extends string>({ items, value, onChange, className }: TabsProps<T>) {
  return (
    <div className={cn("flex items-center gap-1", className)}>
      {items.map(item => (
        <button
          key={item.value}
          className={cn(
            "h-8 rounded-md px-3 text-sm font-medium text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink",
            value === item.value && "bg-surface-subtle text-ink shadow-sm",
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
