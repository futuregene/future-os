import type { ReactNode } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";

export { Switch } from "../../components/ui/Switch";

/**
 * A row that opens a page of its own.
 *
 * A settings page whose sections are long — a machine's skills, its models, its
 * providers — cannot stay one scrolling column: what the user came for is at the
 * top, and everything below it is a list they scroll past to reach the next
 * setting. The row states what is behind it and how much there is.
 */
export function SettingsLinkRow({
  description,
  disabled,
  onClick,
  title,
}: {
  description?: ReactNode;
  disabled?: boolean;
  onClick: () => void;
  title: ReactNode;
}) {
  return (
    <button
      className="flex w-full items-center gap-4 py-3.5 text-left transition-colors hover:bg-surface-subtle disabled:cursor-default disabled:text-ink-muted disabled:hover:bg-transparent"
      disabled={disabled}
      onClick={onClick}
      type="button"
    >
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-ink">{title}</div>
        {description ? <div className="mt-0.5 text-xs leading-5 text-ink-muted">{description}</div> : null}
      </div>
      <ChevronRight aria-hidden className="size-4 shrink-0 text-ink-muted" />
    </button>
  );
}

/**
 * One of those pages: a way back to where the user came from, named, then the
 * page itself.
 *
 * The heading names the *page*, not just "back": a list of a machine's models
 * reached by a scroll and a click needs to say where it is, and the back control
 * needs to say where it goes — with several machines paired, "back" alone is
 * ambiguous.
 */
export function SettingsSubPage({
  backLabel,
  children,
  onBack,
  title,
}: {
  backLabel: string;
  children: ReactNode;
  onBack: () => void;
  title: string;
}) {
  return (
    <div className="space-y-5">
      <div className="flex items-center gap-2">
        <button
          className="inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium text-accent transition-colors hover:bg-accent-soft"
          onClick={onBack}
          type="button"
        >
          <ChevronLeft aria-hidden className="size-3.5" />
          {backLabel}
        </button>
        <h2 className="min-w-0 truncate text-sm font-semibold text-ink">{title}</h2>
      </div>
      {children}
    </div>
  );
}

export function SettingsList({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-lg border border-line-soft bg-surface px-4 [&>*+*]:border-t [&>*+*]:border-line-soft">
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
