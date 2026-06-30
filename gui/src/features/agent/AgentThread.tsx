import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { StoredApprovalRequest, StoredThread } from "../../integrations/storage/threadStore";
import type { AgentMessage, MessageAttachment } from "./agentThreadTypes";
import { useCallback, useEffect } from "react";
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
            activeApproval ? "pb-[28rem]" : "pb-48",
          )}
          data-chat-scroll="true"
          onScroll={handleScroll}
        >
          <div className="mx-auto w-full max-w-4xl">
            {loadingThread
              ? (
                  <div className="py-8 text-sm text-ink-soft">Loading FutureOS thread...</div>
                )
              : !thread && !loadingStore
                  ? (
                      <div className="py-8 text-sm text-ink-soft">No active thread.</div>
                    )
                  : (
                      <MessageList
                        messages={messages}
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
        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10 bg-gradient-to-t from-surface from-80% to-transparent px-8 pb-5 pt-10">
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
              disabled={!thread || loadingThread || loadingStore}
              modelId={modelId}
              modelOptions={modelOptions}
              onModelChange={onModelChange}
              thinkingLevel={thinkingLevel}
              onThinkingLevelChange={onThinkingLevelChange}
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
  const notice = agentNotice(connection, { onOpenModels, onOpenProviders, onRetry });
  return (
    <div className="pointer-events-auto mx-auto w-full max-w-3xl rounded-md border border-warning-line bg-warning-soft px-3 py-2 text-xs leading-5 text-warning shadow-sm">
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
): AgentNotice {
  const retry = { label: "重试", onClick: actions.onRetry };

  // Can't reach the agent at all.
  if (connection.status === "disconnected") {
    if (connection.kind === "agent_unavailable") {
      return {
        title: "Future Agent 未运行",
        detail: "请先启动 Future Agent，然后点击重试。",
        action: retry,
      };
    }
    if (connection.kind === "model_error") {
      return {
        title: "模型加载失败",
        detail: connection.error ?? "Agent 可连接，但获取模型列表失败。",
        action: retry,
      };
    }
    return {
      title: "连接异常",
      detail: connection.error ?? "请检查 FUTURE_AGENT_GRPC_ADDR 后重试。",
      action: retry,
    };
  }

  // Connected, but no usable models: distinguish "not configured" from "empty".
  if (connection.readiness === "needs_login") {
    return {
      title: "尚未登录",
      detail: "已连接 Future Agent，但还没有可用模型。请连接 FutureGene 登录，或添加自定义提供商。",
      action: { label: "前往登录", onClick: actions.onOpenProviders },
    };
  }
  return {
    title: "没有可用模型",
    detail: "已配置提供商，但模型列表为空。请检查模型是否已启用，或确认账号配额 / 权限。",
    action: { label: "模型设置", onClick: actions.onOpenModels },
  };
}
