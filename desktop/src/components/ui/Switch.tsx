import { cn } from "../../lib/cn";

export function Switch({
  checked,
  disabled,
  label,
  mixed = false,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label?: string;
  /**
   * "Some of the things this switch stands for are on, some are off" — a
   * provider row whose models are partly hidden. A plain boolean cannot say
   * that: `checked={every(...)}` renders a half-hidden group as off, so the
   * user reads the whole group as unavailable. Assistive technology and the
   * harness both read the third state from `aria-checked="mixed"`, and
   * clicking it turns everything on (the same intent as clicking a partial
   * checkbox in a tree).
   */
  mixed?: boolean;
  onChange: (checked: boolean) => void;
}) {
  const knob = mixed ? "translate-x-2.25" : checked ? "translate-x-4.5" : "translate-x-0.5";
  return (
    <button
      aria-checked={mixed ? "mixed" : checked}
      aria-label={label}
      className={cn(
        "relative inline-flex h-6 w-10 shrink-0 cursor-pointer items-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-50",
        mixed ? "bg-line" : checked ? "bg-ink-strong" : "bg-line",
      )}
      disabled={disabled}
      onClick={() => onChange(mixed ? true : !checked)}
      role="switch"
      type="button"
    >
      <span
        className={cn(
          "inline-block size-5 transform rounded-full bg-surface shadow-xs transition-transform",
          knob,
        )}
      />
    </button>
  );
}
