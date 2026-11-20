import type { StoredThread } from "../../integrations/storage/threadStore";
import type { ThreadRunInfo } from "./hooks/useThreadStore";
import { CircleAlert, ListChecks } from "lucide-react";
import { memo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useChoiceCardsEnabled } from "../../features/markdown/ChoiceCardsContext";
import { useCachedAgentState } from "../../integrations/agent/agentStateCache";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { ThreadItemMenu } from "./ActivityRailMenus";
import { RailRow, RailRowIndicator, RailRowMenuButton, RailRowSpacer, RailRowTitle, RailRowTreeToggle } from "./RailRow";

/**
 * A single sidebar thread row: title, run/unread indicator, and actions menu.
 * When the sidebar is in selection mode, a checkbox replaces the run indicator.
 *
 * Memoized: the rail re-renders whenever any run/streaming status changes,
 * but a row's props are all per-thread (stable callbacks receive the thread
 * as an argument), so unchanged rows must not re-render — each row also
 * subscribes to the agent-state cache via useCachedAgentState.
 */
function ThreadListItemImpl({
  active,
  ageLabel,
  archived,
  compact,
  depth = 0,
  hasChildren = false,
  expanded = false,
  onToggleExpanded,
  isStreaming,
  menuOpen,
  pendingApprovalCount,
  runStatus,
  selected,
  selectionMode,
  thread,
  unread,
  onDeleteThread,
  onMenuOpenChange,
  onRenameThread,
  onRestoreThread,
  onArchiveThread,
  onSelectThread,
  onTogglePinThread,
  onToggleSelection,
}: {
  active: boolean;
  /**
   * How old this conversation is, e.g. "3 minutes ago" — the recent view's
   * second column.
   *
   * Optional, and absent everywhere the rail draws its tree: that list is
   * ranked by workspace, where an age on every row is noise that costs the
   * title its width. It is here rather than in the caller so both views draw the
   * same row, and a title that truncates does so the same way in both.
   */
  ageLabel?: string;
  archived?: boolean;
  compact?: boolean;
  depth?: number;
  hasChildren?: boolean;
  expanded?: boolean;
  onToggleExpanded?: (thread: StoredThread) => void;
  /** Whether the agent reports this session is streaming (e.g. TUI-initiated). */
  isStreaming?: boolean;
  menuOpen: boolean;
  /**
   * Pending approvals in this thread; badged on the rail item when the
   * thread is NOT the open one (the open thread shows the card above the
   * composer instead).
   */
  pendingApprovalCount?: number;
  runStatus?: ThreadRunInfo;
  selected?: boolean;
  selectionMode?: boolean;
  thread: StoredThread;
  unread?: boolean;
  onDeleteThread: (thread: StoredThread) => void;
  /** Receives the row's thread so the rail can pass one stable callback to every row. */
  onMenuOpenChange: (thread: StoredThread, open: boolean) => void;
  onRenameThread: (thread: StoredThread) => void;
  onRestoreThread: (thread: StoredThread) => void;
  /** Absent in a list that offers no archive action for its rows. */
  onArchiveThread?: (thread: StoredThread) => void;
  onSelectThread: (thread: StoredThread) => void;
  onTogglePinThread: (thread: StoredThread) => void;
  onToggleSelection?: (thread: StoredThread) => void;
}) {
  const { t } = useTranslation("layout");
  const menuButtonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useDismissableLayer<HTMLDivElement>({
    enabled: menuOpen,
    onDismiss: () => onMenuOpenChange(thread, false),
    onEscapeDismiss: () => {
      onMenuOpenChange(thread, false);
      menuButtonRef.current?.focus();
    },
  });

  // Agent session_name is authoritative; DB title is fallback.
  // Use the reactive hook so the title updates automatically when the agent
  // state cache is populated by the background prefetch — no click required.
  const agentState = useCachedAgentState(thread.id);
  const displayTitle = agentState?.sessionName || thread.title;
  const choiceCards = useChoiceCardsEnabled();

  // Effective running status: use the local run-status first (most accurate
  // for GUI-initiated runs), but fall back to the agent-reported is_streaming
  // when another client (e.g. TUI) started a prompt on this session.
  const effectiveRunStatus: ThreadRunInfo["status"] | undefined
    = runStatus?.status === "running" || runStatus?.status === "queued"
      ? runStatus.status
      : isStreaming
        ? "running"
        : runStatus?.status;
  // A choice card is only worth flagging while the conversation is at rest: the
  // run that offered it finished, and nothing has been sent since. Read from the
  // run's own row (`offersChoice`), so a follow-up turn — which is a new row —
  // clears the mark without any second write to keep in step. With choice cards
  // turned off the mark goes too: a "waiting on your answer" badge over a reply
  // whose card is not drawn names a control the reader cannot see.
  const choiceWaiting = choiceCards && runStatus?.offersChoice === true
    && effectiveRunStatus === "completed";

  return (
    <RailRow
      active={active}
      compact={compact}
      depth={depth}
      rowRef={menuRef}
      title={displayTitle}
      onContextMenu={() => onMenuOpenChange(thread, true)}
      onSelect={() => onSelectThread(thread)}
    >
      {hasChildren
        ? (
            <RailRowTreeToggle
              expanded={expanded}
              label={t(expanded ? "activityRail.collapseThread" : "activityRail.expandThread", { title: displayTitle })}
              onToggle={() => onToggleExpanded?.(thread)}
            />
          )
        : null}
      <RailRowTitle grow={!ageLabel} muted={archived}>{displayTitle}</RailRowTitle>
      {ageLabel
        ? (
            <>
              <RailRowSpacer />
              {/* One step of ink while the row is the open one: `ink-muted` is
                  the floor-level tone (3:1 on `surface`), and the selection
                  surface is a tint of the accent, so the timestamp loses about
                  a third of a step on it — the second column of the one row the
                  reader is most likely to scan. `ink-soft` keeps it legible in
                  both renditions. */}
              <span className={cn("pointer-events-none shrink-0 text-2xs tabular-nums", active ? "text-ink-soft" : "text-ink-muted")}>{ageLabel}</span>
            </>
          )
        : null}
      {archived ? <span className="pointer-events-none shrink-0 text-2xs text-ink-muted group-hover/thread:hidden">{t("activityRail.archived")}</span> : null}
      {selectionMode
        ? (
            <input
              aria-label={t("activityRail.selectThread", { title: displayTitle })}
              checked={selected}
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
              {!active && pendingApprovalCount && pendingApprovalCount > 0
                ? (
                    <span
                      aria-label={t("activityRail.pendingApproval", { count: pendingApprovalCount })}
                      className="pointer-events-none inline-flex size-5 shrink-0 items-center justify-center text-warning"
                      title={t("activityRail.pendingApproval", { count: pendingApprovalCount })}
                    >
                      <CircleAlert className="size-3.5" />
                    </span>
                  )
                : null}
              {!active && choiceWaiting
                ? (
                    <span
                      aria-label={t("activityRail.pendingChoice")}
                      // The approval badge's ink, not the accent: both marks say
                      // the same thing to a reader scanning the list — this
                      // conversation cannot move without you — and a blue one read
                      // as a quieter, informational state next to it. The *shape*
                      // still tells the two apart (a card waits for the next send;
                      // an approval blocks the agent).
                      className="pointer-events-none inline-flex size-5 shrink-0 items-center justify-center text-warning"
                      data-thread-choice="true"
                      title={t("activityRail.pendingChoice")}
                    >
                      <ListChecks className="size-3.5" />
                    </span>
                  )
                : null}
              <RailRowIndicator status={effectiveRunStatus} unread={unread} />
              <RailRowMenuButton
                buttonRef={menuButtonRef}
                label={t("activityRail.threadActions", { title: displayTitle })}
                menuOpen={menuOpen}
                onToggle={() => onMenuOpenChange(thread, !menuOpen)}
              />
            </>
          )}
      {menuOpen
        ? (
            <ThreadItemMenu
              archived={archived}
              onArchive={onArchiveThread ? () => onArchiveThread(thread) : undefined}
              pinned={thread.pinned}
              onClose={() => onMenuOpenChange(thread, false)}
              onDelete={() => onDeleteThread(thread)}
              onRename={() => onRenameThread(thread)}
              onRestore={() => onRestoreThread(thread)}
              onTogglePin={() => onTogglePinThread(thread)}
            />
          )
        : null}
    </RailRow>
  );
}

export const ThreadListItem = memo(ThreadListItemImpl);
