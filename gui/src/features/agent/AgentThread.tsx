import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { ApprovalTier } from "../../integrations/storage/appSettings";
import type { StoredApprovalRequest, StoredThread } from "../../integrations/storage/threadStore";
import type { AgentMessage, MessageAttachment } from "./agentThreadTypes";
import { ArrowDown } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect } from "react";
import { useTranslation } from "react-i18next";
import { FloatingScrollbar } from "../../components/ui/FloatingScrollbar";
import { cn } from "../../lib/cn";
import { onFutureEvent } from "../../lib/futureEvents";
import { useFloatingScrollbar } from "../../lib/useFloatingScrollbar";
import { ApprovalPrompt } from "./ApprovalPrompt";
import { buildContinuePrompt, loadRunResumeSummary, previousUserForRun } from "./buildContinuePrompt";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { ThreadHeader } from "./ThreadHeader";
import { useAgentThreadState } from "./useAgentThreadState";
import { useStickyAutoScroll } from "./useStickyAutoScroll";

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
  onThreadActivity,
  onToggleLeftPanel,
}: AgentThreadProps) {
  const { t } = useTranslation("agent");
  const {
    handleAbort,
    handleSend,
    loadingThread,
    messages,
    saveScrollPosition,
    consumeRestoredScrollTop,
  } = useAgentThreadState({
    thread,
    loadingStore,
    modelId,
    thinkingLevel,
    pendingPrompt,
    onPromptConsumed,
    onThreadActivity,
  });

  const {
    scrollRef,
    scrollbar,
    updateFloatingScrollbar,
    handleScroll: handleScrollbarVisibility,
    handleThumbPointerDown,
  } = useFloatingScrollbar();

  // Sticky auto-scroll: follow streaming output only while pinned near the
  // bottom; re-pins on thread switch and follows the growing message list.
  const { handleScroll: handleStickyScroll, scrollToLatest, showJumpToLatest } = useStickyAutoScroll({
    scrollRef,
    resetKey: thread?.id ?? null,
    contentKey: messages,
    onScroll: handleScrollbarVisibility,
    onContentSettled: () => updateFloatingScrollbar(false),
  });

  // Compose scroll handler: track position for cache + delegate to sticky logic.
  const handleScroll = useCallback(() => {
    const el = scrollRef.current;
    if (el && thread?.id) saveScrollPosition(thread.id, el.scrollTop);
    handleStickyScroll();
  }, [saveScrollPosition, handleStickyScroll, scrollRef, thread?.id]);

  // Restore scroll position from cache after messages render (before paint).
  useLayoutEffect(() => {
    const top = consumeRestoredScrollTop();
    const el = scrollRef.current;
    if (top !== null && top > 0 && el) {
      // Use requestAnimationFrame so the DOM layout is settled.
      requestAnimationFrame(() => {
        el.scrollTop = top;
      });
    }
  }, [consumeRestoredScrollTop, scrollRef]);

  // Save scroll position when leaving this thread.
  useEffect(() => {
    return () => {
      const el = scrollRef.current;
      if (el && thread?.id) saveScrollPosition(thread.id, el.scrollTop);
    };
  }, [saveScrollPosition, scrollRef, thread?.id]);

  // A run is in flight while its assistant bubble is still streaming; the agent
  // rejects a concurrent prompt, so the composer is disabled until it settles.
  const isSending = messages.some(
    message => message.role === "assistant" && message.status === "streaming",
  );

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

  const handleRetryRun = useCallback((runId: string, triggerMessageId?: string | null) => {
    const source = triggerMessageId
      ? messages.find(message => message.id === triggerMessageId && message.role === "user")
      : previousUserForRun(messages, runId);
    if (!source)
      return;

    void handleSend({
      attachments: source.attachments ?? [],
      content: source.content,
    });
  }, [handleSend, messages]);

  useEffect(() => onFutureEvent("recover-run", (detail) => {
    if (detail.action === "retry") {
      handleRetryRun(detail.runId, detail.triggerMessageId);
      return;
    }
    void handleContinueRun(detail.runId);
  }), [handleContinueRun, handleRetryRun]);

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-surface">
      <ThreadHeader
        leftPanelExpanded={leftPanelExpanded}
        thread={thread}
        onToggleLeftPanel={onToggleLeftPanel}
      />
      <div className="group relative min-h-0 flex-1 overflow-hidden">
        <div
          ref={scrollRef}
          className={cn(
            "floating-scrollbar h-full overflow-auto overscroll-none px-8 pt-6",
            activeApproval ? "pb-112" : "pb-48",
          )}
          data-chat-scroll="true"
          onScroll={handleScroll}
        >
          <div className="mx-auto w-full max-w-4xl">
            {loadingThread
              ? (
                  <div className="py-8 text-sm text-ink-soft">{t("thread.loading")}</div>
                )
              : !thread && !loadingStore
                  ? (
                      <div className="py-8 text-sm text-ink-soft">{t("thread.noActiveThread")}</div>
                    )
                  : (
                      <MessageList
                        messages={messages}
                        showThinking={showThinking}
                        workspaceId={thread?.workspaceId}
                        workspacePath={workspacePath}
                        onContinue={handleContinueMessage}
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
              disabled={!thread || loadingThread || loadingStore || isSending}
              modelId={modelId}
              modelOptions={modelOptions}
              modelsEmptyReason={agentConnection.readiness === "all_disabled" ? "all_disabled" : "no_models"}
              onModelChange={onModelChange}
              thinkingLevel={thinkingLevel}
              onThinkingLevelChange={onThinkingLevelChange}
              approvalTier={approvalTier}
              onChangeApprovalTier={onChangeApprovalTier}
              sending={isSending}
              onAbort={() => void handleAbort()}
              onSend={payload => void handleSend(payload)}
              workspaceId={thread?.workspaceId}
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
}: {
  connection: AgentConnectionState;
  onRetry: () => void;
  onOpenAccount: () => void;
  onOpenModels: () => void;
}) {
  const { t } = useTranslation("agent");
  const notice = agentNotice(connection, { onOpenModels, onOpenAccount, onRetry }, t);
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
  actions: { onRetry: () => void; onOpenAccount: () => void; onOpenModels: () => void },
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
      action: { label: t("notice.needsLogin.action"), onClick: actions.onOpenAccount },
    };
  }
  // Models loaded, but the user disabled every one — steer them to re-enable.
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
