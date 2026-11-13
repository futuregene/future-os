import type {
  AgentMessage,
  MessageAttachment,
} from "@future-os/thread-projection";
import type { ReactNode } from "react";
import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type {
  StoredApprovalRequest,
  StoredThread,
} from "../../integrations/storage/threadStore";
import type { ComposerSendPayload } from "./Composer";
import type { ApprovalChipTier } from "./composer/ComposerControls";
import type { SendQueueAttachmentItem } from "./SendQueuePanel";
import type { SessionMentionOption } from "./sessionMention";
import { ArrowDown, History } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef } from "react";
import { useTranslation } from "react-i18next";
import { FloatingScrollbar } from "../../components/ui/FloatingScrollbar";
import { useCachedAgentState } from "../../integrations/agent/agentStateCache";
import { forkThread, openPath } from "../../integrations/storage/threadStore";
import { copyText } from "../../lib/clipboard";
import { cn } from "../../lib/cn";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent, onFutureEvent } from "../../lib/futureEvents";
import { foldsSettledReply, useReplyDetail } from "../../lib/replyDisplay";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { useCommand } from "../keyboard/commands";
import { installRecommendedSkill } from "../skills/installRecommendedSkill";
import { agentQueueNoticeReason, cancelAgentQueuedRun, editAgentQueuedRun, isParkedRow, releaseAgentQueuedRun, useAgentQueue } from "./agentQueue";
import { ApprovalPrompt } from "./ApprovalPrompt";
import {
  buildContinuePrompt,
  loadRunResumeSummary,
  previousUserForRun,
} from "./buildContinuePrompt";
import { Composer } from "./Composer";
import { clearComposerDraftIfMatches } from "./composerDraft";
import { MessageList } from "./MessageList";
import { QuestionNavControl } from "./QuestionNavControl";
import { replyCopyText } from "./replyBlocks";
import { useThreadCompaction } from "./runtime/useThreadCompaction";
import { SendQueuePanel } from "./SendQueuePanel";
import { ThreadHeader } from "./ThreadHeader";
import { ThreadSearch } from "./ThreadSearch";
import { useAgentThreadState } from "./useAgentThreadState";
import { useComposerInset } from "./useComposerInset";
import { useMessagePaging } from "./useMessagePaging";
import { useQuestionNav } from "./useQuestionNav";
import { useSkillRecommendation } from "./useSkillRecommendation";

/** How many user exchanges one loaded page renders. */
const PAGE_USER_EXCHANGES = 10;

/**
 * How much of the conversation's own chrome this view draws.
 *
 * - `full` — the main window: title bar, find-in-conversation, the composer at
 *   the foot, plus the terminal's drag handle. Everything below assumes this.
 * - `read` — one cell of the Vibe panel, and the zoomed cell: the transcript and
 *   whatever needs answering, nothing else. **No composer**, which is what makes
 *   "the app has at most one editable element while the panel is open" a property
 *   of the component tree rather than a rule someone has to remember.
 * - `input` — the panel's input layer: the same transcript trimmed to its tail,
 *   the composer where a dictation belongs, and the caller's own target bar and
 *   hint line around it.
 */
export type AgentThreadChrome = "full" | "read" | "input";

export interface AgentThreadProps {
  thread: StoredThread | null;
  workspacePath?: string | null;
  /** Conversations offered by the composer's `#` menu (see `sessionMentionOptions`). */
  sessionMentions?: SessionMentionOption[];
  agentConnection: AgentConnectionState;
  leftPanelExpanded: boolean;
  loadingStore: boolean;
  modelId: string;
  modelOptions: AgentModelOption[];
  onModelChange: (modelId: string) => void;
  thinkingLevel: string;
  onThinkingLevelChange: (thinkingLevel: string) => void;
  approvalTier: ApprovalChipTier;
  onChangeApprovalTier: (value: ApprovalTier) => void;
  pendingPrompt: {
    attachments?: MessageAttachment[];
    id: string;
    content: string;
    targetThreadId: string;
  } | null;
  activeApproval?: StoredApprovalRequest | null;
  onApprovalDecision: (
    approval: StoredApprovalRequest,
    status: "approved" | "rejected",
  ) => Promise<void>;
  onPromptConsumed: (id: string) => void;
  onRetryAgentConnection: () => void;
  onOpenAccount: () => void;
  onOpenModels: () => void;
  onOpenProviders: () => void;
  onForked: (threadId: string) => void;
  onThreadActivity: () => void;
  onToggleLeftPanel: () => void;
  /** Skill-recommendation toggle (appSettings.skillRecommend). */
  skillRecommend: boolean;
  /** Future session status; recommendation requires an authenticated session. */
  futureSessionStatus: string;
  /** Future balance in credits; recommendation requires a positive balance. */
  futureBalance: number | null;
  /**
   * Optional header affordance supplied by the shell (e.g. the terminal
   * toggle). Kept as a node so this feature does not depend on the terminal.
   */
  headerAction?: ReactNode;
  /**
   * Which machine this conversation is on, when there is more than one to be on.
   *
   * The shell's answer, like `headerAction`: this view draws a conversation and
   * knows nothing about pairing, and a local conversation is on this computer by
   * definition — which is worth saying only once another computer is in the list.
   */
  headerMachine?: ReactNode;
  /**
   * Which workspace this conversation works in, when it is in one.
   *
   * The shell's answer again: the workspace a conversation belongs to is one of
   * the store's rows, which this view is handed conversations from and does not
   * read itself.
   */
  headerWorkspace?: ReactNode;
  /**
   * How much of the conversation's chrome to draw. See `AgentThreadChrome`.
   */
  chrome?: AgentThreadChrome;
  /**
   * Restore a shelved conversation from the read-only banner.
   *
   * An archived conversation opens read-only: its history pages and reads like
   * any other, but nothing can be sent into it until it is restored. The shell
   * owns the archive state, so it owns the command; this view only knows it is
   * showing a shelved row (`thread.status === "archived"`).
   */
  onRestoreArchived?: (thread: StoredThread) => void;
  /**
   * Whether this instance may claim the app-wide commands a conversation owns
   * (`chat.stop`).
   *
   * Off for every cell of the panel's grid except the focused one: the registry
   * keeps one handler per command, so several mounted cells would mean "the last
   * one to mount wins" — and stopping a run would abort whichever conversation
   * happened to render last.
   */
  ownsCommands?: boolean;
  /**
   * The input layer only: called when the layer's own key is pressed on an empty
   * draft. The panel closes the layer; this view does not know what a layer is.
   */
  onLeaveInput?: () => void;
  /** The input layer only: the panel's target bar, above the transcript. */
  inputHeader?: ReactNode;
  /** The input layer only: the panel's hint line, below the composer. */
  inputFooter?: ReactNode;
  /**
   * How much of the composer's control row to draw (see `Composer.controls`).
   *
   * Only the panel passes `none`, for its input layer: the values move into the menu
   * and a read-only line stays on screen. Undefined means "whatever the composer
   * does by default", which is what keeps the main window untouched.
   */
  composerControls?: "full" | "none";
  /**
   * What a send means while this conversation's run is still in flight.
   *
   * Only the panel passes it: the draft is handed over and queued instead of being
   * dropped, and `false` means "one is already queued" — the text then stays in the
   * editor (`Composer.onQueue`).
   */
  onQueue?: (payload: ComposerSendPayload) => boolean;
}

export function AgentThread({
  thread,
  workspacePath,
  sessionMentions,
  agentConnection,
  leftPanelExpanded,
  loadingStore,
  modelId,
  modelOptions,
  onModelChange,
  thinkingLevel,
  onThinkingLevelChange,
  approvalTier,
  onChangeApprovalTier,
  pendingPrompt,
  activeApproval,
  onApprovalDecision,
  onPromptConsumed,
  onRetryAgentConnection,
  onOpenAccount,
  onOpenModels,
  onOpenProviders,
  onForked,
  onThreadActivity,
  onToggleLeftPanel,
  skillRecommend,
  futureSessionStatus,
  futureBalance,
  headerAction,
  headerMachine,
  headerWorkspace,
  chrome = "full",
  ownsCommands = true,
  onLeaveInput,
  inputHeader,
  inputFooter,
  composerControls,
  onQueue,
  onRestoreArchived,
}: AgentThreadProps) {
  const { t } = useTranslation("agent");
  const agentState = useCachedAgentState(thread?.id);
  const {
    handleAbort,
    handleQueuedSend,
    handleSend,
    loadingThread,
    loadingIndicator,
    hasOlderHistory,
    loadOlderHistory,
    loadAllHistoryForSearch,
    historyError,
    historyStatus,
    sessionChanged,
    retryHistory,
    messages,
    renderWorkspace,
  } = useAgentThreadState({
    thread,
    workspacePath,
    loadingStore,
    modelId,
    thinkingLevel,
    pendingPrompt,
    onPromptConsumed,
    onThreadActivity,
  });

  // Mirror the message list so stable callbacks (handleFork/handleRetryRun)
  // can read the latest messages without depending on the array itself — the
  // array changes identity on every streaming push, and listing it as a dep
  // recreated the callbacks each push, defeating MessageBlock's memo for the
  // whole visible window (and re-subscribing the recover-run effect).
  const messagesRef = useCommittedRef(messages);
  // One idempotency key belongs to one user intent, not permanently to a fork
  // point. Keep it across failed retries, then release it after success so the
  // user may deliberately create another branch from the same message later.
  const pendingForkRequestsRef = useRef(new Map<string, string>());
  const searchRootRef = useRef<HTMLDivElement>(null);
  const { composerRef, composerHeight } = useComposerInset();
  const handleCompactContext = useThreadCompaction(thread);

  const {
    scrollRef,
    scrollbar,
    updateFloatingScrollbar,
    handleScroll: handleScrollbarVisibility,
    handleThumbPointerDown,
  } = useFloatingScrollbar();

  // Sticky auto-scroll: follow streaming output only while pinned near the
  // bottom; follows the growing message list while pinned. The view is keyed
  // by thread id, so each conversation starts on a fresh instance pinned to
  // the latest message.
  const {
    visibleMessages,
    preserveViewport,
    scrollToLatest,
    showJumpToLatest,
    showLoadOlderHint,
    coolingDown,
    handleScroll: handlePagingScroll,
    loadOlder,
    prepareSearch,
    revealSearchMatch,
  } = useMessagePaging({
    messages,
    scrollRef,
    userExchangeCount: PAGE_USER_EXCHANGES,
    hasOlderHistory,
    loadOlderHistory,
    loadAllHistoryForSearch,
    onScroll: handleScrollbarVisibility,
    followEnabled: !loadingIndicator,
    onContentSettled: () => updateFloatingScrollbar(false),
  });

  // Question navigation: jump between user questions in the transcript.
  // The take-over callback is read from a ref so it stays stable without
  // triggering a render-time ref update lint warning.
  const takeOverQuestionNavRef = useRef(() => {});
  useEffect(() => {
    takeOverQuestionNavRef.current = preserveViewport;
  }, [preserveViewport]);
  const {
    hasPrevious: hasPreviousQuestion,
    hasNext: hasNextQuestion,
    goToPrevious: goToPreviousQuestion,
    goToNext: goToNextQuestion,
  } = useQuestionNav({
    messages: visibleMessages,
    scrollRef,
    onTakeOver: () => takeOverQuestionNavRef.current(),
  });

  // A run is in flight while its assistant bubble is still streaming; the agent
  // rejects a concurrent prompt, so the composer is disabled until it settles.
  // The streaming bubble always belongs to the trailing turn (after the last
  // user message), so scan backwards only until the first user message instead
  // of the whole list — this runs on every streaming push.
  let isReplyStreaming = false;
  for (let i = messages.length - 1; i >= 0; i--) {
    const message = messages[i]!;
    if (message.role === "user")
      break;
    if (message.role === "assistant" && message.status === "streaming") {
      isReplyStreaming = true;
      break;
    }
  }
  // The reading setting the reply rows themselves read, so "copy the last
  // answer" copies what the answer is currently showing (see `replyCopyText`).
  const replyDetail = useReplyDetail();

  // One answer to "can a message leave this conversation right now": no active
  // thread, no history still loading, and a history the Agent can still continue.
  // The composer's own `disabled` and the send queue's flush both read it, so a
  // queue can never fire into a conversation that refuses to be written to.
  const archived = thread?.status === "archived";
  const composerDisabled = archived || !thread || loadingThread || loadingStore || historyStatus?.canContinue === false;

  // ── Send while the reply streams ─────────────────────────────────────────
  // A message written while a reply is streaming leaves immediately and the
  // AGENT holds it (`handleQueuedSend`, `enqueue_coalescing`): a burst folds into
  // one run at the turn boundary, and the row lives in the Agent's own queue, so
  // it survives this window and is visible to every other client on the
  // conversation. Nothing is held here any more — the old in-memory queue could
  // only ever be flushed by the window that filled it.
  // The rows are read back from the Agent (`agentQueue`), which is also where
  // "take it back" and "start it now" go; the panel is a view of that queue, not
  // a second copy of it.
  const agentQueue = useAgentQueue(thread?.id ?? null, isReplyStreaming);
  // A row that could not be taken back or started is a failed action, not
  // nothing: the Agent's sentence is what says why (a run it no longer holds, a
  // session this machine cannot reach), and the row stays where it was.
  const reportQueueFailure = useCallback((error: unknown) => {
    const message = error instanceof Error ? error.message : String(error ?? "");
    emitFutureEvent("toast", {
      message: t("composer.queue.actionFailed", { message }),
      tone: "error",
    });
  }, [t]);
  const handleEnqueueDraft = useCallback((payload: ComposerSendPayload) => {
    if (!thread?.id)
      return false;
    void handleQueuedSend(payload).finally(() => agentQueue.refresh());
    return true;
  }, [agentQueue, handleQueuedSend, thread?.id]);
  const handleQueueRemove = useCallback((runId: string) => {
    if (!thread?.id)
      return;
    void cancelAgentQueuedRun(thread.id, runId)
      // Only this window's own removal is uninteresting news; a failed one is
      // still something that may leave the queue later, by another client.
      .then(() => agentQueue.forget(runId))
      .catch(reportQueueFailure)
      .finally(() => agentQueue.refresh());
  }, [agentQueue, reportQueueFailure, thread?.id]);
  const handleQueueRelease = useCallback((runId: string) => {
    if (!thread?.id)
      return;
    void releaseAgentQueuedRun(thread.id, runId)
      .catch(reportQueueFailure)
      .finally(() => agentQueue.refresh());
  }, [agentQueue, reportQueueFailure, thread?.id]);
  /**
   * Look at one file a waiting message carries, before the run takes it.
   *
   * The path belongs to this machine (the Agent runs here), so this is the same
   * gesture the transcript's chips offer — open it the way the OS would — and not
   * a fetch from another host.
   */
  const handleQueuePreviewAttachment = useCallback((attachment: SendQueueAttachmentItem) => {
    void openPath(attachment.path).catch(() => {
      emitFutureEvent("toast", {
        message: t("attachment.fileMissing", { name: attachment.name }),
        tone: "error",
      });
    });
  }, [t]);
  // Editing a waiting message corrects its words where it already waits. It is
  // deliberately NOT a removal and a re-send: a re-send enters the queue at the
  // back, so fixing a typo in the second of four messages put it after the fourth
  // — the order the reader built, arriving as an order they never chose. The Agent
  // has an in-place edit (`edit_queued_run`), so the row keeps its run id, its
  // position, and the files, images and frozen settings it was accepted with;
  // saving only changes the text. An empty edit is a deletion, and the panel's own
  // bin says that plainly, so it is refused where the reader is looking.
  const handleQueueUpdate = useCallback((runId: string, content: string) => {
    if (!thread?.id)
      return;
    if (!content.trim()) {
      emitFutureEvent("toast", { message: t("composer.queue.emptyEdit"), tone: "error" });
      return;
    }
    void editAgentQueuedRun(thread.id, runId, content)
      .catch(reportQueueFailure)
      .finally(() => agentQueue.refresh());
  }, [agentQueue, reportQueueFailure, t, thread?.id]);

  // Settings → Keyboard: the same interrupt the send button turns into while a
  // reply streams. Registered only then, so the key is unclaimed whenever there
  // is nothing to stop — and a press outside a stream stays the app's own.
  useCommand("chat.stop", () => void handleAbort(), isReplyStreaming && ownsCommands);

  /**
   * The last reply's prose, for `thread.copyReply` — or null when this turn has
   * no answer to copy (nothing yet, or the last thing the agent said was work:
   * a tool call, reasoning, a compaction divider).
   *
   * `replyCopyText` is the same rule the reply's own copy button uses, so the
   * key and the button can never copy two different things.
   */
  const lastReply = useMemo(() => {
    const fold = foldsSettledReply(replyDetail);
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const message = messages[index]!;
      if (message.role !== "assistant")
        continue;
      const text = replyCopyText(message, { fold }).trim();
      if (text.length > 0)
        return text;
      // One empty reply (an interrupted run) must not hide an earlier answer, so
      // keep walking back rather than giving up on the newest one.
    }
    return null;
  }, [messages, replyDetail]);
  useCommand("thread.copyReply", () => {
    if (!lastReply)
      return;
    copyText(lastReply).then(
      () => emitFutureEvent("toast", { message: t("thread.copiedReply") }),
      () => emitFutureEvent("toast", { message: t("common:copyFailed"), tone: "error" }),
    );
  }, ownsCommands && lastReply !== null);
  // The corner's "jump to latest" button from the keyboard. Registered whether
  // or not the button is showing: the button hides while the view is already at
  // the live end, and a key that stopped answering there would read as broken.
  useCommand("thread.latest", () => scrollToLatest(), ownsCommands);

  const handleRetryMessage = useCallback(
    (_message: AgentMessage, source: AgentMessage) => {
      void handleSend(
        {
          attachments: source.attachments ?? [],
          content: source.content,
        },
        // A rejected first send is restored to this conversation's composer
        // draft (see useAgentThreadState). Retrying from the failed bubble
        // delivers that message again, so drop the now-sent copy from the
        // draft at acceptance — otherwise the next visit finds it prefilled.
        () => clearComposerDraftIfMatches(thread?.id, source.content, source.attachments ?? []),
      ).catch(() => {
        // The failure is already surfaced as a toast and a failed bubble;
        // passing `onAccepted` makes handleSend rethrow it here.
      });
    },
    [handleSend, thread?.id],
  );

  const handleContinueMessage = useCallback(
    (message: AgentMessage) => {
      void handleSend({
        attachments: [],
        content: buildContinuePrompt({ message }),
      });
    },
    [handleSend],
  );

  const handleContinueRun = useCallback(
    async (runId: string) => {
      const summary = await loadRunResumeSummary(runId);
      void handleSend({
        attachments: [],
        content: buildContinuePrompt({ summary }),
      });
    },
    [handleSend],
  );

  // Reads messages through messagesRef so this callback stays stable across
  // streaming pushes — it's a dep of the recover-run subscription below, and
  // depending on `messages` re-subscribed the listener on every push (M6).
  const handleRetryRun = useCallback(
    (runId: string, triggerMessageId?: string | null) => {
      const current = messagesRef.current;
      const source = triggerMessageId
        ? current.find(
            message =>
              message.id === triggerMessageId && message.role === "user",
          )
        : previousUserForRun(current, runId);
      if (!source)
        return;

      void handleSend({
        attachments: source.attachments ?? [],
        content: source.content,
      });
    },
    [handleSend, messagesRef],
  );

  useEffect(
    () =>
      onFutureEvent("recover-run", (detail) => {
        if (detail.action === "retry") {
          handleRetryRun(detail.runId, detail.triggerMessageId);
          return;
        }
        void handleContinueRun(detail.runId);
      }),
    [handleContinueRun, handleRetryRun],
  );

  // Reads messages through messagesRef (not a dep): `messages` changes
  // identity on every streaming push, and this callback is passed to every
  // MessageBlock — recreating it each push defeated the list's only memo
  // boundary and re-rendered every visible finalized row (H1).
  const handleFork = useCallback(
    async (aiMessage: AgentMessage) => {
      const current = messagesRef.current;
      if (!thread || !current.length)
        return;
      // Find the user message that triggered this AI response.
      const aiIndex = current.indexOf(aiMessage);
      let userMessage: AgentMessage | undefined;
      for (let i = aiIndex - 1; i >= 0; i--) {
        if (current[i]!.role === "user") {
          userMessage = current[i]!;
          break;
        }
      }
      if (!userMessage)
        return;
      try {
        const sourceEntryId = userMessage.sourceEntryId?.trim();
        if (!sourceEntryId)
          throw new Error("The selected message is not yet persisted.");
        const intentKey = `${thread.id}:${sourceEntryId}`;
        let requestId = pendingForkRequestsRef.current.get(intentKey);
        if (!requestId) {
          requestId = `desktop-fork:${crypto.randomUUID()}`;
          pendingForkRequestsRef.current.set(intentKey, requestId);
        }
        const newThreadId = await forkThread(
          thread.id,
          sourceEntryId,
          requestId,
        );
        pendingForkRequestsRef.current.delete(intentKey);
        onForked(newThreadId);
      }
      catch (error) {
        emitFutureEvent("toast", {
          message: t("message.forkFailed", { message: errorMessage(error) }),
          tone: "error",
        });
      }
    },
    [messagesRef, thread, onForked, t],
  );

  // Stable wrappers for the memoized Composer: inline arrows here would be
  // fresh on every render (and AgentThread renders on every streaming push),
  // which would defeat the memo outright.
  const handleComposerAbort = useCallback(() => {
    void handleAbort();
  }, [handleAbort]);
  const handleComposerSend = useCallback(
    (payload: ComposerSendPayload) => new Promise<void>((resolve, reject) => {
      // The input layer exists to ask one question. Once the draft is handed over
      // (the composer has already cleared it by now), the panel drops back to
      // reading so the answer is not hidden behind the editor.
      if (chrome === "input")
        onLeaveInput?.();
      // Composer waits for delivery; handleSend continues to own the full run.
      void handleSend(payload, resolve).then(resolve, reject);
    }),
    [chrome, handleSend, onLeaveInput],
  );
  // Skill recommendation on any user message (not just a conversation's first
  // turn): the client owns the trigger rules, the agent only answers
  // suggest_skill. One hook instance per open conversation.
  const recommendation = useSkillRecommendation({
    enabled: skillRecommend,
    sessionStatus: futureSessionStatus,
    balance: futureBalance,
  });
  const installRecommended = useCallback(
    (card: { name: string; description: string }) => installRecommendedSkill(card.name),
    [],
  );
  const skillRecommendation = useMemo(() => ({
    card: recommendation.state.recommendation,
    onEvaluate: recommendation.evaluate,
    onInstall: installRecommended,
    onDismiss: recommendation.dismiss,
  }), [recommendation.state.recommendation, recommendation.evaluate, recommendation.dismiss, installRecommended]);

  // Everything below is one transcript in three surroundings. What each chrome
  // keeps or drops is decided here rather than by branching the JSX three ways:
  // `full` is the main window, `read` is a panel cell (no composer at all, which
  // is what keeps the app down to one editable element while the panel is open),
  // `input` is the panel's dictation layer.
  const isRead = chrome === "read";
  const isInput = chrome === "input";
  // Whether this composer keeps a send gate at all. The panel's input layer keeps
  // its own hand-back queue instead (`onQueue`, VIBE.md §5.4), and a reading cell
  // has no composer: everything else sends into the Agent's queue while its run
  // streams. Kept for the composer's own `onEnqueue` wiring below.
  const queueEnabled = !isInput && !isRead && !onQueue;
  // The input layer shows only the tail of the conversation: the two messages a
  // speaker needs to remember what they were talking about, dimmed so the draft
  // is unmistakably the live thing on screen.
  const shownMessages = isInput ? visibleMessages.slice(-2) : visibleMessages;

  const approvalCard = activeApproval
    ? (
        <div className="pointer-events-auto mx-auto w-full max-w-3xl">
          <ApprovalPrompt
            approval={activeApproval}
            onDecision={onApprovalDecision}
            threadMode={thread?.mode}
          />
        </div>
      )
    : null;

  /**
   * The foot of the conversation: what needs answering, then the composer.
   *
   * `full` floats it over the transcript (the gradient is what tells the reader
   * the text continues underneath); `input` places it in the flow so the space
   * below can stay empty for the input method's candidate window.
   */
  const composerBand = (
    <div
      ref={composerRef}
      className={cn(
        "pointer-events-none",
        isInput
          ? "shrink-0 px-8 pb-2 pt-1"
          : "absolute inset-x-0 bottom-0 z-10 bg-linear-to-t from-surface from-80% to-transparent px-8 pb-5 pt-10",
      )}
    >
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-3">
        {/* Never in the input layer: an approval is answered with a two-key
            confirmation, and a card appearing above the editor mid-dictation moves
            the one thing the eye is on. The reading layer holds it, and the target
            bar says one is waiting. */}
        {isInput ? null : approvalCard}
        {archived && !isInput && thread
          ? (
              <div
                className="pointer-events-auto mx-auto flex w-full max-w-3xl items-center justify-between gap-3 rounded-lg border border-line-soft bg-surface px-3.5 py-2.5 text-sm text-ink-soft shadow-panel"
                role="status"
              >
                <span className="min-w-0 flex-1 truncate">
                  {t("thread.archivedReadOnly")}
                </span>
                {onRestoreArchived
                  ? (
                      <button
                        className="inline-flex h-7 shrink-0 items-center rounded-md bg-accent-fill px-3 text-sm font-medium text-white transition-colors hover:bg-accent-fill-hover"
                        onClick={() => onRestoreArchived(thread)}
                        type="button"
                      >
                        {t("thread.restoreArchived")}
                      </button>
                    )
                  : null}
              </div>
            )
          : null}
        {shouldShowAgentNotice(agentConnection)
          ? (
              <AgentConnectionNotice
                connection={agentConnection}
                onOpenModels={onOpenModels}
                onOpenAccount={onOpenAccount}
                onOpenProviders={onOpenProviders}
                onRetry={onRetryAgentConnection}
              />
            )
          : null}
        <div className="pointer-events-auto mx-auto flex w-full max-w-3xl items-center justify-between gap-3">
          {showJumpToLatest
            ? (
                <button
                  type="button"
                  onClick={scrollToLatest}
                  aria-label={t("thread.jumpToLatest")}
                  title={t("thread.jumpToLatest")}
                  className="mx-auto flex items-center gap-1 rounded-full border border-line-soft bg-surface px-3 py-1 text-xs text-ink-soft shadow-panel transition-colors hover:text-ink"
                >
                  <ArrowDown className="size-3.5" />
                  {t("thread.jumpToLatest")}
                </button>
              )
            : <div />}
          <QuestionNavControl
            hasNext={hasNextQuestion}
            hasPrevious={hasPreviousQuestion}
            onNext={goToNextQuestion}
            onPrevious={goToPreviousQuestion}
          />
        </div>
        {/* A message written while this reply streams is already the Agent's
            (see `handleEnqueueDraft`): it waits above the transcript as its own
            turn, so nothing is mirrored here. */}
        {/* What the Agent is holding for this conversation, above the box that
            filled it: a queued message is not yet gone, so it is looked at, fixed
            and taken back here. The rows are the Agent's (`agentQueue`), so this
            panel disappears with the window and the message does not. */}
        {/* A row that left the queue without running — folded into the turn
            ahead, taken back elsewhere, or lost with a deleted conversation —
            has no event of its own; its terminal ack is the only trace. Said
            outside the panel because it matters most once the panel is empty. */}
        {queueEnabled && agentQueue.notice
          ? (
              <p
                className="pointer-events-auto mx-auto mb-2 w-full max-w-3xl text-xs text-ink-muted"
                data-queue-notice=""
              >
                {t(`composer.queue.notice.${agentQueueNoticeReason(agentQueue.notice.reason)}`, {
                  reason: agentQueue.notice.reason,
                })}
                {agentQueue.notice.count > 1
                  ? ` · ${t("composer.queue.notice.more", { count: agentQueue.notice.count })}`
                  : ""}
              </p>
            )
          : null}
        {queueEnabled && agentQueue.rows.length > 0
          ? (
              <div className="pointer-events-auto mx-auto w-full max-w-3xl">
                <SendQueuePanel
                  // An Agent too old to report a row's files answers without the
                  // field: that row's editor then says an edit sends the words
                  // alone. A row that does report them carries them into the
                  // re-send, and shows them before that.
                  attachmentsUnreported
                  items={agentQueue.rows.map(row => ({
                    attachments: row.attachments,
                    content: row.text,
                    id: row.runId,
                    parked: isParkedRow(row),
                  }))}
                  onPreviewAttachment={handleQueuePreviewAttachment}
                  onRelease={handleQueueRelease}
                  onRemove={handleQueueRemove}
                  onUpdate={handleQueueUpdate}
                />
              </div>
            )
          : null}
        <Composer
          className="pointer-events-auto mx-auto w-full max-w-3xl"
          disabled={composerDisabled}
          modelId={modelId}
          modelOptions={modelOptions}
          modelsEmptyReason={
            agentConnection.readiness === "all_disabled"
              ? "all_disabled"
              : "no_models"
          }
          onModelChange={onModelChange}
          thinkingLevel={thinkingLevel}
          onThinkingLevelChange={onThinkingLevelChange}
          approvalTier={approvalTier}
          futureSessionStatus={futureSessionStatus}
          onChangeApprovalTier={onChangeApprovalTier}
          sending={isReplyStreaming}
          onAbort={handleComposerAbort}
          onCompactContext={
            thread?.agentSessionId ? handleCompactContext : undefined
          }
          compactionInProgress={agentState?.isCompacting ?? false}
          // The panel's send key on an empty draft closes the layer instead of
          // doing nothing. Everywhere else an empty submit is simply not a send,
          // so the prop is absent and that command goes unregistered.
          onEmptySubmit={isInput ? onLeaveInput : undefined}
          // The input layer is for speaking: a popup over the transcript would
          // also eat the next utterance's keys (`VIBE.md` §5.3).
          triggers={!isInput}
          controls={composerControls}
          onQueue={onQueue}
          onEnqueue={queueEnabled ? handleEnqueueDraft : undefined}
          onSend={handleComposerSend}
          workspaceId={thread?.workspaceId}
          sessionMentions={sessionMentions}
          draftKey={thread?.id}
          skillRecommendation={isInput ? undefined : skillRecommendation}
        />
      </div>
    </div>
  );

  const transcript = (
    <div className="group relative min-h-0 flex-1 overflow-hidden">
      {chrome === "full" && thread
        ? (
            <ThreadSearch
              contentKey={visibleMessages}
              onPrepareSearch={prepareSearch}
              onRevealMatch={revealSearchMatch}
              rootRef={searchRootRef}
            />
          )
        : null}
      {sessionChanged && (
        <div role="status" className="text-ink-soft px-4 py-2 text-sm">
          {t("thread.sessionChanged")}
        </div>
      )}
      {historyStatus?.canContinue === false && (
        <div role="status" className="text-ink-soft px-4 py-2 text-sm">
          {t(historyStatus.source === "legacy" ? "thread.historyRecoveryReadonly" : "thread.historyReadonly")}
        </div>
      )}
      {historyError && (
        <div role="alert" className="text-danger px-4 py-2 text-sm">
          {historyError}
          <button
            type="button"
            className="ml-2 underline"
            onClick={() => void retryHistory()}
          >
            {t("common:retry")}
          </button>
        </div>
      )}
      {showLoadOlderHint
        ? (
            <div className="pointer-events-none absolute inset-x-0 top-0 z-20 flex justify-center px-8 pt-5">
              <button
                type="button"
                onClick={loadOlder}
                disabled={coolingDown}
                aria-busy={coolingDown}
                aria-label={t("thread.loadOlder")}
                title={t("thread.loadOlder")}
                className="pointer-events-auto flex animate-pop-in items-center gap-1.5 rounded-full border border-line-soft bg-surface px-3 py-1 text-xs text-ink-soft shadow-panel transition-colors hover:text-ink"
              >
                <History className="size-3.5" />
                {t("thread.loadOlder")}
              </button>
            </div>
          )
        : null}
      <div
        ref={scrollRef}
        data-chat-scroll="true"
        className={cn(
          "floating-scrollbar h-full overflow-auto overscroll-none",
          // The panel's reading layer is read from further away and has no composer band
          // under it, so it gets more air at both ends than the main window's transcript:
          // a conversation should not start flush against the bar above it. The main window
          // keeps its own top inset and gains a bottom one, which it never had — the
          // floating composer covered the last line instead of clearing it.
          chrome === "full" ? "px-8 pt-8 pb-10" : "px-6 pt-10 pb-16",
          isInput && "opacity-55",
        )}
        style={{ overflowAnchor: "none" }}
        onScroll={handlePagingScroll}
      >
        {/* Keep the inset inside the observed content so sticky scrolling and
            the floating scrollbar settle when the composer grows or shrinks. */}
        <div
          ref={searchRootRef}
          className={cn("mx-auto w-full", chrome === "full" ? "max-w-4xl" : "max-w-3xl")}
        >
          {loadingIndicator
            ? (
                <div className="py-8 text-sm text-ink-soft">
                  {t("thread.loading")}
                </div>
              )
            : !thread && !loadingStore
                ? (
                    <div className="py-8 text-sm text-ink-soft">
                      {t("thread.noActiveThread")}
                    </div>
                  )
                : (
                    <MessageList
                      messages={shownMessages}
                      // More air between turns where the panel reads: same list, different
                      // rhythm, because the reading layer has no composer competing for
                      // the eye and is read from further away.
                      spacing={isRead || isInput ? "roomy" : "default"}
                      workspaceId={renderWorkspace.workspaceId}
                      workspacePath={renderWorkspace.workspacePath}
                      onContinue={handleContinueMessage}
                      onFork={handleFork}
                      onRetry={handleRetryMessage}
                    />
                  )}
          {isRead ? null : <div aria-hidden="true" style={{ height: composerHeight }} />}
        </div>
      </div>
      <FloatingScrollbar
        scrollbar={scrollbar}
        onPointerDown={handleThumbPointerDown}
      />
      {isRead ? null : composerBand}
    </div>
  );

  return (
    <div
      className={cn(
        "flex h-full min-h-0 flex-col overflow-hidden",
        // A cell is a pane of a larger surface, not a window: it must not paint
        // its own background over the panel's.
        isRead ? "bg-transparent" : "bg-surface",
      )}
    >
      {chrome === "full"
        ? (
            <ThreadHeader
              action={headerAction}
              leftPanelExpanded={leftPanelExpanded}
              machine={headerMachine}
              thread={thread}
              usage={agentState?.usage}
              workspace={headerWorkspace}
              onToggleLeftPanel={onToggleLeftPanel}
            />
          )
        : null}
      {isInput ? inputHeader : null}
      {transcript}
      {/* The read chrome has no composer, so the one thing it must still be able
          to show is a request that needs an answer — at the foot, where the
          composer would be. */}
      {isRead && approvalCard
        ? (
            <div className="shrink-0 border-t border-line-soft/60 px-5 pb-3 pt-2">
              {approvalCard}
            </div>
          )
        : null}
      {isInput
        ? (
            <>
              {/* Room for the input method's candidate window below the editor,
                  so it can never cover the sentence being dictated. */}
              <div aria-hidden="true" className="h-[26vh] shrink-0" />
              {inputFooter}
            </>
          )
        : null}
    </div>
  );
}

function shouldShowAgentNotice(connection: AgentConnectionState) {
  return (
    connection.status === "disconnected"
    || connection.readiness === "needs_login"
    || connection.readiness === "no_models"
    || connection.readiness === "all_disabled"
  );
}

interface AgentNotice {
  title: string;
  detail: string;
  action: { label: string; onClick: () => void };
}

function AgentConnectionNotice({
  connection,
  onRetry,
  onOpenAccount,
  onOpenModels,
  onOpenProviders,
}: {
  connection: AgentConnectionState;
  onRetry: () => void;
  onOpenAccount: () => void;
  onOpenModels: () => void;
  onOpenProviders: () => void;
}) {
  const { t } = useTranslation("agent");
  const notice = agentNotice(
    connection,
    { onOpenModels, onOpenAccount, onOpenProviders, onRetry },
    t,
  );
  return (
    <div className="pointer-events-auto mx-auto w-full max-w-3xl rounded-md border border-warning-line bg-warning-soft px-3 py-2 text-xs leading-5 text-warning shadow-xs">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-medium">{notice.title}</span>
        <button
          className="h-7 rounded-md bg-surface px-2 text-xs font-medium text-warning ring-1 ring-warning-line transition-colors hover:bg-warning-soft"
          onClick={notice.action.onClick}
          type="button"
        >
          {notice.action.label}
        </button>
      </div>
      <div className="mt-1 text-warning">{notice.detail}</div>
    </div>
  );
}

function agentNotice(
  connection: AgentConnectionState,
  actions: {
    onRetry: () => void;
    onOpenAccount: () => void;
    onOpenModels: () => void;
    onOpenProviders: () => void;
  },
  t: (key: string) => string,
): AgentNotice {
  const retry = { label: t("notice.retry"), onClick: actions.onRetry };

  // Can't reach the agent at all.
  if (connection.status === "disconnected") {
    if (connection.kind === "agent_unavailable") {
      return {
        title: t("notice.agentUnavailable.title"),
        detail: t("notice.agentUnavailable.detail"),
        action: retry,
      };
    }
    if (connection.kind === "model_error") {
      return {
        title: t("notice.modelError.title"),
        detail: connection.error ?? t("notice.modelError.detail"),
        action: retry,
      };
    }
    return {
      title: t("notice.connectionError.title"),
      detail: connection.error ?? t("notice.connectionError.detail"),
      action: retry,
    };
  }

  // Connected, but no usable models: distinguish "not configured" from "empty".
  if (connection.readiness === "needs_login") {
    return {
      title: t("notice.needsLogin.title"),
      detail: t("notice.needsLogin.detail"),
      action: {
        label: t("notice.needsLogin.action"),
        onClick: actions.onOpenProviders,
      },
    };
  }
  // Models loaded, but the user disabled every one — guide them to re-enable.
  if (connection.readiness === "all_disabled") {
    return {
      title: t("notice.allModelsDisabled.title"),
      detail: t("notice.allModelsDisabled.detail"),
      action: {
        label: t("notice.allModelsDisabled.action"),
        onClick: actions.onOpenModels,
      },
    };
  }
  return {
    title: t("notice.noModels.title"),
    detail: t("notice.noModels.detail"),
    action: {
      label: t("notice.noModels.action"),
      onClick: actions.onOpenModels,
    },
  };
}
