import type { ReactNode } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "../../components/ui/Button";

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
  ariaLabel,
  description,
  disabled,
  onClick,
  title,
}: {
  /**
   * The control's accessible name, when the visible title alone does not say
   * enough. A page of these inside one machine's settings repeats words the rail
   * also uses ("Models", "Tasks", "Skills"), and a screen reader hears only the
   * word: naming the machine says whose models these are.
   */
  ariaLabel?: string;
  description?: ReactNode;
  disabled?: boolean;
  onClick: () => void;
  title: ReactNode;
}) {
  return (
    <button
      aria-label={ariaLabel}
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
 * A settings page: a header with the way back at the **top left**, then the page
 * itself.
 *
 * A full page rather than a section that swaps in place, because that is what
 * navigating into a page means: the header sits in the page's own title-bar row,
 * the back control is where a reader looks for it, and the content scrolls under
 * it. Every level uses this shape — a machine's settings, and each long list
 * inside them — so "back" always means the same thing and is always in the same
 * place.
 */
export function SettingsPage({
  backLabel,
  children,
  icon,
  onBack,
  title,
}: {
  backLabel: string;
  children: ReactNode;
  icon?: ReactNode;
  onBack: () => void;
  title: ReactNode;
}) {
  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header className="flex h-12 shrink-0 select-none items-center gap-3 border-b border-line-soft/40 px-4">
        <Button leftIcon={<ChevronLeft className="size-4" />} onClick={onBack} size="xs" variant="ghost">
          {backLabel}
        </Button>
        {icon}
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-ink">{title}</span>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-8">
        <div className="mx-auto w-full max-w-3xl space-y-6">{children}</div>
      </div>
    </section>
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
