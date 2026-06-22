import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

export function Switch({
  checked,
  disabled,
  label,
  onChange,
}: {
  checked: boolean;
  disabled?: boolean;
  label?: string;
  onChange: (checked: boolean) => void;
}) {
  return (
    <button
      aria-checked={checked}
      aria-label={label}
      className={cn(
        "relative inline-flex h-6 w-10 shrink-0 cursor-pointer items-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-50",
        checked ? "bg-slate-900" : "bg-slate-200",
      )}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      role="switch"
      type="button"
    >
      <span
        className={cn(
          "inline-block size-5 transform rounded-full bg-white shadow-sm transition-transform",
          checked ? "translate-x-[18px]" : "translate-x-[2px]",
        )}
      />
    </button>
  );
}

export function SettingsList({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-lg border border-line-soft bg-white px-4 [&>*+*]:border-t [&>*+*]:border-line-soft">
      {children}
    </div>
  );
}

export function SettingsRow({
  children,
  description,
  title,
}: {
  children?: ReactNode;
  description?: ReactNode;
  title: ReactNode;
}) {
  return (
    <div className="flex items-center gap-4 py-3.5">
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-ink">{title}</div>
        {description ? <div className="mt-0.5 text-xs leading-5 text-ink-muted">{description}</div> : null}
      </div>
      {children ? <div className="flex shrink-0 items-center justify-end">{children}</div> : null}
    </div>
  );
}

export function SettingsSection({
  action,
  children,
  description,
  title,
}: {
  action?: ReactNode;
  children: ReactNode;
  description?: ReactNode;
  title?: ReactNode;
}) {
  return (
    <section className="space-y-2.5">
      {title || action
        ? (
            <div className="flex items-end justify-between gap-3">
              <div>
                {title ? <h3 className="text-sm font-semibold text-ink">{title}</h3> : null}
                {description ? <p className="mt-0.5 text-xs leading-5 text-ink-muted">{description}</p> : null}
              </div>
              {action ?? null}
            </div>
          )
        : null}
      {children}
    </section>
  );
}
