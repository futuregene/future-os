import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { StoredApprovalRequest, StoredThread } from "../../integrations/storage/threadStore";
import type { AgentMessage, MessageAttachment } from "./agentThreadTypes";
import { useCallback, useEffect } from "react";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { onFutureEvent } from "../../lib/futureEvents";
import { ApprovalPrompt } from "./ApprovalPrompt";
import { buildContinuePrompt, loadRunResumeSummary, previousUserForRun } from "./buildContinuePrompt";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { ThreadHeader } from "./ThreadHeader";
import { useAgentThreadState } from "./useAgentThreadState";

interface AgentThreadProps {
  thread: StoredThread | null;
  agentConnection: AgentConnectionState;
  leftPanelExpanded: boolean;
  loadingStore: boolean;
  modelId: string;
  modelOptions: AgentModelOption[];
  onModelChange: (modelId: string) => void;
  thinkingLevel: string;
  onThinkingLevelChange: (thinkingLevel: string) => void;
  autoApprove: boolean;
  onToggleAutoApprove: (value: boolean) => void;
  showThinking: boolean;
  pendingPrompt: { attachments?: MessageAttachment[]; id: string; content: string } | null;
  activeApproval?: StoredApprovalRequest | null;
  onApprovalDecision: (approval: StoredApprovalRequest, status: "approved" | "rejected") => Promise<void>;
  onPromptConsumed: (id: string) => void;
  onRetryAgentConnection: () => void;
  onOpenProviders: () => void;
  onOpenModels: () => void;
  onThreadActivity: () => void;
  onToggleLeftPanel: () => void;
}

export function AgentThread({
  thread,
  agentConnection,
  leftPanelExpanded,
  loadingStore,
  modelId,
  modelOptions,
  onModelChange,
  thinkingLevel,
  onThinkingLevelChange,
  autoApprove,
  onToggleAutoApprove,
  showThinking,
  pendingPrompt,
  activeApproval,
  onApprovalDecision,
  onPromptConsumed,
  onRetryAgentConnection,
  onOpenProviders,
  onOpenModels,
  onThreadActivity,
  onToggleLeftPanel,
}: AgentThreadProps) {
  const { t } = useTranslation("agent");
  const {
    handleScroll,
    handleSend,
    loadingThread,
    messages,
    scrollRef,
    scrollbar,
  } = useAgentThreadState({
    thread,
    loadingStore,
    modelId,
    thinkingLevel,
    pendingPrompt,
    onPromptConsumed,
    onThreadActivity,
  });

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
      <div className="relative min-h-0 flex-1 overflow-hidden">
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
                        onContinue={handleContinueMessage}
                        onRetry={handleRetryMessage}
                      />
                    )}
          </div>
        </div>
        <div
          className={cn(
            "pointer-events-none absolute right-1 top-0 z-20 w-1.5 rounded-full bg-line transition-opacity duration-300",
            scrollbar.visible ? "opacity-80" : "opacity-0",
          )}
          style={{
            height: `${scrollbar.height}px`,
            transform: `translateY(${scrollbar.top}px)`,
          }}
        />
        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10 bg-linear-to-t from-surface from-80% to-transparent px-8 pb-5 pt-10">
          <div className="mx-auto flex w-full max-w-4xl flex-col gap-3">
            {activeApproval
              ? (
                  <div className="pointer-events-auto mx-auto w-full max-w-3xl">
                    <ApprovalPrompt
                      approval={activeApproval}
                      onDecision={onApprovalDecision}
                    />
                  </div>
                )
              : null}
            {shouldShowAgentNotice(agentConnection)
              ? (
                  <AgentConnectionNotice
                    connection={agentConnection}
                    onOpenModels={onOpenModels}
                    onOpenProviders={onOpenProviders}
                    onRetry={onRetryAgentConnection}
                  />
                )
              : null}
            <Composer
              className="pointer-events-auto mx-auto w-full max-w-3xl"
              disabled={!thread || loadingThread || loadingStore || isSending}
              modelId={modelId}
              modelOptions={modelOptions}
              onModelChange={onModelChange}
              thinkingLevel={thinkingLevel}
              onThinkingLevelChange={onThinkingLevelChange}
              autoApprove={autoApprove}
              onToggleAutoApprove={onToggleAutoApprove}
              onSend={handleSend}
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
    || connection.readiness === "no_models";
}

interface AgentNotice {
  title: string;
  detail: string;
  action: { label: string; onClick: () => void };
}

function AgentConnectionNotice({
  connection,
  onRetry,
  onOpenProviders,
  onOpenModels,
}: {
  connection: AgentConnectionState;
  onRetry: () => void;
  onOpenProviders: () => void;
  onOpenModels: () => void;
}) {
  const { t } = useTranslation("agent");
  const notice = agentNotice(connection, { onOpenModels, onOpenProviders, onRetry }, t);
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
  actions: { onRetry: () => void; onOpenProviders: () => void; onOpenModels: () => void },
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
  return {
    title: t("notice.noModels.title"),
    detail: t("notice.noModels.detail"),
    action: { label: t("notice.noModels.action"), onClick: actions.onOpenModels },
  };
}
