import type { StoredThread } from "../../integrations/storage/threadStore";
import type { ThreadRunInfo } from "./hooks/useThreadStore";
import { ChevronDown, ChevronRight, CircleAlert, MoreHorizontal } from "lucide-react";
import { memo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { useCachedAgentState } from "../../integrations/agent/agentStateCache";
import { cn } from "../../lib/cn";
import { useDismissableLayer } from "../../lib/useDismissableLayer";
import { ThreadItemMenu } from "./ActivityRailMenus";

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
    <div
      ref={menuRef}
      className={cn(
        // Full-width row; workspace threads (compact) indent their content via
        // padding so the highlight still spans the full width (req 2).
        "group/thread relative flex w-full items-center gap-1 rounded-md pr-2 text-left transition-colors hover:bg-surface-subtle",
        compact ? "h-7" : "h-8",
        active && "bg-surface-subtle text-ink",
      )}
      style={{ paddingLeft: (compact ? 28 : 8) + depth * 16 }}
      // Right-click anywhere on the row opens the same actions menu as the
      // `...` button.
      onContextMenu={(event) => {
        event.preventDefault();
        onMenuOpenChange(thread, true);
      }}
    >
      {/* Full-row click target so the whole (highlighted) row selects the
          thread. Content below sits on top but is pointer-events-none so clicks
          fall through to this button; the actions trigger and menu keep a higher
          stacking (z-10 / z-40) so they stay clickable and never mis-fire. */}
      <button
        aria-label={displayTitle}
        className="absolute inset-0 rounded-md"
        onClick={() => onSelectThread(thread)}
        title={displayTitle}
        type="button"
      />
      {hasChildren
        ? (
            <button
              aria-expanded={expanded}
              aria-label={t(expanded ? "activityRail.collapseThread" : "activityRail.expandThread", { title: displayTitle })}
              className="relative z-10 inline-flex size-4 shrink-0 items-center justify-center rounded text-ink-muted hover:text-ink-soft"
              onClick={() => onToggleExpanded?.(thread)}
              type="button"
            >
              {expanded ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
            </button>
          )
        : <span className="pointer-events-none size-4 shrink-0" />}
      <span
        className={cn(
          "pointer-events-none min-w-0 flex-1 truncate text-sm font-medium",
          archived ? "text-ink-muted" : "text-ink-soft",
        )}
      >
        {displayTitle}
      </span>
      {archived ? <span className="pointer-events-none shrink-0 text-[11px] text-ink-muted group-hover/thread:hidden">{t("activityRail.archived")}</span> : null}
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
              <span className="pointer-events-none flex shrink-0">
                <ThreadRunIndicator status={effectiveRunStatus} unread={unread} />
              </span>
              <button
                ref={menuButtonRef}
                aria-expanded={menuOpen}
                aria-haspopup="menu"
                aria-label={t("activityRail.threadActions", { title: displayTitle })}
                className={cn(
                  "relative z-10 hidden size-5 shrink-0 items-center justify-center rounded text-ink-muted transition-colors hover:bg-surface-subtle hover:text-ink-soft group-hover/thread:inline-flex",
                  menuOpen && "inline-flex",
                )}
                onClick={(event) => {
                  event.stopPropagation();
                  onMenuOpenChange(thread, !menuOpen);
                }}
                title={t("activityRail.threadActions", { title: displayTitle })}
                type="button"
              >
                <MoreHorizontal className="size-3.5" />
              </button>
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
    </div>
  );
}

export const ThreadListItem = memo(ThreadListItemImpl);

function ThreadRunIndicator({ status, unread }: { status?: ThreadRunInfo["status"]; unread?: boolean }) {
  const { t } = useTranslation("layout");
  // Reserved-width placeholder so idle rows (and the hover state) stay aligned.
  const placeholder = <span className="size-5 shrink-0 group-hover/thread:hidden" />;

  if (status === "queued" || status === "running" || status === "waiting_approval") {
    return (
      <span
        aria-label={t("activityRail.running")}
        className="inline-flex size-5 shrink-0 items-center justify-center group-hover/thread:hidden"
        title={t("activityRail.running")}
      >
        <span className="size-3 animate-spin rounded-full border-2 border-accent-soft border-t-accent" />
      </span>
    );
  }

  // A finished run is "unread" until the thread is opened: green when it
  // completed, red when it failed. Once read no dot shows. `cancelled` is a
  // deliberate user action (they aborted the run), so it never needs surfacing
  // as unread — it falls through to the empty placeholder below.
  if (unread && (status === "completed" || status === "failed")) {
    const failed = status === "failed";
    const label = failed ? t("activityRail.failed") : t("activityRail.completed");
    return (
      <span
        aria-label={label}
        className="inline-flex size-5 shrink-0 items-center justify-center group-hover/thread:hidden"
        title={label}
      >
        <span className={cn("size-2 rounded-full", failed ? "bg-danger" : "bg-success")} />
      </span>
    );
  }

  return placeholder;
}
