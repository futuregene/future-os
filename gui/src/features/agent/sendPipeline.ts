import type { Dispatch, SetStateAction } from "react";
import type { StoredRun, StoredThread } from "../../integrations/storage/threadStore";
import type { AgentMessage } from "./agentThreadTypes";
import type { ComposerSendPayload } from "./Composer";
import i18n from "../../i18n";
import { sendPromptToFutureAgent } from "../../integrations/agent/agentClient";
import { appendMessage, createRun, storedTimeToIso } from "../../integrations/storage/threadStore";
import { errorMessage } from "../../lib/errors";
import { upsertFutureReferenceData } from "../markdown/futureReferenceStore";
import {
  buildAgentFailureContent,
  matchesSettledRun,
  updateRunStatusSafe,
} from "./agentMessageFormatters";
import { buildReferencePrompt } from "./buildReferencePrompt";
import { attachmentInputs, stringifyMessageContent } from "./messageContent";
import { persistImageAttachments } from "./threadAttachments";
import {
  clientId,
  deriveRenderFields,
  loadCurrentRun,
  patchMessage,
  runDurationMs,
  safeListRunEvents,
  updatePendingMessageFromRunEvents,
} from "./threadRunProjection";

export interface SendPipelineDeps {
  thread: StoredThread;
  modelId: string;
  thinkingLevel: string;
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>;
  setRecentRun: (run: StoredRun) => void;
  refreshRecentRun: (threadId: string, workspaceId?: string | null) => Promise<void>;
  onThreadActivity: () => void;
  // True while this send still owns the foreground view. A newer send or a
  // thread switch flips it false; every view-mutating step re-checks it so a
  // superseded send can't write stale state into the current thread.
  isCurrentSend: () => boolean;
}

/**
 * The end-to-end send pipeline: optimistic bubbles → attachment import + inline
 * context → persistence → run creation → live streaming poll → the four settle
 * branches (user abort, interrupted stream, clean completion, thrown error).
 * Kept as a non-React function so it stays unit-testable and out of the hook's
 * effect graph; the in-flight lock (`sendingRef`) and send generation live in
 * the `useSendMessage` wrapper that calls this.
 */
export async function runSendPipeline(
  { thread, modelId, thinkingLevel, setMessages, setRecentRun, refreshRecentRun, onThreadActivity, isCurrentSend }: SendPipelineDeps,
  { attachments, content }: ComposerSendPayload,
): Promise<void> {
  // Validate and prepare images before adding optimistic messages. A failed
  // decode rejects the send, leaves the composer draft intact, and surfaces a
  // concrete error instead of showing an attachment the model never received.
  const importedAttachments = await persistImageAttachments(attachments, thread.id);

  // Timer handle is local to this send, not a shared ref: a prior fix regression
  // let one thread's send clear another thread's stream timer, freezing the live
  // bubble. Ownership stays with the closure.
  let streamTimer: number | null = null;
  const clearStreamTimer = () => {
    if (streamTimer !== null) {
      window.clearInterval(streamTimer);
      streamTimer = null;
    }
  };
  const optimisticUserId = clientId("pending_user");
  const pendingId = clientId("pending");
  const runStartAnchorMs = Date.now();
  const optimisticUserMessage: AgentMessage = {
    id: optimisticUserId,
    role: "user",
    authorKey: "author.you",
    content,
    status: "complete",
    createdAt: new Date().toISOString(),
    attachments: importedAttachments,
  };
  const assistantMessage: AgentMessage = {
    id: pendingId,
    role: "assistant",
    authorKey: "author.researchCopilot",
    content: "",
    status: "streaming",
    createdAt: new Date(runStartAnchorMs).toISOString(),
    // Mid-reasoning from the outset; the footer shows a "thinking…" hint (when
    // show-thinking is off) instead of a top-of-message activity line.
    thinkingActive: true,
    modelId,
    runStartedAt: runStartAnchorMs,
  };
  setMessages(current => [...current, optimisticUserMessage, assistantMessage]);
  onThreadActivity();

  let run: StoredRun | null = null;

  try {
    const messageContent = importedAttachments.length > 0
      ? stringifyMessageContent(content, importedAttachments)
      : content;
    const promptContent = await buildReferencePrompt(thread.workspaceId, content, content);

    if (isCurrentSend()) {
      patchMessage(setMessages, optimisticUserId, { attachments: importedAttachments });
    }

    const storedUserMessage = await appendMessage({
      threadId: thread.id,
      role: "user",
      contentType: importedAttachments.length > 0 ? "mixed" : "markdown",
      content: messageContent,
      status: "complete",
    });

    if (isCurrentSend()) {
      patchMessage(setMessages, optimisticUserId, {
        id: storedUserMessage.id,
        createdAt: storedTimeToIso(storedUserMessage.createdAt),
      });
    }

    run = await createRun({
      threadId: thread.id,
      triggerMessageId: storedUserMessage.id,
      modelId,
    });

    if (isCurrentSend()) {
      setRecentRun(run);
      upsertFutureReferenceData(thread.workspaceId, "run", run.id, run);
      patchMessage(setMessages, pendingId, { runId: run?.id ?? null });
    }

    clearStreamTimer();
    if (isCurrentSend()) {
      streamTimer = window.setInterval(() => {
        if (run && isCurrentSend()) {
          void updatePendingMessageFromRunEvents(run.id, pendingId, setMessages, isCurrentSend);
        }
      }, 220);
    }

    const agentSessionId = thread.agentSessionId?.trim() || null;
    const reply = await sendPromptToFutureAgent(
      promptContent,
      thread.id,
      agentSessionId,
      run.id,
      modelId,
      // All attachments (images + files) travel by original path. The agent
      // decides per attachment: an image goes inline (image_url) when the model
      // accepts image input, otherwise — and for every non-image file — the path
      // is surfaced for the agent's own tools to read.
      attachmentInputs(importedAttachments),
      thinkingLevel,
    );
    clearStreamTimer();

    // No cleanup here: non-image files were never copied, and pasted/downloaded
    // images already had their temp original moved into the thread's origin dir
    // by persistImageAttachments (which deletes the temp copy).

    const currentRun = await loadCurrentRun(thread.id, run.id);
    if (currentRun && matchesSettledRun(currentRun.status)) {
      // The run settled while we awaited the agent. For a user abort
      // (`cancelled`) the agent stopped mid-reply: keep the partial text so it
      // survives a reload instead of vanishing, and finalize the streaming
      // bubble in place. Other settled statuses were finalized by whoever set
      // them — just release the lock (via `finally`).
      if (currentRun.status === "cancelled") {
        const partial = reply.content.trim();
        if (partial) {
          // Persist the partial reply regardless of whether this view still owns
          // the send: aborting and then immediately switching threads flips
          // `isCurrentSend()` false, and gating persistence on it dropped the
          // already-generated text permanently. This local send is the
          // only writer for the run, so there's no double-insert; on return to
          // the thread the reload restores it (stopped, per run.status).
          const storedAssistantMessage = await appendMessage({
            threadId: thread.id,
            runId: run.id,
            role: "assistant",
            contentType: "markdown",
            content: partial,
            status: "complete",
          });
          if (isCurrentSend()) {
            const abortedRender = deriveRenderFields(
              await safeListRunEvents(run.id),
              storedAssistantMessage.content,
            );
            patchMessage(setMessages, pendingId, {
              id: storedAssistantMessage.id,
              runId: storedAssistantMessage.runId,
              content: abortedRender.content,
              segments: abortedRender.segments,
              status: storedAssistantMessage.status,
              createdAt: storedTimeToIso(storedAssistantMessage.createdAt),
              outputTokens: abortedRender.outputTokens,
              stopped: true,
            });
            onThreadActivity();
          }
        }
        else if (isCurrentSend()) {
          // Aborted before any text landed (e.g. still in the thinking phase).
          // There's nothing to persist, but the pending bubble must leave
          // "streaming" — otherwise `isSending` stays true, so the composer is
          // stuck on the stop button and the generating/thinking indicators
          // linger. Finalize it in place (keeping any thinking the poll
          // accumulated), drop the thinking flag, and mark it stopped.
          patchMessage(setMessages, pendingId, {
            status: "complete",
            thinkingActive: false,
            stopped: true,
          });
          onThreadActivity();
        }
      }
      return;
    }
    // The stream closed before the agent signalled a clean end: the text is a
    // truncated prefix, not a finished answer. Persist it (so the partial isn't
    // lost) but finalize the run and bubble as failed rather than silently
    // presenting a cut-off reply as complete.
    if (!reply.complete) {
      const interruptedMessage = i18n.t("agent:thread.responseInterrupted");
      await updateRunStatusSafe(run.id, "failed", interruptedMessage);
      if (isCurrentSend()) {
        await refreshRecentRun(thread.id, thread.workspaceId);
      }
      const storedAssistantMessage = await appendMessage({
        threadId: thread.id,
        runId: run.id,
        role: "assistant",
        contentType: "markdown",
        content: reply.content.trim() || buildAgentFailureContent(interruptedMessage),
        status: "failed",
      });
      const partialRender = deriveRenderFields(
        await safeListRunEvents(run.id),
        storedAssistantMessage.content,
      );
      if (isCurrentSend()) {
        patchMessage(setMessages, pendingId, {
          id: storedAssistantMessage.id,
          runId: storedAssistantMessage.runId,
          content: partialRender.content,
          segments: partialRender.segments,
          status: storedAssistantMessage.status,
          createdAt: storedTimeToIso(storedAssistantMessage.createdAt),
          outputTokens: partialRender.outputTokens,
        });
        onThreadActivity();
      }
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
      content: reply.content.trim() || i18n.t("agent:thread.agentDoneNoText"),
      status: "complete",
    });

    // Streaming polls can lag the final text tail; re-project the now-complete
    // events so the inline segments match the persisted reply exactly.
    const finalRender = deriveRenderFields(
      await safeListRunEvents(run.id),
      storedAssistantMessage.content,
    );
    const settledRun = await loadCurrentRun(thread.id, run.id);
    const durationMs = runDurationMs(settledRun, runStartAnchorMs);

    if (isCurrentSend()) {
      patchMessage(setMessages, pendingId, {
        id: storedAssistantMessage.id,
        runId: storedAssistantMessage.runId,
        content: finalRender.content,
        segments: finalRender.segments,
        status: storedAssistantMessage.status,
        createdAt: storedTimeToIso(storedAssistantMessage.createdAt),
        modelId: settledRun?.modelId ?? modelId,
        durationMs,
        outputTokens: finalRender.outputTokens,
      });
      onThreadActivity();
    }
  }
  catch (error) {
    clearStreamTimer();

    const message = errorMessage(error);
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
      patchMessage(setMessages, pendingId, previous => ({
        id: storedAssistantMessage?.id ?? previous.id,
        runId: storedAssistantMessage?.runId ?? previous.runId,
        content: storedAssistantMessage?.content ?? buildAgentFailureContent(message),
        status: storedAssistantMessage?.status ?? "failed",
        createdAt: storedAssistantMessage
          ? storedTimeToIso(storedAssistantMessage.createdAt)
          : previous.createdAt,
      }));
      onThreadActivity();
    }
  }
}
