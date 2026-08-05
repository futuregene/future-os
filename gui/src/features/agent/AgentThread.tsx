import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { StoredApprovalRequest, StoredThread } from "../../integrations/storage/threadStore";
import type { AgentMessage, MessageAttachment } from "./agentThreadTypes";
import type { ComposerSendPayload } from "./Composer";
import { ArrowDown, History } from "lucide-react";
import { useCallback, useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { FloatingScrollbar } from "../../components/ui/FloatingScrollbar";
import { forkThread } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent, onFutureEvent } from "../../lib/futureEvents";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { ApprovalPrompt } from "./ApprovalPrompt";
import { buildContinuePrompt, loadRunResumeSummary, previousUserForRun } from "./buildContinuePrompt";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { ThreadHeader } from "./ThreadHeader";
import { useAgentThreadState } from "./useAgentThreadState";
import { useMessagePaging } from "./useMessagePaging";
import { useStickyAutoScroll } from "./useStickyAutoScroll";

/** How many user exchanges one loaded page renders. */
const PAGE_USER_EXCHANGES = 10;

interface AgentThreadProps {
  thread: StoredThread | null;
  workspacePath?: string | null;
  agentConnection: AgentConnectionState;
  leftPanelExpanded: boolean;
  loadingStore: boolean;
  modelId: string;
  modelOptions: AgentModelOption[];
  onModelChange: (modelId: string) => void;
  thinkingLevel: string;
  onThinkingLevelChange: (thinkingLevel: string) => void;
  approvalTier: ApprovalTier;
  onChangeApprovalTier: (value: ApprovalTier) => void;
  showThinking: boolean;
  pendingPrompt: { attachments?: MessageAttachment[]; id: string; content: string; targetThreadId: string } | null;
  activeApproval?: StoredApprovalRequest | null;
  onApprovalDecision: (approval: StoredApprovalRequest, status: "approved" | "rejected") => Promise<void>;
  onPromptConsumed: (id: string) => void;
  onRetryAgentConnection: () => void;
  onOpenAccount: () => void;
  onOpenModels: () => void;
  onOpenProviders: () => void;
  onForked: (threadId: string) => void;
  onThreadActivity: () => void;
  onToggleLeftPanel: () => void;
}

export function AgentThread({
  thread,
  workspacePath,
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
  showThinking,
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
}: AgentThreadProps) {
  const { t } = useTranslation("agent");
  const {
    handleAbort,
    handleSend,
    loadingThread,
    loadingIndicator,
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
  const messagesRef = useRef(messages);
  messagesRef.current = messages;

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
  const { handleScroll, scrollToLatest, showJumpToLatest } = useStickyAutoScroll({
    scrollRef,
    contentKey: messages,
    onScroll: handleScrollbarVisibility,
    onContentSettled: () => updateFloatingScrollbar(false),
  });

  // Windowed rendering for long threads: only the last PAGE_USER_EXCHANGES
  // exchanges render; loading an older page is a sync window change (the full
  // list stays in memory) with scroll anchoring so the viewport never jumps.
  // The paging hook composes the sticky auto-scroll handler via `onScroll`, so
  // one scroll event reaches both.
  const {
    visibleMessages,
    showLoadOlderHint,
    handleScroll: handlePagingScroll,
    loadOlder,
  } = useMessagePaging({
    messages,
    scrollRef,
    userExchangeCount: PAGE_USER_EXCHANGES,
    onScroll: handleScroll,
  });

  // When loading completes (initial load or thread switch), scroll to the
  // latest message.  useStickyAutoScroll's useLayoutEffect fires on contentKey
  // (messages) changes, but during loading the MessageList isn't in the DOM yet
  // — so that scroll is a no-op.  This catches the transition.
  useEffect(() => {
    if (!loadingThread) {
      // Wait one tick for the MessageList to render.
      requestAnimationFrame(() => scrollToLatest());
    }
  }, [loadingThread, scrollToLatest]);

  // A run is in flight while its assistant bubble is still streaming; the agent
  // rejects a concurrent prompt, so the composer is disabled until it settles.
  // The streaming bubble always belongs to the trailing turn (after the last
  // user message), so scan backwards only until the first user message instead
  // of the whole list — this runs on every streaming push.
  let isSending = false;
  for (let i = messages.length - 1; i >= 0; i--) {
    const message = messages[i]!;
    if (message.role === "user")
      break;
    if (message.role === "assistant" && message.status === "streaming") {
      isSending = true;
      break;
    }
  }

  const handleRetryMessage = useCallback((_message: AgentMessage, source: AgentMessage) => {
    void handleSend({
      attachments: source.attachments ?? [],
      content: source.content,
    });
  }, [handleSend]);

  const handleContinueMessage = useCallback((message: AgentMessage) => {
    void handleSend({
      attachments: [],
      content: buildContinuePrompt({ message }),
    });
  }, [handleSend]);

  const handleContinueRun = useCallback(async (runId: string) => {
    const summary = await loadRunResumeSummary(runId);
    void handleSend({
      attachments: [],
      content: buildContinuePrompt({ runId, summary }),
    });
  }, [handleSend]);

  // Reads messages through messagesRef so this callback stays stable across
  // streaming pushes — it's a dep of the recover-run subscription below, and
  // depending on `messages` re-subscribed the listener on every push (M6).
  const handleRetryRun = useCallback((runId: string, triggerMessageId?: string | null) => {
    const current = messagesRef.current;
    const source = triggerMessageId
      ? current.find(message => message.id === triggerMessageId && message.role === "user")
      : previousUserForRun(current, runId);
    if (!source)
      return;

    void handleSend({
      attachments: source.attachments ?? [],
      content: source.content,
    });
  }, [handleSend]);

  useEffect(() => onFutureEvent("recover-run", (detail) => {
    if (detail.action === "retry") {
      handleRetryRun(detail.runId, detail.triggerMessageId);
      return;
    }
    void handleContinueRun(detail.runId);
  }), [handleContinueRun, handleRetryRun]);

  // Reads messages through messagesRef (not a dep): `messages` changes
  // identity on every streaming push, and this callback is passed to every
  // MessageBlock — recreating it each push defeated the list's only memo
  // boundary and re-rendered every visible finalized row (H1).
  const handleFork = useCallback(async (aiMessage: AgentMessage) => {
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
    // 0-based ordinal among user messages — the fork point, robust to two
    // identical prompts (content is only a fallback on the backend).
    const userMessageIndex = current
      .filter(message => message.role === "user")
      .indexOf(userMessage);
    try {
      const newThreadId = await forkThread(thread.id, userMessage.content, userMessageIndex);
      onForked(newThreadId);
    }
    catch (error) {
      emitFutureEvent("toast", { message: t("message.forkFailed", { message: errorMessage(error) }), tone: "error" });
    }
  }, [thread, onForked, t]);

  // Stable wrappers for the memoized Composer: inline arrows here would be
  // fresh on every render (and AgentThread renders on every streaming push),
  // which would defeat the memo outright.
  const handleComposerAbort = useCallback(() => {
    void handleAbort();
  }, [handleAbort]);
  const handleComposerSend = useCallback((payload: ComposerSendPayload) => {
    void handleSend(payload);
  }, [handleSend]);

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-surface">
      <ThreadHeader
        leftPanelExpanded={leftPanelExpanded}
        thread={thread}
        onToggleLeftPanel={onToggleLeftPanel}
      />
      <div className="group relative min-h-0 flex-1 overflow-hidden">
        {showLoadOlderHint
          ? (
              <div className="pointer-events-none absolute inset-x-0 top-0 z-20 flex justify-center px-8 pt-5">
                <button
                  type="button"
                  onClick={loadOlder}
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
          className={cn(
            "floating-scrollbar h-full overflow-auto overscroll-none px-8 pt-6",
            activeApproval ? "pb-112" : "pb-48",
          )}
          data-chat-scroll="true"
          onScroll={handlePagingScroll}
        >
          <div className="mx-auto w-full max-w-4xl">
            {loadingIndicator
              ? (
                  <div className="py-8 text-sm text-ink-soft">{t("thread.loading")}</div>
                )
              : !thread && !loadingStore
                  ? (
                      <div className="py-8 text-sm text-ink-soft">{t("thread.noActiveThread")}</div>
                    )
                  : (
                      <MessageList
                        messages={visibleMessages}
                        showThinking={showThinking}
                        workspaceId={renderWorkspace.workspaceId}
                        workspacePath={renderWorkspace.workspacePath}
                        onContinue={handleContinueMessage}
                        onFork={handleFork}
                        onRetry={handleRetryMessage}
                      />
                    )}
          </div>
        </div>
        <FloatingScrollbar scrollbar={scrollbar} onPointerDown={handleThumbPointerDown} />
        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10 bg-linear-to-t from-surface from-80% to-transparent px-8 pb-5 pt-10">
          <div className="mx-auto flex w-full max-w-4xl flex-col gap-3">
            {activeApproval
              ? (
                  <div className="pointer-events-auto mx-auto w-full max-w-3xl">
                    <ApprovalPrompt
                      approval={activeApproval}
                      onDecision={onApprovalDecision}
                      threadMode={thread?.mode}
                    />
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
            {showJumpToLatest
              ? (
                  <button
                    type="button"
                    onClick={scrollToLatest}
                    aria-label={t("thread.jumpToLatest")}
                    title={t("thread.jumpToLatest")}
                    className="pointer-events-auto mx-auto flex items-center gap-1 rounded-full border border-line-soft bg-surface px-3 py-1 text-xs text-ink-soft shadow-panel transition-colors hover:text-ink"
                  >
                    <ArrowDown className="size-3.5" />
                    {t("thread.jumpToLatest")}
                  </button>
                )
              : null}
            <Composer
              className="pointer-events-auto mx-auto w-full max-w-3xl"
              disabled={!thread || loadingThread || loadingStore}
              modelId={modelId}
              modelOptions={modelOptions}
              modelsEmptyReason={agentConnection.readiness === "all_disabled" ? "all_disabled" : "no_models"}
              onModelChange={onModelChange}
              thinkingLevel={thinkingLevel}
              onThinkingLevelChange={onThinkingLevelChange}
              approvalTier={approvalTier}
              onChangeApprovalTier={onChangeApprovalTier}
              sending={isSending}
              onAbort={handleComposerAbort}
              onSend={handleComposerSend}
              workspaceId={thread?.workspaceId}
              draftKey={thread?.id}
            />
          </div>
        </div>
      </div>
    </div>
  );
}

function shouldShowAgentNotice(connection: AgentConnectionState) {
  return connection.status === "disconnected"
    || connection.readiness === "needs_login"
    || connection.readiness === "no_models"
    || connection.readiness === "all_disabled";
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
  const notice = agentNotice(connection, { onOpenModels, onOpenAccount, onOpenProviders, onRetry }, t);
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
  actions: { onRetry: () => void; onOpenAccount: () => void; onOpenModels: () => void; onOpenProviders: () => void },
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
      action: { label: t("notice.needsLogin.action"), onClick: actions.onOpenProviders },
    };
  }
  // Models loaded, but the user disabled every one — guide them to re-enable.
  if (connection.readiness === "all_disabled") {
    return {
      title: t("notice.allModelsDisabled.title"),
      detail: t("notice.allModelsDisabled.detail"),
      action: { label: t("notice.allModelsDisabled.action"), onClick: actions.onOpenModels },
    };
  }
  return {
    title: t("notice.noModels.title"),
    detail: t("notice.noModels.detail"),
    action: { label: t("notice.noModels.action"), onClick: actions.onOpenModels },
  };
}
