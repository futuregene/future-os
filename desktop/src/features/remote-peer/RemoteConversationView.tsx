import type { AgentMessage } from "@future-os/thread-projection";
import type { RemotePeer } from "./remotePeerClient";
import type { RemoteApproval } from "./useRemoteApprovals";
import { ReceiptText } from "lucide-react";
import { useCallback, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { Button } from "../../components/ui/Button";
import { IconButton } from "../../components/ui/IconButton";
import { startWindowDrag } from "../../lib/windowDrag";
import { MessageBlock } from "../agent/MessageBlock";
import { peerBadgeText } from "./peerIcons";
import { useRemoteActivityTargets, withRemoteTargets } from "./remoteActivityTargets";

/**
 * A remote conversation: the session's transcript plus a composer that sends to
 * the host.
 *
 * The transcript is drawn by this app's own `MessageBlock`, over messages
 * projected by the shared projector (`entriesToMessages`). It used to render the
 * host's raw *entries*, one message each — and an entry is one event on that
 * host's journal, so a single reply arrived as a thinking entry, several tool
 * entries and a text entry, each with its own author line. On a real
 * conversation that is a column of "assistant", most of them empty.
 *
 * What the projector cannot supply is a tool call's target: the host's history
 * page is byte-budgeted and strips a shell call's arguments, so those are fetched
 * back and filled in before rendering (`useRemoteActivityTargets`).
 */
export function RemoteConversationView({
  approvals,
  approvalErrors,
  approvalPending,
  desktopId,
  composer,
  error,
  hasMore,
  leftPanelExpanded,
  loading,
  loadingOlder,
  messages,
  onContinueRun,
  onDecideApproval,
  onFork,
  onLoadOlder,
  onOpenUsage,
  onRetry,
  onToggleLeftPanel,
  peer,
  persistedEntryIds,
  streaming,
  title,
  sessionId,
}: {
  approvals: RemoteApproval[];
  approvalErrors: Record<string, string>;
  approvalPending: string | null;
  /** The host this conversation lives on, for reads that address it. */
  desktopId: string;
  composer: React.ReactNode;
  messages: AgentMessage[];
  error: string | null;
  hasMore: boolean;
  /** The panel toggle, as on this app's own conversation header. */
  leftPanelExpanded: boolean;
  loading: boolean;
  loadingOlder: boolean;
  /**
   * Resume a failed run, on the host that stopped it.
   *
   * Offered only for a failed run, because that is the host's own precondition:
   * it refuses to continue anything that did not fail.
   */
  onContinueRun: (runId: string) => void;
  onDecideApproval: (approval: RemoteApproval, decision: "allow" | "deny") => void;
  /**
   * Branch the conversation at the turn that produced an assistant reply, and
   * open the child. `forkable` is false when that turn is not persisted on the
   * host yet, which the caller must report rather than send.
   */
  onFork: (sourceEntryId: string, forkable: boolean) => void;
  onLoadOlder: () => void;
  /** Open this conversation's account book (token usage and amount). */
  onOpenUsage: () => void;
  onRetry: () => void;
  onToggleLeftPanel: () => void;
  peer: RemotePeer | undefined;
  /** The entry ids the host will accept as a stored record (see the timeline). */
  persistedEntryIds: ReadonlySet<string>;
  streaming: boolean;
  title: string;
  /** The conversation's session id on that host. */
  sessionId: string;
}) {
  const { t } = useTranslation("remotePeer");
  const [hoveredId, setHoveredId] = useState<string | null>(null);
  const targets = useRemoteActivityTargets(desktopId, sessionId, messages);
  const rendered = withRemoteTargets(messages, targets);

  /**
   * Auto-load older history at the top of the transcript.
   *
   * The local conversation has always done this; a button alone means a reader
   * who scrolls up hits the ceiling of what was loaded and has to notice a
   * control instead of getting more history. The listener tracks the previous
   * offset so only an upward move triggers it — arriving at the top by any other
   * route (a short transcript, a re-read that resets the offset) must not start
   * a chain of loads.
   */
  const lastScrollTopRef = useRef(0);
  const handleScroll = useCallback((event: React.UIEvent<HTMLDivElement>) => {
    const element = event.currentTarget;
    const top = element.scrollTop;
    const previous = lastScrollTopRef.current;
    lastScrollTopRef.current = top;
    // Only when the reader moved up to the top, and only when there is more.
    if (top <= 48 && previous > top && hasMore && !loadingOlder && !loading)
      onLoadOlder();
  }, [hasMore, loadingOlder, loading, onLoadOlder]);

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      {/* Same header row as this app's own conversation (`ThreadHeader`): the
          panel toggle and the title at the left, the conversation's own actions
          at the right, same height and padding — a remote conversation is a
          conversation, and its frame should not announce otherwise.
          `pr-14` reserves the collapsed right panel's corner, as there. */}
      <header
        className="flex h-12 shrink-0 select-none items-center gap-2 border-b border-line-soft/40 pl-4 pr-14"
        onMouseDown={startWindowDrag}
      >
        <div className="mr-3 flex min-w-0 flex-1 items-center" data-tauri-drag-region>
          <LeftPanelTitlebarToggle expanded={leftPanelExpanded} onToggle={onToggleLeftPanel} />
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold text-ink">{title}</div>
          </div>
        </div>
        {/* The conversation's own account book, in the same place and the same
            shape as this app's own header. An icon rather than the amount: the
            header is for the conversation, and a figure that changes every turn
            draws the eye away from it. */}
        <IconButton
          aria-haspopup="dialog"
          data-testid="remote-thread-usage"
          icon={<ReceiptText className="size-4" />}
          label={t("usageTitle")}
          onClick={onOpenUsage}
        />
        {streaming
          ? <span className="shrink-0 px-1 text-xs text-accent">{t("statusRunning")}</span>
          : null}
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4" onScroll={handleScroll}>
        {hasMore
          ? (
              <div className="mb-4 flex justify-center">
                <Button disabled={loadingOlder} onClick={onLoadOlder} size="sm" variant="ghost">
                  {loadingOlder ? t("loadingOlder") : t("loadOlder")}
                </Button>
              </div>
            )
          : null}

        {loading
          ? <p className="text-center text-xs text-ink-muted">{t("loadingHistory")}</p>
          : null}

        {error
          ? (
              <div className="mx-auto max-w-3xl rounded-md border border-danger-line bg-danger-soft px-3 py-2 text-sm text-danger">
                <div>{error}</div>
                <Button className="mt-2" onClick={onRetry} size="sm" variant="ghost">{t("retry")}</Button>
              </div>
            )
          : null}

        {!loading && !error && rendered.length === 0
          ? <p className="text-center text-xs text-ink-muted">{t("emptyConversation")}</p>
          : null}

        <div className="space-y-4">
          {rendered.map((message, index) => (
            <MessageBlock
              dataMessageId={message.id}
              hovered={hoveredId === message.id}
              isLast={index === rendered.length - 1}
              key={message.id}
              // The host refuses to continue anything that did not fail, so the
              // affordance appears exactly where it would be accepted — and it
              // is addressed by the host's own run id.
              onContinue={message.status === "failed" && message.runId
                ? () => onContinueRun(message.runId!)
                : undefined}
              onFork={() => forkAt(rendered, index, persistedEntryIds, onFork)}
              onHover={setHoveredId}
              onLeave={id => setHoveredId(current => (current === id ? null : current))}
              message={streaming && index === rendered.length - 1 && message.role === "assistant"
                // The host reports a run in flight; the reply's own status says
                // whether *that* reply is the one still arriving.
                ? { ...message, status: "streaming" }
                : message}
            />
          ))}
        </div>
      </div>

      {/* Approvals sit above the composer, not inside the transcript: they are a
          decision the user has to make now, and a card that scrolls away with
          history is a card that gets missed while the host waits. */}
      {approvals.length > 0
        ? (
            <div className="shrink-0 space-y-2 border-t border-line-soft bg-surface-subtle px-4 py-3">
              {approvals.map(approval => (
                <ApprovalCard
                  approval={approval}
                  error={approvalErrors[approval.id] ?? null}
                  key={approval.id}
                  onDecide={onDecideApproval}
                  pending={approvalPending === approval.id}
                  peerName={peer ? peerBadgeText(peer, peer.desktopId) : ""}
                />
              ))}
            </div>
          )
        : null}

      {composer}
    </section>
  );
}

/**
 * Branch at the turn a reply rests on.
 *
 * A reply is not itself a stored turn: the host forks at the *user* entry that
 * produced it, and only accepts an id from its own store. A turn that arrived as
 * a live push has an event id instead, which the host cannot resolve — that is
 * reported rather than sent, because sending it would fork at whatever happened
 * to share the name.
 */
function forkAt(
  messages: AgentMessage[],
  index: number,
  persistedEntryIds: ReadonlySet<string>,
  onFork: (sourceEntryId: string, forkable: boolean) => void,
): void {
  const message = messages[index];
  if (!message || message.role === "user")
    return;
  for (let i = index - 1; i >= 0; i -= 1) {
    const previous = messages[i]!;
    if (previous.role !== "user")
      continue;
    const sourceEntryId = previous.sourceEntryId;
    if (!sourceEntryId)
      return;
    onFork(sourceEntryId, persistedEntryIds.has(sourceEntryId));
    return;
  }
}

/**
 * A pending approval on the host.
 *
 * The card states where the tool will run, because the decision is easy to make
 * wrongly: "allow" on a card that does not name the machine reads as a local
 * action, and the whole point of this feature is that it is not.
 */
function ApprovalCard({
  approval,
  error,
  onDecide,
  pending,
  peerName,
}: {
  approval: RemoteApproval;
  error: string | null;
  onDecide: (approval: RemoteApproval, decision: "allow" | "deny") => void;
  pending: boolean;
  peerName: string;
}) {
  const { t } = useTranslation("remotePeer");
  return (
    <div className="mx-auto w-full max-w-3xl rounded-lg border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-ink">
            {approval.title || approval.toolName || t("approvalFallbackTitle")}
          </div>
          {approval.summary
            ? <div className="mt-0.5 wrap-break-word text-xs text-ink-muted">{approval.summary}</div>
            : null}
          <div className="mt-1 text-xs text-ink-muted">
            {t("runsOn", { name: peerName })}
            {approval.riskLevel ? ` · ${approval.riskLevel}` : ""}
          </div>
          {error ? <div className="mt-1 text-xs text-danger">{error}</div> : null}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button disabled={pending} onClick={() => onDecide(approval, "deny")} size="sm" variant="ghost">
            {t("deny")}
          </Button>
          <Button disabled={pending} onClick={() => onDecide(approval, "allow")} size="sm" variant="primary">
            {t("allow")}
          </Button>
        </div>
      </div>
    </div>
  );
}
