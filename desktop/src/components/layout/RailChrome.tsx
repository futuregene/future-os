import type { ReactNode } from "react";
import { ChevronDown, ChevronRight, Folder, Pin } from "lucide-react";
import { cn } from "../../lib/cn";

/**
 * The activity rail's section and group headings.
 *
 * Shared with the merged list for the same reason the rows are: a heading is
 * how a reader tells one list apart from another, and the merged list's headings
 * had drifted into a plain label — no section header at all, no folder on a
 * workspace group, no fold control, and therefore no way to recognise the list
 * as the one they were looking at a moment ago.
 */

/**
 * A section's own heading: the sticky label and its controls.
 *
 * Sticky and opaque on purpose: the label has to stay readable while the list
 * scrolls under it, or the reader loses which section they are in.
 */
export function RailSectionHeader({
  children,
  controls,
  title,
}: {
  children: ReactNode;
  /** Buttons for the right edge (fold, new, menu). */
  controls?: ReactNode;
  title?: string;
}) {
  return (
    <div className="sticky top-0 z-20 flex h-6 items-center justify-between bg-surface px-2 text-xs font-medium text-ink-muted">
      <span title={title}>{children}</span>
      {controls ? <div className="flex items-center gap-0.5">{controls}</div> : null}
    </div>
  );
}

/** The disclosure control a section or group heading folds with. */
export function RailDisclosure({
  collapsed,
  label,
  onToggle,
}: {
  collapsed: boolean;
  label: string;
  onToggle: () => void;
}) {
  return (
    <button
      aria-expanded={!collapsed}
      aria-label={label}
      className="inline-flex size-4 shrink-0 items-center justify-center text-ink-muted transition-colors hover:text-ink-soft"
      onClick={onToggle}
      type="button"
    >
      {collapsed ? <ChevronRight className="size-3.5" /> : <ChevronDown className="size-3.5" />}
    </button>
  );
}

/**
 * The same fold control sized for a section heading's button row.
 *
 * A section's chevron sits in a 20px button rather than a 16px one, which is
 * what the local rail has always used; the difference is the heading's own and
 * is kept in one place so both lists keep it.
 */
export function RailSectionToggle({
  collapsed,
  label,
  onToggle,
}: {
  collapsed: boolean;
  label: string;
  onToggle: () => void;
}) {
  return (
    <button
      aria-expanded={!collapsed}
      aria-label={label}
      className="inline-flex size-5 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft"
      onClick={onToggle}
      title={label}
      type="button"
    >
      {collapsed ? <ChevronRight className="size-3.5" /> : <ChevronDown className="size-3.5" />}
    </button>
  );
}

/** An icon-only control in a section heading. */
export function RailHeaderButton({
  icon,
  label,
  onClick,
}: {
  icon: ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      className="inline-flex size-5 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft"
      onClick={onClick}
      title={label}
      type="button"
    >
      {icon}
    </button>
  );
}

/**
 * A group's heading: its fold control, its name, and whatever qualifies it.
 *
 * `onSelect` is what makes a workspace heading a way into the workspace (the
 * local rail selects every conversation in it); a group with nothing to select —
 * a machine's chat section — passes none and the name is not a button.
 */
export function RailGroupHeader({
  collapsed,
  collapsedLabel,
  expandedLabel,
  glyph,
  menu,
  name,
  onSelect,
  onToggle,
  pinned,
  testId,
}: {
  collapsed: boolean;
  collapsedLabel: string;
  expandedLabel: string;
  /** Trailing qualifier: the machine a group belongs to. */
  glyph?: ReactNode;
  menu?: ReactNode;
  name: string;
  onSelect?: () => void;
  onToggle: () => void;
  pinned?: boolean;
  testId?: string;
}) {
  const label = (
    <>
      <Folder className="size-4 shrink-0 text-ink-soft" />
      <span className="min-w-0 truncate text-sm font-medium text-ink-soft" data-testid={testId} title={name}>
        {name}
      </span>
      {pinned ? <Pin aria-hidden className="size-3.5 shrink-0 text-accent" /> : null}
      {glyph}
    </>
  );

  return (
    <div
      className={cn(
        "group flex h-7 w-full items-center gap-1 rounded-md px-2 text-left transition-colors hover:bg-surface-subtle",
      )}
    >
      <RailDisclosure collapsed={collapsed} label={collapsed ? collapsedLabel : expandedLabel} onToggle={onToggle} />
      {/* gap-1 (not gap-2): the name then lands on the same 48px column as its
          first-level titles, matching the folder icon's role as this header's
          version of the row toggle column. */}
      {onSelect
        ? (
            <button className="flex min-w-0 items-center gap-1 text-left" onClick={onSelect} type="button">
              {label}
            </button>
          )
        : <span className="flex min-w-0 items-center gap-1 text-left">{label}</span>}
      {/* The machine's icon rides with the name (above), so the space that is
          left over goes here, at the end — where it separates the heading from
          its menu instead of pushing the icon out to it. */}
      <span className="min-w-0 flex-1" />
      {menu}
    </div>
  );
}
