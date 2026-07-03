import type { Dispatch, SetStateAction } from "react";
import type { StoredRun, StoredRunEvent, StoredThread } from "../../integrations/storage/threadStore";
import type { AgentMessage, MessageAttachment, MessageSegment } from "./agentThreadTypes";
import type { ComposerSendPayload } from "./Composer";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import i18n from "../../i18n";
import { sendPromptToFutureAgent } from "../../integrations/agent/agentClient";
import {
  abortRun,
  appendMessage,
  createRun,
  deleteTempAttachment,
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
import { buildInlineAttachmentContext, generateImageThumbnail, imageAttachmentPaths, stringifyMessageContent } from "./attachments";
import { buildReferencePrompt } from "./buildReferencePrompt";

interface UseAgentThreadStateInput {
  thread: StoredThread | null;
  loadingStore: boolean;
  modelId: string;
  thinkingLevel: string;
  pendingPrompt: { attachments?: MessageAttachment[]; id: string; content: string } | null;
  onPromptConsumed: (id: string) => void;
  onThreadActivity: () => void;
}

export function useAgentThreadState({
  thread,
  loadingStore,
  modelId,
  thinkingLevel,
  pendingPrompt,
  onPromptConsumed,
  onThreadActivity,
}: UseAgentThreadStateInput) {
  const [messages, setMessages] = useState<AgentMessage[]>([]);
  const [loadingThread, setLoadingThread] = useState(true);
  const [recentRun, setRecentRun] = useState<StoredRun | null>(null);
  const [scrollbar, setScrollbar] = useState({ height: 0, top: 0, visible: false });
  const scrollRef = useRef<HTMLDivElement>(null);
  const scrollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const consumedPromptRef = useRef<string | null>(null);
  const sendGenerationRef = useRef(0);
  // True while a prompt is in flight for this thread. The agent rejects a second
  // concurrent prompt for the same session, so guard every send path with it.
  const sendingRef = useRef(false);
  const streamTimerRef = useRef<number | null>(null);
  const threadId = thread?.id ?? null;

  // The run this thread is currently executing, if any. Runs stream server-side
  // and persist their events regardless of which thread is in the foreground, so
  // this is the anchor for re-attaching a live preview to a conversation that was
  // started, backgrounded, and returned to (or picked up after an app reload) —
  // the in-flight `handleSend` only drives the view while its own thread stays
  // foreground. Guarded on `threadId` because `recentRun` lags a thread switch by
  // one poll, and a stale run from the previous thread must not leak into this
  // one. Null once the run settles.
  const activeRunId
    = recentRun && recentRun.threadId === threadId && !matchesSettledRun(recentRun.status)
      ? recentRun.id
      : null;
  // Epoch-ms anchor for the live elapsed timer of a re-attached run. Stable while
  // the run stays active (derived from persisted run times), so it doesn't churn
  // the resume effect the way the `recentRun` object identity would.
  const activeRunStartedAt = activeRunId ? (recentRun?.startedAt ?? recentRun?.createdAt ?? null) : null;
  const prevActiveRunIdRef = useRef<string | null>(null);

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
    try {
      const runs = await listRuns(threadId);
      if (generation !== recentRunGenRef.current) {
        return;
      }
      const latestRun = runs[0] ?? null;
      setRecentRun(latestRun);
      if (latestRun) {
        upsertFutureReferenceData(workspaceId, "run", latestRun.id, latestRun);
      }
    }
    catch {
      // Run-status refresh is best-effort: a failure here must not blank the
      // thread (it runs alongside listMessages in loadThreadMessages via
      // Promise.all) or abort an in-flight send. Keep the previous recentRun
      // until the next poll. The waiting-approval prompt is rendered separately
      // by AgentThread from `activeApproval`, so no message rewrite is needed.
    }
  }, []);

  // Reload a thread's messages from the store without flipping the full-screen
  // loading state — used to swap a synthetic streaming bubble for the persisted
  // assistant message once a background run settles.
  const reloadMessagesQuiet = useCallback(async (targetThreadId: string) => {
    try {
      const storedMessages = await listMessages(targetThreadId);
      const agentMessages = storedMessages.map(toAgentMessage);
      const restoredMessages = await restoreMessageActivities(agentMessages, targetThreadId);
      setMessages(restoredMessages);
    }
    catch {
      // Best-effort refresh: keep the current messages on failure.
    }
  }, []);

  const handleSend = useCallback(async ({ attachments, content }: ComposerSendPayload) => {
    if (!thread)
      return;
    // One prompt at a time per session: a second send (composer, retry, or
    // continue) while one is running would be rejected by the agent
    // ("already running"), so drop it before anything is optimistically added.
    if (sendingRef.current)
      return;
    sendingRef.current = true;

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
    const runStartAnchorMs = Date.now();
    const optimisticUserMessage: AgentMessage = {
      id: optimisticUserId,
      role: "user",
      author: i18n.t("agent:author.you"),
      authorKey: "author.you",
      content,
      status: "complete",
      createdAt: new Date().toISOString(),
      attachments,
    };
    const assistantMessage: AgentMessage = {
      id: pendingId,
      role: "assistant",
      author: i18n.t("agent:author.researchCopilot"),
      authorKey: "author.researchCopilot",
      content: "",
      status: "streaming",
      createdAt: new Date(runStartAnchorMs).toISOString(),
      activityItems: thinkingActivity(),
      modelId,
      runStartedAt: runStartAnchorMs,
    };
    setMessages(current => [...current, optimisticUserMessage, assistantMessage]);
    onThreadActivity();

    let run: StoredRun | null = null;

    try {
      const importedAttachments = await withImageThumbnails(
        await importChatAttachments(thread, attachments),
      );

      // Extract PDF/text into the model-facing prompt only; keep the visible
      // bubble (messageContent) free of the bulky inlined text. inlineContext is
      // persisted in the stored message so a resend can reuse it.
      const inlineContext = await buildInlineAttachmentContext(importedAttachments);
      const messageContent = importedAttachments.length > 0
        ? stringifyMessageContent(content, importedAttachments, inlineContext)
        : content;
      const promptContent = await buildReferencePrompt(
        thread.workspaceId,
        content,
        inlineContext ? `${content}${inlineContext}` : content,
      );

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
        // Always forward image attachments. The per-model supportsImages flag
        // comes from the provider catalog's modality data, which is unreliable
        // for the Future provider (capable models like Qwen-VL were mis-flagged
        // text-only, so images were silently dropped). A genuinely text-only
        // model returns a clear API error instead — better than the model
        // replying "I don't see an image".
        imageAttachmentPaths(importedAttachments),
        thinkingLevel,
      );
      clearStreamTimer();

      // Chat attachments were copied into the artifact store, so the pasted temp
      // originals are now redundant. (Guarded to our temp dir; user-picked files
      // are rejected and left untouched. Workspace threads keep temps for resend.)
      if (thread.mode === "chat") {
        void Promise.all(attachments.map(item => deleteTempAttachment(item.path).catch(() => {})));
      }

      const currentRun = await loadCurrentRun(thread.id, run.id);
      if (currentRun && matchesSettledRun(currentRun.status)) {
        // The run settled while we awaited the agent. For a user abort
        // (`cancelled`) the agent stopped mid-reply: keep the partial text so it
        // survives a reload instead of vanishing, and finalize the streaming
        // bubble in place. Other settled statuses were finalized by whoever set
        // them — just release the lock (via `finally`).
        if (currentRun.status === "cancelled" && isCurrentSend()) {
          const partial = reply.trim();
          if (partial) {
            const storedAssistantMessage = await appendMessage({
              threadId: thread.id,
              runId: run.id,
              role: "assistant",
              contentType: "markdown",
              content: partial,
              status: "complete",
            });
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
          }
          else {
            // Aborted before any text landed (e.g. still in the thinking phase).
            // There's nothing to persist, but the pending bubble must leave
            // "streaming" — otherwise `isSending` stays true, so the composer is
            // stuck on the stop button and the "generating"/activity indicators
            // linger. Finalize it in place: keep whatever thinking the poll
            // accumulated, clear the still-"running" activity lines, mark stopped.
            patchMessage(setMessages, pendingId, {
              status: "complete",
              activityItems: [],
              stopped: true,
            });
          }
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
        content: reply.trim() || i18n.t("agent:thread.agentDoneNoText"),
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
    finally {
      // Release the in-flight lock — but only if a newer send/thread switch
      // hasn't already taken over (it owns the flag then).
      if (isCurrentSend())
        sendingRef.current = false;
    }
  }, [modelId, onThreadActivity, refreshRecentRun, thinkingLevel, thread]);

  // Interrupt the in-flight run for this thread. Best-effort: the backend stops
  // the agent and marks the run `cancelled`; the in-flight `handleSend` then
  // settles the streaming bubble to the partial reply (see the cancelled branch
  // above), and refreshing `recentRun` clears `activeRunId` so the resume effect
  // reconciles. Safe to call when nothing is running (resolves to a no-op).
  const handleAbort = useCallback(async () => {
    if (!threadId)
      return;
    const runId
      = recentRun && recentRun.threadId === threadId && !matchesSettledRun(recentRun.status)
        ? recentRun.id
        : messages.find(message => message.role === "assistant" && message.status === "streaming")?.runId ?? null;
    if (!runId)
      return;
    try {
      await abortRun({ threadId, runId });
    }
    catch {
      // The run may already have finished; the refresh below still reconciles.
    }
    await refreshRecentRun(threadId, thread?.workspaceId);
    onThreadActivity();
  }, [messages, onThreadActivity, recentRun, refreshRecentRun, thread?.workspaceId, threadId]);

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
      // A switch away abandons any in-flight send for this thread; let the new
      // thread send freely (its run is a different session).
      sendingRef.current = false;
      // Drop the run tracked for the outgoing thread so the incoming thread's
      // settle detection doesn't fire against a stale run id.
      prevActiveRunIdRef.current = null;
      if (streamTimerRef.current !== null) {
        window.clearInterval(streamTimerRef.current);
        streamTimerRef.current = null;
      }
    };
  }, [threadId]);

  // Re-attach a live preview to a run that this view didn't start (or is no
  // longer driving): a conversation started, backgrounded, and returned to while
  // still running, or one picked up after an app reload. While a local send owns
  // the view (`sendingRef`), that path renders the stream itself, so skip.
  //
  // The poll UPSERTS the streaming bubble every tick (not a one-time insert) so it
  // survives a `loadThreadMessages` array-replace that lands mid-stream — the next
  // tick simply re-inserts it. That resilience is what makes a returned-to run
  // reconnect instead of showing an empty bubble the reload silently dropped.
  useEffect(() => {
    if (!threadId || !activeRunId || sendingRef.current)
      return;

    const runId = activeRunId;
    const startedAt = activeRunStartedAt;
    let cancelled = false;
    const tick = () => {
      if (cancelled)
        return;
      void upsertStreamingPreview(runId, startedAt, setMessages, () => !cancelled);
    };
    tick();
    const timer = window.setInterval(tick, 220);

    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [activeRunId, activeRunStartedAt, threadId]);

  // When a run this view was previewing (but did not itself start) settles,
  // reload the thread so the synthetic streaming bubble is replaced by the
  // persisted assistant message.
  useEffect(() => {
    const previous = prevActiveRunIdRef.current;
    prevActiveRunIdRef.current = activeRunId;
    if (previous && !activeRunId && !sendingRef.current && threadId) {
      void reloadMessagesQuiet(threadId);
    }
  }, [activeRunId, reloadMessagesQuiet, threadId]);

  // A remote (phone/web) client can drive this thread's session in the
  // background. This view never started that run, and the recent-run poll below
  // only self-sustains once a run is already in flight — so a fresh remote run
  // on an idle foreground thread would otherwise go unnoticed here (only the
  // sidebar's independent run-status poll would spin). On the backend's
  // remote-activity signal for THIS thread, pull the new run (which arms the
  // live-preview + settle-reload machinery) and reload messages so the phone's
  // user bubble shows immediately. Skip while a local send owns the view.
  useEffect(() => {
    if (!threadId)
      return;
    let cancelled = false;
    const unlisten = listen<string>("remote-activity", (event) => {
      if (cancelled || event.payload !== threadId || sendingRef.current)
        return;
      void refreshRecentRun(threadId, thread?.workspaceId);
      void reloadMessagesQuiet(threadId);
    });
    return () => {
      cancelled = true;
      void unlisten.then(stop => stop());
    };
  }, [refreshRecentRun, reloadMessagesQuiet, thread?.workspaceId, threadId]);

  useEffect(() => {
    // Hand-rolled cancel guard (not useAsyncResource): loads messages, refreshes
    // the recent run, and restores activities into several states at once, which
    // doesn't map onto the primitive's single-resource shape. See gui/CLAUDE.md §4.
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
        const restoredMessages = await restoreMessageActivities(agentMessages, threadId);
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
              author: i18n.t("agent:author.system"),
              authorKey: "author.system",
              content: i18n.t("agent:thread.messagesLoadFailed", { message }),
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
    handleAbort,
    handleScroll,
    handleSend,
    loadingThread,
    messages,
    recentRun,
    scrollRef,
    scrollbar,
  };
}

/** Apply a patch to the single message with `id`, leaving the rest untouched. */
function patchMessage(
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>,
  id: string,
  patch: Partial<AgentMessage> | ((message: AgentMessage) => Partial<AgentMessage>),
) {
  setMessages(current =>
    current.map(message =>
      message.id === id
        ? { ...message, ...(typeof patch === "function" ? patch(message) : patch) }
        : message,
    ),
  );
}

/**
 * Render an in-flight run's live events as a streaming assistant bubble, keyed by
 * a stable `stream_<runId>` id. Unlike {@link updatePendingMessageFromRunEvents}
 * (which patches an existing optimistic bubble), this UPSERTS: it inserts the
 * bubble when missing and updates it in place otherwise, so it re-attaches to a
 * conversation the current view didn't start and survives store reloads that
 * replace the message array. Once a persisted assistant message for the run
 * exists (the run settled and was reloaded), it steps aside and adds nothing.
 */
async function upsertStreamingPreview(
  runId: string,
  runStartedAt: number | null,
  setMessages: Dispatch<SetStateAction<AgentMessage[]>>,
  shouldApply: () => boolean = () => true,
) {
  try {
    const events = await listRunEvents(runId);
    if (!shouldApply())
      return;

    const projection = buildAssistantRunProjection(events);
    const bubbleId = `stream_${runId}`;

    setMessages((current) => {
      // A persisted assistant message already carries this run — the run
      // settled and the thread was reloaded; don't resurrect a synthetic bubble.
      if (current.some(message => message.runId === runId && message.id !== bubbleId))
        return current;

      const content = projection.content.trim();
      const activityItems = projection.activityItems.length > 0 ? projection.activityItems : thinkingActivity();
      const existingIndex = current.findIndex(message => message.id === bubbleId);

      if (existingIndex === -1) {
        const bubble: AgentMessage = {
          id: bubbleId,
          role: "assistant",
          author: i18n.t("agent:author.researchCopilot"),
          authorKey: "author.researchCopilot",
          content,
          status: "streaming",
          createdAt: new Date().toISOString(),
          activityItems,
          segments: projection.segments,
          outputTokens: projection.outputTokens,
          // Feed MessageMeta's live elapsed timer so a re-attached run keeps
          // ticking instead of dropping its duration stat on switch-back.
          runStartedAt: runStartedAt ?? undefined,
          runId,
        };
        return [...current, bubble];
      }

      return current.map((message, index) =>
        index === existingIndex
          ? {
              ...message,
              activityItems,
              segments: projection.segments,
              content: content || message.content,
              outputTokens: projection.outputTokens,
            }
          : message,
      );
    });
  }
  catch {
    // Live preview is best-effort; the final assistant message still lands when
    // the run settles and the thread reloads.
  }
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
              // Live content is derived from the same events as segments, so the
              // two stay consistent — safe to render segments inline immediately.
              segments: projection.segments,
              content: projection.content.trim() ? projection.content : message.content,
              // Tokens accumulate as each LLM call reports usage (lands at the
              // end of each call); shown as the real count, no estimate.
              outputTokens: projection.outputTokens,
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

/**
 * Derive the renderable content + ordered segments from a run's events. Segments
 * are only trusted when the events actually carried the assistant text — the
 * stored reply (from the gRPC return) is otherwise authoritative, so legacy data
 * and text-only-via-gRPC turns fall back to flat content + activity list.
 */
function deriveRenderFields(
  events: StoredRunEvent[],
  fallbackContent: string,
): { content: string; segments?: MessageSegment[]; outputTokens: number } {
  const projection = buildAssistantRunProjection(events);
  if (projection.content.trim()) {
    return {
      content: projection.content,
      segments: projection.segments,
      outputTokens: projection.outputTokens,
    };
  }
  return { content: fallbackContent, outputTokens: projection.outputTokens };
}

async function safeListRunEvents(runId: string): Promise<StoredRunEvent[]> {
  try {
    return await listRunEvents(runId);
  }
  catch {
    return [];
  }
}

/**
 * Exact model run time from the persisted run; falls back to wall-clock since
 * the send anchor while the run is still settling. Null when neither is known.
 */
function runDurationMs(run: StoredRun | null | undefined, fallbackStartMs?: number): number | null {
  if (run?.startedAt && run?.endedAt && run.endedAt >= run.startedAt) {
    return run.endedAt - run.startedAt;
  }
  if (typeof fallbackStartMs === "number") {
    return Math.max(0, Date.now() - fallbackStartMs);
  }
  return null;
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

async function restoreMessageActivities(messages: AgentMessage[], threadId: string) {
  const runs = await listRuns(threadId).catch(() => [] as StoredRun[]);
  const runById = new Map(runs.map(run => [run.id, run] as const));
  const projectionEntries = await Promise.all(
    messages.map(async (message) => {
      if (message.role !== "assistant" || !message.runId)
        return [message.id, null] as const;

      try {
        return [message.id, buildAssistantRunProjection(await listRunEvents(message.runId))] as const;
      }
      catch {
        return [message.id, null] as const;
      }
    }),
  );
  const projectionByMessageId = new Map(projectionEntries);

  return messages.map((message) => {
    const projection = projectionByMessageId.get(message.id);
    const run = message.runId ? runById.get(message.runId) ?? null : null;
    const meta: Partial<AgentMessage> = run
      ? { modelId: run.modelId ?? message.modelId, durationMs: runDurationMs(run), stopped: run.status === "cancelled" }
      : {};
    if (!projection)
      return { ...message, ...meta };
    // Trust event-derived inline ordering only when the events carried the
    // assistant text; otherwise keep the flat activity list (legacy fallback).
    const withSegments = projection.content.trim()
      ? { ...message, ...meta, activityItems: projection.activityItems, segments: projection.segments }
      : { ...message, ...meta, activityItems: projection.activityItems };
    return { ...withSegments, outputTokens: projection.outputTokens };
  });
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
        ...attachment,
        artifactId: artifact.id,
        path: artifact.path ?? attachment.path,
      };
    }),
  );
}

// Generate a cached thumbnail for image attachments so the thread can show a
// small preview without loading the full-size original.
async function withImageThumbnails(attachments: MessageAttachment[]) {
  return Promise.all(
    attachments.map(async (attachment) => {
      if (attachment.kind !== "image") {
        return attachment;
      }
      const thumbnail = await generateImageThumbnail(attachment.path);
      return thumbnail ? { ...attachment, thumbnail } : attachment;
    }),
  );
}
