import type { AgentMessage } from "@future-os/thread-projection";
import type { RemotePeer } from "./remotePeerClient";
import type { RemoteApproval } from "./useRemoteApprovals";
import { ArrowDown, Check, History, ReceiptText, X } from "lucide-react";
import { useCallback, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { Button } from "../../components/ui/Button";
import { IconButton } from "../../components/ui/IconButton";
import { startWindowDrag } from "../../lib/windowDrag";
import { MessageBlock } from "../agent/MessageBlock";
import { useStickyAutoScroll } from "../agent/useStickyAutoScroll";
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
  live,
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
  /**
   * The reply the host is writing right now, drawn after the transcript.
   *
   * The same `MessageBlock` and the same shape a local run's live reply has, so a
   * reader watching another machine answer sees what they would see here: the
   * text as it arrives, inline tool rows that resolve in place, reasoning, and the
   * elapsed timer.
   */
  live: AgentMessage | null;
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
  const transcript = withRemoteTargets(messages, targets);
  /**
   * The reply being written, run through the same tool-target fetch as history.
   *
   * A live reply gets its targets from the pushed events, so this is a no-op for
   * it in practice — but it is the same list the viewport keys on, and leaving the
   * live reply out of it would make the transcript's own rendered form differ
   * between a reply in flight and the same reply a second later.
   */
  const rendered = useMemo(
    () => (live ? [...transcript, ...withRemoteTargets([live], targets)] : transcript),
    [transcript, targets, live],
  );

  /**
   * The transcript's viewport: this app's own sticky auto-scroll.
   *
   * `useStickyAutoScroll` is the hook this app's own conversations use, and
   * reusing it is what makes a remote conversation behave like a conversation:
   *
   * - it **opens on the newest message** and follows the reply as it streams;
   * - it stops following the moment the reader scrolls up, so reading history is
   *   not a fight with the view, and offers "jump to latest" once they are far;
   * - it **restores the reading anchor when older history is prepended**, which is
   *   the part a plain scroll container gets wrong: `scrollTop` is measured from
   *   the top of the content, so growing the content above the reader throws them
   *   backwards exactly as they asked for more.
   *
   * The phone is the same shape by a different mechanism: its timeline is an
   * inverted list whose offset 0 is the newest message, and it holds a
   * `following` flag with the same rules. Same behaviour, same page size, one
   * implementation each — the frame is what is shared here, not the plumbing.
   */
  const scrollRef = useRef<HTMLDivElement>(null);
  const lastScrollTopRef = useRef(0);
  /**
   * Reaching the top asks for more history.
   *
   * The container comes from the event rather than from the ref, because that is
   * the element the reader scrolled: nothing here has a "no container" case to
   * guard, and a guard that cannot fire is a guard nothing can trust.
   *
   * Run *before* the hook settles the viewport, so it compares the positions the
   * reader produced rather than the ones the hook's own correction writes.
   */
  const loadOlderAtTop = useCallback((container: HTMLElement) => {
    const top = container.scrollTop;
    const previous = lastScrollTopRef.current;
    lastScrollTopRef.current = top;
    // An *upward* move to the top, so arriving there some other way (a short
    // transcript, a re-read, the anchor being restored) cannot start a chain of
    // loads.
    if (top <= 48 && previous > top && hasMore && !loadingOlder && !loading)
      onLoadOlder();
  }, [hasMore, loadingOlder, loading, onLoadOlder]);

  const {
    handleScroll,
    scrollToLatest,
    showJumpToLatest,
  } = useStickyAutoScroll({
    // The projected transcript, not the raw entries: this is what the view
    // renders, and a fetch that fills in a tool target re-settles the viewport
    // the same way a pushed message does.
    contentKey: rendered,
    followEnabled: !loading,
    scrollRef,
  });

  return (
    <section className="flex h-full min-h-0 flex-col overflow-hidden bg-surface">
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
          // The local header's own size and spacing (`ThreadHeader`): the button
          // is one of the frame's fixed measurements, and a reader who moves
          // between the two conversations sees the same control in the same
          // place, at the same size.
          className="mr-2 size-8"
          data-testid="remote-thread-usage"
          icon={<ReceiptText className="size-4" />}
          label={t("usageTitle")}
          onClick={onOpenUsage}
        />
        {streaming
          ? <span className="shrink-0 px-1 text-xs text-accent">{t("statusRunning")}</span>
          : null}
      </header>

      {/* The transcript, and the band of controls over its foot.

The same two layers this app's own conversation uses (`AgentThread`):
          the transcript scrolls the full height of the frame and the composer
          rides *over* it behind a fade, rather than sitting under it behind a
          rule. That is the difference a reader notices first when the two
          conversations are put side by side — a hard edge under the last
          message reads as a second panel, not as one conversation. */}
      <div className="group relative min-h-0 flex-1 overflow-hidden">
        {/* Reaching the top is what asks for older history (see
            `loadOlderAtTop`); this is the same affordance the local transcript
            floats over its top edge for the reader who would rather click. */}
        {hasMore
          ? (
              <div className="pointer-events-none absolute inset-x-0 top-0 z-20 flex justify-center px-8 pt-5">
                <button
                  aria-busy={loadingOlder}
                  aria-label={t("loadOlder")}
                  className="pointer-events-auto flex animate-pop-in items-center gap-1.5 rounded-full border border-line-soft bg-surface px-3 py-1 text-xs text-ink-soft shadow-panel transition-colors hover:text-ink disabled:opacity-50"
                  disabled={loadingOlder}
                  onClick={onLoadOlder}
                  title={t("loadOlder")}
                  type="button"
                >
                  <History className="size-3.5" />
                  {loadingOlder ? t("loadingOlder") : t("loadOlder")}
                </button>
              </div>
            )
          : null}

        <div
          className="floating-scrollbar h-full overflow-auto overscroll-none px-8 pt-6"
          data-chat-scroll="true"
          style={{ overflowAnchor: "none" }}
          // One handler for both jobs, in the order that matters: this decides
          // whether the reader is asking for older pages, then the hook settles
          // where the viewport should be.
          onScroll={(event) => {
            loadOlderAtTop(event.currentTarget);
            handleScroll();
          }}
          ref={scrollRef}
        >
          {/* The reading column, at the same width the local transcript uses:
              `max-w-4xl` here, the message list and the composer inside it
              settling on `max-w-3xl`. */}
          <div className="mx-auto w-full max-w-4xl">
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

            {/* `space-y-5`, the spacing this app's own message list uses: the two
                transcripts put the same gap between turns. */}
            <div className="space-y-5">
              {rendered.map((message, index) => {
                /**
                 * Whether this turn is a settled point.
                 *
                 * The last row is the one still being written — the reply in flight
                 * (a turn of its own) or, before its first frame, the reply the host
                 * is about to replace. Nothing it offers can be acted on: a fork
                 * branches at a settled turn, and the host answers one prompt at a
                 * time, so a Retry would be refused. A settled earlier turn keeps
                 * both, which is why this is per row and not a blanket `!streaming`.
                 */
                const settled = index !== rendered.length - 1 || !streaming;
                return (
                  <MessageBlock
                    dataMessageId={message.id}
                    hovered={hoveredId === message.id}
                    isLast={index === rendered.length - 1}
                    key={message.id}
                    // The host refuses to continue anything that did not fail, so the
                    // affordance appears exactly where it would be accepted — and it
                    // is addressed by the host's own run id.
                    onContinue={settled && message.status === "failed" && message.runId
                      ? () => onContinueRun(message.runId!)
                      : undefined}
                    onFork={settled ? () => forkAt(rendered, index, persistedEntryIds, onFork) : undefined}
                    onHover={setHoveredId}
                    onLeave={id => setHoveredId(current => (current === id ? null : current))}
                    // No status is stamped on: a reply's own status is what it is. The
                    // view used to mark the *last settled* reply "streaming" while the
                    // host reported a run in flight, which put a cursor on the previous
                    // answer and left the new one invisible.
                    message={message}
                  />
                );
              })}
            </div>
          </div>
        </div>

        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10 bg-linear-to-t from-surface from-80% to-transparent px-8 pb-5 pt-10">
          <div className="mx-auto flex w-full max-w-4xl flex-col gap-3">
            {/* Approvals sit above the composer, not inside the transcript: they
                are a decision the user has to make now, and a card that scrolls
                away with history is a card that gets missed while the host waits.
                The same slot the local conversation gives them. */}
            {approvals.map(approval => (
              <div className="pointer-events-auto mx-auto w-full max-w-3xl" key={approval.id}>
                <ApprovalCard
                  approval={approval}
                  error={approvalErrors[approval.id] ?? null}
                  onDecide={onDecideApproval}
                  pending={approvalPending === approval.id}
                  peerName={peer ? peerBadgeText(peer, peer.desktopId) : ""}
                />
              </div>
            ))}

            {/* Reading history is not a fight with the view: once the reader has
                scrolled away, following stops and this is how they come back. */}
            {showJumpToLatest
              ? (
                  <button
                    aria-label={t("jumpToLatest")}
                    className="pointer-events-auto mx-auto flex items-center gap-1 rounded-full border border-line-soft bg-surface px-3 py-1 text-xs text-ink-soft shadow-panel transition-colors hover:text-ink"
                    onClick={scrollToLatest}
                    title={t("jumpToLatest")}
                    type="button"
                  >
                    <ArrowDown className="size-3.5" />
                    {t("jumpToLatest")}
                  </button>
                )
              : null}

            {composer}
          </div>
        </div>
      </div>
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
    // The local approval card's own frame (`ApprovalPrompt`): same border, same
    // radius, same padding and shadow, and the same button row — deny at the
    // left with its icon, allow at the right as the primary action, both at the
    // size the local card uses. The two cards answer the same question on the
    // same screen, so a reader should not be able to tell which machine the
    // tool runs on from the card's shape alone.
    <section className="rounded-lg border border-line bg-surface/95 p-4 shadow-panel backdrop-blur">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <span className="size-2 shrink-0 rounded-full bg-warning" />
          <h2 className="truncate text-base font-semibold text-ink">
            {approval.title || approval.toolName || t("approvalFallbackTitle")}
          </h2>
        </div>
      </div>
      {approval.summary
        ? (
            <pre className="mt-3 max-h-[33vh] overflow-auto whitespace-pre-wrap wrap-break-word rounded-md bg-surface-subtle p-3 text-xs leading-5 text-ink-soft">
              <code className="block min-w-0">{approval.summary}</code>
            </pre>
          )
        : null}
      {/* The one thing a remote approval has to say that a local one does not: the
          machine the tool will run on. "Allow" on a card that did not name it
          reads as a local action, and the whole point of this feature is that it
          is not. */}
      <div className="mt-3 text-xs leading-5 text-ink-muted">
        {t("runsOn", { name: peerName })}
        {approval.riskLevel ? ` · ${approval.riskLevel}` : ""}
      </div>
      {error ? <div className="mt-3 text-xs leading-5 text-danger">{error}</div> : null}
      <div className="mt-4 flex items-center justify-between gap-3">
        <Button
          className="shadow-xs"
          disabled={pending}
          leftIcon={<X className="size-3.5" />}
          onClick={() => onDecide(approval, "deny")}
          variant="toolbar"
        >
          {t("deny")}
        </Button>
        <div className="flex items-center gap-2">
          <Button
            className="shadow-xs"
            disabled={pending}
            leftIcon={<Check className="size-3.5" />}
            onClick={() => onDecide(approval, "allow")}
            variant="primary"
          >
            {t("allow")}
          </Button>
        </div>
      </div>
    </section>
  );
}
