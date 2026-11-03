import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import { Archive, ChevronDown, ChevronRight, Folder } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { ArchivedThreadMenu } from "./ActivityRailMenus";
import { RailDisclosure } from "./RailChrome";
import { RailRow, RailRowMenuButton, RailRowTitle } from "./RailRow";

/**
 * One archived conversation tree's row in the archived view.
 *
 * Drawn exactly like an active row — title, optional tree count, a hover-revealed
 * "more" menu — because it is the same kind of thing: a conversation the reader
 * can open, rename, delete or restore. What it does *not* do is invent a second
 * row language: there are no inline action buttons and no leading icon, so the
 * archived list reads as the same list with different contents.
 *
 * The tree is the unit: `members` is every conversation archived with this root,
 * so restoring it brings all of them back.
 */
export function ArchivedThreadRow({
  active,
  members,
  onDelete,
  onOpen,
  onRename,
  onRestore,
  onToggleSelection,
  selected,
  selectionMode,
  thread,
}: {
  active: boolean;
  /** How many conversations this archived tree holds. */
  members: number;
  onDelete: (thread: StoredThread) => void;
  onOpen: (thread: StoredThread) => void;
  onRename: (thread: StoredThread) => void;
  onRestore: (thread: StoredThread) => void;
  onToggleSelection?: (thread: StoredThread) => void;
  selected?: boolean;
  selectionMode?: boolean;
  thread: StoredThread;
}) {
  const { t } = useTranslation("layout");
  const [menuOpen, setMenuOpen] = useState(false);
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useDismissableLayer<HTMLDivElement>({
    enabled: menuOpen,
    onDismiss: () => setMenuOpen(false),
    onEscapeDismiss: () => {
      setMenuOpen(false);
      menuButtonRef.current?.focus();
    },
  });

  return (
    <RailRow
      active={active}
      compact
      onContextMenu={() => setMenuOpen(true)}
      onSelect={() => onOpen(thread)}
      rowRef={menuRef}
      title={thread.title}
    >
      <RailRowTitle>{thread.title}</RailRowTitle>
      {selectionMode
        ? (
            <input
              aria-label={t("activityRail.selectThread", { title: thread.title })}
              checked={selected ?? false}
              className="relative z-10 size-5 shrink-0 rounded border-line accent-accent"
              onChange={(event) => {
                event.stopPropagation();
                onToggleSelection?.(thread);
              }}
              type="checkbox"
            />
          )
        : (
            <>
              {members > 1
                ? (
                    <span className="pointer-events-none shrink-0 text-2xs tabular-nums text-ink-muted group-hover/thread:hidden">
                      {t("activityRail.archivedTreeMembers", { count: members })}
                    </span>
                  )
                : null}
              <RailRowMenuButton
                buttonRef={menuButtonRef}
                label={t("activityRail.threadActions", { title: thread.title })}
                menuOpen={menuOpen}
                onToggle={() => setMenuOpen(!menuOpen)}
              />
              {menuOpen
                ? (
                    <ArchivedThreadMenu
                      onClose={() => setMenuOpen(false)}
                      onDelete={() => onDelete(thread)}
                      onRename={() => onRename(thread)}
                      onRestore={() => onRestore(thread)}
                    />
                  )
                : null}
            </>
          )}
    </RailRow>
  );
}

/**
 * One archived workspace's heading in the archived view.
 *
 * Drawn like an active workspace's heading — disclosure, folder, name, then the
 * controls folded into a "more" menu — because it is the same thing with a
 * different status. The archived list used to spell "restore" out as a text
 * button on the heading, which made it the one control in the rail that did not
 * fold away; a reader with several shelved workspaces saw a column of them.
 *
 * Expanding it browses the workspace's conversations (they keep their own
 * `active` status — the workspace, not they, is what was shelved), so the reader
 * can open one read-only before deciding to restore everything.
 */
export function ArchivedWorkspaceGroup({
  children,
  count,
  expanded,
  menu,
  onOpenMenu,
  onToggle,
  workspace,
}: {
  children?: React.ReactNode;
  count: number | null;
  expanded: boolean;
  /** The heading's actions menu (rendered by the owner, which knows the commands). */
  menu: React.ReactNode;
  /** Right-click on the heading asks the owner to open that same menu. */
  onOpenMenu: () => void;
  onToggle: () => void;
  workspace: StoredWorkspace;
}) {
  const { t } = useTranslation("layout");
  return (
    <div className="space-y-0.5">
      <div
        className="group/workspace flex h-7 w-full items-center gap-1 rounded-md px-2 text-left transition-colors hover:bg-surface-subtle"
        onContextMenu={(event) => {
          event.preventDefault();
          onOpenMenu();
        }}
      >
        {/* The same disclosure the active list's workspace headings fold with:
            a +/− pair read as a different control from the chevron one row
            above, and a group heading is a group heading whichever list it is
            in. */}
        <RailDisclosure collapsed={!expanded} label={expanded ? t("activityRail.collapseWorkspace") : t("activityRail.expandWorkspace")} onToggle={onToggle} />
        <Folder className="size-4 shrink-0 text-ink-soft" />
        <span className="min-w-0 flex-1 truncate text-13 font-semibold text-ink" title={workspace.name}>
          {workspace.name}
        </span>
        {count !== null
          ? <span className="shrink-0 text-2xs tabular-nums text-ink-muted">{count}</span>
          : null}
        {menu}
      </div>
      {expanded ? children : null}
    </div>
  );
}

/**
 * Section heading shared by the archived view's two halves.
 *
 * Laid out exactly like the active list's section headings — the label on the
 * left, one control cluster at the right end — because the fold is the same
 * gesture in both lists. The chevron used to sit before the label here, which
 * put the one control a reader reaches for at the head of the section on the
 * opposite side from where the rest of the rail keeps it.
 */
export function ArchivedSectionHeading({
  action,
  collapsed,
  label,
  icon,
  onToggle,
}: {
  /** Optional trailing control (the archived list's selection menu). */
  action?: React.ReactNode;
  /** Present when the section folds; the chevron only draws with `onToggle`. */
  collapsed?: boolean;
  label: string;
  icon?: React.ReactNode;
  /** Fold/unfold this section — the same one-click the active sections have. */
  onToggle?: () => void;
}) {
  return (
    <div className="sticky top-0 z-20 flex h-6 items-center justify-between bg-surface px-2 text-xs font-medium text-ink-muted">
      <span className="flex min-w-0 items-center gap-1.5">
        {icon ?? <Archive className="size-3.5 shrink-0" />}
        <span className="truncate">{label}</span>
      </span>
      {onToggle || action
        ? (
            <div className="flex items-center gap-0.5">
              {onToggle
                ? (
                    <button
                      aria-expanded={collapsed !== true}
                      aria-label={label}
                      className="inline-flex size-5 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft"
                      onClick={onToggle}
                      title={label}
                      type="button"
                    >
                      {collapsed ? <ChevronRight className="size-3.5" /> : <ChevronDown className="size-3.5" />}
                    </button>
                  )
                : null}
              {action}
            </div>
          )
        : null}
    </div>
  );
}

/** Empty state: the archived view has nothing in this half. */
export function ArchivedEmpty({ label }: { label: string }) {
  return <div className={cn("px-2 py-1 text-xs text-ink-muted")}>{label}</div>;
}

/**
 * The conversations inside one archived workspace, drawn as plain rows.
 *
 * The rows come from the shell's archive view (which fetched them for this
 * group), not from a private fetch here: the same array is what a batch
 * selection addresses, so it must be one copy. They keep their own `active`
 * status — the workspace, not they, is what was shelved — so opening one shows
 * it read-only before the reader decides to restore anything.
 */
export function ArchivedWorkspaceContents({
  hasMore,
  loading,
  onLoadMore,
  onOpen,
  onToggleSelection,
  selectedIds,
  selectionMode,
  threads,
}: {
  /** Whether this group's own list has another page. */
  hasMore?: boolean;
  loading: boolean;
  onLoadMore?: () => void;
  onOpen: (thread: StoredThread) => void;
  onToggleSelection?: (thread: StoredThread) => void;
  /** Ids of the rows ticked in a batch. */
  selectedIds?: ReadonlySet<string>;
  selectionMode?: boolean;
  threads: StoredThread[] | undefined;
}) {
  const { t } = useTranslation("layout");
  if (threads === undefined)
    return <div className="px-4 py-1 text-xs text-ink-muted">{t("activityRail.loading")}</div>;
  if (loading && threads.length === 0)
    return <div className="px-4 py-1 text-xs text-ink-muted">{t("activityRail.loading")}</div>;
  if (threads.length === 0)
    return <div className="px-4 py-1 text-xs text-ink-muted">{t("activityRail.archivedThreadsEmpty")}</div>;
  return (
    <div className="space-y-0.5 pl-2">
      {threads.map(thread => (
        <RailRow active={false} compact key={thread.id} onSelect={() => onOpen(thread)} title={thread.title}>
          <RailRowTitle>{thread.title}</RailRowTitle>
          {selectionMode
            ? (
                <input
                  aria-label={t("activityRail.selectThread", { title: thread.title })}
                  checked={selectedIds?.has(thread.id) ?? false}
                  className="relative z-10 size-5 shrink-0 rounded border-line accent-accent"
                  onChange={(event) => {
                    event.stopPropagation();
                    onToggleSelection?.(thread);
                  }}
                  type="checkbox"
                />
              )
            : null}
        </RailRow>
      ))}
      {hasMore
        ? (
            <button
              className="w-full rounded-md px-2 py-1 text-left text-xs text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft"
              onClick={onLoadMore}
              type="button"
            >
              {t("activityRail.loadMore")}
            </button>
          )
        : null}
    </div>
  );
}
