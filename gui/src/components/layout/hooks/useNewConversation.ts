import type { Dispatch, SetStateAction } from "react";
import type { MessageAttachment } from "../../../features/agent/agentThreadTypes";
import type { NewConversationStart } from "../../../features/agent/NewConversation";
import type { ActivitySection } from "../ActivityRail";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import i18n from "../../../i18n";
import { createThread } from "../../../integrations/storage/threadStore";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";

export interface PendingPrompt {
  attachments?: MessageAttachment[];
  id: string;
  content: string;
  /**
   * The thread this prompt was composed for. The consumer must only send it
   *  when this matches the active thread, so a mid-load thread switch can't
   *  deliver the first message into the wrong conversation.
   */
  targetThreadId: string;
}

interface UseNewConversationParams {
  refreshStore: (nextActiveThreadId?: string) => Promise<void>;
  syncSelection: (modelId: string, thinkingLevel: string) => void;
  setSection: Dispatch<SetStateAction<ActivitySection>>;
  setCenterMode: Dispatch<SetStateAction<"thread" | "new-chat">>;
}

export interface NewConversationFlow {
  pendingPrompt: PendingPrompt | null;
  /** Create the thread, prime the selection, and stage the first message. */
  startNewConversation: (input: NewConversationStart) => Promise<void>;
  /** Drop the staged prompt once the thread has consumed it (by id). */
  consumePendingPrompt: (id: string) => void;
}

/**
 * Owns the new-conversation flow: creating a thread from the composer's first
 * message, then staging that message as a pending prompt for the freshly
 * selected thread to send. AppShell just wires the returned handlers.
 */
export function useNewConversation({
  refreshStore,
  syncSelection,
  setSection,
  setCenterMode,
}: UseNewConversationParams): NewConversationFlow {
  const { t } = useTranslation("layout");
  const [pendingPrompt, setPendingPrompt] = useState<PendingPrompt | null>(null);

  async function startNewConversation(input: NewConversationStart) {
    try {
      const title = deriveThreadTitle(input.content);
      const thread = await createThread({
        mode: input.mode,
        title,
        workspaceId: input.workspace?.id,
        workspaceName: input.workspace?.label,
        workspacePath: input.workspace?.path,
        modelId: input.modelId,
        thinkingLevel: input.thinkingLevel,
      });
      syncSelection(input.modelId, input.thinkingLevel);
      await refreshStore(thread.id);
      setSection(thread.mode === "workspace" ? "workspace" : "chat");
      setCenterMode("thread");
      setPendingPrompt({
        attachments: input.attachments,
        id: newPendingPromptId(thread.id),
        content: input.content,
        targetThreadId: thread.id,
      });
    }
    catch (error) {
      // Surface the failure and rethrow so the composer keeps the draft (it
      // only clears when this promise resolves — see ComposerProps.onSend).
      emitFutureEvent("toast", {
        message: t("appShell.startConversationFailed", { error: errorMessage(error) }),
        tone: "error",
      });
      throw error;
    }
  }

  function consumePendingPrompt(id: string) {
    setPendingPrompt(current => (current?.id === id ? null : current));
  }

  return {
    pendingPrompt,
    startNewConversation,
    consumePendingPrompt,
  };
}

function deriveThreadTitle(content: string) {
  const compact = content.replace(/\s+/g, " ").trim();
  if (!compact)
    return i18n.t("layout:appShell.newChat");
  return compact.length > 28 ? `${compact.slice(0, 28)}...` : compact;
}

let pendingPromptCounter = 0;

function newPendingPromptId(threadId: string) {
  pendingPromptCounter += 1;
  return `${threadId}:${Date.now()}:${pendingPromptCounter}`;
}
