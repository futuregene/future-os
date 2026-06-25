import type { Dispatch, SetStateAction } from "react";
import type { AgentModelOption } from "../../integrations/agent/agentClient";
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import type { AgentActivityItem, AgentMessage, MessageAttachment } from "./agentThreadTypes";
import type { ComposerSendPayload } from "./Composer";
import { useCallback, useEffect, useRef, useState } from "react";
import { modelSupportsImages, modelThinkingLevel, sendPromptToFutureAgent } from "../../integrations/agent/agentClient";
import {
  appendMessage,
  createRun,
  importAttachmentArtifact,
  listMessages,
  listRunEvents,
  listRuns,
  storedTimeToIso,
} from "../../integrations/storage/threadStore";
import { upsertFutureReferenceData } from "../markdown/futureReferenceStore";
import { buildAssistantRunProjection, thinkingActivity } from "./agentActivity";
import {
  buildAgentFailureContent,
  matchesSettledRun,
  toAgentMessage,
  updateRunStatusSafe,
} from "./agentMessageFormatters";
import { buildPromptWithAttachments, imageAttachmentPaths, stringifyMessageContent } from "./attachments";
import { buildReferencePrompt } from "./buildReferencePrompt";

interface UseAgentThreadStateInput {
  thread: StoredThread | null;
  loadingStore: boolean;
  modelId: string;
  modelOptions: AgentModelOption[];
  pendingPrompt: { attachments?: MessageAttachment[]; id: string; content: string } | null;
  onPromptConsumed: (id: string) => void;
  onThreadActivity: () => void;
}

export function useAgentThreadState({
  thread,
  loadingStore,
  modelId,
  modelOptions,
  pendingPrompt,
  onPromptConsumed,
  onThreadActivity,
}: UseAgentThreadStateInput) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [loadingThread, setLoadingThread] = useState(true);
  const [recentRun, setRecentRun] = useState<StoredRun | null>(null);
  const [recentRunEventCount, setRecentRunEventCount] = useState(0);
  const [scrollbar, setScrollbar] = useState({ height: 0, top: 0, visible: false });
  const scrollRef = useRef<HTMLDivElement>(null);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const consumedPromptRef = useRef<string | null>(null);
  const sendGenerationRef = useRef(0);
  const streamTimerRef = useRef<number | null>(null);
  const threadId = thread?.id ?? null;

  const updateFloatingScrollbar = useCallback((visible: boolean) => {
    const scrollContainer = scrollRef.current;
    if (!scrollContainer)
      return;

    const { clientHeight, scrollHeight, scrollTop } = scrollContainer;
    const scrollbarInset = 4;
    const canScroll = scrollHeight > clientHeight;
    const minThumbHeight = 36;
    const height = canScroll
      ? Math.max(minThumbHeight, (clientHeight / scrollHeight) * (clientHeight - scrollbarInset * 2))
      : 0;
    const maxTop = clientHeight - scrollbarInset * 2 - height;
    const top = canScroll ? scrollbarInset + (scrollTop / (scrollHeight - clientHeight)) * maxTop : scrollbarInset;

    setScrollbar({ height, top, visible: visible && canScroll });
  }, []);

  const handleScroll = useCallback(() => {
    updateFloatingScrollbar(true);

    if (scrollTimerRef.current) {
      clearTimeout(scrollTimerRef.current);
    }

    scrollTimerRef.current = setTimeout(() => {
      updateFloatingScrollbar(false);
    }, 1200);
  }, [updateFloatingScrollbar]);

  // Guard against overlapping refreshes (poll tick, send, thread switch) where a
  // slow response lands after a newer one and writes stale run state — e.g. a
  // previous thread's run after switching. Newest call wins.
  const recentRunGenRef = useRef(0);
  const refreshRecentRun = useCallback(async (threadId: string, workspaceId?: string | null) => {
    const generation = ++recentRunGenRef.current;
    const runs = await listRuns(threadId);
    if (generation !== recentRunGenRef.current) {
      return;
    }
    const latestRun = runs[0] ?? null;
    setRecentRun(latestRun);
    if (latestRun) {
      upsertFutureReferenceData(workspaceId, "run", latestRun.id, latestRun);
    }
    if (latestRun?.status === "waiting_approval") {
      setMessages(current =>
        current.map(message =>
          message.id.startsWith("pending_")
            ? {
                ...message,
                content: "正在等待你审批一个操作。请在输入框上方查看并决定是否继续。",
              }
            : message,
        ),
      );
    }

    if (latestRun) {
      const events = await listRunEvents(latestRun.id);
      if (generation !== recentRunGenRef.current) {
        return;
      }
      setRecentRunEventCount(events.length);
    }
    else {
      setRecentRunEventCount(0);
    }
  }, []);

  const handleSend = useCallback(async ({ attachments, content }: ComposerSendPayload) => {
    if (!thread)
      return;

    const sendGeneration = sendGenerationRef.current + 1;
    sendGenerationRef.current = sendGeneration;
    const isCurrentSend = () => sendGenerationRef.current === sendGeneration;
    const clearStreamTimer = () => {
      if (streamTimerRef.current !== null) {
        window.clearInterval(streamTimerRef.current);
        streamTimerRef.current = null;
      }
    };
    const optimisticUserId = clientId("pending_user");
    const pendingId = clientId("pending");
    const optimisticUserMessage: AgentMessage = {
      id: optimisticUserId,
      role: "user",
      author: "You",
      content,
      status: "complete",
      createdAt: new Date().toISOString(),
      attachments,
    };
    const assistantMessage: AgentMessage = {
      id: pendingId,
      role: "assistant",
      author: "Research Copilot",
      content: "",
      status: "streaming",
      createdAt: new Date().toISOString(),
      activityItems: thinkingActivity(),
    };
    setMessages(current => [...current, optimisticUserMessage, assistantMessage]);
    onThreadActivity();

    let run: StoredRun | null = null;

    try {
      const importedAttachments = await importChatAttachments(thread, attachments);

      const messageContent = importedAttachments.length > 0
        ? stringifyMessageContent(content, importedAttachments)
        : content;
      const promptContent = await buildReferencePrompt(
        thread.workspaceId,
        content,
        buildPromptWithAttachments(content, importedAttachments),
      );

      if (isCurrentSend()) {
        setMessages(current =>
          current.map(message =>
            message.id === optimisticUserId
              ? { ...message, attachments: importedAttachments }
              : message,
          ),
        );
      }

      const storedUserMessage = await appendMessage({
        threadId: thread.id,
        role: "user",
        contentType: importedAttachments.length > 0 ? "mixed" : "markdown",
        content: messageContent,
        status: "complete",
      });

      if (isCurrentSend()) {
        setMessages(current =>
          current.map(message =>
            message.id === optimisticUserId
              ? {
                  ...message,
                  id: storedUserMessage.id,
                  createdAt: storedTimeToIso(storedUserMessage.createdAt),
                }
              : message,
          ),
        );
      }

      run = await createRun({
        threadId: thread.id,
        triggerMessageId: storedUserMessage.id,
        modelId,
      });

      if (isCurrentSend()) {
        setRecentRun(run);
        upsertFutureReferenceData(thread.workspaceId, "run", run.id, run);
        setRecentRunEventCount(0);
        setMessages(current =>
          current.map(message =>
            message.id === pendingId
              ? { ...message, runId: run?.id ?? null }
              : message,
          ),
        );
      }

      clearStreamTimer();
      if (isCurrentSend()) {
        streamTimerRef.current = window.setInterval(() => {
          if (run && isCurrentSend()) {
            void updatePendingMessageFromRunEvents(run.id, pendingId, setMessages, isCurrentSend);
          }
        }, 220);
      }

      const agentSessionId = thread.agentSessionId?.trim() || thread.id;
      const reply = await sendPromptToFutureAgent(
        promptContent,
        thread.id,
        agentSessionId,
        run.id,
        modelId,
        modelSupportsImages(modelId, modelOptions) ? imageAttachmentPaths(importedAttachments) : [],
        modelThinkingLevel(modelId, modelOptions),
      );
      clearStreamTimer();

      const currentRun = await loadCurrentRun(thread.id, run.id);
      if (currentRun && matchesSettledRun(currentRun.status)) {
        return;
      }
      await updateRunStatusSafe(run.id, "completed");
      if (isCurrentSend()) {
        await refreshRecentRun(thread.id, thread.workspaceId);
      }
      const storedAssistantMessage = await appendMessage({
        threadId: thread.id,
        runId: run.id,
        role: "assistant",
        contentType: "markdown",
        content: reply.trim() || "Future Agent 已完成，但没有返回文本。",
        status: "complete",
      });

      if (isCurrentSend()) {
        setMessages(current =>
          current.map(message =>
            message.id === pendingId
              ? {
                  ...message,
                  id: storedAssistantMessage.id,
                  runId: storedAssistantMessage.runId,
                  content: storedAssistantMessage.content,
                  status: storedAssistantMessage.status,
                  createdAt: storedTimeToIso(storedAssistantMessage.createdAt),
                }
              : message,
          ),
        );
        onThreadActivity();
      }
    }
    catch (error) {
      clearStreamTimer();

      const message = error instanceof Error ? error.message : String(error);
      if (run) {
        const currentRun = await loadCurrentRun(thread.id, run.id);
        if (!currentRun || !matchesSettledRun(currentRun.status)) {
          await updateRunStatusSafe(run.id, "failed", message);
        }
        if (isCurrentSend()) {
          await refreshRecentRun(thread.id, thread.workspaceId);
        }
      }
      const storedAssistantMessage = run
        ? await appendMessage({
            threadId: thread.id,
            runId: run.id,
            role: "assistant",
            contentType: "markdown",
            content: buildAgentFailureContent(message),
            status: "failed",
          })
        : null;
      if (isCurrentSend()) {
        setMessages(current =>
          current.map(item =>
            item.id === pendingId
              ? {
                  ...item,
                  id: storedAssistantMessage?.id ?? item.id,
                  runId: storedAssistantMessage?.runId ?? item.runId,
                  content: storedAssistantMessage?.content ?? buildAgentFailureContent(message),
                  status: storedAssistantMessage?.status ?? "failed",
                  createdAt: storedAssistantMessage
                    ? storedTimeToIso(storedAssistantMessage.createdAt)
                    : item.createdAt,
                }
              : item,
          ),
        );
        onThreadActivity();
      }
    }
  }, [modelId, modelOptions, onThreadActivity, refreshRecentRun, thread]);

  useEffect(() => {
    const scrollContainer = scrollRef.current;
    if (!scrollContainer)
      return;

    scrollContainer.scrollTo({
      top: scrollContainer.scrollHeight,
      behavior: "auto",
    });
    updateFloatingScrollbar(false);
  }, [messages, updateFloatingScrollbar]);

  useEffect(() => {
    updateFloatingScrollbar(false);

    return () => {
      if (scrollTimerRef.current) {
        clearTimeout(scrollTimerRef.current);
      }
    };
  }, [updateFloatingScrollbar]);

  useEffect(() => {
    return () => {
      sendGenerationRef.current += 1;
      if (streamTimerRef.current !== null) {
        window.clearInterval(streamTimerRef.current);
        streamTimerRef.current = null;
      }
    };
  }, [threadId]);

  useEffect(() => {
    let cancelled = false;

    async function loadThreadMessages() {
      if (!threadId) {
        setMessages([]);
        setLoadingThread(false);
        return;
      }

      setLoadingThread(true);
      try {
        const [storedMessages] = await Promise.all([listMessages(threadId), refreshRecentRun(threadId, thread?.workspaceId)]);
        const agentMessages = storedMessages.map(toAgentMessage);
        const restoredMessages = await restoreMessageActivities(agentMessages);
        if (!cancelled) {
          setMessages(restoredMessages);
        }
      }
      catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        if (!cancelled) {
          setMessages([
            {
              id: "store_error",
              role: "assistant",
              author: "FutureOS",
              content: `FutureOS 消息读取失败：${message}`,
              createdAt: new Date().toISOString(),
            },
          ]);
        }
      }
      finally {
        if (!cancelled) {
          setLoadingThread(false);
        }
      }
    }

    void loadThreadMessages();

    return () => {
      cancelled = true;
    };
  }, [refreshRecentRun, thread?.workspaceId, threadId]);

  useEffect(() => {
    if (!threadId || !recentRun || matchesSettledRun(recentRun.status))
      return;

    const timer = window.setInterval(() => {
      void refreshRecentRun(threadId, thread?.workspaceId);
    }, 1500);

    return () => window.clearInterval(timer);
  }, [recentRun, refreshRecentRun, thread?.workspaceId, threadId]);

  useEffect(() => {
    if (!thread || loadingThread || loadingStore || !pendingPrompt)
      return;
    if (consumedPromptRef.current === pendingPrompt.id)
      return;

    consumedPromptRef.current = pendingPrompt.id;
    onPromptConsumed(pendingPrompt.id);
    void handleSend({ attachments: pendingPrompt.attachments ?? [], content: pendingPrompt.content });
  }, [handleSend, loadingStore, loadingThread, onPromptConsumed, pendingPrompt, thread]);

  return {
    handleScroll,
    handleSend,
    loadingThread,
    messages,
    recentRun,
    recentRunEventCount,
    scrollRef,
    scrollbar,
  };
}

async function updatePendingMessageFromRunEvents(
  runId: string,
  pendingId: string,
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>,
  shouldApply: () => boolean = () => true,
) {
  try {
    const events = await listRunEvents(runId);
    if (!shouldApply())
      return;

    const projection = buildAssistantRunProjection(events);

    if (!projection.content.trim() && projection.activityItems.length === 0)
      return;

    setMessages(current =>
      current.map(message =>
        message.id === pendingId
          ? {
              ...message,
              activityItems: projection.activityItems,
              content: projection.content.trim() ? projection.content : message.content,
            }
          : message,
      ),
    );
  }
  catch {
    // Streaming preview is best-effort. The final assistant message still
    // lands when the command returns.
  }
}

let clientIdCounter = 0;

function clientId(prefix: string) {
  clientIdCounter += 1;
  return `${prefix}_${Date.now()}_${clientIdCounter}`;
}

async function loadCurrentRun(threadId: string, runId: string) {
  try {
    const runs = await listRuns(threadId);
    return runs.find(run => run.id === runId) ?? null;
  }
  catch {
    return null;
  }
}

async function restoreMessageActivities(messages: AgentMessage[]) {
  const activityEntries = await Promise.all(
    messages.map(async (message) => {
      if (message.role !== "assistant" || !message.runId)
        return [message.id, [] as AgentActivityItem[]] as const;

      try {
        const events = await listRunEvents(message.runId);
        return [message.id, buildAssistantRunProjection(events).activityItems] as const;
      }
      catch {
        return [message.id, [] as AgentActivityItem[]] as const;
      }
    }),
  );
  const activitiesByMessageId = new Map(activityEntries);

  return messages.map(message => ({
    ...message,
    activityItems: activitiesByMessageId.get(message.id) ?? message.activityItems,
  }));
}

async function importChatAttachments(thread: StoredThread, attachments: MessageAttachment[]) {
  if (thread.mode !== "chat") {
    return attachments;
  }

  return Promise.all(
    attachments.map(async (attachment) => {
      const artifact = await importAttachmentArtifact({
        path: attachment.path,
        threadId: thread.id,
      });

      return {
        artifactId: artifact.id,
        name: attachment.name,
        path: artifact.path ?? attachment.path,
      };
    }),
  );
}
