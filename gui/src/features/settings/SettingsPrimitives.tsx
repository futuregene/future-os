import type { ReactNode } from "react";

export { Switch } from "../../components/ui/Switch";

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
