import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import { Archive, Folder } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listThreadsPage } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { ArchivedThreadMenu } from "./ActivityRailMenus";
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
 * Expanding it browses the workspace's conversations (they keep their own
 * `active` status — the workspace, not they, is what was shelved), so the reader
 * can open one read-only before deciding to restore the whole workspace.
 */
export function ArchivedWorkspaceGroup({
  children,
  count,
  expanded,
  onRestore,
  onToggle,
  workspace,
}: {
  children?: React.ReactNode;
  count: number | null;
  expanded: boolean;
  onRestore: (workspace: StoredWorkspace) => void;
  onToggle: () => void;
  workspace: StoredWorkspace;
}) {
  const { t } = useTranslation("layout");
  return (
    <div className="space-y-0.5">
      <div className="flex h-7 w-full items-center gap-1 rounded-md px-2 text-left transition-colors hover:bg-surface-subtle">
        <button
          aria-expanded={expanded}
          aria-label={expanded ? t("activityRail.collapseWorkspace") : t("activityRail.expandWorkspace")}
          className="inline-flex size-4 shrink-0 items-center justify-center rounded text-ink-muted hover:text-ink-soft"
          onClick={onToggle}
          type="button"
        >
          <span className="text-xs leading-none">{expanded ? "−" : "+"}</span>
        </button>
        <Folder className="size-4 shrink-0 text-ink-muted" />
        <span className="min-w-0 flex-1 truncate text-13 font-medium text-ink-soft" title={workspace.name}>
          {workspace.name}
        </span>
        {count !== null
          ? <span className="shrink-0 text-2xs tabular-nums text-ink-muted">{count}</span>
          : null}
        <button
          className="shrink-0 rounded px-1.5 text-2xs text-accent transition-colors hover:bg-surface-subtle"
          onClick={() => onRestore(workspace)}
          type="button"
        >
          {t("activityRail.restoreWorkspace")}
        </button>
      </div>
      {expanded ? children : null}
    </div>
  );
}

/** Section heading shared by the archived view's two halves. */
export function ArchivedSectionHeading({
  action,
  label,
  icon,
}: {
  /** Optional trailing control (the archived list's selection menu). */
  action?: React.ReactNode;
  label: string;
  icon?: React.ReactNode;
}) {
  return (
    <div className="sticky top-0 z-20 flex h-6 items-center gap-1.5 bg-surface px-2 text-xs font-medium text-ink-muted">
      {icon ?? <Archive className="size-3.5" />}
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {action}
    </div>
  );
}

/** Empty state: the archived view has nothing in this half. */
export function ArchivedEmpty({ label }: { label: string }) {
  return <div className={cn("px-2 py-1 text-xs text-ink-muted")}>{label}</div>;
}

/**
 * The conversations inside one archived workspace, fetched when the group is
 * expanded.
 *
 * They keep their own `active` status — the workspace, not they, is what was
 * shelved — so this reads the `workspace-archived:<id>` scope, which returns
 * them whatever their status. A reader can therefore open one read-only and look
 * before deciding to restore the whole workspace.
 */
export function ArchivedWorkspaceContents({
  onOpen,
  workspaceId,
}: {
  onOpen: (thread: StoredThread) => void;
  workspaceId: string;
}) {
  const { t } = useTranslation("layout");
  const [threads, setThreads] = useState<StoredThread[] | null>(null);
  useEffect(() => {
    let cancelled = false;
    void listThreadsPage(`workspace-archived:${workspaceId}`, null, 50)
      .then((page) => {
        if (!cancelled)
          setThreads(page.threads);
      })
      .catch(() => {
        if (!cancelled)
          setThreads([]);
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  if (threads === null)
    return <div className="px-4 py-1 text-xs text-ink-muted">{t("activityRail.loading")}</div>;
  if (threads.length === 0)
    return <div className="px-4 py-1 text-xs text-ink-muted">{t("activityRail.archivedThreadsEmpty")}</div>;
  return (
    <div className="space-y-0.5 pl-2">
      {threads.map(thread => (
        <RailRow active={false} compact key={thread.id} onSelect={() => onOpen(thread)} title={thread.title}>
          <RailRowTitle>{thread.title}</RailRowTitle>
        </RailRow>
      ))}
    </div>
  );
}
