import type { StoredThread, StoredWorkspace } from "../../integrations/storage/threadStore";
import { Archive, Folder, MessageSquare } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listThreadsPage } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { RailRow } from "./RailRow";

/**
 * One archived conversation tree's row in the archived view.
 *
 * The row is a browse target first (clicking it opens the read-only detail) and
 * an action target second (its own menu: restore, delete). The tree is the unit:
 * `members` is every conversation archived with this root, so the count says how
 * much a restore brings back instead of making the reader guess.
 */
export function ArchivedThreadRow({
  active,
  members,
  onOpen,
  onRestore,
  onDelete,
  thread,
  workspaceName,
}: {
  active: boolean;
  /** How many conversations this archived tree holds. */
  members: number;
  onOpen: (thread: StoredThread) => void;
  onRestore: (thread: StoredThread) => void;
  onDelete: (thread: StoredThread) => void;
  thread: StoredThread;
  workspaceName: string | null;
}) {
  const { t } = useTranslation("layout");
  return (
    <div className="group/archive-row relative">
      <RailRow active={active} compact onSelect={() => onOpen(thread)} title={thread.title}>
        <MessageSquare className="size-3.5 shrink-0 text-ink-muted" />
        <span className="min-w-0 flex-1 truncate text-13 font-medium text-ink-soft">{thread.title}</span>
        <span className="pointer-events-none shrink-0 text-2xs tabular-nums text-ink-muted">
          {members > 1 ? t("activityRail.archivedTreeMembers", { count: members }) : null}
        </span>
      </RailRow>
      <div className="flex items-center gap-2 pl-7 pr-2 text-2xs text-ink-muted">
        {workspaceName ? <span className="truncate">{workspaceName}</span> : null}
        <span className="ml-auto flex shrink-0 items-center gap-2">
          <button
            className="rounded px-1 text-2xs text-ink-soft transition-colors hover:bg-surface-subtle hover:text-ink"
            onClick={() => onRestore(thread)}
            type="button"
          >
            {t("activityRail.restore")}
          </button>
          <button
            className="rounded px-1 text-2xs text-ink-muted transition-colors hover:bg-danger-soft hover:text-danger"
            onClick={() => onDelete(thread)}
            type="button"
          >
            {t("activityRail.delete")}
          </button>
        </span>
      </div>
    </div>
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
export function ArchivedSectionHeading({ label, icon }: { label: string; icon?: React.ReactNode }) {
  return (
    <div className="sticky top-0 z-20 flex h-6 items-center gap-1.5 bg-surface px-2 text-xs font-medium text-ink-muted">
      {icon ?? <Archive className="size-3.5" />}
      <span>{label}</span>
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
          <MessageSquare className="size-3.5 shrink-0 text-ink-muted" />
          <span className="min-w-0 flex-1 truncate text-13 font-medium text-ink-soft">{thread.title}</span>
        </RailRow>
      ))}
    </div>
  );
}
