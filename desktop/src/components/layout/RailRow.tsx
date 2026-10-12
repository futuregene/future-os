import type { ReactNode, Ref } from "react";
import type { ThreadRunInfo } from "./hooks/useThreadStore";
import { MoreHorizontal } from "lucide-react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";

/**
 * The frame, the run indicator and the actions trigger that every row in the
 * activity rail shares.
 *
 * These three are extracted because they are what makes a list look like one
 * list, and because keeping them in two components had already let them diverge:
 * the local row reserved a fixed 20px slot for its indicator and revealed its
 * menu without moving anything, while the remote row drew a smaller dot inline
 * and held its menu open with a 24px button that stayed in the layout — so the
 * merged list's rows were a different height, a different indent and a different
 * spacing from the local ones. Two implementations that must look identical is a
 * rule nothing enforces; one implementation is.
 *
 * What is *not* here is deliberate: the leading tree toggle, the approval badge,
 * the selection checkbox and the source icon are each one list's own, and a
 * shared slot for all of them would be a prop bag rather than a shell.
 */

/** A row's frame: geometry, the full-width click target, and hover/active. */
export function RailRow({
  active,
  children,
  compact,
  depth = 0,
  onContextMenu,
  onSelect,
  rowRef,
  title,
}: {
  active: boolean;
  /** Row content, laid out left to right after the click target. */
  children: ReactNode;
  /**
   * Whether this is a row inside a group rather than a top-level one.
   *
   * The rail's own rule, unchanged: a workspace's conversations are one step in
   * and a notch shorter, so a group and its heading read as one unit.
   */
  compact?: boolean;
  depth?: number;
  /** Right-click anywhere on the row opens the same menu as its trigger. */
  onContextMenu?: () => void;
  onSelect: () => void;
  rowRef?: Ref<HTMLDivElement>;
  /** Also the frame's `aria-label` and tooltip via the click target. */
  title: string;
}) {
  return (
    <div
      ref={rowRef}
      className={cn(
        "group/thread relative flex w-full items-center gap-1 rounded-md pr-2 text-left transition-colors hover:bg-surface-subtle",
        compact ? "h-7" : "h-8",
        active && "bg-surface-subtle text-ink",
      )}
      // Row start = the title column of a leaf (chat/pinned 16, workspace 28,
      // unchanged); each level adds 16px of toggle + 4px of gap, so a child's
      // row start lands exactly on its parent's title column.
      style={{ paddingLeft: (compact ? 28 : 16) + depth * 20 }}
      onContextMenu={onContextMenu
        ? (event) => {
            event.preventDefault();
            onContextMenu();
          }
        : undefined}
    >
      {/* Full-row click target so the whole (highlighted) row selects it.
          Content below sits on top but is pointer-events-none so clicks fall
          through to this button; the actions trigger and menu keep a higher
          stacking (z-10 / z-40) so they stay clickable and never mis-fire. */}
      <button
        aria-label={title}
        className="absolute inset-0 rounded-md"
        onClick={onSelect}
        title={title}
        type="button"
      />
      {children}
    </div>
  );
}

/** A row's title, at the one size and weight every row uses. */
export function RailRowTitle({ children, grow = true, muted }: { children: ReactNode; grow?: boolean; muted?: boolean }) {
  return (
    <span
      className={cn(
        "pointer-events-none min-w-0 truncate text-sm font-medium",
        // `flex-1` while the title is the row's last flexible element (its
        // indicator and menu belong at the right edge). A row that puts a
        // qualifier *after* the title — the machine a conversation is on —
        // passes `grow={false}` and adds the spacer itself, so the qualifier
        // sits against the title instead of a pane's width away from it.
        grow && "flex-1",
        muted ? "text-ink-muted" : "text-ink-soft",
      )}
    >
      {children}
    </span>
  );
}

/**
 * The flexible space between a row's content and its right-hand controls.
 *
 * Needed wherever something follows the title, because the title then cannot be
 * the flexible element itself.
 */
export function RailRowSpacer() {
  return <span className="min-w-0 flex-1" />;
}

/**
 * The row's run indicator, in a slot whose width is always reserved.
 *
 * The reservation is the point: without it a row with nothing to report is
 * narrower than one that is running, and every title in the list shifts the
 * moment a run starts or finishes.
 *
 * The reservation has to hold *while the pointer is over the row* too, which is
 * when the row is busiest: the actions trigger takes the indicator's place on
 * hover. Hiding only the glyph inside left this wrapper in the layout as a
 * zero-width flex item, and its two gaps stayed — so the title, which is the
 * row's flexible element, lost 4px and re-wrapped its last word under the
 * pointer, while a remote row (whose title cannot flex) did not move at all.
 * Hide the slot itself; the trailing width is then the same either way, and the
 * trigger lands exactly where the indicator was.
 */
export function RailRowIndicator({ status, unread }: { status?: ThreadRunInfo["status"]; unread?: boolean }) {
  return (
    <span className="pointer-events-none flex shrink-0 group-hover/thread:hidden">
      <RunIndicator status={status} unread={unread} />
    </span>
  );
}

export function RunIndicator({ status, unread }: { status?: ThreadRunInfo["status"]; unread?: boolean }) {
  const { t } = useTranslation("layout");
  // Reserved-width placeholder so idle rows (and the hover state) stay aligned.
  const placeholder = <span className="size-5 shrink-0" />;

  if (status === "queued" || status === "running" || status === "waiting_approval") {
    return (
      <span
        aria-label={t("activityRail.running")}
        className="inline-flex size-5 shrink-0 items-center justify-center"
        title={t("activityRail.running")}
      >
        <span className="size-3 animate-spin rounded-full border-2 border-accent-soft border-t-accent" />
      </span>
    );
  }

  // A finished run is "unread" until the row is opened: green when it
  // completed, red when it failed. Once read no dot shows. `cancelled` is a
  // deliberate user action (they aborted the run), so it never needs surfacing
  // as unread — it falls through to the empty placeholder below.
  if (unread && (status === "completed" || status === "failed")) {
    const failed = status === "failed";
    const label = failed ? t("activityRail.failed") : t("activityRail.completed");
    return (
      <span
        aria-label={label}
        className="inline-flex size-5 shrink-0 items-center justify-center"
        title={label}
      >
        <span className={cn("size-2 rounded-full", failed ? "bg-danger" : "bg-success")} />
      </span>
    );
  }

  return placeholder;
}

/**
 * The row's actions trigger.
 *
 * `hidden` → `inline-flex` rather than opacity: the indicator's slot already
 * keeps the column steady, and a trigger that occupies space while invisible
 * would shift the title on hover instead. It stays mounted either way, so a
 * control that appears on hover is still reachable by keyboard focus.
 */
export function RailRowMenuButton({
  buttonRef,
  label,
  menuOpen,
  onToggle,
}: {
  buttonRef?: Ref<HTMLButtonElement>;
  label: string;
  menuOpen: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      ref={buttonRef}
      aria-expanded={menuOpen}
      aria-haspopup="menu"
      aria-label={label}
      className={cn(
        "relative z-10 hidden size-5 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft group-hover/thread:inline-flex",
        menuOpen && "inline-flex",
      )}
      onClick={(event) => {
        // Without this the click would also reach the row's click target
        // underneath and open what the user wanted to act on.
        event.stopPropagation();
        onToggle();
      }}
      title={label}
      type="button"
    >
      <MoreHorizontal className="size-3.5" />
    </button>
  );
}
