import type { StoredThread } from "../../integrations/storage/threadStore";
import type { ThreadRunInfo } from "./hooks/useThreadStore";
import { CircleAlert } from "lucide-react";
import { memo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useCachedAgentState } from "../../integrations/agent/agentStateCache";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { ThreadItemMenu } from "./ActivityRailMenus";
import { RailRow, RailRowIndicator, RailRowMenuButton, RailRowTitle, RailRowTreeToggle } from "./RailRow";

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
  onSelectThread,
  onTogglePinThread,
  onToggleSelection,
}: {
  active: boolean;
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

  // Effective running status: use the local run-status first (most accurate
  // for GUI-initiated runs), but fall back to the agent-reported is_streaming
  // when another client (e.g. TUI) started a prompt on this session.
  const effectiveRunStatus: ThreadRunInfo["status"] | undefined
    = runStatus?.status === "running" || runStatus?.status === "queued"
      ? runStatus.status
      : isStreaming
        ? "running"
        : runStatus?.status;

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
      <RailRowTitle muted={archived}>{displayTitle}</RailRowTitle>
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
