import type { AgentModelOption } from "../../integrations/agent/models";
import type { StoredApprovalRequest, StoredThread } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./types";
import { cn } from "../../lib/cn";
import { ApprovalPrompt } from "./ApprovalPrompt";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { ThreadHeader } from "./ThreadHeader";
import { useAgentThreadController } from "./useAgentThreadController";

interface AgentThreadProps {
  thread: StoredThread | null;
  leftPanelExpanded: boolean;
  loadingStore: boolean;
  modelId: string;
  modelOptions: AgentModelOption[];
  onModelChange: (modelId: string) => void;
  pendingPrompt: { attachments?: MessageAttachment[]; id: string; content: string } | null;
  activeApproval?: StoredApprovalRequest | null;
  onApprovalDecision: (approval: StoredApprovalRequest, status: "approved" | "rejected") => Promise<void>;
  onPromptConsumed: (id: string) => void;
  onThreadActivity: () => void;
  onToggleLeftPanel: () => void;
}

export function AgentThread({
  thread,
  leftPanelExpanded,
  loadingStore,
  modelId,
  modelOptions,
  onModelChange,
  pendingPrompt,
  activeApproval,
  onApprovalDecision,
  onPromptConsumed,
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
  } = useAgentThreadController({
    thread,
    loadingStore,
    modelId,
    modelOptions,
    pendingPrompt,
    onPromptConsumed,
    onThreadActivity,
  });

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
                      <MessageList messages={messages} workspaceId={thread?.workspaceId} />
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
