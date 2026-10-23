import type { MessageAttachment } from "@future-os/thread-projection";
import type { Dispatch, SetStateAction } from "react";
import type { NewConversationStart } from "../../../features/agent/NewConversation";
import type { ActivitySection } from "../ActivityRail";
import { plainTitleText } from "@future-os/markdown";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import i18n from "../../../i18n";
import { updateCachedAgentState } from "../../../integrations/agent/agentStateCache";
import { validateImageAttachment } from "../../../integrations/storage/files";
import { createThread, pinThread } from "../../../integrations/storage/threadStore";
import { errorMessage } from "../../../lib/errors";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { truncate } from "../../../lib/objects";
import { writeNewConversationTarget } from "./newConversationTarget";

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
  /**
   * Whether the next thread created should be pinned, asked **after** it exists.
   *
   * The Vibe panel's add tile answers "start a new conversation" by opening this screen —
   * a first message is what creates a thread, so the screen that asks for one is the right
   * place — and its promise is that adding is pinning. The panel therefore arms a flag
   * through `onNewConversation` and this asks it once, at creation, rather than the panel
   * watching the store for a thread it did not create.
   *
   * Absent everywhere else, and asked at most once per creation.
   */
  pinCreated?: () => boolean;
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
  pinCreated,
}: UseNewConversationParams): NewConversationFlow {
  const { t } = useTranslation("layout");
  const [pendingPrompt, setPendingPrompt] = useState<PendingPrompt | null>(null);

  async function startNewConversation(input: NewConversationStart) {
    try {
      // Revalidate image sources immediately before creating the thread. The
      // composer validated them when added, but a file can be moved or replaced
      // while the draft is open. Rejecting here keeps the new-conversation
      // composer and its draft intact instead of creating an empty thread.
      for (const attachment of input.attachments ?? []) {
        if (attachment.kind !== "image")
          continue;
        try {
          await validateImageAttachment(attachment.path);
        }
        catch {
          throw new Error(i18n.t("agent:attachment.imageUnreadable", { name: attachment.name }));
        }
      }
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
      // The panel asked for this one to be pinned (`pinCreated`): do it here, where the
      // thread first exists, so the pin cannot be lost between two screens.
      if (pinCreated?.()) {
        try {
          await pinThread({ pinned: true, threadId: thread.id });
        }
        catch (error) {
          // The conversation exists and is open; a pin that failed is worth saying and not
          // worth failing the send for.
          emitFutureEvent("toast", {
            message: t("appShell.startConversationFailed", { error: errorMessage(error) }),
            tone: "error",
          });
        }
      }
      // The target of the conversation that now exists is the default for the
      // next one (`newConversationTarget.ts`). Recorded after `createThread`, so
      // a creation that fails keeps the previous preference instead of adopting
      // a selection that never became a conversation.
      writeNewConversationTarget({
        mode: input.mode,
        workspaceId: input.mode === "workspace" ? input.workspace?.id ?? null : null,
      });
      syncSelection(input.modelId, input.thinkingLevel);
      // Prime the agent state cache with the user's explicit selections so
      // AgentThread's activeThinkingLevel picks them up instead of falling
      // back to the model's (possibly absent) default before the first
      // prompt creates the agent session.
      updateCachedAgentState(thread.id, {
        model: input.modelId,
        thinkingLevel: input.thinkingLevel,
      });
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
  // A title is text, not markdown: the first message may carry a picked
  // conversation as `[title](futureos://session/<id>)`, and the row must not
  // read as that URL the way the raw source would. The rule is the shared one
  // the Agent applies to the name it mints (`plainTitleText`).
  const compact = plainTitleText(content);
  if (!compact)
    return i18n.t("layout:appShell.newChat");
  return truncate(compact, 28);
}

let pendingPromptCounter = 0;

function newPendingPromptId(threadId: string) {
  pendingPromptCounter += 1;
  return `${threadId}:${Date.now()}:${pendingPromptCounter}`;
}
