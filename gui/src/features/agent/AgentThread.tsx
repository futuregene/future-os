import type { AgentConnectionState } from "../../components/layout/AppShell";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { StoredApprovalRequest, StoredRunEvent, StoredThread, StoredToolCall, StoredToolOutput } from "../../integrations/storage/threadStore";
import type { AgentMessage, MessageAttachment } from "./agentThreadTypes";
import { useCallback, useEffect } from "react";
import { listRunEvents, listToolCalls, listToolOutputs } from "../../integrations/storage/threadStore";
import { cn } from "../../lib/cn";
import { onFutureEvent } from "../../lib/futureEvents";
import { ApprovalPrompt } from "./ApprovalPrompt";
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
    modelOptions,
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
            "pointer-events-none absolute right-1 top-0 z-20 w-1.5 rounded-full bg-slate-300 transition-opacity duration-300",
            scrollbar.visible ? "opacity-80" : "opacity-0",
          )}
          style={{
            height: `${scrollbar.height}px`,
            transform: `translateY(${scrollbar.top}px)`,
          }}
        />
        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10 px-8 pb-5">
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
    <div className="pointer-events-auto mx-auto w-full max-w-3xl rounded-md border border-amber-200 bg-amber-50 px-3 py-2 text-xs leading-5 text-amber-800 shadow-sm">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="font-medium">{notice.title}</span>
        <button
          className="h-7 rounded-md bg-surface px-2 text-xs font-medium text-amber-800 ring-1 ring-amber-200 transition-colors hover:bg-amber-100"
          onClick={notice.action.onClick}
          type="button"
        >
          {notice.action.label}
        </button>
      </div>
      <div className="mt-1 text-amber-700">{notice.detail}</div>
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

function buildContinuePrompt({
  message,
  runId,
  summary,
}: {
  message?: AgentMessage;
  runId?: string;
  summary?: string;
}) {
  const effectiveRunId = runId ?? message?.runId ?? null;
  const lines = [
    "继续上一个任务。",
    "请基于当前线程、最近一次运行状态和工作区当前文件状态继续推进。",
    "不要重复执行已经成功完成的副作用操作；如果需要再次写入、删除、执行复杂命令，请先说明原因并遵守审批策略。",
  ];

  if (effectiveRunId) {
    lines.push("", `最近 Run: ${effectiveRunId}`);
  }
  if (message?.content?.trim()) {
    lines.push("", "上一条失败消息摘要:", truncateForPrompt(message.content.trim(), 1200));
  }
  if (summary?.trim()) {
    lines.push("", "已执行内容摘要:", summary.trim());
  }

  return lines.join("\n");
}

async function loadRunResumeSummary(runId: string) {
  try {
    const [events, tools] = await Promise.all([
      listRunEvents(runId),
      listToolCalls(runId),
    ]);
    const outputEntries = await Promise.all(
      tools.slice(0, 8).map(async (tool) => {
        try {
          return [tool.id, await listToolOutputs(tool.id)] as const;
        }
        catch {
          return [tool.id, [] as StoredToolOutput[]] as const;
        }
      }),
    );
    const outputsByTool = Object.fromEntries(outputEntries);
    return summarizeRunForPrompt(events, tools, outputsByTool);
  }
  catch (error) {
    return `Run 摘要加载失败：${error instanceof Error ? error.message : String(error)}`;
  }
}

function summarizeRunForPrompt(
  events: StoredRunEvent[],
  tools: StoredToolCall[],
  outputsByTool: Record<string, StoredToolOutput[]>,
) {
  const lines: string[] = [];
  if (tools.length > 0) {
    lines.push("工具调用:");
    for (const tool of tools.slice(0, 8)) {
      const command = toolCommand(tool.input) ?? tool.input ?? tool.name;
      const outputs = outputsByTool[tool.id] ?? [];
      const outputSummary = outputs
        .map(output => output.content ?? output.kind)
        .filter(Boolean)
        .map(value => truncateForPrompt(value, 240))
        .join(" | ");
      lines.push(`- ${tool.name} [${tool.status}]: ${truncateForPrompt(command, 360)}${outputSummary ? ` => ${outputSummary}` : ""}`);
    }
    if (tools.length > 8) {
      lines.push(`- 还有 ${tools.length - 8} 个工具调用未展开。`);
    }
  }

  const finalEvents = events
    .filter(event => ["error", "agent_error", "agent_end", "tool_end", "tool_result"].includes(event.eventType))
    .slice(-6);
  if (finalEvents.length > 0) {
    lines.push("最近事件:");
    for (const event of finalEvents) {
      lines.push(`- ${event.eventType}: ${truncateForPrompt(event.payload ?? "", 360)}`);
    }
  }

  return lines.join("\n");
}

function previousUserForRun(messages: AgentMessage[], runId: string) {
  const runMessageIndex = messages.findIndex(message => message.runId === runId && message.role === "assistant");
  const startIndex = runMessageIndex >= 0 ? runMessageIndex - 1 : messages.length - 1;
  for (let index = startIndex; index >= 0; index -= 1) {
    if (messages[index].role === "user") {
      return messages[index];
    }
  }
  return null;
}

function toolCommand(input: string | null | undefined) {
  if (!input)
    return null;

  let current: unknown = input;
  for (let index = 0; index < 3; index += 1) {
    if (isRecord(current)) {
      const value = current.command;
      return typeof value === "string" && value.trim() ? value : null;
    }
    if (typeof current !== "string")
      return null;

    try {
      current = JSON.parse(current) as unknown;
    }
    catch {
      return null;
    }
  }

  return null;
}

function truncateForPrompt(value: string, limit: number) {
  const normalized = value.replace(/\s+/g, " ").trim();
  return normalized.length > limit ? `${normalized.slice(0, limit)}...` : normalized;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
