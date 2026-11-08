import type { ReactNode } from "react";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "../../components/ui/Button";
import { cn } from "../../lib/cn";

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
   * also uses ("Models", "Scheduled tasks", "Skills"), and a screen reader hears only the
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
      // `w-full px-4`: the row carries the card's inset rather than sitting
      // inside it, so the hover fill reaches both of the card's edges and the
      // between-rows divider spans the same width. Padding on the card instead
      // left the fill 16px short of each edge, which read as a band floating in
      // a white frame (a paired machine's "Fusion" row had exactly that).
      // `w-full` is required, not redundant: a `<button>` shrinks to its content
      // in a block flow — dropping it here shrank the fill *and* the divider to
      // the length of the longest line of text. The card's `overflow-hidden`
      // keeps the fill off its rounded corners, which is also why the focus ring
      // is drawn `inset` (an outset ring would be clipped at the row's edges).
      className="flex w-full items-center gap-4 px-4 py-3.5 text-left transition-colors hover:bg-surface-subtle focus-visible:ring-2 focus-visible:ring-focus focus-visible:ring-inset focus-visible:outline-none disabled:cursor-default disabled:text-ink-muted disabled:hover:bg-transparent"
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

/**
 * The card a settings page's rows sit in.
 *
 * No horizontal padding here: each row carries its own (`SettingsRow`,
 * `SettingsLinkRow`), because a row that paints a hover fill has to reach the
 * card's edges — inside a card-level inset that fill stopped 16px short on both
 * sides, and so did the between-rows dividers. Clipped, so the fill of a row
 * touching an edge is rounded off with the card instead of squaring it — which
 * is also why those rows draw their focus ring `inset`.
 */
export function SettingsList({ children }: { children: ReactNode }) {
  return (
    <div className="overflow-hidden rounded-lg border border-line-soft bg-surface [&>*+*]:border-t [&>*+*]:border-line-soft">
      {children}
    </div>
  );
}

export function SettingsRow({
  children,
  description,
  stacked,
  title,
}: {
  children?: ReactNode;
  description?: ReactNode;
  /**
   * Put the control **below** the label instead of beside it.
   *
   * For a control that is itself as wide as the column — a grid of glyphs, not a
   * switch. The row shape cannot hold both: the label is the only item that can
   * give width, so it collapses to zero (the title drawn one character per line)
   * while the control, held at its unwrapped width by `shrink-0`, runs out of
   * the card. Both were measured on the machine icon row before this existed:
   * the label 0px wide, the picker 247px past the card's edge.
   */
  stacked?: boolean;
  title: ReactNode;
}) {
  return (
    <div className={cn("flex gap-4 px-4 py-3.5", stacked ? "flex-col" : "items-center")}>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium text-ink">{title}</div>
        {description ? <div className="mt-0.5 text-xs leading-5 text-ink-muted">{description}</div> : null}
      </div>
      {children
        ? (
            <div className={cn("flex items-center", stacked ? "min-w-0 justify-start" : "shrink-0 justify-end")}>
              {children}
            </div>
          )
        : null}
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
