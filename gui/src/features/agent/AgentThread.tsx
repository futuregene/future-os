import type { StoredThread } from "../../integrations/storage/threadStore";
import type { MessageAttachment } from "./types";
import { useEffect, useState } from "react";
import { cn } from "../../lib/cn";
import { Composer } from "./Composer";
import { MessageList } from "./MessageList";
import { ThreadHeader } from "./ThreadHeader";
import { useAgentThreadController } from "./useAgentThreadController";

interface AgentThreadProps {
  thread: StoredThread | null;
  leftPanelExpanded: boolean;
  loadingStore: boolean;
  modelId: string;
  onModelChange: (modelId: string) => void;
  pendingPrompt: { attachments?: MessageAttachment[]; id: string; content: string } | null;
  onArchiveThread: () => void;
  onDeleteThread: () => void;
  onPromptConsumed: (id: string) => void;
  onRenameThread: () => void;
  onThreadActivity: () => void;
  onToggleLeftPanel: () => void;
  onTogglePinThread: () => void;
}

export function AgentThread({
  thread,
  leftPanelExpanded,
  loadingStore,
  modelId,
  onModelChange,
  pendingPrompt,
  onArchiveThread,
  onDeleteThread,
  onPromptConsumed,
  onRenameThread,
  onThreadActivity,
  onToggleLeftPanel,
  onTogglePinThread,
}: AgentThreadProps) {
  const [threadMenuOpen, setThreadMenuOpen] = useState(false);
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
    pendingPrompt,
    onPromptConsumed,
    onThreadActivity,
  });

  useEffect(() => {
    setThreadMenuOpen(false);
  }, [thread?.id]);

  return (
    <div className="flex h-full min-h-0 flex-col overflow-hidden bg-surface">
      <ThreadHeader
        leftPanelExpanded={leftPanelExpanded}
        menuOpen={threadMenuOpen}
        thread={thread}
        onArchiveThread={onArchiveThread}
        onDeleteThread={onDeleteThread}
        onMenuOpenChange={setThreadMenuOpen}
        onRenameThread={onRenameThread}
        onToggleLeftPanel={onToggleLeftPanel}
        onTogglePinThread={onTogglePinThread}
      />
      <div className="relative min-h-0 flex-1 overflow-hidden">
        <div
          ref={scrollRef}
          className="floating-scrollbar h-full overflow-auto overscroll-none px-8 pb-48 pt-6"
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
                      <MessageList messages={messages} />
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
          <div className="mx-auto w-full max-w-4xl">
            <Composer
              className="pointer-events-auto mx-auto max-w-3xl"
              disabled={!thread || loadingThread || loadingStore}
              modelId={modelId}
              onModelChange={onModelChange}
              onSend={handleSend}
            />
          </div>
        </div>
      </div>
    </div>
  );
}
